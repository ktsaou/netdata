// SPDX-License-Identifier: GPL-3.0-or-later
//! Exact Count prefixes at query boundaries; raw history stays provider/page owned.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    frontend::ast::NodeId,
    matrix_reader::{Geometry, MatrixWindow},
    query_scopes::Grid,
    range_cache::{Folded, Input, Request},
    range_reduce::{FoldKind, FoldOptions, RangeFunction, finish_count, fold},
    storage::{
        Control, CursorAction, OwnedTemporary, QueryProvider, Series, StorageError,
        TemporaryFloatBits, TemporaryFloatBitsRequest, TimeBounds, WalkStart, walk_observed,
    },
    value::SampleValueRef,
    vector_matching::ReplayKey,
};
use std::cell::{Cell, RefCell};

const HEADER_WORDS: u64 = 5;
const GENERATION: usize = 0;
const REVOKED: usize = 1;
const UPPER: usize = 2;
const LOWER: usize = 3;
const TOTAL: usize = 4;
type Header = [u64; HEADER_WORDS as usize];

#[derive(Clone, Copy, PartialEq, Eq)]
struct Scope {
    key: ReplayKey,
    selector: NodeId,
    geometry: Geometry,
    range_ns: i64,
    parameter_bits: [u64; 2],
}
impl Scope {
    fn new(request: Request) -> Self {
        Self {
            key: ReplayKey::new(
                request.options,
                0,
                request.lookback_ns,
                request.window.data_bounds,
            )
            .at(0, 0),
            selector: request.selector,
            geometry: request.window.into(),
            range_ns: request.range_ns,
            parameter_bits: request.parameters.map(f64::to_bits),
        }
    }
}

#[derive(Clone, Copy)]
struct Layout {
    rows: u64,
    steps: u64,
    words: u64,
}
impl Layout {
    fn new(rows: usize, grid: Grid) -> Option<Self> {
        let distance = grid.end_ms.checked_sub(grid.start_ms)?;
        if distance < grid.interval_ms || grid.interval_ms <= 0 {
            return None;
        }
        let steps = u64::try_from(distance / grid.interval_ms)
            .ok()?
            .checked_add(1)?;
        let rows = u64::try_from(rows).ok()?;
        let words = steps
            .checked_mul(2)?
            .checked_add(HEADER_WORDS)?
            .checked_mul(rows)?;
        words.checked_mul(8)?;
        Some(Self { rows, steps, words })
    }
    fn header(self, row: usize) -> Result<u64, StorageError> {
        if row as u64 >= self.rows {
            return Err(StorageError::Invalid("Count prefix row out of bounds"));
        }
        Ok(row as u64 * HEADER_WORDS)
    }
    fn cell(self, row: usize, upper: bool, index: u64) -> Result<u64, StorageError> {
        self.header(row)?;
        if index >= self.steps {
            return Err(StorageError::Invalid("Count prefix clock out of bounds"));
        }
        Ok(self.rows * HEADER_WORDS
            + row as u64 * self.steps * 2
            + u64::from(upper) * self.steps
            + index)
    }
}

struct Store<'q> {
    backing: OwnedTemporary<'q>,
    scope: Scope,
    layout: Layout,
    header_cache: Option<(usize, Header)>,
}
impl<'q> Store<'q> {
    fn create<Q: QueryProvider>(
        query: &'q Q,
        scope: Scope,
        layout: Layout,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<Self>, StorageError> {
        let request = TemporaryFloatBitsRequest {
            value_count: layout.words,
            accounting: account,
        };
        request.extent_bytes()?;
        control.check()?;
        account.provider_call();
        let Some(backing) = query
            .open_temporary_float_bits(request, control)
            .map_err(|error| error.accounted(account))?
        else {
            return Ok(None);
        };
        let mut store = Self {
            backing,
            scope,
            layout,
            header_cache: None,
        };
        // Only initialized headers authorize reads; lanes become readable after a committed frontier.
        let mut offset = 0;
        let headers = layout.rows * HEADER_WORDS;
        while offset < headers {
            let length = (headers - offset).min(1024) as usize;
            store.write(offset, &[0; 1024][..length], control, account)?;
            offset += length as u64;
        }
        Ok(Some(store))
    }
    fn header(
        &mut self,
        row: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Header, StorageError> {
        control.check()?;
        if let Some((cached_row, header)) = self.header_cache
            && cached_row == row
        {
            account.work(HEADER_WORDS);
            return Ok(header);
        }
        let header = self.read(self.layout.header(row)?, control, account)?;
        self.header_cache = Some((row, header));
        Ok(header)
    }
    fn commit_header(
        &mut self,
        row: usize,
        header: Header,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        self.header_cache = None;
        self.write(self.layout.header(row)?, &header, control, account)?;
        self.header_cache = Some((row, header));
        Ok(())
    }
    fn read<const N: usize>(
        &mut self,
        offset: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<[u64; N], StorageError> {
        control.check()?;
        let mut words = [0; N];
        account.provider_call();
        account.temporary_transfer(false, None);
        self.backing
            .read_into(offset, &mut words, control)
            .map_err(|error| error.accounted(account))?;
        account.temporary_transfer(false, Some(N as u64 * 8));
        account.work(N as u64);
        control.check()?;
        Ok(words)
    }
    fn write(
        &mut self,
        offset: u64,
        words: &[u64],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        account.work(words.len() as u64);
        crate::temporary_float_bits::write(&mut self.backing, offset, words, control, account)
    }
    fn count(
        &mut self,
        row: usize,
        index: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<u64, StorageError> {
        let [lower] = self.read(self.layout.cell(row, false, index)?, control, account)?;
        let [upper] = self.read(self.layout.cell(row, true, index)?, control, account)?;
        upper
            .checked_sub(lower)
            .ok_or(StorageError::Invalid("Count prefix order changed"))
    }
}

struct Entry<'q> {
    node: NodeId,
    selector: NodeId,
    grid: Grid,
    layout: Layout,
    attempted: bool,
    store: Option<Store<'q>>,
}
pub(crate) struct Registry<'q> {
    entries: RefCell<AccountedVec<Entry<'q>>>,
    generation: Cell<u64>,
}
impl<'q> Registry<'q> {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            entries: RefCell::new(AccountedVec::new(account, MemoryKind::Workspace)),
            generation: Cell::new(1),
        }
    }
    pub(crate) fn register(&self, node: NodeId, selector: NodeId, rows: usize, grid: Grid) {
        if rows != 0
            && let Some(layout) = Layout::new(rows, grid)
        {
            self.entries.borrow_mut().push(Entry {
                node,
                selector,
                grid,
                layout,
                attempted: false,
                store: None,
            });
        }
    }
    pub(crate) fn start_final(&self) {
        self.generation.set(2);
    }

    pub(crate) fn evaluate<Q: QueryProvider, S: Series>(
        &self,
        query: &'q Q,
        request: Request,
        series: &S,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Folded, StorageError> {
        let entry = {
            let mut entries = self.entries.borrow_mut();
            let mut found = None;
            for (position, entry) in entries.iter_mut().enumerate() {
                control.check()?;
                account.work(1);
                if entry.node == request.owner {
                    if admitted(entry, request) {
                        let initialize = !entry.attempted;
                        entry.attempted = true;
                        found = Some((position, entry.layout, initialize, entry.store.take()));
                    }
                    break;
                }
            }
            found
        };
        let Some((position, layout, initialize, mut store)) = entry else {
            return ordinary(series, request, control, account);
        };
        let result = (|| {
            if initialize {
                match Store::create(query, Scope::new(request), layout, control, account) {
                    Ok(value) => store = value,
                    Err(error) => return after_source(error, series, request, control, account),
                }
            }
            let Some(store) = store
                .as_mut()
                .filter(|store| store.scope == Scope::new(request))
            else {
                return ordinary(series, request, control, account);
            };
            evaluate(
                store,
                self.generation.get(),
                series,
                request,
                control,
                account,
            )
        })();
        // Provider callbacks and recursive evaluators never retain the registry's RefCell borrow.
        self.entries.borrow_mut()[position].store = store;
        result
    }
}

fn admitted(entry: &Entry<'_>, request: Request) -> bool {
    let w = request.window;
    let o = request.options;
    let index = request.position;
    let Some(distance) = index.checked_mul(o.step_ms) else {
        return false;
    };
    request.selector == entry.selector
        && request.function == RangeFunction::Fold(FoldKind::Count)
        && entry.grid
            == Grid {
                start_ms: o.start_ms,
                end_ms: o.end_ms,
                interval_ms: o.step_ms,
            }
        && index >= 0
        && (index as u64) < entry.layout.steps
        && o.start_ms.checked_add(distance) == Some(o.time_ms)
        && w.first_end_ms.checked_add(distance) == Some(w.end_ms)
        && w.step_ms == o.step_ms
        && w.step_ms > 0
        && w.step_ms < w.range_ms
        && w.first_end_ms.checked_sub(w.range_ms).is_some()
        && w.end_ms.checked_sub(w.range_ms).is_some()
        && !w.statistics
        && w.buffer_ms.is_none()
        && w.reduce_delta
        && request.range_ns / 1_000_000 == w.range_ms
        && request.input == Input::fold(bounds(w), None)
        && (entry.attempted || index == 0)
}
fn bounds(window: MatrixWindow) -> Option<TimeBounds> {
    let lower = window.end_ms.checked_sub(window.range_ms)?.checked_add(1)?;
    let start_ms = lower.max(window.data_bounds.start_ms);
    let end_ms = window.end_ms.min(window.data_bounds.end_ms);
    (start_ms <= end_ms).then_some(TimeBounds { start_ms, end_ms })
}
fn ordinary<S: Series>(
    series: &S,
    request: Request,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Folded, StorageError> {
    let Input::Fold {
        bounds,
        anchor_start_ms,
    } = request.input
    else {
        unreachable!("Count input")
    };
    Ok(fold(
        series,
        bounds.map(|[start_ms, end_ms]| TimeBounds { start_ms, end_ms }),
        FoldKind::Count,
        FoldOptions {
            start_timestamps: request.options.start_timestamps,
            matrix: Some(request.window),
            anchor_start_ms,
        },
        control,
        account,
    )?
    .into())
}
fn after_source<S: Series>(
    error: StorageError,
    series: &S,
    request: Request,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Folded, StorageError> {
    if matches!(error, StorageError::Interrupted(_)) {
        return Err(error);
    }
    // A helper failure cannot suppress the ordinary source's validation error.
    crate::matrix_reader::probe::<_, StorageError>(
        series,
        bounds(request.window),
        request.window,
        control,
        account,
        crate::matrix_reader::ignore_sample::<StorageError>,
    )?;
    Err(error)
}

struct Capture<'a, 'q> {
    store: &'a mut Store<'q>,
    row: usize,
    next_lower: u64,
    total: u64,
    window: MatrixWindow,
    pending: Option<StorageError>,
    words: [u64; 32],
    length: usize,
}
impl Capture<'_, '_> {
    fn flush(&mut self, control: &Control, account: &QueryAccounting) -> Result<(), StorageError> {
        if self.length != 0 && self.pending.is_none() {
            let offset =
                self.store
                    .layout
                    .cell(self.row, false, self.next_lower - self.length as u64)?;
            if let Err(error) =
                self.store
                    .write(offset, &self.words[..self.length], control, account)
            {
                if matches!(error, StorageError::Interrupted(_)) {
                    return Err(error);
                }
                self.pending = Some(error);
            }
            self.length = 0;
        }
        control.check()?;
        Ok(())
    }
    fn through(
        &mut self,
        time_ms: i64,
        inclusive: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        while self.next_lower < self.store.layout.steps && self.pending.is_none() {
            control.check()?;
            let boundary = (self.window.first_end_ms as i128) - self.window.range_ms as i128
                + self.next_lower as i128 * self.window.step_ms as i128;
            if boundary > time_ms as i128 || (!inclusive && boundary == time_ms as i128) {
                break;
            }
            self.words[self.length] = self.total;
            self.length += 1;
            self.next_lower += 1;
            if self.length == self.words.len() {
                self.flush(control, account)?;
            }
        }
        control.check()?;
        Ok(())
    }
    fn sample(
        &mut self,
        time_ms: i64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        self.through(time_ms, false, control, account)?;
        self.total = self
            .total
            .checked_add(1)
            .ok_or(StorageError::Invalid("Count prefix overflow"))?;
        self.through(time_ms, true, control, account)
    }
}

fn evaluate<S: Series>(
    store: &mut Store<'_>,
    generation: u64,
    series: &S,
    request: Request,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Folded, StorageError> {
    let row = request.ordinal;
    let index = request.position as u64;
    let mut header: Header = match store.header(row, control, account) {
        Ok(header) => header,
        Err(error) => return after_source(error, series, request, control, account),
    };
    if header[REVOKED] != 0 {
        return ordinary(series, request, control, account);
    }
    if header[GENERATION] != generation {
        header = [generation, 0, 0, 0, 0]
    }
    if index < header[UPPER] {
        return store.count(row, index, control, account).map(Folded::count);
    }
    if index != header[UPPER] || (index != 0 && index >= header[LOWER]) {
        return ordinary(series, request, control, account);
    }
    let lower = if index == 0 {
        0
    } else {
        match store.read::<1>(store.layout.cell(row, false, index)?, control, account) {
            Ok([value]) => value,
            Err(error) => return after_source(error, series, request, control, account),
        }
    };
    let w = request.window;
    let mut capture = Capture {
        store,
        row,
        next_lower: header[LOWER],
        total: header[TOTAL],
        window: w,
        pending: None,
        words: [0; 32],
        length: 0,
    };
    let mut native = false;
    let mut acquisition = None;
    if index == 0 {
        acquisition = Some(crate::matrix_reader::probe::<_, StorageError>(
            series,
            bounds(w),
            w,
            control,
            account,
            |sample| {
                if matches!(sample.value, SampleValueRef::Float(_)) {
                    capture.sample(sample.time_ms, control, account)?;
                }
                Ok(())
            },
        )?);
        native = acquisition
            .as_ref()
            .is_some_and(|value| value.histograms_allocated);
    } else {
        let previous = w
            .end_ms
            .checked_sub(w.step_ms)
            .expect("admitted adjacent clock");
        let _ = walk_observed::<_, StorageError>(
            series,
            w.data_bounds,
            WalkStart::Seek(previous),
            control,
            account,
            |sample, _| {
                control.check()?;
                account.work(1);
                if sample.time_ms > w.end_ms {
                    return Ok(CursorAction::Stop);
                }
                if sample.time_ms > previous {
                    if matches!(sample.value, SampleValueRef::Float(_)) {
                        if !sample.value.is_stale() {
                            capture.sample(sample.time_ms, control, account)?;
                        }
                    } else {
                        native = true;
                    }
                }
                Ok(if sample.time_ms == w.end_ms {
                    CursorAction::Stop
                } else {
                    CursorAction::Next
                })
            },
        )?;
    }
    control.check()?;
    if let Some(error) = capture.pending.take() {
        return Err(error);
    }
    if native {
        capture.flush(control, account)?;
        if let Some(error) = capture.pending.take() {
            return Err(error);
        }
        let result = if let Some(acquisition) = acquisition {
            finish_count(series, bounds(w), w, acquisition, control, account)?.into()
        } else {
            ordinary(series, request, control, account)?
        };
        header[REVOKED] = 1;
        capture.store.commit_header(row, header, control, account)?;
        return Ok(result);
    }
    capture.through(w.end_ms, true, control, account)?;
    capture.flush(control, account)?;
    if let Some(error) = capture.pending.take() {
        return Err(error);
    }
    let units = capture
        .total
        .checked_sub(lower)
        .ok_or(StorageError::Invalid("Count prefix order changed"))?;
    let fresh = capture.total - header[TOTAL];
    let upper = capture.store.layout.cell(row, true, index)?;
    capture
        .store
        .write(upper, &[capture.total], control, account)?;
    let header = [generation, 0, index + 1, capture.next_lower, capture.total];
    capture.store.commit_header(row, header, control, account)?;
    account.samples_read(fresh);
    Ok(Folded::count(units))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        evaluator::EvaluationOptions,
        result::OwnedValue,
        storage::{ProviderResources, SelectionRequest},
        test_storage::GeneratedSeries,
        value::{STALE_NAN_BITS, Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

    #[derive(Default)]
    struct Provider {
        unavailable: bool,
        opens: Cell<usize>,
        closes: Cell<usize>,
        writes: Cell<usize>,
        fail_write: Cell<Option<usize>>,
        fail_read: Cell<bool>,
        fail_open: Cell<bool>,
        cancel_write: Cell<bool>,
    }
    struct Backing<'a> {
        provider: &'a Provider,
        words: AccountedVec<u64>,
        initialized: Vec<bool>,
    }
    impl Drop for Backing<'_> {
        fn drop(&mut self) {
            self.provider.closes.set(self.provider.closes.get() + 1);
        }
    }
    impl TemporaryFloatBits for Backing<'_> {
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn read_into(
            &mut self,
            offset: u64,
            values: &mut [u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            control.check()?;
            if self.provider.fail_read.get() {
                return Err(StorageError::Invalid("Count test read"));
            }
            let start = offset as usize;
            assert!(
                self.initialized[start..start + values.len()]
                    .iter()
                    .all(|value| *value),
                "unwritten prefix read"
            );
            values.copy_from_slice(&self.words[start..start + values.len()]);
            Ok(())
        }
        fn write_at(
            &mut self,
            offset: u64,
            values: &[u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            control.check()?;
            let call = self.provider.writes.get() + 1;
            self.provider.writes.set(call);
            if self.provider.cancel_write.get() {
                control.cancellation.cancel();
            }
            if self.provider.fail_write.get() == Some(call) {
                return Err(StorageError::Invalid("Count test write"));
            }
            let start = offset as usize;
            self.words[start..start + values.len()].copy_from_slice(values);
            self.initialized[start..start + values.len()].fill(true);
            Ok(())
        }
    }
    impl QueryProvider for &Provider {
        type Selection<'a>
            = crate::test_storage::FixtureSelection<'a>
        where
            Self: 'a;
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn select(
            &self,
            _: SelectionRequest<'_>,
            _: &Control,
        ) -> Result<Self::Selection<'_>, StorageError> {
            unreachable!()
        }
        fn open_temporary_float_bits(
            &self,
            request: TemporaryFloatBitsRequest<'_>,
            _: &Control,
        ) -> Result<Option<OwnedTemporary<'_>>, StorageError> {
            self.opens.set(self.opens.get() + 1);
            if self.fail_open.get() {
                return Err(StorageError::Invalid("Count test open"));
            }
            if self.unavailable {
                return Ok(None);
            }
            let mut words = AccountedVec::new(request.accounting, MemoryKind::ProviderHeld);
            words.resize(request.value_count as usize, u64::MAX);
            Ok(Some(OwnedTemporary::new(
                Backing {
                    provider: self,
                    words,
                    initialized: vec![false; request.value_count as usize],
                },
                request.accounting,
            )))
        }
    }
    fn request(index: i64) -> Request {
        let window = MatrixWindow {
            first_end_ms: -10,
            end_ms: -10 + index * 10,
            range_ms: 40,
            buffer_ms: None,
            step_ms: 10,
            data_bounds: TimeBounds {
                start_ms: -100,
                end_ms: 100,
            },
            statistics: false,
            reduce_delta: true,
            resume: None,
        };
        Request {
            owner: NodeId(1),
            selector: NodeId(0),
            ordinal: 0,
            options: EvaluationOptions {
                time_ms: window.end_ms,
                start_ms: -10,
                end_ms: 90,
                step_ms: 10,
                lookback_ms: 300_000,
                start_timestamps: false,
                delayed_name_removal: false,
            },
            lookback_ns: 300_000_000_000,
            position: index,
            function: RangeFunction::Fold(FoldKind::Count),
            parameters: [0.0; 2],
            range_ns: 40_000_000,
            input: Input::fold(bounds(window), None),
            window,
        }
    }
    fn registry(account: &QueryAccounting) -> Registry<'_> {
        let registry = Registry::new(account);
        registry.register(
            NodeId(1),
            NodeId(0),
            2,
            Grid {
                start_ms: -10,
                end_ms: 90,
                interval_ms: 10,
            },
        );
        registry
    }
    fn sample(index: usize) -> Sample {
        Sample {
            time_ms: index as i64 - 100,
            start_time_ms: 0,
            value: SampleValue::Float(match index % 13 {
                0 => f64::from_bits(STALE_NAN_BITS),
                1 => f64::NAN,
                2 => f64::INFINITY,
                3 => -0.0,
                _ => index as f64,
            }),
        }
    }
    fn sparse(index: usize) -> Sample {
        let mut value = sample(index);
        value.time_ms = index as i64 * 17 - 100;
        value
    }
    fn number(result: &Folded) -> Option<u64> {
        result.value.as_ref().map(|value| match value {
            OwnedValue::Float(value) => value.to_bits(),
            _ => panic!("Count result kind"),
        })
    }
    #[test]
    fn single_clock_ranges_do_not_open_backing() {
        for page in [1, 17, 1024] {
            for distance in [0, 1, 9] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let provider = Provider::default();
                provider.fail_open.set(true);
                let query = &provider;
                let mut request = request(0);
                request.options.end_ms = request.options.start_ms + distance;
                let registry = Registry::new(&account);
                registry.register(
                    request.owner,
                    request.selector,
                    1,
                    Grid {
                        start_ms: request.options.start_ms,
                        end_ms: request.options.end_ms,
                        interval_ms: request.options.step_ms,
                    },
                );
                let series = GeneratedSeries::new(201, page, sample);
                let expected = ordinary(&series, request, &control, &account).unwrap();
                for phase in 0..2 {
                    if phase != 0 {
                        registry.start_final();
                    }
                    let actual = registry
                        .evaluate(&query, request, &series, &control, &account)
                        .unwrap();
                    assert_eq!(number(&actual), number(&expected));
                    assert_eq!(provider.opens.get(), 0);
                }
                assert!(registry.entries.borrow().is_empty());
                drop(registry);
                account.snapshot().assert_memory_released();
            }
        }
    }
    #[test]
    fn boundaries_pages_completed_replay_and_fresh_final_generation() {
        for page in [1, 17, 1024] {
            for (samples, make) in [
                (201, sample as fn(usize) -> Sample),
                (12, sparse),
                (0, sample),
            ] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let provider = Provider::default();
                let query = &provider;
                let registry = registry(&account);
                let series = GeneratedSeries::new(samples, page, make);
                for phase in 0..2 {
                    if phase != 0 {
                        registry.start_final();
                    }
                    let before = series.counters.opened.load(Ordering::Relaxed);
                    for index in 0..11 {
                        let expected =
                            ordinary(&series, request(index), &control, &account).unwrap();
                        let actual = registry
                            .evaluate(&query, request(index), &series, &control, &account)
                            .unwrap();
                        assert_eq!(
                            number(&actual),
                            number(&expected),
                            "page={page},clock={index}"
                        );
                        assert_eq!(actual.sample_units(), expected.sample_units());
                    }
                    assert!(series.counters.opened.load(Ordering::Relaxed) > before);
                    let before = series.counters.opened.load(Ordering::Relaxed);
                    let read = account.snapshot().statistics.samples.samples_read;
                    for index in (0..11).rev() {
                        let actual = registry
                            .evaluate(&query, request(index), &series, &control, &account)
                            .unwrap();
                        assert_eq!(
                            number(&actual),
                            (actual.sample_units() != 0)
                                .then_some((actual.sample_units() as f64).to_bits())
                        );
                    }
                    assert_eq!(series.counters.opened.load(Ordering::Relaxed), before);
                    assert_eq!(account.snapshot().statistics.samples.samples_read, read);
                }
                assert_eq!(provider.opens.get(), 1);
                drop(registry);
                assert_eq!(provider.closes.get(), 1);
                assert_eq!(series.counters.active.load(Ordering::Relaxed), 0);
                account.snapshot().assert_memory_released();
            }
        }
    }
    #[test]
    fn absent_capability_is_remembered_and_scope_mismatches_replay() {
        for unavailable in [true, false] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let provider = Provider {
                unavailable,
                ..Provider::default()
            };
            let query = &provider;
            let registry = registry(&account);
            let series = GeneratedSeries::new(201, 17, sample);
            for index in 0..11 {
                let actual = registry
                    .evaluate(&query, request(index), &series, &control, &account)
                    .unwrap();
                let expected = ordinary(&series, request(index), &control, &account).unwrap();
                assert_eq!(number(&actual), number(&expected));
            }
            for mismatch in 0..8 {
                let mut request = request(1);
                match mismatch {
                    0 => request.lookback_ns += 1,
                    1 => request.range_ns += 1,
                    2 => request.options.delayed_name_removal = true,
                    3 => request.window.data_bounds.start_ms += 1,
                    4 => request.parameters[0] = -0.0,
                    5 => request.window.first_end_ms -= 10,
                    6 => request.options.start_timestamps = true,
                    _ => request.options.lookback_ms += 1,
                }
                let before = series.counters.opened.load(Ordering::Relaxed);
                let actual = registry
                    .evaluate(&query, request, &series, &control, &account)
                    .unwrap();
                assert!(series.counters.opened.load(Ordering::Relaxed) > before);
                let expected = ordinary(&series, request, &control, &account).unwrap();
                assert_eq!(number(&actual), number(&expected));
            }
            assert_eq!(provider.opens.get(), 1);
            drop(registry);
            account.snapshot().assert_memory_released();
        }
    }
    #[test]
    fn backing_failure_waits_for_source_and_never_commits_a_partial_frontier() {
        for fail in 0..3 {
            for source_fails in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let provider = Provider::default();
                if fail == 0 {
                    provider.fail_open.set(true);
                }
                if fail == 1 {
                    provider.fail_write.set(Some(1));
                }
                if fail == 2 {
                    provider.fail_write.set(Some(3));
                }
                let query = &provider;
                let registry = registry(&account);
                let mut series = GeneratedSeries::new(201, 1, sample);
                if source_fails {
                    series.fail_page = Some(30);
                }
                let result = registry.evaluate(&query, request(0), &series, &control, &account);
                let error = result.err().expect("actual provider failure");
                if source_fails {
                    assert!(matches!(error, StorageError::Provider(_)), "{error:?}");
                } else {
                    assert!(matches!(error, StorageError::Invalid(_)), "{error:?}");
                }
                assert!(series.counters.opened.load(Ordering::Relaxed) > 0);
                drop(error);
                if fail == 2 {
                    let mut entries = registry.entries.borrow_mut();
                    let store = entries[0].store.as_mut().unwrap();
                    let header: Header = store.read(0, &control, &account).unwrap();
                    assert_eq!(header, [0; 5]);
                }
                drop(registry);
                assert_eq!(series.counters.active.load(Ordering::Relaxed), 0);
                account.snapshot().assert_memory_released();
            }
        }
    }
    fn native_at(index: usize, target: usize, stale: bool) -> Sample {
        let mut value = sample(index);
        if index == target {
            value.value = SampleValue::FloatHistogram(crate::histogram::FloatHistogram {
                schema: 0,
                count: 1.0,
                sum: if stale {
                    f64::from_bits(STALE_NAN_BITS)
                } else {
                    1.0
                },
                ..Default::default()
            });
        }
        value
    }
    fn first_native(index: usize) -> Sample {
        native_at(index, 60, false)
    }
    fn tail_native(index: usize) -> Sample {
        native_at(index, 95, false)
    }
    fn tail_stale_native(index: usize) -> Sample {
        native_at(index, 95, true)
    }
    #[test]
    fn native_and_stale_native_revoke_prefixes_including_the_final_phase() {
        for page in [1, 17, 1024] {
            for make in [
                first_native as fn(usize) -> Sample,
                tail_native,
                tail_stale_native,
            ] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let provider = Provider::default();
                let query = &provider;
                let registry = registry(&account);
                let mut series = GeneratedSeries::new(201, page, make);
                for index in 0..3 {
                    let actual = registry
                        .evaluate(&query, request(index), &series, &control, &account)
                        .unwrap();
                    let expected = ordinary(&series, request(index), &control, &account).unwrap();
                    assert_eq!(number(&actual), number(&expected));
                    assert_eq!(actual.sample_units(), expected.sample_units());
                }
                // Completed Float cells cannot authorize replay after any native allocation.
                for index in 0..3 {
                    let before = series.counters.opened.load(Ordering::Relaxed);
                    drop(
                        registry
                            .evaluate(&query, request(index), &series, &control, &account)
                            .unwrap(),
                    );
                    assert!(series.counters.opened.load(Ordering::Relaxed) > before);
                }
                registry.start_final();
                series.sample = sample;
                for index in 0..3 {
                    let before = series.counters.opened.load(Ordering::Relaxed);
                    let actual = registry
                        .evaluate(&query, request(index), &series, &control, &account)
                        .unwrap();
                    assert!(series.counters.opened.load(Ordering::Relaxed) > before);
                    let expected = ordinary(&series, request(index), &control, &account).unwrap();
                    assert_eq!(number(&actual), number(&expected));
                }
                drop(registry);
                account.snapshot().assert_memory_released();
            }
        }
    }
    #[test]
    fn prefix_credits_only_new_float_consumption_and_validates_fresh_final() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let provider = Provider::default();
        let query = &provider;
        let registry = registry(&account);
        let mut series = GeneratedSeries::new(201, 17, sample);
        for index in 0..11 {
            let before = account.snapshot().statistics.samples.samples_read;
            let result = registry
                .evaluate(&query, request(index), &series, &control, &account)
                .unwrap();
            let lower = if index == 0 { -50 } else { -20 + index * 10 };
            let upper = -10 + index * 10;
            let expected = (0..201)
                .map(sample)
                .filter(|sample| {
                    sample.time_ms > lower
                        && sample.time_ms <= upper
                        && !sample.as_ref().value.is_stale()
                })
                .count() as u64;
            assert_eq!(
                account.snapshot().statistics.samples.samples_read - before,
                expected
            );
            assert!(result.sample_units() >= expected);
        }
        registry.start_final();
        series.sample = first_native;
        let actual = registry
            .evaluate(&query, request(0), &series, &control, &account)
            .unwrap();
        let expected = ordinary(&series, request(0), &control, &account).unwrap();
        assert_eq!(number(&actual), number(&expected));
        assert_eq!(actual.sample_units(), expected.sample_units());
        drop(actual);
        drop(expected);
        drop(registry);
        account.snapshot().assert_memory_released();
    }
    #[test]
    fn cancellation_during_backing_and_read_failures_release_every_owner() {
        for cancel in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let provider = Provider::default();
            let query = &provider;
            let registry = registry(&account);
            let mut series = GeneratedSeries::new(201, 1, sample);
            drop(
                registry
                    .evaluate(&query, request(0), &series, &control, &account)
                    .unwrap(),
            );
            if cancel {
                provider.cancel_write.set(true);
            } else {
                provider.fail_read.set(true);
                series.fail_page = Some(3);
            }
            let error = registry
                .evaluate(&query, request(1), &series, &control, &account)
                .err()
                .unwrap();
            if cancel {
                assert!(matches!(error, StorageError::Interrupted(_)));
            } else {
                assert!(matches!(error, StorageError::Provider(_)), "{error:?}");
            }
            drop(error);
            drop(registry);
            assert_eq!(provider.opens.get(), provider.closes.get());
            assert_eq!(series.counters.active.load(Ordering::Relaxed), 0);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn buffered_backing_errors_precede_native_fallback_but_follow_source_validation() {
        for make in [
            first_native as fn(usize) -> Sample,
            tail_native,
            tail_stale_native,
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let provider = Provider::default();
            let query = &provider;
            let registry = registry(&account);
            let series = GeneratedSeries::new(201, 1, make);
            let first = matches!(make(60).value, SampleValue::FloatHistogram(_));
            let index = if first {
                0
            } else {
                drop(
                    registry
                        .evaluate(&query, request(0), &series, &control, &account)
                        .unwrap(),
                );
                1
            };
            provider
                .fail_write
                .set(Some(provider.writes.get() + if first { 2 } else { 1 }));
            let before = series.counters.opened.load(Ordering::Relaxed);
            let error = registry
                .evaluate(&query, request(index), &series, &control, &account)
                .err()
                .unwrap();
            assert_eq!(error, StorageError::Invalid("Count test write"));
            assert_eq!(
                series.counters.opened.load(Ordering::Relaxed),
                before + 1,
                "native consumption must not hide a staged write failure"
            );
            drop(registry);
            account.snapshot().assert_memory_released();
        }
        for source_fails in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let provider = Provider::default();
            provider.fail_write.set(Some(2));
            let query = &provider;
            let registry = Registry::new(&account);
            let mut request = request(0);
            request.options.start_ms = 0;
            request.options.time_ms = 0;
            request.options.end_ms = 400;
            request.options.step_ms = 1;
            request.window.first_end_ms = 0;
            request.window.end_ms = 0;
            request.window.step_ms = 1;
            request.input = Input::fold(bounds(request.window), None);
            registry.register(
                request.owner,
                request.selector,
                1,
                Grid {
                    start_ms: 0,
                    end_ms: 400,
                    interval_ms: 1,
                },
            );
            let mut series = GeneratedSeries::new(201, 1, sample);
            if source_fails {
                series.fail_page = Some(40);
            }
            let error = registry
                .evaluate(&query, request, &series, &control, &account)
                .err()
                .unwrap();
            assert_eq!(
                provider.writes.get(),
                2,
                "actual full-buffer write failed during the source probe"
            );
            if source_fails {
                assert!(matches!(error, StorageError::Provider(_)), "{error:?}");
            } else {
                assert_eq!(error, StorageError::Invalid("Count test write"));
            }
            drop(error);
            drop(registry);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn layout_overflow_and_exact_integer_subtraction() {
        assert!(
            Layout::new(
                usize::MAX,
                Grid {
                    start_ms: 0,
                    end_ms: i64::MAX,
                    interval_ms: 1
                }
            )
            .is_none()
        );
        assert!(
            Layout::new(
                1,
                Grid {
                    start_ms: i64::MIN,
                    end_ms: i64::MAX,
                    interval_ms: 1
                }
            )
            .is_none()
        );
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let provider = Provider::default();
        let query = &provider;
        let layout = Layout::new(
            2,
            Grid {
                start_ms: 0,
                end_ms: 10,
                interval_ms: 1,
            },
        )
        .unwrap();
        let mut store = Store::create(&query, Scope::new(request(0)), layout, &control, &account)
            .unwrap()
            .unwrap();
        assert!(layout.header(2).is_err());
        assert!(layout.cell(0, false, 11).is_err());
        for row in 0..2 {
            store
                .write(
                    layout.cell(row, false, 0).unwrap(),
                    &[(1 << 53) + 1],
                    &control,
                    &account,
                )
                .unwrap();
            store
                .write(
                    layout.cell(row, true, 0).unwrap(),
                    &[(1 << 53) + 2],
                    &control,
                    &account,
                )
                .unwrap();
            assert_eq!(store.count(row, 0, &control, &account).unwrap(), 1);
        }
        drop(store);
        account.snapshot().assert_memory_released();
    }
}
