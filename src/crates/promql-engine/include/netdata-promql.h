/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef NETDATA_PROMQL_H
#define NETDATA_PROMQL_H
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <float.h>
#ifdef __cplusplus
extern "C" {
static_assert(sizeof(double) == 8 && DBL_MANT_DIG == 53 && DBL_MAX_EXP == 1024, "PromQL requires binary64 double");
#else
_Static_assert(sizeof(double) == 8 && DBL_MANT_DIG == 53 && DBL_MAX_EXP == 1024, "PromQL requires binary64 double");
#endif

/* Caller contract:
 * All accessed memory is live, initialized, aligned, and contained in its allocation.
 * Out slots are writable and disjoint from inputs and other out slots.
 * Raw counts are bytes/elements, never NUL-terminated strings. Empty views may use NULL.
 * Correct Rust-owned handle types must remain live through calls; release cannot race any access.
 * Foreign callbacks return normally, without exceptions, longjmp, or Rust unwinding.
 * Calls execute synchronously on the caller thread. The engine installs no process hooks/runtime.
 * Shape/version checks cannot establish pointer liveness or prevent caller memory corruption.
 * Ordinary Rust unwind panics return INTERNAL_ERROR; abort/OOM/invalid-pointer faults are not recovered.
 */
#define NETDATA_PROMQL_ABI_VERSION UINT32_C(4)
#define NETDATA_PROMQL_OK UINT32_C(0)
#define NETDATA_PROMQL_INVALID_ARGUMENT UINT32_C(1)
#define NETDATA_PROMQL_PARSE_ERROR UINT32_C(2)
#define NETDATA_PROMQL_CANCELLED UINT32_C(3)
#define NETDATA_PROMQL_DEADLINE UINT32_C(4)
#define NETDATA_PROMQL_PROVIDER_ERROR UINT32_C(5)
#define NETDATA_PROMQL_OUTPUT_ERROR UINT32_C(6)
#define NETDATA_PROMQL_EVALUATION_ERROR UINT32_C(7)
#define NETDATA_PROMQL_INTERNAL_ERROR UINT32_C(8)
#define NETDATA_PROMQL_UNSUPPORTED UINT32_C(9)
#define NETDATA_PROMQL_END UINT32_C(10)
#define NETDATA_PROMQL_VALUE_FLOAT UINT32_C(1)
#define NETDATA_PROMQL_VALUE_INTEGER_HISTOGRAM UINT32_C(2)
#define NETDATA_PROMQL_VALUE_FLOAT_HISTOGRAM UINT32_C(3)
#define NETDATA_PROMQL_TYPE_SCALAR UINT32_C(1)
#define NETDATA_PROMQL_TYPE_VECTOR UINT32_C(2)
#define NETDATA_PROMQL_TYPE_MATRIX UINT32_C(3)
#define NETDATA_PROMQL_TYPE_STRING UINT32_C(4)
#define NETDATA_PROMQL_CHUNK_ANNOTATIONS UINT32_C(1)
#define NETDATA_PROMQL_CHUNK_SCALAR UINT32_C(2)
#define NETDATA_PROMQL_CHUNK_STRING UINT32_C(3)
#define NETDATA_PROMQL_CHUNK_SERIES UINT32_C(4)
#define NETDATA_PROMQL_REQUEST_INSTANT UINT32_C(1)
#define NETDATA_PROMQL_REQUEST_RANGE UINT32_C(2)
#define NETDATA_PROMQL_EXPERIMENTAL_FUNCTIONS UINT32_C(1)
#define NETDATA_PROMQL_BINOP_FILL_MODIFIERS UINT32_C(2)
#define NETDATA_PROMQL_EXTENDED_RANGE_SELECTORS UINT32_C(4)
#define NETDATA_PROMQL_DURATION_EXPRESSIONS UINT32_C(8)
#define NETDATA_PROMQL_START_TIMESTAMPS UINT32_C(16)
#define NETDATA_PROMQL_DELAYED_NAME_REMOVAL UINT32_C(32)
#define NETDATA_PROMQL_OPTION_FLAGS UINT32_C(63)
#define NETDATA_PROMQL_MEMORY_KINDS 8
#define NETDATA_PROMQL_MEMORY_COMPILED 0
#define NETDATA_PROMQL_MEMORY_WORKSPACE 1
#define NETDATA_PROMQL_MEMORY_RESULT 2
#define NETDATA_PROMQL_MEMORY_PROVIDER_HELD 3
#define NETDATA_PROMQL_MEMORY_BORROWED_VIEW 4
#define NETDATA_PROMQL_MEMORY_REGEX_ESTIMATE 5
#define NETDATA_PROMQL_MEMORY_SHARED_CACHE_REFERENCE 6
#define NETDATA_PROMQL_MEMORY_ACCOUNTING 7
#define NETDATA_PROMQL_ERROR_PARSE UINT32_C(1)
#define NETDATA_PROMQL_ERROR_PROVIDER UINT32_C(2)
#define NETDATA_PROMQL_ERROR_OUTPUT UINT32_C(3)
#define NETDATA_PROMQL_ERROR_STORAGE_INVALID UINT32_C(4)
#define NETDATA_PROMQL_ERROR_PREPARATION UINT32_C(5)
#define NETDATA_PROMQL_ERROR_RUNTIME UINT32_C(6)
#define NETDATA_PROMQL_ERROR_EXPANSION UINT32_C(7)
#define NETDATA_PROMQL_ERROR_EVALUATION_INVALID UINT32_C(8)
#define NETDATA_PROMQL_ERROR_UNSUPPORTED UINT32_C(9)
#define NETDATA_PROMQL_ERROR_CANCELLED UINT32_C(10)
#define NETDATA_PROMQL_ERROR_DEADLINE UINT32_C(11)
#define NETDATA_PROMQL_ERROR_INTERNAL UINT32_C(12)
#define NETDATA_PROMQL_INFO UINT32_C(0)
#define NETDATA_PROMQL_WARNING UINT32_C(1)
#define NETDATA_PROMQL_MATCH_EQUAL UINT32_C(0)
#define NETDATA_PROMQL_MATCH_NOT_EQUAL UINT32_C(1)
#define NETDATA_PROMQL_MATCH_REGEX UINT32_C(2)
#define NETDATA_PROMQL_MATCH_NOT_REGEX UINT32_C(3)
#define NETDATA_PROMQL_UNKNOWN_RESET UINT32_C(0)
#define NETDATA_PROMQL_COUNTER_RESET UINT32_C(1)
#define NETDATA_PROMQL_NOT_RESET UINT32_C(2)
#define NETDATA_PROMQL_GAUGE UINT32_C(3)
typedef struct netdata_promql_engine_t netdata_promql_engine_t;
typedef struct netdata_promql_query_t netdata_promql_query_t;
typedef struct netdata_promql_result_t netdata_promql_result_t;
typedef struct netdata_promql_report_t netdata_promql_report_t;
typedef struct netdata_promql_control_t netdata_promql_control_t;
typedef struct netdata_promql_callback_control_t netdata_promql_callback_control_t;
typedef struct netdata_promql_accounting_context_t netdata_promql_accounting_context_t;
typedef struct netdata_promql_allocation_handle_t netdata_promql_allocation_handle_t;
typedef struct netdata_promql_selector_t netdata_promql_selector_t;

typedef struct netdata_promql_header_t {
    uint32_t abi_version;
    uint32_t struct_size;
} netdata_promql_header_t;

typedef struct netdata_promql_bytes_t {
    const uint8_t * data;
    uint64_t len;
} netdata_promql_bytes_t;

typedef struct netdata_promql_label_t {
    netdata_promql_bytes_t name;
    netdata_promql_bytes_t value;
} netdata_promql_label_t;

typedef struct netdata_promql_labels_t {
    const netdata_promql_label_t * data;
    uint64_t len;
} netdata_promql_labels_t;

typedef struct netdata_promql_matcher_t {
    uint32_t kind;
    uint32_t reserved;
    netdata_promql_bytes_t name;
    netdata_promql_bytes_t value;
} netdata_promql_matcher_t;

typedef struct netdata_promql_matchers_t {
    const netdata_promql_matcher_t * data;
    uint64_t len;
} netdata_promql_matchers_t;

typedef struct netdata_promql_byte_array_t {
    const netdata_promql_bytes_t * data;
    uint64_t len;
} netdata_promql_byte_array_t;
typedef struct netdata_promql_span_t { int32_t offset; uint32_t length; } netdata_promql_span_t;

typedef struct netdata_promql_span_array_t {
    const netdata_promql_span_t * data;
    uint64_t len;
    uint32_t present;
    uint32_t reserved;
} netdata_promql_span_array_t;

typedef struct netdata_promql_i64_array_t {
    const int64_t * data;
    uint64_t len;
    uint32_t present;
    uint32_t reserved;
} netdata_promql_i64_array_t;

typedef struct netdata_promql_f64_array_t {
    const double * data;
    uint64_t len;
    uint32_t present;
    uint32_t reserved;
} netdata_promql_f64_array_t;

typedef struct netdata_promql_integer_histogram_t {
    uint32_t reset_hint;
    int32_t schema;
    uint64_t zero_threshold_bits;
    uint64_t zero_count;
    uint64_t count;
    uint64_t sum_bits;
    netdata_promql_span_array_t positive_spans;
    netdata_promql_span_array_t negative_spans;
    netdata_promql_i64_array_t positive_buckets;
    netdata_promql_i64_array_t negative_buckets;
    netdata_promql_f64_array_t custom_values;
} netdata_promql_integer_histogram_t;

typedef struct netdata_promql_float_histogram_t {
    uint32_t reset_hint;
    int32_t schema;
    uint64_t zero_threshold_bits;
    uint64_t zero_count_bits;
    uint64_t count_bits;
    uint64_t sum_bits;
    netdata_promql_span_array_t positive_spans;
    netdata_promql_span_array_t negative_spans;
    netdata_promql_f64_array_t positive_buckets;
    netdata_promql_f64_array_t negative_buckets;
    netdata_promql_f64_array_t custom_values;
} netdata_promql_float_histogram_t;

typedef struct netdata_promql_histogram_bucket_t {
    uint64_t lower_bits;
    uint64_t upper_bits;
    uint64_t count_bits;
    uint32_t lower_inclusive;
    uint32_t upper_inclusive;
} netdata_promql_histogram_bucket_t;

typedef union netdata_promql_value_payload_t {
    uint64_t float_bits;
    const netdata_promql_integer_histogram_t * integer_histogram;
    const netdata_promql_float_histogram_t * float_histogram;
} netdata_promql_value_payload_t;

typedef struct netdata_promql_value_t {
    uint32_t kind;
    uint32_t reserved;
    netdata_promql_value_payload_t payload;
} netdata_promql_value_t;

typedef struct netdata_promql_sample_t {
    int64_t time_ms;
    int64_t start_time_ms;
    netdata_promql_value_t value;
} netdata_promql_sample_t;

typedef struct netdata_promql_samples_t {
    const netdata_promql_sample_t * data;
    uint64_t len;
} netdata_promql_samples_t;

typedef struct netdata_promql_resources_t {
    uint64_t held_bytes;
    uint64_t shared_cache_bytes;
} netdata_promql_resources_t;

typedef struct netdata_promql_bounds_t {
    int64_t start_ms;
    int64_t end_ms;
} netdata_promql_bounds_t;

typedef struct netdata_promql_position_t {
    uint64_t start;
    uint64_t end;
} netdata_promql_position_t;

typedef struct netdata_promql_provider_diagnostic_t {
    uint32_t severity;
    uint32_t position_present;
    netdata_promql_position_t position;
    netdata_promql_bytes_t message;
} netdata_promql_provider_diagnostic_t;

typedef struct netdata_promql_provider_diagnostics_t {
    const netdata_promql_provider_diagnostic_t * data;
    uint64_t len;
} netdata_promql_provider_diagnostics_t;

typedef struct netdata_promql_annotation_t {
    uint32_t severity;
    uint32_t position_present;
    netdata_promql_position_t position;
    int64_t occurrences;
    netdata_promql_bytes_t raw_message;
    netdata_promql_bytes_t message;
} netdata_promql_annotation_t;

typedef struct netdata_promql_annotations_t {
    const netdata_promql_annotation_t * data;
    uint64_t len;
} netdata_promql_annotations_t;

typedef struct netdata_promql_selection_request_t {
    netdata_promql_matchers_t matchers;
    netdata_promql_bounds_t bounds;
    int64_t step_ms;
    int64_t range_ms;
    netdata_promql_bytes_t function;
    uint32_t by;
    uint32_t grouping_present;
    netdata_promql_byte_array_t grouping;
} netdata_promql_selection_request_t;

typedef struct netdata_promql_series_open_t {
    void * handle;
    netdata_promql_labels_t labels;
} netdata_promql_series_open_t;

typedef struct netdata_promql_cursor_open_t {
    void * handle;
    uint64_t max_page_samples;
} netdata_promql_cursor_open_t;

typedef struct netdata_promql_page_t {
    netdata_promql_samples_t samples;
    netdata_promql_resources_t resources;
} netdata_promql_page_t;

typedef void (*netdata_promql_close_t)(void *, void *, const netdata_promql_accounting_context_t *);

typedef netdata_promql_resources_t (*netdata_promql_resource_callback_t)(void *, void *, const netdata_promql_accounting_context_t *);

typedef uint32_t (*netdata_promql_query_open_callback_t)(void *, netdata_promql_bounds_t, const netdata_promql_callback_control_t *, void * *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_selection_open_callback_t)(void *, void *, const netdata_promql_selection_request_t *, const netdata_promql_callback_control_t *, void * *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_selection_next_callback_t)(void *, void *, const netdata_promql_callback_control_t *, netdata_promql_series_open_t *, netdata_promql_provider_diagnostics_t *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_cursor_open_callback_t)(void *, void *, netdata_promql_bounds_t, const netdata_promql_callback_control_t *, netdata_promql_cursor_open_t *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_seek_callback_t)(void *, void *, int64_t, const netdata_promql_callback_control_t *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_next_callback_t)(void *, void *, const netdata_promql_callback_control_t *, netdata_promql_page_t *, netdata_promql_resources_t *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_temporary_create_callback_t)(void *, void *, uint64_t, const netdata_promql_callback_control_t *, void * *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_temporary_read_callback_t)(void *, void *, uint64_t, uint64_t *, uint64_t, const netdata_promql_callback_control_t *, uint64_t *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_temporary_write_callback_t)(void *, void *, uint64_t, const uint64_t *, uint64_t, const netdata_promql_callback_control_t *, uint64_t *, netdata_promql_bytes_t *);

typedef struct netdata_promql_provider_t {
    netdata_promql_header_t header;
    void * context;
    netdata_promql_query_open_callback_t query_open;
    netdata_promql_close_t query_close;
    netdata_promql_resource_callback_t query_resources;
    netdata_promql_selection_open_callback_t selection_open;
    netdata_promql_close_t selection_close;
    netdata_promql_resource_callback_t selection_resources;
    netdata_promql_selection_next_callback_t selection_next;
    netdata_promql_close_t series_close;
    netdata_promql_resource_callback_t series_resources;
    netdata_promql_cursor_open_callback_t cursor_open;
    netdata_promql_close_t cursor_close;
    netdata_promql_resource_callback_t cursor_resources;
    netdata_promql_seek_callback_t cursor_seek;
    netdata_promql_next_callback_t cursor_next;
    netdata_promql_temporary_create_callback_t temporary_create;
    netdata_promql_temporary_read_callback_t temporary_read;
    netdata_promql_temporary_write_callback_t temporary_write;
    netdata_promql_resource_callback_t temporary_resources;
    netdata_promql_close_t temporary_close;
} netdata_promql_provider_t;

typedef struct netdata_promql_engine_options_t {
    netdata_promql_header_t header;
    uint32_t flags;
    uint32_t reserved;
    int64_t lookback_ms;
    int64_t no_step_subquery_interval_ms;
} netdata_promql_engine_options_t;

typedef struct netdata_promql_control_options_t {
    netdata_promql_header_t header;
    uint32_t deadline_present;
    uint32_t nanoseconds;
    uint64_t seconds;
} netdata_promql_control_options_t;

typedef struct netdata_promql_duration_t {
    uint64_t seconds;
    uint32_t nanoseconds;
    uint32_t reserved;
} netdata_promql_duration_t;

typedef struct netdata_promql_control_state_t {
    uint32_t status;
    uint32_t deadline_present;
    netdata_promql_duration_t remaining;
} netdata_promql_control_state_t;

typedef struct netdata_promql_request_t {
    netdata_promql_header_t header;
    uint32_t kind;
    uint32_t reserved;
    int64_t start_ms;
    int64_t end_ms;
    int64_t step_ms;
} netdata_promql_request_t;

typedef struct netdata_promql_timestamp_t {
    int64_t unix_seconds;
    uint32_t nanoseconds;
    uint32_t reserved;
} netdata_promql_timestamp_t;

typedef struct netdata_promql_numeric_text_t {
    uint32_t len;
    uint32_t reserved;
    uint8_t bytes[384];
} netdata_promql_numeric_text_t;

/* UTC timestamps retain full precision until the source-defined clock projections.
 * nanoseconds must be below 1000000000; all reserved fields must be zero.
 * The caller resolves lookback against its configuration, in nanoseconds.
 * INSTANT uses start; RANGE uses start/end and a positive interval_ns.
 * Positive sub-millisecond intervals reach the evaluator's zero-grid execution error.
 */
typedef struct netdata_promql_precise_request_t {
    netdata_promql_header_t header;
    uint32_t kind;
    uint32_t reserved;
    netdata_promql_timestamp_t start;
    netdata_promql_timestamp_t end;
    int64_t interval_ns;
    int64_t lookback_ns;
} netdata_promql_precise_request_t;

typedef struct netdata_promql_output_header_t {
    uint32_t value_type;
    uint32_t reserved;
    int64_t start_ms;
    int64_t end_ms;
    int64_t step_ms;
} netdata_promql_output_header_t;

typedef struct netdata_promql_scalar_t {
    int64_t time_ms;
    uint64_t value_bits;
} netdata_promql_scalar_t;

typedef struct netdata_promql_string_value_t {
    int64_t time_ms;
    netdata_promql_bytes_t value;
} netdata_promql_string_value_t;

typedef struct netdata_promql_series_chunk_t {
    uint64_t series_id;
    netdata_promql_labels_t labels;
    netdata_promql_samples_t points;
} netdata_promql_series_chunk_t;

typedef union netdata_promql_chunk_payload_t {
    netdata_promql_annotations_t annotations;
    netdata_promql_scalar_t scalar;
    netdata_promql_string_value_t string;
    netdata_promql_series_chunk_t series;
} netdata_promql_chunk_payload_t;

typedef struct netdata_promql_chunk_t {
    uint32_t kind;
    uint32_t reserved;
    netdata_promql_chunk_payload_t payload;
} netdata_promql_chunk_t;

typedef struct netdata_promql_memory_observation_t {
    uint64_t current;
    uint64_t peak;
    uint64_t acquired;
    uint64_t released;
    uint32_t overflowed;
    uint32_t reserved;
} netdata_promql_memory_observation_t;

typedef struct netdata_promql_reference_observation_t {
    uint64_t current;
    uint64_t peak;
    uint64_t acquired;
    uint64_t released;
    uint32_t overflowed;
    uint32_t reserved;
} netdata_promql_reference_observation_t;

typedef struct netdata_promql_cancellation_references_t {
    netdata_promql_reference_observation_t engine_owned;
    netdata_promql_reference_observation_t external;
} netdata_promql_cancellation_references_t;

typedef struct netdata_promql_temporary_storage_observations_t {
    netdata_promql_memory_observation_t extent;
    netdata_promql_reference_observation_t stores;
    uint64_t read_calls;
    uint64_t write_calls;
    uint64_t read_bytes;
    uint64_t written_bytes;
    uint32_t counter_overflowed;
    uint32_t reserved;
} netdata_promql_temporary_storage_observations_t;

typedef struct netdata_promql_statistics_timings_t {
    netdata_promql_duration_t eval_total_time;
    netdata_promql_duration_t result_sort_time;
    netdata_promql_duration_t query_preparation_time;
    netdata_promql_duration_t inner_eval_time;
    netdata_promql_duration_t exec_queue_time;
    netdata_promql_duration_t exec_total_time;
} netdata_promql_statistics_timings_t;

typedef struct netdata_promql_statistics_samples_t {
    uint64_t total_queryable_samples;
    uint64_t samples_read;
    uint64_t peak_samples;
} netdata_promql_statistics_samples_t;

typedef struct netdata_promql_statistics_t {
    netdata_promql_statistics_timings_t timings;
    netdata_promql_statistics_samples_t samples;
} netdata_promql_statistics_t;

/* Finalized before commit. Timings observe Rust phases; peak counts real owning sample values.
 * Native histogram counts use pinned Go AMD64 float-equivalent units. Queue time is zero.
 * Fetched pages, byte allocations and work remain separate observations below. */
typedef struct netdata_promql_query_observations_t {
    netdata_promql_statistics_t statistics;
    netdata_promql_memory_observation_t memory[NETDATA_PROMQL_MEMORY_KINDS];
    netdata_promql_cancellation_references_t cancellation_references;
    netdata_promql_temporary_storage_observations_t temporary_storage;
    uint64_t provider_calls;
    uint64_t pages;
    uint64_t samples_fetched;
    uint64_t work;
    uint64_t output_samples;
    netdata_promql_duration_t elapsed;
    uint32_t counter_overflowed;
    uint32_t reserved;
} netdata_promql_query_observations_t;

typedef struct netdata_promql_engine_observations_t {
    netdata_promql_memory_observation_t memory[NETDATA_PROMQL_MEMORY_KINDS];
    netdata_promql_cancellation_references_t cancellation_references;
    netdata_promql_temporary_storage_observations_t temporary_storage;
    uint64_t active_executions;
    uint64_t peak_executions;
    uint64_t completed_executions;
    netdata_promql_duration_t elapsed;
    uint32_t counter_overflowed;
    uint32_t reserved;
} netdata_promql_engine_observations_t;

typedef uint32_t (*netdata_promql_begin_callback_t)(void *, const netdata_promql_output_header_t *, const netdata_promql_callback_control_t *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_chunk_callback_t)(void *, const netdata_promql_chunk_t *, const netdata_promql_callback_control_t *, netdata_promql_bytes_t *);

typedef uint32_t (*netdata_promql_commit_callback_t)(void *, const netdata_promql_query_observations_t *, const netdata_promql_callback_control_t *, netdata_promql_bytes_t *);
typedef uint32_t (*netdata_promql_histogram_bucket_callback_t)(void *, const netdata_promql_histogram_bucket_t *, const netdata_promql_callback_control_t *, netdata_promql_bytes_t *);
typedef uint32_t (*netdata_promql_sort_compare_callback_t)(void *, uint64_t left, uint64_t right, const netdata_promql_callback_control_t *, int32_t *ordering, netdata_promql_bytes_t *);

typedef void (*netdata_promql_abort_callback_t)(void *, const netdata_promql_accounting_context_t *);

typedef struct netdata_promql_output_t {
    netdata_promql_header_t header;
    void * context;
    netdata_promql_begin_callback_t begin;
    netdata_promql_chunk_callback_t chunk;
    netdata_promql_commit_callback_t commit;
    netdata_promql_abort_callback_t abort;
} netdata_promql_output_t;

typedef struct netdata_promql_error_t {
    uint32_t kind;
    uint32_t position_present;
    netdata_promql_position_t position;
    netdata_promql_bytes_t message;
} netdata_promql_error_t;

typedef struct netdata_promql_result_info_t {
    netdata_promql_output_header_t header;
    uint32_t scalar_present;
    uint32_t string_present;
    uint64_t series_count;
    uint64_t annotation_count;
} netdata_promql_result_info_t;

typedef struct netdata_promql_series_info_t {
    uint64_t series_id;
    uint64_t label_count;
    uint64_t point_count;
} netdata_promql_series_info_t;

typedef struct netdata_promql_point_info_t {
    int64_t time_ms;
    uint32_t value_kind;
    uint32_t reserved;
    uint64_t value_bits;
} netdata_promql_point_info_t;

/* Only active value/union fields are inspected. Optional arrays distinguish absent from present-empty.
 * Absent arrays have len == 0; reserved fields are zero. Reset hints preserve any uint8 value.
 * Array elements are double objects, not uint64 objects; memcpy preserves bits without type-punning.
 */
static inline uint64_t netdata_promql_double_bits(double value) {
    uint64_t bits; memcpy(&bits, &value, sizeof(bits)); return bits;
}
static inline double netdata_promql_double_from_bits(uint64_t bits) {
    double value; memcpy(&value, &bits, sizeof(value)); return value;
}

/* Provider contract:
 * query_open receives inclusive source-computed data bounds.
 * Successful opens (including NULL opaque tokens) receive exactly one paired close.
 * Failed opens clean their partial resources before returning; they receive no engine close.
 * Selection may conservatively prefilter; Rust remains authoritative for matching.
 * Series metadata is sorted by unique raw label names and lives until series_close,
 * independently of selection_close. Successful selection_next transfers a retained series.
 * Each materialized label name/value is less than 16777216 bytes; this explicitly excludes
 * the pinned stringlabels encoder's broken equality boundary. Total labelset bytes are not capped.
 * Terminal diagnostics are returned only with END/error and live until selection_close.
 * Each cursor supports repeatable seek/read input, a positive bounded maximum page, and strictly
 * increasing samples. Neither complete histories nor raw values across all metrics may be retained.
 * Page/histogram/array views live until the same cursor's next operation or close; other cursors
 * cannot invalidate them. Scheduled selectors may advance within a retained page without a callback.
 * Errors are observed at actual physical operations; callbacks have no fixed per-query call roster.
 * next publishes resource snapshots on item, END and error.
 * Resources exclude descendants, directly accounted adapter buffers, and bytes covered by allocation handles.
 * Snapshots describe residual owner memory; never subtract the query's total leased bytes from each owner.
 * Shared cache is external. BorrowedView measures exposed views separately from physical allocation storage.
 * Returned error bytes stay live until the next callback; the engine copies them first.
 * Callback control is borrowed only for that callback. Poll it cooperatively during blocking work.
 * Resource/close callbacks receive the same actual-query accounting capability without a cancellation contract.
 * temporary_create/read/write/resources/close are either all present or all absent. Missing backing is an
 * error only when evaluation needs it. create receives the query token and logical 64-bit word count (8N bytes).
 * Words hold exact float bits or opaque computed records; the provider must not interpret their contents.
 * Offsets and transfer lengths count uint64 words. Buffers live only during the callback. OK requires
 * the exact requested transfer count; short/oversized transfers are errors. Preserve every bit, including NaNs.
 * Providers choose backing and clean it on close; the core never creates files or retains a complete raw window.
 * Temporary extent/store counts are logical backing, separate from memory. Traffic reports attempted calls and
 * accepted logical bytes, excluding partially failed transfers, physical blocks and kernel-cache residency.
 */

/* Output contract:
 * begin/chunk are provisional; commit makes output final; abort discards provisional output.
 * After a valid output table is registered, any failure invokes abort exactly once, even before begin.
 * Borrowed chunk descriptors/payloads expire when their callback returns; copy retained values.
 * Returned errors win over concurrent cancellation. No fallible construction follows successful commit.
 * abort is infallible. All callbacks and context remain valid until execution returns.
 * abort receives actual-query accounting even before begin when a valid query was established; otherwise NULL.
 * All caps/admission/implicit deadlines are disabled. Owned collection may grow with derived output.
 */

uint32_t netdata_promql_abi_version(void);
/* Every caller must check ABI_VERSION before using tables or observation outputs; ABI2/ABI3 callers must rebuild.
 * Observation outputs have no size prefix and require the complete ABI4 layout.
 */
uint32_t netdata_promql_engine_options_default(netdata_promql_engine_options_t *out);
uint32_t netdata_promql_control_options_default(netdata_promql_control_options_t *out);
/* NULL options select native defaults; a NULL optional call control creates a private engine-owned
 * uncancelled token with no deadline. Explicit deadlines activate once at control creation and never rearm.
 */
uint32_t netdata_promql_engine_create(const netdata_promql_engine_options_t *, netdata_promql_engine_t **);
uint32_t netdata_promql_engine_release(netdata_promql_engine_t *);
uint32_t netdata_promql_engine_observations(const netdata_promql_engine_t *, netdata_promql_engine_observations_t *);
uint32_t netdata_promql_control_create(const netdata_promql_engine_t *, const netdata_promql_control_options_t *, netdata_promql_control_t **);
uint32_t netdata_promql_control_cancel(const netdata_promql_control_t *);
uint32_t netdata_promql_control_state(const netdata_promql_control_t *, netdata_promql_control_state_t *);
uint32_t netdata_promql_control_release(netdata_promql_control_t *);
uint32_t netdata_promql_callback_control_state(const netdata_promql_callback_control_t *, netdata_promql_control_state_t *);
/* Accounting contexts are borrowed only during the current callback and must not be retained or released.
 * Acquired handles retain their actual creator query independently of callback/query/report/engine lifetimes.
 * Report requested live allocation bytes, excluding allocator-internal overhead; these calls do not allocate
 * or free foreign storage. Resize reports the new live size of one allocation. Two overlapping allocations
 * require two handles, including allocate/copy/free replacement. Release after the foreign storage is freed.
 * Valid payload kinds: COMPILED, WORKSPACE, RESULT, PROVIDER_HELD, BORROWED_VIEW. Shared cache/pool views
 * use BORROWED_VIEW and do not imply physical ownership. Handle wrapper bytes are charged precisely to
 * ACCOUNTING. Observation/bootstrap categories are invalid payload kinds. Zero-byte handles are valid.
 * Acquire initializes its out handle to NULL on failure. Release must not race any use of the handle.
 * Normal callbacks obtain accounting through callback_accounting; cleanup receives it directly.
 */
uint32_t netdata_promql_callback_accounting(const netdata_promql_callback_control_t *, const netdata_promql_accounting_context_t **);
uint32_t netdata_promql_allocation_acquire(const netdata_promql_accounting_context_t *, uint32_t kind, uint64_t bytes, netdata_promql_allocation_handle_t **);
uint32_t netdata_promql_allocation_resize(const netdata_promql_allocation_handle_t *, uint64_t bytes);
uint32_t netdata_promql_allocation_release(netdata_promql_allocation_handle_t *);
uint32_t netdata_promql_compile(const netdata_promql_engine_t *, netdata_promql_bytes_t, const netdata_promql_control_t *, netdata_promql_query_t **, netdata_promql_report_t **);
uint32_t netdata_promql_query_release(netdata_promql_query_t *);
uint32_t netdata_promql_query_source(const netdata_promql_query_t *, netdata_promql_bytes_t *);
uint32_t netdata_promql_query_type(const netdata_promql_query_t *, uint32_t *);
uint32_t netdata_promql_query_compilation_observations(const netdata_promql_query_t *, netdata_promql_query_observations_t *);
uint32_t netdata_promql_execute(const netdata_promql_query_t *, const netdata_promql_provider_t *, const netdata_promql_request_t *, const netdata_promql_control_t *, const netdata_promql_output_t *, netdata_promql_report_t **);
uint32_t netdata_promql_collect(const netdata_promql_query_t *, const netdata_promql_provider_t *, const netdata_promql_request_t *, const netdata_promql_control_t *, netdata_promql_result_t **, netdata_promql_report_t **);
uint32_t netdata_promql_execute_precise(const netdata_promql_query_t *, const netdata_promql_provider_t *, const netdata_promql_precise_request_t *, const netdata_promql_control_t *, const netdata_promql_output_t *, netdata_promql_report_t **);
uint32_t netdata_promql_collect_precise(const netdata_promql_query_t *, const netdata_promql_provider_t *, const netdata_promql_precise_request_t *, const netdata_promql_control_t *, netdata_promql_result_t **, netdata_promql_report_t **);
/* Numeric protocol parsing uses caller control and returns an independently owned report.
 * Parser errors have no PromQL source position. Successful values retain no borrowed input.
 * Formatting returns ASCII without JSON quotes or a trailing NUL; len selects the valid bytes.
 */
uint32_t netdata_promql_api_parse_time(const netdata_promql_engine_t *, netdata_promql_bytes_t, const netdata_promql_control_t *, netdata_promql_timestamp_t *, netdata_promql_report_t **);
uint32_t netdata_promql_api_parse_duration(const netdata_promql_engine_t *, netdata_promql_bytes_t, const netdata_promql_control_t *, int64_t *, netdata_promql_report_t **);
/* Empty/zero means no requested limit. Parse signed decimal using the pinned AMD64 int64 contract;
 * negative nonzero values are rejected. A requested limit remains caller output policy, not engine enforcement.
 */
uint32_t netdata_promql_api_parse_limit(const netdata_promql_engine_t *, netdata_promql_bytes_t, const netdata_promql_control_t *, int64_t *, netdata_promql_report_t **);
uint32_t netdata_promql_api_float_text(uint64_t value_bits, netdata_promql_numeric_text_t *);
uint32_t netdata_promql_api_scalar_float_text(uint64_t value_bits, netdata_promql_numeric_text_t *);
uint32_t netdata_promql_api_timestamp_text(int64_t time_ms, netdata_promql_numeric_text_t *);
uint32_t netdata_promql_api_scalar_timestamp_text(int64_t time_ms, netdata_promql_numeric_text_t *);
/* Allocation-free Go17 Unicode IsPrint classification for caller-composed protocol diagnostics.
 * All u32 inputs are accepted; invalid Unicode values are not printable. Output is0 or1. */
uint32_t netdata_promql_api_rune_is_print(uint32_t value, uint32_t *printable);
/* Synchronous helper usable only with a live enclosing callback control. The descriptor and
 * its arrays are borrowed for this call. Buckets include zero counts; consumers choose filtering.
 * A child report owns returned error bytes: retain it until the enclosing callback returns,
 * or copy those bytes before releasing it. The enclosing output transaction owns abort.
 */
uint32_t netdata_promql_callback_histogram_buckets(const netdata_promql_float_histogram_t *, const netdata_promql_callback_control_t *, netdata_promql_histogram_bucket_callback_t, void *, netdata_promql_report_t **);
/* Sort caller-owned indices with the pinned Go algorithm, including equal-key provenance. The index array is
 * exclusively borrowed and mutated during this synchronous call: the comparator MUST NOT access it, directly
 * or through aliases. Compare immutable metadata identified by the supplied index values; return negative,
 * zero or positive ordering on OK. Report/index storage must not overlap. Count zero accepts a NULL array.
 * Uses the enclosing callback's control/account; observes array exposure and sort work, without copying indices.
 * Failure may leave a partial permutation. This helper owns its child report and no surrounding output transaction;
 * the caller handles abort, and keeps child error bytes live until copied or the enclosing callback returns.
 */
uint32_t netdata_promql_callback_sort_indices(uint64_t *indices, uint64_t count, const netdata_promql_callback_control_t *, netdata_promql_sort_compare_callback_t, void *, netdata_promql_report_t **);
/* Selector syntax is parsed independently of the later API empty-matcher guard.
 * Source/matcher views remain valid until selector_release. Labels passed to matches must be
 * unique and sorted by their raw name bytes. The empty-matcher guard applies to every supplied set.
 */
uint32_t netdata_promql_selector_compile(const netdata_promql_engine_t *, netdata_promql_bytes_t, const netdata_promql_control_t *, netdata_promql_selector_t **, netdata_promql_report_t **);
uint32_t netdata_promql_selector_release(netdata_promql_selector_t *);
uint32_t netdata_promql_selector_matcher_count(const netdata_promql_selector_t *, uint64_t *);
uint32_t netdata_promql_selector_matcher(const netdata_promql_selector_t *, uint64_t index, netdata_promql_matcher_t *);
uint32_t netdata_promql_selector_nonempty(const netdata_promql_selector_t *, const netdata_promql_control_t *, uint32_t *);
uint32_t netdata_promql_selector_matches(const netdata_promql_selector_t *, netdata_promql_labels_t, const netdata_promql_control_t *, uint32_t *);
/* Metadata-only operation: bounds go directly to query/selection callbacks; no sample cursor or float backing is opened.
 * With zero selectors, enumerate all metadata; otherwise visit each supplied selector in order. The caller owns
 * union/deduplication, final ordering and endpoint limits. Each series is validated and released after its callback.
 * Output begins as VECTOR and emits label-only SERIES chunks with increasing IDs and zero points. Returning END
 * from such a row requests successful early stop; this is distinct from cancellation and ordinary execute output.
 * All other output/error/cleanup and query allocation-handle contracts apply. Commit follows provider cleanup.
 * Selector handles remain live through this synchronous call. API empty-matcher guards remain caller validation.
 */
uint32_t netdata_promql_discover(const netdata_promql_engine_t *, const netdata_promql_provider_t *, netdata_promql_bounds_t,
    const netdata_promql_selector_t *const *, uint64_t selector_count, const netdata_promql_control_t *,
    const netdata_promql_output_t *, netdata_promql_report_t **);
uint32_t netdata_promql_report_release(netdata_promql_report_t *);
/* Invoke an infallible caller callback with this report's actual borrowed account. This supports accounted
 * caller preparation/error-body construction outside execute callbacks. Keep the report live throughout the call;
 * borrow the account only in the callback. Retained foreign allocations use independently owned handles.
 * The helper has no cancellation/commit semantics; the caller owns its construction status and publication.
 */
uint32_t netdata_promql_report_accounted(const netdata_promql_report_t *, netdata_promql_close_t, void *context, void *handle);
uint32_t netdata_promql_report_status(const netdata_promql_report_t *, uint32_t *);
uint32_t netdata_promql_report_error_count(const netdata_promql_report_t *, uint64_t *);
uint32_t netdata_promql_report_error(const netdata_promql_report_t *, uint64_t, netdata_promql_error_t *);
uint32_t netdata_promql_report_annotation_count(const netdata_promql_report_t *, uint64_t *);
uint32_t netdata_promql_report_annotation(const netdata_promql_report_t *, uint64_t, netdata_promql_annotation_t *);
uint32_t netdata_promql_report_observations(const netdata_promql_report_t *, netdata_promql_query_observations_t *);
uint32_t netdata_promql_report_live_observations(const netdata_promql_report_t *, netdata_promql_query_observations_t *);
uint32_t netdata_promql_result_release(netdata_promql_result_t *);
uint32_t netdata_promql_result_info(const netdata_promql_result_t *, netdata_promql_result_info_t *);
uint32_t netdata_promql_result_scalar(const netdata_promql_result_t *, netdata_promql_scalar_t *);
uint32_t netdata_promql_result_string(const netdata_promql_result_t *, netdata_promql_string_value_t *);
uint32_t netdata_promql_result_series_info(const netdata_promql_result_t *, uint64_t, netdata_promql_series_info_t *);
uint32_t netdata_promql_result_label(const netdata_promql_result_t *, uint64_t, uint64_t, netdata_promql_label_t *);
uint32_t netdata_promql_result_point(const netdata_promql_result_t *, uint64_t, uint64_t, netdata_promql_point_info_t *);
uint32_t netdata_promql_result_histogram(const netdata_promql_result_t *, uint64_t, uint64_t, netdata_promql_float_histogram_t *);
uint32_t netdata_promql_result_annotation(const netdata_promql_result_t *, uint64_t, netdata_promql_annotation_t *);
uint32_t netdata_promql_result_live_observations(const netdata_promql_result_t *, netdata_promql_query_observations_t *);

/* Output handles are initialized to NULL after their slots validate. Shape errors may have no report.
 * Completion observations are fixed; live observations follow independently retained ownership.
 * Result/report/query views expire only at their respective paired release. C must not free Rust memory.
 * Engine elapsed sums completed execution-attempt durations (including overlaps), not CPU/uptime/compile time.
 * Compilation elapsed belongs to the compilation account. Cooperative deadlines do not preempt callbacks.
 * Borrowed/provider-held/shared-cache categories are observations, not a deduplicated process RSS total.
 * Regex backend/table memory is explicitly estimated; no total including it is an exact allocation bound.
 */
#ifdef __cplusplus
}
#endif
#endif
