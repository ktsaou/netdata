// SPDX-License-Identifier: GPL-3.0-or-later
//! Query-scoped, repeatable typed storage. Providers own their handles and pages.
use crate::{
    accounting::{
        AccountedVec, EngineAccounting, MemoryKind, MemoryLease, QueryAccounting, ReferenceLease,
    },
    frontend::ast::Matcher,
    shared::Strong,
    value::{Sample, SampleRef, SampleValueRef},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LabelRef<'a> {
    pub name: &'a [u8],
    pub value: &'a [u8],
}
pub type LabelsRef<'a> = &'a [LabelRef<'a>];
pub fn label<'a>(labels: LabelsRef<'a>, name: &[u8]) -> &'a [u8] {
    labels
        .binary_search_by(|label| label.name.cmp(name))
        .map_or(&[], |index| labels[index].value)
}
pub fn matches(labels: LabelsRef<'_>, matchers: &[Option<Matcher>]) -> bool {
    matches_in(labels, matchers, None)
}
pub fn matches_in(
    labels: LabelsRef<'_>,
    matchers: &[Option<Matcher>],
    accounting: Option<&QueryAccounting>,
) -> bool {
    matchers
        .iter()
        .flatten()
        .all(|matcher| matcher.matches_in(label(labels, &matcher.name), accounting))
}

pub fn matches_checked(
    labels: LabelsRef<'_>,
    matchers: &[Option<Matcher>],
    accounting: Option<&QueryAccounting>,
    control: &Control,
) -> Result<bool, Interrupted> {
    control.check()?;
    for matcher in matchers.iter().flatten() {
        control.check()?;
        let (mut begin, mut end) = (0, labels.len());
        let mut value: &[u8] = &[];
        while begin < end {
            control.check()?;
            let middle = begin + (end - begin) / 2;
            match crate::binary_labels::compare_with(
                labels[middle].name,
                &matcher.name,
                control,
                accounting,
            )? {
                std::cmp::Ordering::Less => begin = middle + 1,
                std::cmp::Ordering::Greater => end = middle,
                std::cmp::Ordering::Equal => {
                    value = labels[middle].value;
                    break;
                }
            }
        }
        if !matcher.matches_with(value, accounting, Some(control))? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Clone, Debug)]
pub struct Cancellation(Strong<AtomicBool>);
impl Default for Cancellation {
    /// Caller-created tokens remain outside every engine's physical memory census.
    fn default() -> Self {
        Self(Strong::new_in(AtomicBool::new(false), None))
    }
}
impl Cancellation {
    pub(crate) fn new_for_engine(engine: &EngineAccounting) -> Self {
        Self(Strong::new_for_engine(AtomicBool::new(false), engine))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
#[derive(Clone, Debug, Default)]
pub struct Control {
    pub cancellation: Cancellation,
    pub deadline: Option<Instant>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interrupted {
    Cancelled,
    Deadline,
}
// Drop the token alias before releasing its reference census and query owner.
pub(crate) struct ControlObservation {
    _cancellation: Cancellation,
    _reference: ReferenceLease,
}
impl Control {
    pub(crate) fn observe(&self, account: &QueryAccounting) -> ControlObservation {
        let cancellation = self.cancellation.clone();
        let reference = account.hold_cancellation_reference(cancellation.0.engine_owned());
        ControlObservation {
            _cancellation: cancellation,
            _reference: reference,
        }
    }
    pub fn check(&self) -> Result<(), Interrupted> {
        if self.cancellation.cancelled() {
            return Err(Interrupted::Cancelled);
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(Interrupted::Deadline);
        }
        Ok(())
    }
}
#[derive(Debug, Eq, PartialEq)]
pub enum StorageError {
    Interrupted(Interrupted),
    Invalid(&'static str),
    Provider(crate::accounting::AccountedVec<u8>),
}
impl StorageError {
    pub(crate) fn accounted(mut self, account: &QueryAccounting) -> Self {
        if let Self::Provider(bytes) = &mut self {
            bytes.account_if_unobserved(&crate::accounting::AllocationContext::new(
                account,
                MemoryKind::Workspace,
            ));
        }
        self
    }
}
impl From<Interrupted> for StorageError {
    fn from(value: Interrupted) -> Self {
        Self::Interrupted(value)
    }
}
/// Query-held resources are distinct from the existing shared cache.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProviderResources {
    /// Bytes owned exclusively by this handle, excluding descendants and shared cache.
    pub held_bytes: u64,
    pub shared_cache_bytes: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct TimeBounds {
    pub start_ms: i64,
    pub end_ms: i64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectionHints<'a> {
    pub step_ms: i64,
    pub range_ms: i64,
    pub function: &'a str,
    pub by: bool,
    pub grouping: Option<&'a [AccountedVec<u8>]>,
}
#[derive(Clone, Copy, Debug)]
pub struct SelectionRequest<'a> {
    pub matchers: &'a [Option<Matcher>],
    pub bounds: TimeBounds,
    /// Hints borrow the compiled plan for this callback; retained copies belong to the provider.
    pub hints: SelectionHints<'a>,
    /// Filtering may clone this owner; directly charged scratch is excluded from provider snapshots.
    pub accounting: Option<&'a QueryAccounting>,
}
#[derive(Debug)]
pub struct SamplePage<'a> {
    pub samples: PageSamples<'a>,
    /// Snapshot of the cursor owner's resources after producing this page.
    pub resources: ProviderResources,
}

#[derive(Clone, Copy, Debug)]
pub enum PageSamples<'a> {
    Views(&'a [SampleRef<'a>]),
    Samples(&'a [Sample]),
    Adapted(AdaptedPage<'a>),
}

/// Adapter descriptors have their own charge; the foreign page is observed before conversion can be interrupted.
#[derive(Clone, Copy, Debug)]
pub struct AdaptedPage<'a> {
    pub(crate) samples: &'a [SampleRef<'a>],
    pub(crate) observation: &'a MemoryLease,
}

impl<'a> PageSamples<'a> {
    pub fn len(self) -> usize {
        match self {
            Self::Views(samples) => samples.len(),
            Self::Samples(samples) => samples.len(),
            Self::Adapted(page) => page.samples.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn sample(self, index: usize) -> SampleRef<'a> {
        match self {
            Self::Views(samples) => samples[index],
            Self::Samples(samples) => samples[index].as_ref(),
            Self::Adapted(page) => page.samples[index],
        }
    }
}

/// Each successful scope, selection, series or cursor releases its resource on Drop.
pub trait Provider {
    type Query<'a>: QueryProvider
    where
        Self: 'a;
    /// Inclusive original query bounds, before adjusted evaluation offsets.
    fn open(&self, bounds: TimeBounds, control: &Control) -> Result<Self::Query<'_>, StorageError>;
}
pub trait QueryProvider {
    type Selection<'a>: SeriesSelection + 'a
    where
        Self: 'a;
    fn resources(&self) -> ProviderResources;
    fn select(
        &self,
        request: SelectionRequest<'_>,
        control: &Control,
    ) -> Result<Self::Selection<'_>, StorageError>;
}
pub trait SeriesSelection {
    type Series: Series;
    fn resources(&self) -> ProviderResources;
    fn next(&mut self, control: &Control) -> Result<Option<Self::Series>, StorageError>;
    /// Terminal diagnostics are observed after metadata exhaustion, including a returned error.
    fn diagnostic_count(&self) -> usize {
        0
    }
    /// The view cannot escape its callback; severity is independent of message text.
    fn diagnostic<R>(
        &self,
        _index: usize,
        _visit: impl for<'a> FnOnce(ProviderDiagnosticRef<'a>) -> R,
    ) -> R {
        panic!("selection has no diagnostics")
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ProviderDiagnosticRef<'a> {
    pub message: &'a [u8],
    pub severity: crate::annotations::Severity,
    pub position: Option<crate::frontend::Span>,
}
pub trait Series {
    type Cursor<'a>: SampleCursor
    where
        Self: 'a;
    /// Label names are unique and sorted by raw bytes until this handle is released.
    fn label_count(&self) -> usize;
    fn label(&self, index: usize) -> LabelRef<'_>;
    fn label_views<'a>(
        &'a self,
        account: &QueryAccounting,
    ) -> crate::accounting::AccountedVec<LabelRef<'a>> {
        let mut views = crate::accounting::AccountedVec::new(account, MemoryKind::Workspace);
        for index in 0..self.label_count() {
            views.push(self.label(index));
        }
        views
    }
    fn label_views_checked<'a>(
        &'a self,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<crate::accounting::AccountedVec<LabelRef<'a>>, Interrupted> {
        let mut views = crate::accounting::AccountedVec::new(account, MemoryKind::Workspace);
        for index in 0..self.label_count() {
            control.check()?;
            account.work(1);
            views.push(self.label(index));
        }
        control.check()?;
        Ok(views)
    }
    fn resources(&self) -> ProviderResources;
    fn cursor(
        &self,
        bounds: TimeBounds,
        control: &Control,
    ) -> Result<Self::Cursor<'_>, StorageError>;
}
pub trait SampleCursor {
    fn resources(&self) -> ProviderResources;
    fn max_page_samples(&self) -> usize;
    /// The following batch begins at the first timestamp at or after this value.
    fn seek(&mut self, time_ms: i64, control: &Control) -> Result<(), StorageError>;
    /// Nested views expire at the next cursor operation; EOF is distinct from an error.
    /// Publish the post-call resource snapshot for success, EOF and returned errors.
    fn next(
        &mut self,
        control: &Control,
        resources: &mut ProviderResources,
    ) -> Result<Option<SamplePage<'_>>, StorageError>;
}
pub fn validate_labels(labels: LabelsRef<'_>) -> Result<(), StorageError> {
    if labels.iter().any(|label| label.name.is_empty()) {
        return Err(StorageError::Invalid("empty label name"));
    }
    if labels.windows(2).any(|pair| pair[0].name >= pair[1].name) {
        return Err(StorageError::Invalid(
            "label names are not unique and sorted",
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct ResourceLease {
    held: MemoryLease,
    cache: MemoryLease,
}

/// Release accounting after the provider destructor has released its resource.
pub(crate) struct ResourceOwner<T> {
    pub(crate) value: T,
    pub(crate) lease: ResourceLease,
}
impl<T> ResourceOwner<T> {
    pub(crate) fn new(value: T, resources: ProviderResources, account: &QueryAccounting) -> Self {
        Self {
            value,
            lease: ResourceLease::new(account, resources),
        }
    }
}

impl ResourceLease {
    pub(crate) fn quartet(
        a: (&mut Self, ProviderResources),
        b: (&mut Self, ProviderResources),
        c: (&mut Self, ProviderResources),
        d: (&mut Self, ProviderResources),
    ) {
        MemoryLease::reconcile([
            (&mut a.0.held, a.1.held_bytes),
            (&mut a.0.cache, a.1.shared_cache_bytes),
            (&mut b.0.held, b.1.held_bytes),
            (&mut b.0.cache, b.1.shared_cache_bytes),
            (&mut c.0.held, c.1.held_bytes),
            (&mut c.0.cache, c.1.shared_cache_bytes),
            (&mut d.0.held, d.1.held_bytes),
            (&mut d.0.cache, d.1.shared_cache_bytes),
        ]);
    }
    pub(crate) fn pair(a: &mut Self, ar: ProviderResources, b: &mut Self, br: ProviderResources) {
        MemoryLease::reconcile([
            (&mut a.held, ar.held_bytes),
            (&mut a.cache, ar.shared_cache_bytes),
            (&mut b.held, br.held_bytes),
            (&mut b.cache, br.shared_cache_bytes),
        ]);
    }
    pub(crate) fn triple(
        a: &mut Self,
        ar: ProviderResources,
        b: &mut Self,
        br: ProviderResources,
        c: &mut Self,
        cr: ProviderResources,
    ) {
        MemoryLease::reconcile([
            (&mut a.held, ar.held_bytes),
            (&mut a.cache, ar.shared_cache_bytes),
            (&mut b.held, br.held_bytes),
            (&mut b.cache, br.shared_cache_bytes),
            (&mut c.held, cr.held_bytes),
            (&mut c.cache, cr.shared_cache_bytes),
        ]);
    }
    pub(crate) fn new(account: &QueryAccounting, resources: ProviderResources) -> Self {
        Self {
            held: account.hold(MemoryKind::ProviderHeld, resources.held_bytes),
            cache: account.hold(
                MemoryKind::SharedCacheReference,
                resources.shared_cache_bytes,
            ),
        }
    }

    pub(crate) fn update(&mut self, resources: ProviderResources) {
        self.held.resize(resources.held_bytes);
        self.cache.resize(resources.shared_cache_bytes);
    }
}

#[derive(Clone, Copy)]
struct ByteRange {
    start: usize,
    end: usize,
}
fn expose<T>(
    ranges: &mut crate::accounting::AccountedVec<ByteRange>,
    values: Option<&[T]>,
) -> Result<(), StorageError> {
    let values = values.unwrap_or(&[]);
    let bytes = values
        .len()
        .checked_mul(size_of::<T>())
        .ok_or(StorageError::Invalid("slice size overflow"))?;
    if bytes != 0 {
        let start = values.as_ptr() as usize;
        let end = start
            .checked_add(bytes)
            .ok_or(StorageError::Invalid("slice address overflow"))?;
        ranges.push(ByteRange { start, end });
    }
    Ok(())
}
pub(crate) fn page_bytes(
    samples: PageSamples<'_>,
    account: &QueryAccounting,
    borrow: &mut MemoryLease,
) -> Result<u64, StorageError> {
    if let PageSamples::Adapted(page) = samples {
        return Ok(page.observation.bytes());
    }
    let header_bytes = samples
        .len()
        .checked_mul(match samples {
            PageSamples::Views(_) => size_of::<SampleRef<'_>>(),
            PageSamples::Samples(_) => size_of::<Sample>(),
            PageSamples::Adapted(_) => unreachable!("already observed foreign page"),
        })
        .ok_or(StorageError::Invalid("slice size overflow"))?;
    borrow.resize(header_bytes as u64);
    let mut ranges = crate::accounting::AccountedVec::new(account, MemoryKind::Workspace);
    match samples {
        PageSamples::Views(samples) => expose(&mut ranges, Some(samples))?,
        PageSamples::Samples(samples) => expose(&mut ranges, Some(samples))?,
        PageSamples::Adapted(_) => unreachable!("already observed foreign page"),
    }
    // Observe the complete bounded page before a pending cancellation discards its borrow.
    for index in 0..samples.len() {
        let sample = samples.sample(index);
        match sample.value {
            SampleValueRef::Float(_) => (),
            SampleValueRef::FloatHistogram(h) => {
                expose(&mut ranges, h.positive_spans)?;
                expose(&mut ranges, h.negative_spans)?;
                expose(&mut ranges, h.positive_buckets)?;
                expose(&mut ranges, h.negative_buckets)?;
                expose(&mut ranges, h.custom_values)?;
            }
            SampleValueRef::IntegerHistogram(h) => {
                expose(&mut ranges, h.positive_spans)?;
                expose(&mut ranges, h.negative_spans)?;
                expose(&mut ranges, h.positive_buckets)?;
                expose(&mut ranges, h.negative_buckets)?;
                expose(&mut ranges, h.custom_values)?;
            }
        };
    }
    ranges.sort_unstable_by_key(|range| range.start);
    let mut bytes = 0u64;
    let mut end = 0;
    for range in ranges.iter() {
        if range.end > end {
            bytes = bytes
                .checked_add((range.end - range.start.max(end)) as u64)
                .ok_or(StorageError::Invalid("page size overflow"))?;
            end = range.end;
        }
    }
    borrow.resize(bytes);
    Ok(bytes)
}

/// Visit a single series with no retained raw window or cross-series page cache.
pub fn scan<S: Series>(
    series: &S,
    bounds: TimeBounds,
    control: &Control,
    account: &QueryAccounting,
    visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    scan_with(series, bounds, control, account, visit)
}

pub fn scan_with<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: TimeBounds,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
) -> Result<(), E> {
    scan_until(series, bounds, control, account, |sample| {
        visit(sample).map(|()| std::ops::ControlFlow::Continue(()))
    })
    .map(|_| ())
}

pub(crate) fn scan_until<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: TimeBounds,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<std::ops::ControlFlow<()>, E>,
) -> Result<std::ops::ControlFlow<()>, E> {
    walk(
        series,
        bounds,
        WalkStart::Seek(bounds.start_ms),
        control,
        account,
        |sample| {
            visit(sample).map(|action| {
                if action.is_break() {
                    CursorAction::Stop
                } else {
                    CursorAction::Next
                }
            })
        },
    )
}

#[derive(Clone, Copy)]
pub(crate) enum CursorAction {
    Next,
    Seek(i64),
    SeekInitial(i64),
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CursorOrigin {
    InitialNext,
    Next,
    SeekLanding,
}

pub(crate) enum WalkStart<'a> {
    Next,
    Seek(i64),
    Probe(&'a mut Option<StorageError>),
}

pub(crate) enum InitialSample {
    Ready,
    Empty,
    Error(AccountedVec<u8>),
}

pub(crate) fn initialize<S: Series>(
    series: &S,
    bounds: TimeBounds,
    control: &Control,
    account: &QueryAccounting,
) -> Result<InitialSample, StorageError> {
    let mut failure = None;
    let outcome = walk::<_, StorageError>(
        series,
        bounds,
        WalkStart::Probe(&mut failure),
        control,
        account,
        |_| Ok(CursorAction::Stop),
    )?;
    Ok(match failure {
        Some(StorageError::Provider(bytes)) => InitialSample::Error(bytes),
        Some(_) => unreachable!("only initial provider read errors are latched"),
        None if outcome.is_break() => InitialSample::Ready,
        None => InitialSample::Empty,
    })
}

pub(crate) fn walk<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: TimeBounds,
    start: WalkStart<'_>,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<CursorAction, E>,
) -> Result<std::ops::ControlFlow<()>, E> {
    walk_observed(series, bounds, start, control, account, |sample, _| {
        visit(sample)
    })
}

pub(crate) fn walk_observed<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: TimeBounds,
    start: WalkStart<'_>,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>, CursorOrigin) -> Result<CursorAction, E>,
) -> Result<std::ops::ControlFlow<()>, E> {
    let _control_observation = control.observe(account);
    control.check()?;
    account.provider_call();
    let cursor = series
        .cursor(bounds, control)
        .map_err(|error| error.accounted(account))?;
    let resources = cursor.resources();
    let mut cursor = ResourceOwner::new(cursor, resources, account);
    let maximum = cursor.value.max_page_samples();
    if maximum == 0 {
        return Err(StorageError::Invalid("zero cursor page size").into());
    }
    let mut previous = None;
    let (mut seek, mut initial_failure) = match start {
        WalkStart::Next => (None, None),
        WalkStart::Seek(target) => (Some(target), None),
        WalkStart::Probe(failure) => (None, Some(failure)),
    };
    let mut origin = if seek.is_some() {
        CursorOrigin::SeekLanding
    } else {
        CursorOrigin::InitialNext
    };
    let mut initial_floor = None;
    let mut first = true;
    let mut minimum = bounds.start_ms;
    loop {
        if let Some(target) = seek.take() {
            control.check()?;
            account.provider_call();
            let outcome = cursor
                .value
                .seek(target, control)
                .map_err(|error| error.accounted(account));
            cursor.lease.update(cursor.value.resources());
            outcome?;
            minimum = target.max(bounds.start_ms);
        }
        control.check()?;
        account.provider_call();
        let mut resources = cursor.value.resources();
        let page = match cursor
            .value
            .next(control, &mut resources)
            .map_err(|error| error.accounted(account))
        {
            Ok(Some(page)) => page,
            Ok(None) => {
                cursor.lease.update(resources);
                break;
            }
            Err(error) => {
                cursor.lease.update(resources);
                if first
                    && matches!(error, StorageError::Provider(_))
                    && let Some(failure) = initial_failure.as_deref_mut()
                {
                    *failure = Some(error);
                    break;
                }
                return Err(error.into());
            }
        };
        first = false;
        cursor.lease.update(resources);
        if page.resources != resources {
            return Err(StorageError::Invalid("inconsistent page resource snapshot").into());
        }
        if page.samples.is_empty() || page.samples.len() > maximum {
            return Err(StorageError::Invalid("invalid cursor page size").into());
        }
        if !matches!(page.samples, PageSamples::Adapted(_)) {
            account.page(page.samples.len());
        }
        let mut borrow = account.hold(MemoryKind::BorrowedView, 0);
        page_bytes(page.samples, account, &mut borrow)?;
        let mut action = CursorAction::Next;
        let mut consumed = None;
        for index in 0..page.samples.len() {
            let sample = page.samples.sample(index);
            control.check()?;
            if sample.time_ms < bounds.start_ms || sample.time_ms > bounds.end_ms {
                return Err(StorageError::Invalid("sample outside cursor bounds").into());
            }
            if sample.time_ms < minimum {
                return Err(StorageError::Invalid("sample before cursor seek target").into());
            }
            if initial_floor.is_some_and(|time| sample.time_ms < time) {
                return Err(
                    StorageError::Invalid("initial seek landed before current sample").into(),
                );
            }
            if previous.is_some_and(|time| sample.time_ms <= time) {
                return Err(
                    StorageError::Invalid("sample timestamps are not strictly increasing").into(),
                );
            }
            previous = Some(sample.time_ms);
            account.work(1);
            if matches!(action, CursorAction::Next) {
                let initial = origin == CursorOrigin::InitialNext;
                action = visit(sample, origin)?;
                origin = CursorOrigin::Next;
                consumed = Some(sample.time_ms);
                if let CursorAction::Seek(target) = action
                    && target <= sample.time_ms
                {
                    return Err(StorageError::Invalid("nonadvancing cursor seek").into());
                }
                if let CursorAction::SeekInitial(target) = action
                    && (!initial || target > sample.time_ms)
                {
                    return Err(StorageError::Invalid("invalid initial current seek").into());
                }
            }
        }
        // A seek may revisit the validated but semantically unread tail.
        match action {
            CursorAction::Next => (),
            CursorAction::Seek(target) => {
                previous = consumed;
                seek = Some(target);
                origin = CursorOrigin::SeekLanding;
            }
            CursorAction::SeekInitial(target) => {
                // Reset alone permits the same current point to be returned by Seek.
                previous = None;
                initial_floor = consumed;
                seek = Some(target);
                origin = CursorOrigin::SeekLanding;
            }
            CursorAction::Stop => return Ok(std::ops::ControlFlow::Break(())),
        }
    }
    Ok(std::ops::ControlFlow::Continue(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{FloatHistogramRef, Span},
    };
    #[test]
    fn moving_seeks_can_revisit_the_validated_tail_or_reach_eof() {
        use crate::{
            test_storage::GeneratedSeries,
            value::{Sample, SampleValue},
        };
        fn sample(index: usize) -> Sample {
            Sample {
                time_ms: 10 * (index as i64 + 1),
                start_time_ms: 0,
                value: SampleValue::Float(index as f64),
            }
        }
        for page_size in [1, 17, 1024] {
            for target in [20, 40, 50] {
                let source = GeneratedSeries::new(4, page_size, sample);
                let account = EngineAccounting::default().query();
                let mut visited = Vec::new();
                let result = walk::<_, StorageError>(
                    &source,
                    TimeBounds {
                        start_ms: 0,
                        end_ms: 100,
                    },
                    WalkStart::Next,
                    &Control::default(),
                    &account,
                    |sample| {
                        visited.push(sample.time_ms);
                        Ok(if sample.time_ms == 10 {
                            CursorAction::Seek(target)
                        } else {
                            CursorAction::Stop
                        })
                    },
                )
                .unwrap();
                assert_eq!(
                    visited,
                    if target == 50 {
                        vec![10]
                    } else {
                        vec![10, target]
                    }
                );
                assert_eq!(result.is_continue(), target == 50);
                assert_eq!(source.counters.next_calls.load(Ordering::Relaxed), 2);
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
                assert_eq!(source.counters.closed.load(Ordering::Relaxed), 1);
                assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                account.snapshot().assert_memory_released();
            }
        }
    }
    #[test]
    fn unread_tail_validation_preserves_earlier_visitor_errors() {
        use crate::{
            test_storage::GeneratedSeries,
            value::{Sample, SampleValue},
        };
        fn sample(index: usize) -> Sample {
            Sample {
                time_ms: [1, 2, 2][index],
                start_time_ms: 0,
                value: SampleValue::Float(1.0),
            }
        }
        for mode in 0..3 {
            let source = GeneratedSeries::new(3, 17, sample);
            let account = EngineAccounting::default().query();
            let mut visited = 0;
            let result = walk::<_, StorageError>(
                &source,
                TimeBounds {
                    start_ms: 0,
                    end_ms: 10,
                },
                WalkStart::Next,
                &Control::default(),
                &account,
                |_| {
                    visited += 1;
                    match mode {
                        0 => Ok(CursorAction::Stop),
                        1 => Ok(CursorAction::Seek(2)),
                        _ => {
                            let mut bytes = AccountedVec::new(&account, MemoryKind::Workspace);
                            bytes.extend_from_slice(b"earlier semantic failure");
                            Err(StorageError::Provider(bytes))
                        }
                    }
                },
            );
            if mode == 2 {
                let Err(StorageError::Provider(bytes)) = result else {
                    panic!("the semantic error must precede the malformed tail")
                };
                assert_eq!(&*bytes, b"earlier semantic failure");
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current,
                    bytes.capacity() as u64
                );
            } else {
                assert_eq!(
                    result,
                    Err(StorageError::Invalid(
                        "sample timestamps are not strictly increasing"
                    ))
                );
            }
            assert_eq!(visited, 1);
            assert_eq!(source.counters.next_calls.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.closed.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
            account.snapshot().assert_memory_released();
        }
    }
    #[test]
    fn initial_probe_releases_each_cursor_and_retains_only_error_bytes() {
        use crate::test_storage::{GeneratedSeries, counter};
        for mode in 0..3 {
            let account = EngineAccounting::default().query();
            let mut source = GeneratedSeries::new(usize::from(mode != 0), 17, counter);
            if mode == 2 {
                source.fail_page = Some(0);
            }
            let initial = initialize(
                &source,
                TimeBounds {
                    start_ms: 0,
                    end_ms: 2000,
                },
                &Control::default(),
                &account,
            )
            .unwrap();
            match (&initial, mode) {
                (InitialSample::Empty, 0) | (InitialSample::Ready, 1) => (),
                (InitialSample::Error(bytes), 2) => {
                    assert_eq!(&**bytes, b"injected page failure");
                    assert_eq!(
                        bytes.context().unwrap().accounting().test_identity(),
                        account.test_identity()
                    );
                    assert_eq!(
                        account.snapshot().memory[MemoryKind::Workspace as usize].current,
                        bytes.capacity() as u64
                    );
                }
                _ => panic!("unexpected initial probe status"),
            }
            assert_eq!(source.counters.next_calls.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.closed.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
            for kind in [MemoryKind::ProviderHeld, MemoryKind::BorrowedView] {
                assert_eq!(account.snapshot().memory[kind as usize].current, 0);
            }
            drop((initial, source));
            account.snapshot().assert_memory_released();
        }
    }
    #[test]
    fn standalone_scan_retains_failed_page_bytes_after_releasing_the_cursor() {
        use crate::test_storage::{GeneratedSeries, counter};
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut source = GeneratedSeries::new(100, 2, counter);
        source.fail_page = Some(2);
        let error = scan(
            &source,
            TimeBounds {
                start_ms: 1000,
                end_ms: 100_000,
            },
            &Control::default(),
            &account,
            |_| Ok(()),
        )
        .unwrap_err();
        let StorageError::Provider(bytes) = &error else {
            panic!("expected the injected page failure")
        };
        assert_eq!(&**bytes, b"injected page failure");
        assert_eq!(
            bytes.context().unwrap().accounting().test_identity(),
            account.test_identity()
        );
        assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
        let observed = account.snapshot();
        assert_eq!(
            observed.memory[MemoryKind::Workspace as usize].current,
            bytes.capacity() as u64
        );
        assert_eq!(
            observed.memory[MemoryKind::BorrowedView as usize].current,
            0
        );
        assert_eq!(
            observed.memory[MemoryKind::ProviderHeld as usize].current,
            0
        );
        drop(source);
        assert_eq!(&**bytes, b"injected page failure");
        drop(error);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }
    #[test]
    fn cross_engine_and_external_controls_keep_creation_attribution() {
        use crate::execution::Engine;
        let creator = Engine::default();
        let consumer = Engine::default();
        let token_bytes = Strong::<AtomicBool>::allocation_bytes();
        println!("bookkeeping_token_layout bytes={token_bytes}");
        let token = creator.cancellation();
        let before = creator.accounting.snapshot();
        let alias = token.clone();
        assert_eq!(creator.accounting.snapshot(), before);
        assert_eq!(
            before.memory[MemoryKind::Accounting as usize].current,
            EngineAccounting::allocation_bytes() + token_bytes
        );
        let account = consumer.accounting.query();
        let control = Control {
            cancellation: alias,
            deadline: Some(Instant::now()),
        };
        let owned = control.observe(&account);
        let external_control = Control::default();
        let external = external_control.observe(&account);
        let observed = account.snapshot();
        assert_eq!(observed.cancellation_references.engine_owned.current, 1);
        assert_eq!(observed.cancellation_references.external.current, 1);
        assert_eq!(
            observed.memory[MemoryKind::Accounting as usize].current,
            QueryAccounting::allocation_bytes()
        );
        assert_eq!(
            consumer.accounting.snapshot().memory[MemoryKind::Accounting as usize].current,
            EngineAccounting::allocation_bytes() + QueryAccounting::allocation_bytes()
        );
        assert_eq!(creator.accounting.snapshot(), before);
        assert_eq!(control.check(), Err(Interrupted::Deadline));
        token.cancel();
        assert!(control.cancellation.cancelled());
        assert_eq!(control.check(), Err(Interrupted::Cancelled));
        drop((owned, external));
        account.snapshot().assert_memory_released();
        drop((control, account));
        consumer.accounting.snapshot().assert_memory_released();
        assert_eq!(
            creator.accounting.snapshot().memory[MemoryKind::Accounting as usize].current,
            EngineAccounting::allocation_bytes() + token_bytes
        );
        let retained_engine = creator.accounting.clone();
        drop(creator);
        drop(token);
        retained_engine.snapshot().assert_memory_released();
    }
    #[test]
    fn concurrent_scoped_aliases_charge_one_owned_token() {
        use crate::execution::Engine;
        let engine = Engine::default();
        let control = Control {
            cancellation: engine.cancellation(),
            deadline: None,
        };
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let accounting = &engine.accounting;
                let control = &control;
                let barrier = &barrier;
                scope.spawn(move || {
                    let account = accounting.query();
                    let guard = control.observe(&account);
                    barrier.wait();
                    assert_eq!(
                        account
                            .snapshot()
                            .cancellation_references
                            .engine_owned
                            .current,
                        1
                    );
                    barrier.wait();
                    drop(guard);
                    account.snapshot().assert_memory_released();
                });
            }
            barrier.wait();
            let observed = engine.accounting.snapshot();
            assert_eq!(observed.cancellation_references.engine_owned.current, 2);
            assert_eq!(observed.cancellation_references.engine_owned.peak, 2);
            assert_eq!(
                observed.memory[MemoryKind::Accounting as usize].current,
                EngineAccounting::allocation_bytes()
                    + 2 * QueryAccounting::allocation_bytes()
                    + Strong::<AtomicBool>::allocation_bytes()
            );
            barrier.wait();
        });
        let observed = engine.accounting.snapshot();
        assert_eq!(observed.cancellation_references.engine_owned.current, 0);
        assert_eq!(observed.cancellation_references.engine_owned.acquired, 2);
        assert_eq!(observed.cancellation_references.engine_owned.released, 2);
        drop(control);
        engine.accounting.snapshot().assert_memory_released();
    }
    #[test]
    fn standalone_scan_and_rate_scopes_balance_references_on_every_return() {
        use crate::{
            execution::Engine,
            range_float::{RateKind, RateValue, RateWindow, rate},
            test_storage::{GeneratedSeries, counter},
        };
        for owned in [false, true] {
            let engine = Engine::default();
            let account = engine.accounting.query();
            let control = Control {
                cancellation: if owned {
                    engine.cancellation()
                } else {
                    Cancellation::default()
                },
                deadline: None,
            };
            let source = GeneratedSeries::new(2, 1, counter);
            let bounds = TimeBounds {
                start_ms: 1000,
                end_ms: 2000,
            };
            let entry = |observed: crate::accounting::QueryObservations| {
                if owned {
                    observed.cancellation_references.engine_owned
                } else {
                    observed.cancellation_references.external
                }
            };
            scan(&source, bounds, &control, &account, |_| {
                assert_eq!(entry(account.snapshot()).current, 1);
                Ok(())
            })
            .unwrap();
            let result = scan_with::<_, StorageError>(&source, bounds, &control, &account, |_| {
                assert_eq!(entry(account.snapshot()).current, 1);
                Err(StorageError::Invalid("synthetic scan callback failure"))
            });
            assert!(matches!(result, Err(StorageError::Invalid(_))));
            control.cancellation.cancel();
            assert_eq!(
                scan(&source, bounds, &control, &account, |_| Ok(())),
                Err(StorageError::Interrupted(Interrupted::Cancelled))
            );
            // Empty rate bounds return before control polling in the source path.
            let result = rate(
                &source,
                RateWindow {
                    start_ms: 0,
                    end_ms: 0,
                    range_ns: 0,
                    start_timestamps: false,
                },
                RateKind::Rate,
                &control,
                &account,
            )
            .unwrap();
            assert_eq!(result.value, RateValue::Empty);
            let observed = entry(account.snapshot());
            assert_eq!(
                (
                    observed.current,
                    observed.peak,
                    observed.acquired,
                    observed.released
                ),
                (0, 1, 4, 4)
            );
            account.snapshot().assert_memory_released();
            drop((account, control));
            engine.accounting.snapshot().assert_memory_released();
        }
    }
    #[test]
    fn stopped_scans_validate_the_complete_returned_page() {
        use crate::{
            test_storage::GeneratedSeries,
            value::{Sample, SampleValue},
        };
        fn sample(index: usize) -> Sample {
            Sample {
                time_ms: [1, 2, 2][index],
                start_time_ms: 0,
                value: SampleValue::Float(1.0),
            }
        }
        let source = GeneratedSeries::new(3, 17, sample);
        let account = EngineAccounting::default().query();
        let mut visited = 0;
        let result = scan_until::<_, StorageError>(
            &source,
            TimeBounds {
                start_ms: 0,
                end_ms: 10,
            },
            &Control::default(),
            &account,
            |_| {
                visited += 1;
                Ok(std::ops::ControlFlow::Break(()))
            },
        );
        assert_eq!(
            result,
            Err(StorageError::Invalid(
                "sample timestamps are not strictly increasing"
            ))
        );
        assert_eq!(visited, 1);
        assert_eq!(account.snapshot().samples_fetched, 3);
        account.snapshot().assert_memory_released();
        assert_eq!(
            source
                .counters
                .active
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }
    #[test]
    fn shared_and_overlapping_page_slices_are_exposed_once() {
        let bounds = [1.0, 2.0, 3.0, 4.0];
        let buckets = [6.0, 7.0];
        let spans = [Span {
            offset: 0,
            length: 2,
        }];
        let histogram = FloatHistogramRef {
            reset_hint: 0,
            schema: -53,
            zero_threshold: 0.0,
            zero_count: 0.0,
            count: 13.0,
            sum: 20.0,
            positive_spans: Some(&spans),
            negative_spans: None,
            positive_buckets: Some(&buckets),
            negative_buckets: None,
            custom_values: Some(&bounds[..3]),
        };
        let views = [
            SampleRef {
                time_ms: 0,
                start_time_ms: 0,
                value: SampleValueRef::FloatHistogram(histogram),
            },
            SampleRef {
                time_ms: 1,
                start_time_ms: 0,
                value: SampleValueRef::FloatHistogram(FloatHistogramRef {
                    custom_values: Some(&bounds[1..]),
                    ..histogram
                }),
            },
        ];
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut borrow = account.hold(MemoryKind::BorrowedView, 0);
        let bytes = page_bytes(PageSamples::Views(&views), &account, &mut borrow).unwrap();
        assert_eq!(
            bytes,
            size_of_val(&views) as u64
                + size_of_val(&bounds) as u64
                + size_of_val(&buckets) as u64
                + size_of_val(&spans) as u64
        );
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].current,
            0
        );
        assert!(account.snapshot().memory[MemoryKind::Workspace as usize].peak > 0);
    }
}
