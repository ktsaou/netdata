// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    frontend::ast::Matcher,
    storage::{
        Control, LabelRef, PageSamples, ProviderResources, SampleCursor, SamplePage, Series,
        StorageError, TimeBounds, matches_checked,
    },
    value::{Sample, SampleValue},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[cfg(test)]
pub(crate) const FULL_LABEL_HASH_COLLISION_VALUES: [&[u8]; 2] = [
    b"Aaaaa\x00\x00\x00\x00\x00\x00\x00\x00",
    b"Bbbbb\x1d\x3c\xa5\x95\x96\x2f\x43\xdd",
];

#[cfg(test)]
pub(crate) mod group_preparation {
    use super::*;
    use crate::{
        evaluator_roster::{Descriptor, HeadSummary},
        frontend::{
            ast::{ParsedExpression, ParserOptions},
            parser,
            regex::Selector,
        },
        label_hash,
        result::OwnedLabels,
    };

    pub(crate) type RawLabels = Vec<(Vec<u8>, Vec<u8>)>;
    pub(crate) type RawCase = (
        AccountedVec<Descriptor>,
        Vec<(u64, RawLabels)>,
        Vec<usize>,
        String,
    );

    #[derive(Debug, PartialEq)]
    pub(crate) struct Identity {
        labels: RawLabels,
        buffers: Vec<(usize, usize)>,
        ordinal: usize,
        hash: u64,
        drop_name: bool,
        first_order: usize,
        sources: std::ops::Range<usize>,
        kinds: u8,
    }

    pub(crate) fn raw(labels: &OwnedLabels) -> RawLabels {
        labels
            .0
            .iter()
            .map(|label| (label.name.to_vec(), label.value.to_vec()))
            .collect()
    }

    pub(crate) fn identities(rows: &[Descriptor]) -> Vec<Identity> {
        rows.iter()
            .map(|row| Identity {
                labels: raw(&row.labels),
                buffers: row
                    .labels
                    .0
                    .iter()
                    .map(|label| (label.name.as_ptr() as usize, label.value.as_ptr() as usize))
                    .collect(),
                ordinal: row.ordinal,
                hash: row.hash,
                drop_name: row.drop_name,
                first_order: row.first_order,
                sources: row.sources.clone(),
                kinds: row.kinds,
            })
            .collect()
    }

    pub(crate) fn expression(
        source: &str,
        account: &QueryAccounting,
        control: &Control,
    ) -> ParsedExpression {
        let context = AllocationContext::new(account, MemoryKind::Compiled);
        let parsed = parser::parse_in(
            source.as_bytes(),
            ParserOptions {
                experimental_functions: true,
                ..ParserOptions::default()
            },
            |pattern| Selector::compile_checked(pattern, Some(&context), Some(control)),
            Some(account),
            Some(control),
        )
        .unwrap();
        assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
        let mut owned = AccountedVec::new(account, MemoryKind::Compiled);
        owned.extend_from_slice(source.as_bytes());
        ParsedExpression {
            source: owned,
            arena: parsed.arena,
            root: parsed.root.unwrap(),
        }
    }

    pub(crate) fn rows(
        raw: Vec<RawLabels>,
        account: &QueryAccounting,
        control: &Control,
    ) -> AccountedVec<Descriptor> {
        let count = raw.len();
        let mut result = AccountedVec::new(account, MemoryKind::Workspace);
        for (position, fields) in raw.into_iter().enumerate() {
            let mut labels = OwnedLabels::new(account, MemoryKind::Workspace);
            for (name, value) in fields {
                labels.push(&name, &value, account, MemoryKind::Workspace);
            }
            let hash = label_hash::full(&labels, control, account).unwrap();
            result.push(Descriptor {
                labels,
                ordinal: 100 + 3 * (count - position),
                hash,
                drop_name: position % 2 == 1,
                first_order: position + 11,
                sources: 2 * position..2 * position + 2,
                kinds: (position % 3 + 1) as u8,
                heads: HeadSummary::default(),
            });
        }
        result
    }

    pub(crate) fn population(
        count: usize,
        width: usize,
        fields: usize,
        unique: bool,
        account: &QueryAccounting,
        control: &Control,
    ) -> AccountedVec<Descriptor> {
        assert!(fields >= 4);
        rows(
            (0..count)
                .rev()
                .map(|index| {
                    let mut labels = vec![
                        (b"__name__".to_vec(), b"m".to_vec()),
                        (b"id".to_vec(), index.to_string().into_bytes()),
                        (
                            b"job".to_vec(),
                            format!(
                                "{}:{:06}",
                                "x".repeat(width),
                                if unique { index } else { 0 }
                            )
                            .into_bytes(),
                        ),
                        (b"pad".to_vec(), vec![b'p'; 16]),
                    ];
                    labels.extend(
                        (4..fields)
                            .map(|field| (format!("tag{field:03}").into_bytes(), vec![b'z'; 64])),
                    );
                    labels
                })
                .collect(),
            account,
            control,
        )
    }

    pub(crate) fn query(operator: &str, fields: usize, without: bool) -> String {
        let grouping = if without {
            format!(
                "without(id,pad{})",
                (4..fields)
                    .map(|field| format!(",tag{field:03}"))
                    .collect::<String>()
            )
        } else {
            "by(job)".into()
        };
        format!(
            "{operator} {grouping}({},m)",
            if operator == "quantile" { "0.5" } else { "2" }
        )
    }

    pub(crate) fn raw_case(
        case: usize,
        reverse: bool,
        account: &QueryAccounting,
        control: &Control,
    ) -> RawCase {
        let fields: &[&[(&[u8], &[u8])]] = &[
            &[(b"a", b"b\xffc\xffd")],
            &[(b"a", b"b"), (b"c", b"d")],
            &[(b"a", b"b\xffc\xffd")],
            &[(b"c", b"d"), (b"a", b"b")],
            &[(b"a", b"")],
            &[],
        ];
        let (grouping, selected, without): (&str, &[&[u8]], bool) = match case {
            0 => ("", &[], false),
            1 => ("by()", &[], false),
            2 => ("by(missing)", &[b"missing"], false),
            3 => ("by(a,c,a)", &[b"a", b"c"], false),
            4 => (
                "by(empty,__type__,__unit__,__name__)",
                &[b"empty", b"__type__", b"__unit__", b"__name__"],
                false,
            ),
            _ => (
                "without(id,empty,__type__,__unit__)",
                &[b"id", b"empty", b"__type__", b"__unit__"],
                true,
            ),
        };
        let mut input = Vec::new();
        for position in 0..6 {
            let index = if reverse { 5 - position } else { position };
            let mut labels = vec![(b"__name__".to_vec(), b"m".to_vec())];
            labels.extend(
                fields[index]
                    .iter()
                    .map(|(name, value)| (name.to_vec(), value.to_vec())),
            );
            labels.extend([
                (b"empty".to_vec(), vec![]),
                (b"__type__".to_vec(), b"counter".to_vec()),
                (b"__unit__".to_vec(), b"bytes".to_vec()),
                (b"id".to_vec(), index.to_string().into_bytes()),
            ]);
            input.push(labels);
        }
        let mut expected = Vec::<(u64, RawLabels)>::new();
        let mut bindings = Vec::new();
        for labels in &input {
            let kept: RawLabels = labels
                .iter()
                .filter(|(name, _)| {
                    if without {
                        name != b"__name__" && !selected.contains(&name.as_slice())
                    } else {
                        selected.contains(&name.as_slice())
                    }
                })
                .cloned()
                .collect();
            let hash = if case <= 1 {
                0
            } else {
                let mut bytes = Vec::new();
                for (name, value) in &kept {
                    bytes.extend_from_slice(name);
                    bytes.push(255);
                    bytes.extend_from_slice(value);
                    bytes.push(255);
                }
                label_hash::bytes(&bytes, control, account).unwrap()
            };
            let position = expected
                .iter()
                .position(|(prior, _)| *prior == hash)
                .unwrap_or_else(|| {
                    expected.push((hash, kept));
                    expected.len() - 1
                });
            bindings.push(position);
        }
        if case == 2 {
            assert_eq!(expected[0].0, 0xef46_db37_51d8_e999);
        }
        if matches!(case, 3 | 5) {
            assert_eq!(expected.len(), 4);
        }
        (
            rows(input, account, control),
            expected,
            bindings,
            grouping.into(),
        )
    }
}

struct WorkInterruption {
    query: usize,
    remaining: u64,
    cancellation: crate::storage::Cancellation,
}
thread_local! {
    static WORK_INTERRUPTION: std::cell::RefCell<Option<WorkInterruption>> = const { std::cell::RefCell::new(None) };
}
pub(crate) fn work_observed(account: &QueryAccounting, amount: u64) {
    WORK_INTERRUPTION.with_borrow_mut(|entry| {
        if let Some(entry) = entry
            && entry.query == account.test_identity()
        {
            entry.remaining = entry.remaining.saturating_sub(amount);
            if entry.remaining == 0 {
                entry.cancellation.cancel();
            }
        }
    });
}

pub(crate) fn interrupt_during<R>(
    account: &QueryAccounting,
    control: &Control,
    operation: impl FnOnce() -> R,
) -> R {
    interrupt_after_work(account, control, 8192, operation)
}

pub(crate) fn interrupt_after_work<R>(
    account: &QueryAccounting,
    control: &Control,
    remaining: u64,
    operation: impl FnOnce() -> R,
) -> R {
    struct Restore(Option<WorkInterruption>);
    impl Drop for Restore {
        fn drop(&mut self) {
            WORK_INTERRUPTION.with_borrow_mut(|entry| *entry = self.0.take());
        }
    }
    // Inject at actual observed work, independently of worker scheduling.
    let previous = WORK_INTERRUPTION.with_borrow_mut(|entry| {
        entry.replace(WorkInterruption {
            query: account.test_identity(),
            remaining,
            cancellation: control.cancellation.clone(),
        })
    });
    let _restore = Restore(previous);
    operation()
}

#[cfg(test)]
struct LabelObservation {
    cancellation: crate::storage::Cancellation,
    cancel_at: Option<usize>,
    reads: usize,
}
#[cfg(test)]
thread_local! {
    static LABEL_OBSERVATION: std::cell::RefCell<Option<LabelObservation>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(crate) fn observe_label_reads<R>(
    control: &Control,
    cancel_at: Option<usize>,
    operation: impl FnOnce() -> R,
) -> (R, usize) {
    struct Restore(Option<LabelObservation>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LABEL_OBSERVATION.with_borrow_mut(|entry| *entry = self.0.take());
        }
    }
    let previous = LABEL_OBSERVATION.with_borrow_mut(|entry| {
        entry.replace(LabelObservation {
            cancellation: control.cancellation.clone(),
            cancel_at,
            reads: 0,
        })
    });
    let _restore = Restore(previous);
    let value = operation();
    let reads = LABEL_OBSERVATION.with_borrow(|entry| entry.as_ref().unwrap().reads);
    (value, reads)
}
#[cfg(test)]
fn label_observed() {
    LABEL_OBSERVATION.with_borrow_mut(|entry| {
        if let Some(entry) = entry {
            entry.reads += 1;
            if entry.cancel_at == Some(entry.reads) {
                entry.cancellation.cancel();
            }
        }
    });
}

#[derive(Default, Debug)]
pub(crate) struct CursorCounts {
    pub opened: AtomicUsize,
    pub closed: AtomicUsize,
    pub active: AtomicUsize,
    pub peak: AtomicUsize,
    pub next_calls: AtomicUsize,
    pub samples_returned: AtomicUsize,
}

pub(crate) struct GeneratedSeries {
    pub count: usize,
    pub page_size: usize,
    pub sample: fn(usize) -> Sample,
    pub seek_index: Option<fn(i64) -> usize>,
    pub counters: Arc<CursorCounts>,
    pub fail_page: Option<usize>,
    #[cfg(test)]
    pub fail_acquisition: Option<usize>,
}

impl GeneratedSeries {
    pub(crate) fn new(count: usize, page_size: usize, sample: fn(usize) -> Sample) -> Self {
        Self {
            count,
            page_size,
            sample,
            seek_index: None,
            counters: Arc::new(CursorCounts::default()),
            fail_page: None,
            #[cfg(test)]
            fail_acquisition: None,
        }
    }
}

pub(crate) struct GeneratedCursor<'a> {
    source: &'a GeneratedSeries,
    bounds: TimeBounds,
    index: usize,
    pages: usize,
    values: Vec<Sample>,
    #[cfg(test)]
    acquisition: usize,
}

impl Series for GeneratedSeries {
    type Cursor<'a> = GeneratedCursor<'a>;
    fn label_count(&self) -> usize {
        0
    }
    fn label(&self, _: usize) -> LabelRef<'_> {
        panic!("unlabelled generated series")
    }
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn cursor(
        &self,
        bounds: TimeBounds,
        control: &Control,
    ) -> Result<Self::Cursor<'_>, StorageError> {
        control.check()?;
        let _acquisition = self.counters.opened.fetch_add(1, Ordering::Relaxed);
        let active = self.counters.active.fetch_add(1, Ordering::Relaxed) + 1;
        self.counters.peak.fetch_max(active, Ordering::Relaxed);
        Ok(GeneratedCursor {
            source: self,
            bounds,
            index: 0,
            pages: 0,
            values: Vec::with_capacity(self.page_size),
            #[cfg(test)]
            acquisition: _acquisition + 1,
        })
    }
}

impl SampleCursor for GeneratedCursor<'_> {
    fn resources(&self) -> ProviderResources {
        let mut bytes = self.values.capacity() * size_of::<Sample>();
        for (index, sample) in self.values.iter().enumerate() {
            let (buffers, bounds) = histogram_buffers(&sample.value);
            bytes += buffers;
            if let Some(bounds) = bounds
                && !self.values[..index].iter().any(|previous| {
                    histogram_buffers(&previous.value)
                        .1
                        .is_some_and(|other| std::ptr::eq(bounds, other))
                })
            {
                bytes += size_of_val(bounds);
            }
        }
        ProviderResources {
            held_bytes: bytes as u64,
            shared_cache_bytes: 0,
        }
    }
    fn max_page_samples(&self) -> usize {
        self.source.page_size
    }
    fn seek(&mut self, time_ms: i64, control: &Control) -> Result<(), StorageError> {
        control.check()?;
        self.values.clear();
        if let Some(seek_index) = self.source.seek_index {
            self.index = seek_index(time_ms).min(self.source.count);
            return Ok(());
        }
        self.index = 0;
        while self.index < self.source.count && (self.source.sample)(self.index).time_ms < time_ms {
            self.index += 1;
        }
        Ok(())
    }
    fn next(
        &mut self,
        control: &Control,
        resources: &mut ProviderResources,
    ) -> Result<Option<SamplePage<'_>>, StorageError> {
        *resources = self.resources();
        control.check()?;
        self.source
            .counters
            .next_calls
            .fetch_add(1, Ordering::Relaxed);
        self.values.clear();
        *resources = self.resources();
        let fail_page = self.source.fail_page == Some(self.pages);
        #[cfg(test)]
        let fail_page = fail_page
            || (self.pages == 0 && self.source.fail_acquisition == Some(self.acquisition));
        if fail_page {
            return Err(StorageError::Provider(
                b"injected page failure".to_vec().into(),
            ));
        }
        while self.index < self.source.count && self.values.len() < self.source.page_size {
            let sample = (self.source.sample)(self.index);
            self.index += 1;
            if sample.time_ms > self.bounds.end_ms {
                break;
            }
            if sample.time_ms >= self.bounds.start_ms {
                self.values.push(sample);
            }
        }
        if self.values.is_empty() {
            return Ok(None);
        }
        self.pages += 1;
        self.source
            .counters
            .samples_returned
            .fetch_add(self.values.len(), Ordering::Relaxed);
        *resources = self.resources();
        Ok(Some(SamplePage {
            resources: self.resources(),
            samples: PageSamples::Samples(&self.values),
        }))
    }
}

fn histogram_buffers(value: &SampleValue) -> (usize, Option<&[f64]>) {
    let (positive_spans, negative_spans, positive, negative, bounds) = match value {
        SampleValue::Float(_) => return (0, None),
        SampleValue::FloatHistogram(h) => (
            h.positive_spans.as_ref().map_or(0, |v| v.capacity()),
            h.negative_spans.as_ref().map_or(0, |v| v.capacity()),
            h.positive_buckets.as_ref().map_or(0, |v| v.capacity()),
            h.negative_buckets.as_ref().map_or(0, |v| v.capacity()),
            h.custom_values.as_deref(),
        ),
        SampleValue::IntegerHistogram(h) => (
            h.positive_spans.as_ref().map_or(0, Vec::capacity),
            h.negative_spans.as_ref().map_or(0, Vec::capacity),
            h.positive_buckets.as_ref().map_or(0, Vec::capacity),
            h.negative_buckets.as_ref().map_or(0, Vec::capacity),
            h.custom_values.as_deref(),
        ),
    };
    let spans = positive_spans + negative_spans;
    (
        spans * size_of::<crate::histogram::Span>() + (positive + negative) * size_of::<f64>(),
        bounds,
    )
}

impl Drop for GeneratedCursor<'_> {
    fn drop(&mut self) {
        self.source.counters.closed.fetch_add(1, Ordering::Relaxed);
        self.source.counters.active.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) fn counter(index: usize) -> Sample {
    Sample {
        time_ms: (index as i64 + 1) * 1000,
        start_time_ms: 0,
        value: SampleValue::Float(index as f64 + 1.0),
    }
}

#[cfg(test)]
pub(crate) fn adjacent_counter(index: usize) -> Sample {
    Sample {
        value: SampleValue::Float(f64::from_bits(0x3ff0_0000_0000_0000 + index as u64 % 2)),
        ..counter(index)
    }
}

#[cfg(test)]
pub(crate) fn subnormal_counter(index: usize) -> Sample {
    Sample {
        value: SampleValue::Float(f64::from_bits(index as u64 + 1)),
        ..counter(index)
    }
}

#[cfg(test)]
pub(crate) fn prefix_ladder(index: usize) -> Sample {
    let position = index % 17;
    Sample {
        value: SampleValue::Float(if position == 0 {
            0.0
        } else {
            f64::from_bits(1 << (4 * (position - 1)))
        }),
        ..counter(index)
    }
}

#[cfg(test)]
pub(crate) fn mad_prefix_ladder(index: usize) -> Sample {
    let position = index % 35;
    Sample {
        // A strict zero majority makes both deviation medians retain the full ladder.
        value: SampleValue::Float(if position < 19 {
            0.0
        } else {
            f64::from_bits(1 << (4 * (position - 19)))
        }),
        ..counter(index)
    }
}

#[cfg(test)]
pub(crate) fn two_classes(index: usize) -> Sample {
    Sample {
        value: SampleValue::Float(if index.is_multiple_of(2) { 1.0 } else { 2.0 }),
        ..counter(index)
    }
}

#[derive(Default)]
pub(crate) struct FixtureCounts {
    pub scopes_opened: AtomicUsize,
    pub scopes_closed: AtomicUsize,
    pub selections_opened: AtomicUsize,
    pub selections_closed: AtomicUsize,
    pub series_opened: AtomicUsize,
    pub series_closed: AtomicUsize,
}
pub(crate) struct FixtureSeries {
    pub labels: Vec<(Vec<u8>, Vec<u8>)>,
    pub source: GeneratedSeries,
}
pub(crate) struct FixtureProvider {
    pub series: Vec<FixtureSeries>,
    pub counts: Arc<FixtureCounts>,
    pub filter_selectors: bool,
}
pub(crate) struct FixtureQuery<'a> {
    provider: &'a FixtureProvider,
}
pub(crate) struct NoTemporary;
impl crate::storage::QueryProvider for NoTemporary {
    type Selection<'a> = FixtureSelection<'a>;
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn select(
        &self,
        _: crate::storage::SelectionRequest<'_>,
        _: &Control,
    ) -> Result<Self::Selection<'_>, StorageError> {
        Err(StorageError::Invalid("test query has no selectors"))
    }
}
pub(crate) struct FixtureSelection<'a> {
    provider: &'a FixtureProvider,
    index: usize,
    matchers: Option<AccountedVec<Option<Matcher>>>,
    accounting: Option<QueryAccounting>,
}
pub(crate) struct FixtureHandle<'a> {
    data: &'a FixtureSeries,
    counts: &'a FixtureCounts,
}

impl crate::storage::Provider for FixtureProvider {
    type Query<'a> = FixtureQuery<'a>;
    fn open(
        &self,
        _bounds: crate::storage::TimeBounds,
        control: &Control,
    ) -> Result<Self::Query<'_>, StorageError> {
        control.check()?;
        self.counts.scopes_opened.fetch_add(1, Ordering::Relaxed);
        Ok(FixtureQuery { provider: self })
    }
}
impl crate::storage::QueryProvider for FixtureQuery<'_> {
    type Selection<'a>
        = FixtureSelection<'a>
    where
        Self: 'a;
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn with_temporary_float_bits<R>(
        &self,
        request: crate::storage::TemporaryFloatBitsRequest<'_>,
        control: &Control,
        visit: impl for<'a> FnOnce(&'a mut dyn crate::storage::TemporaryFloatBits) -> R,
    ) -> Result<R, StorageError> {
        crate::fixture_temporary_storage::with_store(request, control, visit)
    }
    fn select(
        &self,
        request: crate::storage::SelectionRequest<'_>,
        control: &Control,
    ) -> Result<Self::Selection<'_>, StorageError> {
        control.check()?;
        let matchers = if self.provider.filter_selectors {
            let context = request
                .accounting
                .map(|account| AllocationContext::new(account, MemoryKind::ProviderHeld));
            let mut copies = AccountedVec::new_in(context.as_ref());
            for matcher in request.matchers {
                control.check()?;
                if let Some(account) = request.accounting {
                    account.work(1);
                }
                copies.push(
                    matcher
                        .as_ref()
                        .map(|matcher| {
                            matcher.copy_for(request.accounting, MemoryKind::ProviderHeld, control)
                        })
                        .transpose()?,
                );
            }
            Some(copies)
        } else {
            None
        };
        let selection = FixtureSelection {
            provider: self.provider,
            index: 0,
            matchers,
            accounting: self
                .provider
                .filter_selectors
                .then(|| request.accounting.cloned())
                .flatten(),
        };
        self.provider
            .counts
            .selections_opened
            .fetch_add(1, Ordering::Relaxed);
        Ok(selection)
    }
}
impl<'a> crate::storage::SeriesSelection for FixtureSelection<'a> {
    type Series = FixtureHandle<'a>;
    fn resources(&self) -> ProviderResources {
        if self.accounting.is_some() {
            return ProviderResources::default();
        }
        let bytes = self.matchers.as_ref().map_or(0, |matchers| {
            matchers.capacity() * size_of::<Option<Matcher>>()
                + matchers
                    .iter()
                    .flatten()
                    .map(|matcher| matcher.name.capacity() + matcher.value.capacity())
                    .sum::<usize>()
        });
        ProviderResources {
            held_bytes: bytes as u64,
            shared_cache_bytes: 0,
        }
    }
    fn next(&mut self, control: &Control) -> Result<Option<Self::Series>, StorageError> {
        loop {
            control.check()?;
            let Some(data) = self.provider.series.get(self.index) else {
                return Ok(None);
            };
            self.index += 1;
            if let Some(matchers) = &self.matchers {
                let context = self
                    .accounting
                    .as_ref()
                    .map(|account| AllocationContext::new(account, MemoryKind::Workspace));
                let labels = AccountedVec::from_iter_in(
                    data.labels
                        .iter()
                        .map(|(name, value)| LabelRef { name, value }),
                    context.as_ref(),
                );
                if !matches_checked(&labels, matchers, self.accounting.as_ref(), control)? {
                    continue;
                }
            }
            self.provider
                .counts
                .series_opened
                .fetch_add(1, Ordering::Relaxed);
            return Ok(Some(FixtureHandle {
                data,
                counts: &self.provider.counts,
            }));
        }
    }
}
impl Series for FixtureHandle<'_> {
    type Cursor<'a>
        = GeneratedCursor<'a>
    where
        Self: 'a;
    fn label_count(&self) -> usize {
        self.data.labels.len()
    }
    fn label(&self, index: usize) -> LabelRef<'_> {
        #[cfg(test)]
        label_observed();
        LabelRef {
            name: &self.data.labels[index].0,
            value: &self.data.labels[index].1,
        }
    }
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn cursor(
        &self,
        bounds: TimeBounds,
        control: &Control,
    ) -> Result<Self::Cursor<'_>, StorageError> {
        self.data.source.cursor(bounds, control)
    }
}
impl Drop for FixtureQuery<'_> {
    fn drop(&mut self) {
        self.provider
            .counts
            .scopes_closed
            .fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for FixtureSelection<'_> {
    fn drop(&mut self) {
        self.provider
            .counts
            .selections_closed
            .fetch_add(1, Ordering::Relaxed);
    }
}
impl Drop for FixtureHandle<'_> {
    fn drop(&mut self) {
        self.counts.series_closed.fetch_add(1, Ordering::Relaxed);
    }
}
impl FixtureProvider {
    pub(crate) fn new(count: usize, page_size: usize, series_count: usize) -> Self {
        let cursors = Arc::new(CursorCounts::default());
        let series = (0..series_count)
            .map(|index| {
                let mut source = GeneratedSeries::new(count, page_size, counter);
                source.counters = cursors.clone();
                FixtureSeries {
                    labels: vec![
                        (b"__name__".to_vec(), b"counter_total".to_vec()),
                        (b"instance".to_vec(), index.to_string().into_bytes()),
                        (b"job".to_vec(), b"worker".to_vec()),
                    ],
                    source,
                }
            })
            .collect();
        Self {
            series,
            counts: Arc::new(FixtureCounts::default()),
            filter_selectors: false,
        }
    }
    pub(crate) fn assert_closed(&self) {
        assert_eq!(
            self.counts.scopes_opened.load(Ordering::Relaxed),
            self.counts.scopes_closed.load(Ordering::Relaxed)
        );
        assert_eq!(
            self.counts.selections_opened.load(Ordering::Relaxed),
            self.counts.selections_closed.load(Ordering::Relaxed)
        );
        assert_eq!(
            self.counts.series_opened.load(Ordering::Relaxed),
            self.counts.series_closed.load(Ordering::Relaxed)
        );
        for series in &self.series {
            assert_eq!(
                series.source.counters.opened.load(Ordering::Relaxed),
                series.source.counters.closed.load(Ordering::Relaxed)
            );
            assert_eq!(series.source.counters.active.load(Ordering::Relaxed), 0);
        }
    }
}

#[cfg(test)]
mod work_injection_tests {
    use super::*;
    use crate::{accounting::EngineAccounting, storage::Interrupted};
    #[test]
    fn injection_is_query_specific_nested_and_unwind_scoped() {
        let engine = EngineAccounting::default();
        let outer = engine.query();
        let inner = engine.query();
        let first = Control::default();
        let second = Control::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            interrupt_during(&outer, &first, || {
                inner.work(20_000);
                assert!(first.check().is_ok());
                outer.work(8191);
                assert!(first.check().is_ok());
                interrupt_during(&inner, &second, || {
                    inner.work(8192);
                    assert_eq!(second.check(), Err(Interrupted::Cancelled));
                });
                assert!(first.check().is_ok());
                outer.work(1);
                assert_eq!(first.check(), Err(Interrupted::Cancelled));
                panic!("work fixture unwind");
            })
        }));
        assert!(result.is_err());
        let after = Control::default();
        interrupt_during(&outer, &after, || {
            inner.work(8192);
            assert!(after.check().is_ok());
        });
        outer.work(8192);
        assert!(after.check().is_ok());
        outer.snapshot().assert_memory_released();
        inner.snapshot().assert_memory_released();
        drop(outer);
        drop(inner);
        engine.snapshot().assert_memory_released();
    }
}
