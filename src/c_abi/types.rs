// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixed-width declarations mirrored by include/netdata-promql.h.
use std::ffi::c_void;

pub const ABI_VERSION: u32 = 4;
pub const OK: u32 = 0;
pub const INVALID_ARGUMENT: u32 = 1;
pub const PARSE_ERROR: u32 = 2;
pub const CANCELLED: u32 = 3;
pub const DEADLINE: u32 = 4;
pub const PROVIDER_ERROR: u32 = 5;
pub const OUTPUT_ERROR: u32 = 6;
pub const EVALUATION_ERROR: u32 = 7;
pub const INTERNAL_ERROR: u32 = 8;
pub const UNSUPPORTED: u32 = 9;
pub const END: u32 = 10;

pub const VALUE_FLOAT: u32 = 1;
pub const VALUE_INTEGER_HISTOGRAM: u32 = 2;
pub const VALUE_FLOAT_HISTOGRAM: u32 = 3;
pub const TYPE_SCALAR: u32 = 1;
pub const TYPE_VECTOR: u32 = 2;
pub const TYPE_MATRIX: u32 = 3;
pub const TYPE_STRING: u32 = 4;
pub const CHUNK_ANNOTATIONS: u32 = 1;
pub const CHUNK_SCALAR: u32 = 2;
pub const CHUNK_STRING: u32 = 3;
pub const CHUNK_SERIES: u32 = 4;
pub const REQUEST_INSTANT: u32 = 1;
pub const REQUEST_RANGE: u32 = 2;

pub const EXPERIMENTAL_FUNCTIONS: u32 = 1;
pub const BINOP_FILL_MODIFIERS: u32 = 2;
pub const EXTENDED_RANGE_SELECTORS: u32 = 4;
pub const DURATION_EXPRESSIONS: u32 = 8;
pub const START_TIMESTAMPS: u32 = 16;
pub const DELAYED_NAME_REMOVAL: u32 = 32;
pub const OPTION_FLAGS: u32 = 63;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Header {
    pub abi_version: u32,
    pub struct_size: u32,
}
impl Header {
    pub fn of<T>() -> Self {
        Self {
            abi_version: ABI_VERSION,
            struct_size: size_of::<T>() as u32,
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Bytes {
    pub data: *const u8,
    pub len: u64,
}
impl Bytes {
    pub(crate) fn of(value: &[u8]) -> Self {
        Self {
            data: value.as_ptr(),
            len: value.len() as u64,
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Label {
    pub name: Bytes,
    pub value: Bytes,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Labels {
    pub data: *const Label,
    pub len: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Matcher {
    pub kind: u32,
    pub reserved: u32,
    pub name: Bytes,
    pub value: Bytes,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Matchers {
    pub data: *const Matcher,
    pub len: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ByteArray {
    pub data: *const Bytes,
    pub len: u64,
}
pub type Span = crate::histogram::Span;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SpanArray {
    pub data: *const Span,
    pub len: u64,
    pub present: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct I64Array {
    pub data: *const i64,
    pub len: u64,
    pub present: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct F64Array {
    pub data: *const f64,
    pub len: u64,
    pub present: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IntegerHistogram {
    pub reset_hint: u32,
    pub schema: i32,
    pub zero_threshold_bits: u64,
    pub zero_count: u64,
    pub count: u64,
    pub sum_bits: u64,
    pub positive_spans: SpanArray,
    pub negative_spans: SpanArray,
    pub positive_buckets: I64Array,
    pub negative_buckets: I64Array,
    pub custom_values: F64Array,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FloatHistogram {
    pub reset_hint: u32,
    pub schema: i32,
    pub zero_threshold_bits: u64,
    pub zero_count_bits: u64,
    pub count_bits: u64,
    pub sum_bits: u64,
    pub positive_spans: SpanArray,
    pub negative_spans: SpanArray,
    pub positive_buckets: F64Array,
    pub negative_buckets: F64Array,
    pub custom_values: F64Array,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct HistogramBucket {
    pub lower_bits: u64,
    pub upper_bits: u64,
    pub count_bits: u64,
    pub lower_inclusive: u32,
    pub upper_inclusive: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union ValuePayload {
    pub float_bits: u64,
    pub integer_histogram: *const IntegerHistogram,
    pub float_histogram: *const FloatHistogram,
}
impl Default for ValuePayload {
    fn default() -> Self {
        Self { float_bits: 0 }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Value {
    pub kind: u32,
    pub reserved: u32,
    pub payload: ValuePayload,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Sample {
    pub time_ms: i64,
    pub start_time_ms: i64,
    pub value: Value,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Samples {
    pub data: *const Sample,
    pub len: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Resources {
    pub held_bytes: u64,
    pub shared_cache_bytes: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Bounds {
    pub start_ms: i64,
    pub end_ms: i64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Position {
    pub start: u64,
    pub end: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ProviderDiagnostic {
    pub severity: u32,
    pub position_present: u32,
    pub position: Position,
    pub message: Bytes,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ProviderDiagnostics {
    pub data: *const ProviderDiagnostic,
    pub len: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Annotation {
    pub severity: u32,
    pub position_present: u32,
    pub position: Position,
    pub occurrences: i64,
    pub raw_message: Bytes,
    pub message: Bytes,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Annotations {
    pub data: *const Annotation,
    pub len: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SelectionRequest {
    pub matchers: Matchers,
    pub bounds: Bounds,
    pub step_ms: i64,
    pub range_ms: i64,
    pub function: Bytes,
    pub by: u32,
    pub grouping_present: u32,
    pub grouping: ByteArray,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SeriesOpen {
    pub handle: *mut c_void,
    pub labels: Labels,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CursorOpen {
    pub handle: *mut c_void,
    pub max_page_samples: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Page {
    pub samples: Samples,
    pub resources: Resources,
}

macro_rules! opaque { ($($name:ident),*) => { $(#[repr(C)] pub struct $name { _private: [u8; 0] })* }; }
opaque!(
    Engine,
    Query,
    ResultHandle,
    Report,
    Control,
    CallbackControl,
    AccountingContext,
    AllocationHandle,
    Selector
);

pub type Close = unsafe extern "C" fn(*mut c_void, *mut c_void, *const AccountingContext);
pub type ResourceCallback =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *const AccountingContext) -> Resources;
pub type QueryOpenCallback = unsafe extern "C" fn(
    *mut c_void,
    Bounds,
    *const CallbackControl,
    *mut *mut c_void,
    *mut Bytes,
) -> u32;
pub type SelectionOpenCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const SelectionRequest,
    *const CallbackControl,
    *mut *mut c_void,
    *mut Bytes,
) -> u32;
pub type SelectionNextCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const CallbackControl,
    *mut SeriesOpen,
    *mut ProviderDiagnostics,
    *mut Bytes,
) -> u32;
pub type CursorOpenCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    Bounds,
    *const CallbackControl,
    *mut CursorOpen,
    *mut Bytes,
) -> u32;
pub type SeekCallback =
    unsafe extern "C" fn(*mut c_void, *mut c_void, i64, *const CallbackControl, *mut Bytes) -> u32;
pub type NextCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const CallbackControl,
    *mut Page,
    *mut Resources,
    *mut Bytes,
) -> u32;
pub type TemporaryCreateCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    u64,
    *const CallbackControl,
    *mut *mut c_void,
    *mut Bytes,
) -> u32;
pub type TemporaryReadCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    u64,
    *mut u64,
    u64,
    *const CallbackControl,
    *mut u64,
    *mut Bytes,
) -> u32;
pub type TemporaryWriteCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    u64,
    *const u64,
    u64,
    *const CallbackControl,
    *mut u64,
    *mut Bytes,
) -> u32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Provider {
    pub header: Header,
    pub context: *mut c_void,
    pub query_open: Option<QueryOpenCallback>,
    pub query_close: Option<Close>,
    pub query_resources: Option<ResourceCallback>,
    pub selection_open: Option<SelectionOpenCallback>,
    pub selection_close: Option<Close>,
    pub selection_resources: Option<ResourceCallback>,
    pub selection_next: Option<SelectionNextCallback>,
    pub series_close: Option<Close>,
    pub series_resources: Option<ResourceCallback>,
    pub cursor_open: Option<CursorOpenCallback>,
    pub cursor_close: Option<Close>,
    pub cursor_resources: Option<ResourceCallback>,
    pub cursor_seek: Option<SeekCallback>,
    pub cursor_next: Option<NextCallback>,
    pub temporary_create: Option<TemporaryCreateCallback>,
    pub temporary_read: Option<TemporaryReadCallback>,
    pub temporary_write: Option<TemporaryWriteCallback>,
    pub temporary_resources: Option<ResourceCallback>,
    pub temporary_close: Option<Close>,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EngineOptions {
    pub header: Header,
    pub flags: u32,
    pub reserved: u32,
    pub lookback_ms: i64,
    pub no_step_subquery_interval_ms: i64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ControlOptions {
    pub header: Header,
    pub deadline_present: u32,
    pub nanoseconds: u32,
    pub seconds: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Duration {
    pub seconds: u64,
    pub nanoseconds: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ControlState {
    pub status: u32,
    pub deadline_present: u32,
    pub remaining: Duration,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Request {
    pub header: Header,
    pub kind: u32,
    pub reserved: u32,
    pub start_ms: i64,
    pub end_ms: i64,
    pub step_ms: i64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Timestamp {
    pub unix_seconds: i64,
    pub nanoseconds: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NumericText {
    pub len: u32,
    pub reserved: u32,
    pub bytes: [u8; 384],
}
impl Default for NumericText {
    fn default() -> Self {
        Self {
            len: 0,
            reserved: 0,
            bytes: [0; 384],
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PreciseRequest {
    pub header: Header,
    pub kind: u32,
    pub reserved: u32,
    pub start: Timestamp,
    pub end: Timestamp,
    pub interval_ns: i64,
    pub lookback_ns: i64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct OutputHeader {
    pub value_type: u32,
    pub reserved: u32,
    pub start_ms: i64,
    pub end_ms: i64,
    pub step_ms: i64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Scalar {
    pub time_ms: i64,
    pub value_bits: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct StringValue {
    pub time_ms: i64,
    pub value: Bytes,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SeriesChunk {
    pub series_id: u64,
    pub labels: Labels,
    pub points: Samples,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union ChunkPayload {
    pub annotations: Annotations,
    pub scalar: Scalar,
    pub string: StringValue,
    pub series: SeriesChunk,
}
impl Default for ChunkPayload {
    fn default() -> Self {
        Self {
            series: SeriesChunk::default(),
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Chunk {
    pub kind: u32,
    pub reserved: u32,
    pub payload: ChunkPayload,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MemoryObservation {
    pub current: u64,
    pub peak: u64,
    pub acquired: u64,
    pub released: u64,
    pub overflowed: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ReferenceObservation {
    pub current: u64,
    pub peak: u64,
    pub acquired: u64,
    pub released: u64,
    pub overflowed: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CancellationReferences {
    pub engine_owned: ReferenceObservation,
    pub external: ReferenceObservation,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TemporaryStorageObservations {
    pub extent: MemoryObservation,
    pub stores: ReferenceObservation,
    pub read_calls: u64,
    pub write_calls: u64,
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub counter_overflowed: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct StatisticsTimings {
    pub eval_total_time: Duration,
    pub result_sort_time: Duration,
    pub query_preparation_time: Duration,
    pub inner_eval_time: Duration,
    pub exec_queue_time: Duration,
    pub exec_total_time: Duration,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct StatisticsSamples {
    pub total_queryable_samples: u64,
    pub samples_read: u64,
    pub peak_samples: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Statistics {
    pub timings: StatisticsTimings,
    pub samples: StatisticsSamples,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct QueryObservations {
    pub statistics: Statistics,
    pub memory: [MemoryObservation; crate::accounting::MEMORY_KINDS],
    pub cancellation_references: CancellationReferences,
    pub temporary_storage: TemporaryStorageObservations,
    pub provider_calls: u64,
    pub pages: u64,
    pub samples_fetched: u64,
    pub work: u64,
    pub output_samples: u64,
    pub elapsed: Duration,
    pub counter_overflowed: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EngineObservations {
    pub memory: [MemoryObservation; crate::accounting::MEMORY_KINDS],
    pub cancellation_references: CancellationReferences,
    pub temporary_storage: TemporaryStorageObservations,
    pub active_executions: u64,
    pub peak_executions: u64,
    pub completed_executions: u64,
    pub elapsed: Duration,
    pub counter_overflowed: u32,
    pub reserved: u32,
}
pub type BeginCallback = unsafe extern "C" fn(
    *mut c_void,
    *const OutputHeader,
    *const CallbackControl,
    *mut Bytes,
) -> u32;
pub type ChunkCallback =
    unsafe extern "C" fn(*mut c_void, *const Chunk, *const CallbackControl, *mut Bytes) -> u32;
pub type CommitCallback = unsafe extern "C" fn(
    *mut c_void,
    *const QueryObservations,
    *const CallbackControl,
    *mut Bytes,
) -> u32;
pub type AbortCallback = unsafe extern "C" fn(*mut c_void, *const AccountingContext);
pub type HistogramBucketCallback = unsafe extern "C" fn(
    *mut c_void,
    *const HistogramBucket,
    *const CallbackControl,
    *mut Bytes,
) -> u32;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Output {
    pub header: Header,
    pub context: *mut c_void,
    pub begin: Option<BeginCallback>,
    pub chunk: Option<ChunkCallback>,
    pub commit: Option<CommitCallback>,
    pub abort: Option<AbortCallback>,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Error {
    pub kind: u32,
    pub position_present: u32,
    pub position: Position,
    pub message: Bytes,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ResultInfo {
    pub header: OutputHeader,
    pub scalar_present: u32,
    pub string_present: u32,
    pub series_count: u64,
    pub annotation_count: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SeriesInfo {
    pub series_id: u64,
    pub label_count: u64,
    pub point_count: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PointInfo {
    pub time_ms: i64,
    pub value_kind: u32,
    pub reserved: u32,
    pub value_bits: u64,
}

const _: () = assert!(size_of::<usize>() <= size_of::<u64>());
const _: () = assert!(size_of::<f64>() == 8);
