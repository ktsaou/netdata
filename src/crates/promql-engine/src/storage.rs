// SPDX-License-Identifier: GPL-3.0-or-later
//! Query-scoped, repeatable typed storage. Providers own their handles and pages.
use crate::{
    accounting::{MemoryKind, MemoryLease, QueryAccounting},
    frontend::ast::Matcher,
    value::{Sample, SampleRef, SampleValueRef},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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
    matchers
        .iter()
        .flatten()
        .all(|matcher| matcher.matches(label(labels, &matcher.name)))
}

#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
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
impl Control {
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
    Provider(Vec<u8>),
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
#[derive(Clone, Copy, Debug)]
pub struct SelectionRequest<'a> {
    pub matchers: &'a [Option<Matcher>],
    pub bounds: TimeBounds,
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
}

impl<'a> PageSamples<'a> {
    pub fn len(self) -> usize {
        match self {
            Self::Views(samples) => samples.len(),
            Self::Samples(samples) => samples.len(),
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn sample(self, index: usize) -> SampleRef<'a> {
        match self {
            Self::Views(samples) => samples[index],
            Self::Samples(samples) => samples[index].as_ref(),
        }
    }
}

/// Each successful scope, selection, series or cursor releases its resource on Drop.
pub trait Provider {
    type Query<'a>: QueryProvider
    where
        Self: 'a;
    fn open(&self, control: &Control) -> Result<Self::Query<'_>, StorageError>;
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
    let header_bytes = samples
        .len()
        .checked_mul(match samples {
            PageSamples::Views(_) => size_of::<SampleRef<'_>>(),
            PageSamples::Samples(_) => size_of::<Sample>(),
        })
        .ok_or(StorageError::Invalid("slice size overflow"))?;
    borrow.resize(header_bytes as u64);
    let mut ranges = crate::accounting::AccountedVec::new(account, MemoryKind::Workspace);
    match samples {
        PageSamples::Views(samples) => expose(&mut ranges, Some(samples))?,
        PageSamples::Samples(samples) => expose(&mut ranges, Some(samples))?,
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
    control.check()?;
    account.provider_call();
    let cursor = series.cursor(bounds, control)?;
    let resources = cursor.resources();
    let mut cursor = ResourceOwner::new(cursor, resources, account);
    let maximum = cursor.value.max_page_samples();
    if maximum == 0 {
        return Err(StorageError::Invalid("zero cursor page size").into());
    }
    account.provider_call();
    let seek = cursor.value.seek(bounds.start_ms, control);
    cursor.lease.update(cursor.value.resources());
    seek?;
    let mut previous = None;
    loop {
        control.check()?;
        account.provider_call();
        let mut resources = cursor.value.resources();
        let page = match cursor.value.next(control, &mut resources) {
            Ok(Some(page)) => page,
            Ok(None) => {
                cursor.lease.update(resources);
                break;
            }
            Err(error) => {
                cursor.lease.update(resources);
                return Err(error.into());
            }
        };
        cursor.lease.update(resources);
        if page.resources != resources {
            return Err(StorageError::Invalid("inconsistent page resource snapshot").into());
        }
        if page.samples.is_empty() || page.samples.len() > maximum {
            return Err(StorageError::Invalid("invalid cursor page size").into());
        }
        account.page(page.samples.len());
        let mut borrow = account.hold(MemoryKind::BorrowedView, 0);
        page_bytes(page.samples, account, &mut borrow)?;
        for index in 0..page.samples.len() {
            let sample = page.samples.sample(index);
            control.check()?;
            if sample.time_ms < bounds.start_ms || sample.time_ms > bounds.end_ms {
                return Err(StorageError::Invalid("sample outside cursor bounds").into());
            }
            if previous.is_some_and(|time| sample.time_ms <= time) {
                return Err(
                    StorageError::Invalid("sample timestamps are not strictly increasing").into(),
                );
            }
            previous = Some(sample.time_ms);
            account.work(1);
            visit(sample)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{FloatHistogramRef, Span},
    };
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
