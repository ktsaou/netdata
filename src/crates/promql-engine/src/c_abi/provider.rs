// SPDX-License-Identifier: GPL-3.0-or-later
//! Foreign owners retain one bounded borrowed page and close successful opens exactly once.
use super::{
    checked::{self, Footprint, Invalid},
    types,
};
use crate::{
    accounting::{AccountedVec, MemoryKind, MemoryLease, QueryAccounting},
    histogram::{FloatHistogramRef, IntegerHistogramRef},
    storage::{
        self, AdaptedPage, Control, Interrupted, LabelRef, PageSamples, ProviderDiagnosticRef,
        ProviderResources, SampleAccess, SamplePage, StorageError, TimeBounds,
    },
    value::{SampleRef, SampleValueRef},
};
use std::ffi::c_void;

impl From<Invalid> for StorageError {
    fn from(error: Invalid) -> Self {
        Self::Invalid(error.0)
    }
}
fn resources(value: types::Resources) -> ProviderResources {
    ProviderResources {
        held_bytes: value.held_bytes,
        shared_cache_bytes: value.shared_cache_bytes,
    }
}
fn bounds(value: TimeBounds) -> types::Bounds {
    types::Bounds {
        start_ms: value.start_ms,
        end_ms: value.end_ms,
    }
}

pub(crate) struct Provider<'a> {
    table: types::Provider,
    account: &'a QueryAccounting,
}
impl<'a> Provider<'a> {
    pub(crate) unsafe fn new(
        table: *const types::Provider,
        account: &'a QueryAccounting,
    ) -> Result<Self, Invalid> {
        let table = unsafe { checked::versioned(table) }?;
        if table.query_open.is_none()
            || table.query_close.is_none()
            || table.query_resources.is_none()
            || table.selection_open.is_none()
            || table.selection_close.is_none()
            || table.selection_resources.is_none()
            || table.selection_next.is_none()
            || table.series_close.is_none()
            || table.series_resources.is_none()
            || table.cursor_open.is_none()
            || table.cursor_close.is_none()
            || table.cursor_resources.is_none()
            || table.cursor_seek.is_none()
            || table.cursor_next.is_none()
        {
            return Err(Invalid("missing provider callback"));
        }
        let temporary = [
            table.temporary_create.is_some(),
            table.temporary_read.is_some(),
            table.temporary_write.is_some(),
            table.temporary_resources.is_some(),
            table.temporary_close.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        if temporary != 0 && temporary != 5 {
            return Err(Invalid("incomplete temporary storage callbacks"));
        }
        Ok(Self { table, account })
    }
    unsafe fn status(&self, status: u32, error: types::Bytes) -> Result<(), StorageError> {
        match status {
            types::OK => Ok(()),
            types::PROVIDER_ERROR => Err(StorageError::Provider(unsafe {
                checked::returned_error(error, self.account)
            }?)),
            types::CANCELLED => Err(Interrupted::Cancelled.into()),
            types::DEADLINE => Err(Interrupted::Deadline.into()),
            _ => Err(StorageError::Invalid("invalid provider callback status")),
        }
    }
}
struct Owner<'a> {
    provider: &'a Provider<'a>,
    handle: *mut c_void,
    close: types::Close,
    resource: types::ResourceCallback,
}
impl Owner<'_> {
    fn resources(&self) -> ProviderResources {
        self.provider.account.work(1);
        resources(unsafe {
            (self.resource)(
                self.provider.table.context,
                self.handle,
                checked::accounting_context(self.provider.account),
            )
        })
    }
}
impl Drop for Owner<'_> {
    fn drop(&mut self) {
        self.provider.account.work(1);
        // Status, rather than the token's value, transferred ownership.
        unsafe {
            (self.close)(
                self.provider.table.context,
                self.handle,
                checked::accounting_context(self.provider.account),
            )
        };
    }
}
pub(crate) struct Query<'a> {
    owner: Owner<'a>,
}
pub(crate) struct Selection<'a> {
    owner: Owner<'a>,
    diagnostics: AccountedVec<ProviderDiagnosticRef<'a>>,
}
pub(crate) struct Series<'a> {
    owner: Owner<'a>,
    labels: AccountedVec<LabelRef<'a>>,
}
pub(crate) struct Cursor<'a> {
    owner: Owner<'a>,
    samples: ValidatedPage<'a>,
    borrowed: MemoryLease,
    maximum: usize,
}

struct ValidatedPage<'a> {
    data: *const types::Sample,
    length: usize,
    borrow: std::marker::PhantomData<&'a types::Sample>,
}
impl Default for ValidatedPage<'_> {
    fn default() -> Self {
        Self {
            data: std::ptr::NonNull::<types::Sample>::dangling().as_ptr(),
            length: 0,
            borrow: std::marker::PhantomData,
        }
    }
}

// The ABI keeps this validated page and its nested arrays immutable until cursor mutation.
unsafe impl Send for ValidatedPage<'_> {}
unsafe impl Sync for ValidatedPage<'_> {}

impl SampleAccess for ValidatedPage<'_> {
    fn len(&self) -> usize {
        self.length
    }
    fn sample(&self, index: usize) -> SampleRef<'_> {
        // Slice references stay within this access, so moving/dropping the owner cannot protect freed data.
        let samples = unsafe { std::slice::from_raw_parts(self.data, self.length) };
        unsafe { sample_view(samples[index], None) }.expect("validated immutable foreign sample")
    }
}
impl std::fmt::Debug for ValidatedPage<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_list()
            .entries((0..self.len()).map(|index| self.sample(index)))
            .finish()
    }
}
struct Temporary<'a> {
    owner: Owner<'a>,
    count: u64,
}
impl Temporary<'_> {
    fn range(&self, offset: u64, len: usize) -> Result<u64, StorageError> {
        let len =
            u64::try_from(len).map_err(|_| StorageError::Invalid("temporary transfer overflow"))?;
        if offset.checked_add(len).is_none_or(|end| end > self.count) {
            return Err(StorageError::Invalid("temporary transfer out of bounds"));
        }
        Ok(len)
    }
}
impl storage::TemporaryFloatBits for Temporary<'_> {
    fn resources(&self) -> ProviderResources {
        self.owner.resources()
    }
    fn read_into(
        &mut self,
        offset: u64,
        values: &mut [u64],
        control: &Control,
    ) -> Result<(), StorageError> {
        let len = self.range(offset, values.len())?;
        control.check()?;
        let provider = self.owner.provider;
        let mut transferred = 0;
        let mut error = types::Bytes::default();
        let status = unsafe {
            provider
                .table
                .temporary_read
                .expect("validated temporary read")(
                provider.table.context,
                self.owner.handle,
                offset,
                values.as_mut_ptr(),
                len,
                checked::Callback::new(control, provider.account).as_ptr(),
                &mut transferred,
                &mut error,
            )
        };
        unsafe { provider.status(status, error) }?;
        if transferred != len {
            return Err(StorageError::Invalid(
                "temporary read returned incorrect length",
            ));
        }
        Ok(())
    }
    fn write_at(
        &mut self,
        offset: u64,
        values: &[u64],
        control: &Control,
    ) -> Result<(), StorageError> {
        let len = self.range(offset, values.len())?;
        control.check()?;
        let provider = self.owner.provider;
        let mut transferred = 0;
        let mut error = types::Bytes::default();
        let status = unsafe {
            provider
                .table
                .temporary_write
                .expect("validated temporary write")(
                provider.table.context,
                self.owner.handle,
                offset,
                values.as_ptr(),
                len,
                checked::Callback::new(control, provider.account).as_ptr(),
                &mut transferred,
                &mut error,
            )
        };
        unsafe { provider.status(status, error) }?;
        if transferred != len {
            return Err(StorageError::Invalid(
                "temporary write returned incorrect length",
            ));
        }
        Ok(())
    }
}

impl storage::Provider for Provider<'_> {
    type Query<'a>
        = Query<'a>
    where
        Self: 'a;
    fn open(&self, time: TimeBounds, control: &Control) -> Result<Self::Query<'_>, StorageError> {
        let table = &self.table;
        let close = table.query_close.expect("validated query close");
        let resource = table.query_resources.expect("validated query resources");
        let mut handle = std::ptr::null_mut();
        let mut error = types::Bytes::default();
        let status = unsafe {
            table.query_open.expect("validated query open")(
                table.context,
                bounds(time),
                checked::Callback::new(control, self.account).as_ptr(),
                &mut handle,
                &mut error,
            )
        };
        unsafe { self.status(status, error) }?;
        Ok(Query {
            owner: Owner {
                provider: self,
                handle,
                close,
                resource,
            },
        })
    }
}
impl storage::QueryProvider for Query<'_> {
    type Selection<'a>
        = Selection<'a>
    where
        Self: 'a;
    fn resources(&self) -> ProviderResources {
        self.owner.resources()
    }
    fn with_temporary_float_bits<R>(
        &self,
        request: storage::TemporaryFloatBitsRequest<'_>,
        control: &Control,
        visit: impl for<'a> FnOnce(&'a mut dyn storage::TemporaryFloatBits) -> R,
    ) -> Result<R, StorageError> {
        request.extent_bytes()?;
        let provider = self.owner.provider;
        let table = &provider.table;
        let Some(create) = table.temporary_create else {
            return Err(storage::temporary_storage_error(
                provider.account,
                b"temporary float storage unavailable",
            ));
        };
        control.check()?;
        let mut handle = std::ptr::null_mut();
        let mut error = types::Bytes::default();
        let status = unsafe {
            create(
                table.context,
                self.owner.handle,
                request.value_count,
                checked::Callback::new(control, provider.account).as_ptr(),
                &mut handle,
                &mut error,
            )
        };
        unsafe { provider.status(status, error) }?;
        let mut store = Temporary {
            owner: Owner {
                provider,
                handle,
                close: table.temporary_close.expect("validated temporary close"),
                resource: table
                    .temporary_resources
                    .expect("validated temporary resources"),
            },
            count: request.value_count,
        };
        Ok(visit(&mut store))
    }
    fn select(
        &self,
        request: storage::SelectionRequest<'_>,
        control: &Control,
    ) -> Result<Self::Selection<'_>, StorageError> {
        let provider = self.owner.provider;
        let table = &provider.table;
        let close = table.selection_close.expect("validated selection close");
        let resource = table
            .selection_resources
            .expect("validated selection resources");
        let mut matchers = AccountedVec::new(provider.account, MemoryKind::Workspace);
        let mut grouping = AccountedVec::new(provider.account, MemoryKind::Workspace);
        for matcher in request.matchers.iter().flatten() {
            control.check()?;
            provider.account.work(1);
            matchers.push(types::Matcher {
                kind: matcher.kind as u32,
                reserved: 0,
                name: types::Bytes::of(&matcher.name),
                value: types::Bytes::of(&matcher.value),
            });
        }
        for name in request.hints.grouping.into_iter().flatten() {
            control.check()?;
            provider.account.work(1);
            grouping.push(types::Bytes::of(name));
        }
        let request = types::SelectionRequest {
            matchers: types::Matchers {
                data: matchers.as_ptr(),
                len: matchers.len() as u64,
            },
            bounds: bounds(request.bounds),
            step_ms: request.hints.step_ms,
            range_ms: request.hints.range_ms,
            function: types::Bytes::of(request.hints.function.as_bytes()),
            by: u32::from(request.hints.by),
            grouping_present: u32::from(request.hints.grouping.is_some()),
            grouping: types::ByteArray {
                data: grouping.as_ptr(),
                len: grouping.len() as u64,
            },
        };
        control.check()?;
        let mut handle = std::ptr::null_mut();
        let mut error = types::Bytes::default();
        let status = unsafe {
            table.selection_open.expect("validated selection open")(
                table.context,
                self.owner.handle,
                &request,
                checked::Callback::new(control, provider.account).as_ptr(),
                &mut handle,
                &mut error,
            )
        };
        unsafe { provider.status(status, error) }?;
        Ok(Selection {
            owner: Owner {
                provider,
                handle,
                close,
                resource,
            },
            diagnostics: AccountedVec::new(provider.account, MemoryKind::Workspace),
        })
    }
}
impl<'a> storage::SeriesSelection for Selection<'a> {
    type Series = Series<'a>;
    fn resources(&self) -> ProviderResources {
        self.owner.resources()
    }
    fn next(&mut self, control: &Control) -> Result<Option<Self::Series>, StorageError> {
        self.diagnostics.clear();
        let provider = self.owner.provider;
        let table = &provider.table;
        let close = table.series_close.expect("validated series close");
        let resource = table.series_resources.expect("validated series resources");
        let mut series = types::SeriesOpen::default();
        let mut diagnostics = types::ProviderDiagnostics::default();
        let mut error = types::Bytes::default();
        let status = unsafe {
            table.selection_next.expect("validated selection next")(
                table.context,
                self.owner.handle,
                checked::Callback::new(control, provider.account).as_ptr(),
                &mut series,
                &mut diagnostics,
                &mut error,
            )
        };
        if status == types::OK {
            let owner = Owner {
                provider,
                handle: series.handle,
                close,
                resource,
            };
            let mut result = Series {
                owner,
                labels: AccountedVec::new(provider.account, MemoryKind::Workspace),
            };
            let labels = unsafe { checked::slice(series.labels.data, series.labels.len) }?;
            for label in labels {
                control.check()?;
                provider.account.work(1);
                result.labels.push(LabelRef {
                    name: unsafe { checked::bytes(label.name) }?,
                    value: unsafe { checked::bytes(label.value) }?,
                });
            }
            control.check()?;
            return Ok(Some(result));
        }
        // Retain returned error bytes before resource callbacks or terminal-diagnostic construction.
        let returned = if status == types::END {
            Ok(())
        } else {
            unsafe { provider.status(status, error) }
        };
        let diagnostic_control = returned.is_ok().then_some(control);
        if let Some(control) = diagnostic_control {
            control.check()?;
        }
        let notes = unsafe { checked::slice(diagnostics.data, diagnostics.len) }?;
        for note in notes {
            if let Some(control) = diagnostic_control {
                control.check()?;
            }
            provider.account.work(1);
            let severity = match note.severity {
                0 => crate::annotations::Severity::Info,
                1 => crate::annotations::Severity::Warning,
                _ => return Err(StorageError::Invalid("invalid diagnostic severity")),
            };
            let position = if checked::present(note.position_present)? {
                Some(crate::frontend::Span {
                    start: usize::try_from(note.position.start)
                        .map_err(|_| Invalid("diagnostic position overflow"))?,
                    end: usize::try_from(note.position.end)
                        .map_err(|_| Invalid("diagnostic position overflow"))?,
                })
            } else {
                None
            };
            self.diagnostics.push(ProviderDiagnosticRef {
                message: unsafe { checked::bytes(note.message) }?,
                severity,
                position,
            });
        }
        returned?;
        if let Some(control) = diagnostic_control {
            control.check()?;
        }
        Ok(None)
    }
    fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
    }
    fn diagnostic<R>(
        &self,
        index: usize,
        visit: impl for<'note> FnOnce(ProviderDiagnosticRef<'note>) -> R,
    ) -> R {
        visit(self.diagnostics[index])
    }
}
impl storage::Series for Series<'_> {
    type Cursor<'a>
        = Cursor<'a>
    where
        Self: 'a;
    fn label_count(&self) -> usize {
        self.labels.len()
    }
    fn label(&self, index: usize) -> LabelRef<'_> {
        self.labels[index]
    }
    fn resources(&self) -> ProviderResources {
        self.owner.resources()
    }
    fn cursor(
        &self,
        time: TimeBounds,
        control: &Control,
    ) -> Result<Self::Cursor<'_>, StorageError> {
        let provider = self.owner.provider;
        let table = &provider.table;
        let close = table.cursor_close.expect("validated cursor close");
        let resource = table.cursor_resources.expect("validated cursor resources");
        let mut cursor = types::CursorOpen::default();
        let mut error = types::Bytes::default();
        let status = unsafe {
            table.cursor_open.expect("validated cursor open")(
                table.context,
                self.owner.handle,
                bounds(time),
                checked::Callback::new(control, provider.account).as_ptr(),
                &mut cursor,
                &mut error,
            )
        };
        unsafe { provider.status(status, error) }?;
        let owner = Owner {
            provider,
            handle: cursor.handle,
            close,
            resource,
        };
        let mut result = Cursor {
            owner,
            samples: ValidatedPage::default(),
            borrowed: provider.account.hold(MemoryKind::BorrowedView, 0),
            maximum: 0,
        };
        result.maximum = usize::try_from(cursor.max_page_samples)
            .map_err(|_| Invalid("cursor page size overflow"))?;
        if result.maximum == 0 {
            return Err(StorageError::Invalid("zero cursor page size"));
        }
        Ok(result)
    }
}
impl Cursor<'_> {
    fn invalidate(&mut self) {
        self.samples = ValidatedPage::default();
        self.borrowed.resize(0);
    }
}
impl Drop for Cursor<'_> {
    fn drop(&mut self) {
        self.invalidate();
    }
}
impl storage::SampleCursor for Cursor<'_> {
    fn resources(&self) -> ProviderResources {
        self.owner.resources()
    }
    fn max_page_samples(&self) -> usize {
        self.maximum
    }
    fn seek(&mut self, time_ms: i64, control: &Control) -> Result<(), StorageError> {
        self.invalidate();
        let provider = self.owner.provider;
        let mut error = types::Bytes::default();
        let status = unsafe {
            provider.table.cursor_seek.expect("validated cursor seek")(
                provider.table.context,
                self.owner.handle,
                time_ms,
                checked::Callback::new(control, provider.account).as_ptr(),
                &mut error,
            )
        };
        unsafe { provider.status(status, error) }
    }
    fn next(
        &mut self,
        control: &Control,
        snapshot: &mut ProviderResources,
    ) -> Result<Option<SamplePage<'_>>, StorageError> {
        self.invalidate();
        let provider = self.owner.provider;
        let mut page = types::Page::default();
        let mut observed = types::Resources::default();
        let mut error = types::Bytes::default();
        let status = unsafe {
            provider.table.cursor_next.expect("validated cursor next")(
                provider.table.context,
                self.owner.handle,
                checked::Callback::new(control, provider.account).as_ptr(),
                &mut page,
                &mut observed,
                &mut error,
            )
        };
        *snapshot = resources(observed);
        if status == types::END {
            return Ok(None);
        }
        unsafe { provider.status(status, error) }?;
        if page.resources != observed {
            return Err(StorageError::Invalid(
                "inconsistent foreign page resource snapshot",
            ));
        }
        let raw = unsafe { checked::slice(page.samples.data, page.samples.len) }?;
        if raw.is_empty() || raw.len() > self.maximum {
            return Err(StorageError::Invalid("invalid cursor page size"));
        }
        provider.account.page(raw.len());
        self.borrowed.resize(std::mem::size_of_val(raw) as u64);
        let mut footprint = Footprint::new(provider.account);
        footprint.add(page.samples.data, page.samples.len)?;
        for sample in raw {
            provider.account.work(1);
            unsafe { sample_view(*sample, Some(&mut footprint)) }?;
        }
        self.borrowed.resize(footprint.bytes()?);
        raw.len()
            .checked_mul(size_of::<SampleRef<'_>>())
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .ok_or(Invalid("adapter descriptor size overflow"))?;
        for _ in raw {
            control.check()?;
            provider.account.work(1);
        }
        control.check()?;
        self.samples = ValidatedPage {
            data: raw.as_ptr(),
            length: raw.len(),
            borrow: std::marker::PhantomData,
        };
        Ok(Some(SamplePage {
            samples: PageSamples::Adapted(AdaptedPage {
                samples: &self.samples,
                observation: &self.borrowed,
            }),
            resources: *snapshot,
        }))
    }
}

unsafe fn spans<'a>(
    value: types::SpanArray,
) -> Result<Option<&'a [crate::histogram::Span]>, Invalid> {
    unsafe { checked::optional(value.data, value.len, value.present, value.reserved) }
}
unsafe fn floats<'a>(value: types::F64Array) -> Result<Option<&'a [f64]>, Invalid> {
    unsafe { checked::optional(value.data, value.len, value.present, value.reserved) }
}
unsafe fn integers<'a>(value: types::I64Array) -> Result<Option<&'a [i64]>, Invalid> {
    unsafe { checked::optional(value.data, value.len, value.present, value.reserved) }
}
pub(super) unsafe fn float_histogram<'a>(
    pointer: *const types::FloatHistogram,
    footprint: Option<&mut Footprint>,
) -> Result<FloatHistogramRef<'a>, Invalid> {
    let h = unsafe { checked::read(pointer) }?;
    let h = FloatHistogramRef {
        reset_hint: u8::try_from(h.reset_hint)
            .map_err(|_| Invalid("reset hint does not fit byte"))?,
        schema: h.schema,
        zero_threshold: f64::from_bits(h.zero_threshold_bits),
        zero_count: f64::from_bits(h.zero_count_bits),
        count: f64::from_bits(h.count_bits),
        sum: f64::from_bits(h.sum_bits),
        positive_spans: unsafe { spans(h.positive_spans) }?,
        negative_spans: unsafe { spans(h.negative_spans) }?,
        positive_buckets: unsafe { floats(h.positive_buckets) }?,
        negative_buckets: unsafe { floats(h.negative_buckets) }?,
        custom_values: unsafe { floats(h.custom_values) }?,
    };
    if let Some(footprint) = footprint {
        footprint.add(pointer, 1)?;
        footprint.add_slice(h.positive_spans)?;
        footprint.add_slice(h.negative_spans)?;
        footprint.add_slice(h.positive_buckets)?;
        footprint.add_slice(h.negative_buckets)?;
        footprint.add_slice(h.custom_values)?;
    }
    Ok(h)
}
unsafe fn sample_view<'a>(
    value: types::Sample,
    mut footprint: Option<&mut Footprint>,
) -> Result<SampleRef<'a>, Invalid> {
    if value.value.reserved != 0 {
        return Err(Invalid("nonzero reserved value field"));
    }
    let sample = match value.value.kind {
        types::VALUE_FLOAT => {
            SampleValueRef::Float(f64::from_bits(unsafe { value.value.payload.float_bits }))
        }
        types::VALUE_INTEGER_HISTOGRAM => {
            let pointer = unsafe { value.value.payload.integer_histogram };
            let h = unsafe { checked::read(pointer) }?;
            let h = IntegerHistogramRef {
                reset_hint: u8::try_from(h.reset_hint)
                    .map_err(|_| Invalid("reset hint does not fit byte"))?,
                schema: h.schema,
                zero_threshold: f64::from_bits(h.zero_threshold_bits),
                zero_count: h.zero_count,
                count: h.count,
                sum: f64::from_bits(h.sum_bits),
                positive_spans: unsafe { spans(h.positive_spans) }?,
                negative_spans: unsafe { spans(h.negative_spans) }?,
                positive_buckets: unsafe { integers(h.positive_buckets) }?,
                negative_buckets: unsafe { integers(h.negative_buckets) }?,
                custom_values: unsafe { floats(h.custom_values) }?,
            };
            if let Some(footprint) = &mut footprint {
                footprint.add(pointer, 1)?;
                footprint.add_slice(h.positive_spans)?;
                footprint.add_slice(h.negative_spans)?;
                footprint.add_slice(h.positive_buckets)?;
                footprint.add_slice(h.negative_buckets)?;
                footprint.add_slice(h.custom_values)?;
            }
            SampleValueRef::IntegerHistogram(h)
        }
        types::VALUE_FLOAT_HISTOGRAM => {
            let pointer = unsafe { value.value.payload.float_histogram };
            SampleValueRef::FloatHistogram(unsafe { float_histogram(pointer, footprint) }?)
        }
        _ => return Err(Invalid("invalid sample value tag")),
    };
    Ok(SampleRef {
        time_ms: value.time_ms,
        start_time_ms: value.start_time_ms,
        value: sample,
    })
}
