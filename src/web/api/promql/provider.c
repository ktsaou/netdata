// SPDX-License-Identifier: GPL-3.0-or-later

#include "provider-internal.h"

#define PROMQL_STORAGE_PAGE_SAMPLES 1024U
#define PROMQL_STALE_BITS UINT64_C(0x7ff0000000000002)

typedef struct {
    PROMQL_PHYSICAL *physical;
    netdata_promql_bounds_t bounds;
    RRDENG_READER *reader;
    RRDENG_DEMAND_PAGE page;
    uint32_t reader_span;
    time_t next, wanted, last;
    double counter;
    bool projection_started, ended;
    uint32_t error_status;
    netdata_promql_sample_t samples[PROMQL_STORAGE_PAGE_SAMPLES];
} PROMQL_CURSOR;

static uint32_t temporary_failure(PROMQL_PROVIDER_CONTEXT *context, PROMQL_READ_FRAME *frame, int cause,
                                   netdata_promql_bytes_t *error);

static void counted(PROMQL_PROVIDER_CONTEXT *context, uint64_t *counter, uint64_t amount) {
    if(amount > UINT64_MAX - *counter) { *counter = UINT64_MAX; context->counters_overflowed = true; }
    else *counter += amount;
}

static uint32_t query_open(void *context, netdata_promql_bounds_t bounds,
                            const netdata_promql_callback_control_t *control, void **out,
                            netdata_promql_bytes_t *error) {
    PROMQL_READ_FRAME frame;
    promql_read_operation(&frame, control);
    if(!promql_read_poll(&frame)) return frame.interrupted;
    PROMQL_PROVIDER_QUERY *query = promql_malloc(frame.account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, sizeof(*query));
    if(!query) return promql_provider_error(context, "cannot account query provider", error);
    query->context = context;
    query->bounds = bounds;
    query->empty = !promql_storage_interval(bounds, &query->first, &query->last);
    *out = query;
    return NETDATA_PROMQL_OK;
}

static void query_close(void *context __maybe_unused, void *handle,
                         const netdata_promql_accounting_context_t *account __maybe_unused) {
    PROMQL_PROVIDER_QUERY *query = handle;
    internal_fatal(query->used || query->hosts, "PromQL query closed before its provider descendants");
    promql_free(query->physical);
    promql_checkpoint_backing_close(&query->checkpoints);
    promql_free(query);
}

static netdata_promql_resources_t resources(void *context __maybe_unused, void *handle __maybe_unused,
                                            const netdata_promql_accounting_context_t *account __maybe_unused) {
    // Allocation handles already include every query-owned buffer and wrapper.
    return (netdata_promql_resources_t){0};
}

static uint32_t selection_open(void *context, void *query, const netdata_promql_selection_request_t *request,
                                const netdata_promql_callback_control_t *control, void **out,
                                netdata_promql_bytes_t *error) {
    PROMQL_READ_FRAME frame;
    promql_read_operation(&frame, control);
    if(!promql_read_poll(&frame)) return frame.interrupted;
    PROMQL_SELECTION *selection = promql_malloc(frame.account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, sizeof(*selection));
    if(!selection) return promql_provider_error(context, "cannot account metadata selection", error);
    selection->query = query;
    if(!promql_selection_prefilters(selection, request->matchers, frame.account)) {
        promql_selection_destroy(selection);
        return promql_provider_error(context, "cannot account metadata prefilters", error);
    }
    *out = selection;
    return NETDATA_PROMQL_OK;
}

static void selection_close(void *context __maybe_unused, void *handle,
                             const netdata_promql_accounting_context_t *account __maybe_unused) {
    promql_selection_destroy(handle);
}

static uint32_t selection_next(void *context __maybe_unused, void *handle,
                                const netdata_promql_callback_control_t *control, netdata_promql_series_open_t *out,
                                netdata_promql_provider_diagnostics_t *diagnostics, netdata_promql_bytes_t *error) {
    *diagnostics = (netdata_promql_provider_diagnostics_t){0};
    PROMQL_READ_FRAME frame;
    promql_read_operation(&frame, control);
    if(!promql_read_poll(&frame)) return frame.interrupted;
    return promql_metadata_next(handle, &frame, out, error);
}

static void series_close(void *context __maybe_unused, void *handle,
                          const netdata_promql_accounting_context_t *account __maybe_unused) {
    PROMQL_SERIES *series = handle;
    promql_physical_release(series->physical);
    promql_free(series);
}

static uint32_t cursor_restore(PROMQL_CURSOR *cursor, int64_t wanted, PROMQL_READ_FRAME *frame,
                               netdata_promql_bytes_t *error) {
    PROMQL_PHYSICAL *physical = cursor->physical;
    wanted = MAX(MAX(wanted, promql_ceil_seconds(cursor->bounds.start_ms)), INT64_C(1));
    bool ended = cursor->ended;
    if(physical->query->empty || wanted > cursor->last) { cursor->ended = true; return NETDATA_PROMQL_OK; }
    const PROMQL_CHECKPOINT *last = promql_checkpoints_last(physical->checkpoints);
    if(!ended && cursor->projection_started && wanted >= cursor->next &&
       (wanted == cursor->next || !last || last->next <= cursor->next)) {
        cursor->wanted = (time_t)wanted;
        if(!physical->incremental) cursor->next = cursor->wanted;
        return NETDATA_PROMQL_OK;
    }
    PROMQL_CHECKPOINT checkpoint;
    bool found;
    int cause;
    if(!promql_checkpoints_find(physical->checkpoints, &physical->query->checkpoints, wanted,
                                &checkpoint, &found, frame, &cause)) {
        cursor->error_status = temporary_failure(physical->query->context, frame, cause, error);
        return cursor->error_status;
    }
    cursor->wanted = (time_t)wanted;
    cursor->ended = false;
    if(found && (ended || !cursor->projection_started || cursor->wanted < cursor->next ||
                 checkpoint.next > cursor->next)) {
        cursor->next = (time_t)checkpoint.next;
        cursor->counter = netdata_promql_double_from_bits(checkpoint.counter_bits);
        cursor->projection_started = true;
    }
    else if(ended || !cursor->projection_started || cursor->wanted < cursor->next) {
        if(physical->checkpoints) {
            cursor->next = (time_t)checkpoint.next;
            cursor->counter = netdata_promql_double_from_bits(checkpoint.counter_bits);
            cursor->projection_started = true;
        }
        else {
            cursor->next = physical->query->first;
            cursor->counter = 0.0;
            cursor->projection_started = false;
        }
    }
    if(!physical->incremental) cursor->next = MAX(cursor->wanted, (time_t)1);
    return NETDATA_PROMQL_OK;
}

static void cursor_advance(PROMQL_CURSOR *cursor, time_t timestamp, uint32_t increment) {
    if(nd_time_t_add_compare(timestamp, increment, cursor->last) > 0) {
        cursor->next = cursor->last;
        cursor->ended = true;
    }
    else cursor->next = nd_time_t_add_saturating(timestamp, increment);
}

static uint32_t cursor_open(void *context, void *handle, netdata_promql_bounds_t bounds,
                             const netdata_promql_callback_control_t *control, netdata_promql_cursor_open_t *out,
                             netdata_promql_bytes_t *error) {
    PROMQL_READ_FRAME frame;
    promql_read_operation(&frame, control);
    if(!promql_read_poll(&frame)) return frame.interrupted;
    PROMQL_CURSOR *cursor = promql_malloc(frame.account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, sizeof(*cursor));
    if(!cursor) return promql_provider_error(context, "cannot account bounded sample cursor", error);
    cursor->physical = ((PROMQL_SERIES *)handle)->physical;
    cursor->physical->references++;
    cursor->bounds = bounds;
    cursor->last = (time_t)MAX(INT64_C(0),
        MIN(promql_floor_seconds(bounds.end_ms), (int64_t)cursor->physical->query->last));
    cursor->reader_span = UINT32_MAX;
    uint32_t status = cursor_restore(cursor, promql_ceil_seconds(bounds.start_ms), &frame, error);
    if(status != NETDATA_PROMQL_OK) {
        promql_physical_release(cursor->physical);
        promql_free(cursor);
        return status;
    }
    *out = (netdata_promql_cursor_open_t){cursor, PROMQL_STORAGE_PAGE_SAMPLES};
    return NETDATA_PROMQL_OK;
}

static void cursor_close(void *context __maybe_unused, void *handle,
                          const netdata_promql_accounting_context_t *account __maybe_unused) {
    PROMQL_CURSOR *cursor = handle;
    rrdeng_reader_close(cursor->reader, NULL);
    promql_physical_release(cursor->physical);
    promql_free(cursor);
}

static uint32_t cursor_seek(void *context __maybe_unused, void *handle, int64_t timestamp,
                             const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    PROMQL_READ_FRAME frame;
    promql_read_operation(&frame, control);
    if(!promql_read_poll(&frame)) return frame.interrupted;
    PROMQL_CURSOR *cursor = handle;
    if(cursor->error_status) return cursor->error_status;
    return cursor_restore(cursor, promql_ceil_seconds(timestamp), &frame, error);
}

static uint32_t reader_error(PROMQL_CURSOR *cursor, PROMQL_READ_FRAME *frame, RRDENG_READ_ERROR failure,
                              netdata_promql_bytes_t *error) {
    PROMQL_PROVIDER_CONTEXT *context = cursor->physical->query->context;
    if(failure.code == RRDENG_READ_CANCELLED && frame->interrupted != NETDATA_PROMQL_OK)
        cursor->error_status = frame->interrupted;
    else {
        static const char *const causes[] = {"unknown", "cancelled", "I/O", "extent", "checksum", "decompression",
                                             "descriptor", "payload", "expired retention", "allocation accounting"};
        const char *cause = (unsigned)failure.code < sizeof(causes) / sizeof(causes[0]) ? causes[failure.code] : "unknown";
        snprintfz(context->error, sizeof(context->error) - 1, "dbengine %s error (file %u, block %u, slot %u, errno %d)",
                  cause, failure.file, failure.block, failure.slot, failure.system_errno);
        *error = promql_bytes(context->error);
        cursor->error_status = NETDATA_PROMQL_PROVIDER_ERROR;
    }
    return cursor->error_status;
}

static bool checkpoint_add(PROMQL_CURSOR *cursor, PROMQL_READ_FRAME *frame, int *cause) {
    PROMQL_PHYSICAL *physical = cursor->physical;
    PROMQL_CHECKPOINT checkpoint = {.next = (int64_t)cursor->next,
                                    .counter_bits = netdata_promql_double_bits(cursor->counter)};
    return promql_checkpoints_append(&physical->checkpoints, &physical->query->checkpoints,
                                      physical->query->context->backing_directory, checkpoint, frame, cause);
}

static netdata_promql_sample_t sample(time_t timestamp, uint64_t bits) {
    return (netdata_promql_sample_t){.time_ms = (int64_t)timestamp * 1000,
        .value = {.kind = NETDATA_PROMQL_VALUE_FLOAT, .payload.float_bits = bits}};
}

static uint32_t cursor_next(void *context __maybe_unused, void *handle,
                             const netdata_promql_callback_control_t *control, netdata_promql_page_t *out,
                             netdata_promql_resources_t *observations, netdata_promql_bytes_t *error) {
    *out = (netdata_promql_page_t){0};
    *observations = (netdata_promql_resources_t){0};
    PROMQL_CURSOR *cursor = handle;
    if(cursor->error_status) return cursor->error_status;
    if(cursor->ended) return NETDATA_PROMQL_END;
    PROMQL_PHYSICAL *physical = cursor->physical;
    PROMQL_PROVIDER_CONTEXT *owner = physical->query->context;
    PROMQL_READ_FRAME frame;
    RRDENG_READ_OPERATION operation = promql_read_operation(&frame, control);
    size_t count = 0;
    while(!cursor->ended && count < PROMQL_STORAGE_PAGE_SAMPLES && cursor->next <= cursor->last) {
        if(!promql_read_poll(&frame)) { cursor->error_status = frame.interrupted; return frame.interrupted; }
        uint32_t span_index = 0;
        while(span_index < physical->span_count && physical->spans[span_index].last < cursor->next) span_index++;
        if(span_index == physical->span_count) break;
        RRDENG_READ_TIER_SPAN span = physical->spans[span_index];
        cursor->next = MAX(cursor->next, span.first);
        if(cursor->next > cursor->last) break;
        RRDENG_READ_ERROR failure;
        if(cursor->reader_span != span_index) {
            rrdeng_reader_close(cursor->reader, &operation);
            cursor->reader = NULL;
            cursor->page = (RRDENG_DEMAND_PAGE){0};
            if(!rrdeng_reader_open(physical->metrics[span.tier], &physical->promises[span.tier], span.first, span.last,
                                   &operation, &cursor->reader, &failure))
                return reader_error(cursor, &frame, failure, error);
            cursor->reader_span = span_index;
        }
        RRDENG_DEMAND_PAGE page;
        RRDENG_DEMAND_STATUS status = rrdeng_reader_acquire(cursor->reader, cursor->next, &operation, &page, &failure);
        if(status == RRDENG_DEMAND_ERROR) return reader_error(cursor, &frame, failure, error);
        if(status == RRDENG_DEMAND_END) { cursor_advance(cursor, span.last, 1); continue; }
        if(status == RRDENG_DEMAND_GAP) {
            time_t marker = MAX(page.first, cursor->wanted);
            if(marker <= MIN(page.last, cursor->last)) cursor->samples[count++] = sample(marker, PROMQL_STALE_BITS);
            cursor_advance(cursor, page.last, 1);
            continue;
        }
        bool new_page = cursor->page.data != page.data || cursor->page.first != page.first;
        cursor->page = page;
        if(new_page) counted(owner, &owner->storage_pages[span.tier], 1);
        uint32_t position = cursor->next > page.first ?
            (uint32_t)(((uint64_t)(cursor->next - page.first) + page.cadence - 1) / page.cadence) : 0;
        if(position >= page.entries) { cursor_advance(cursor, page.last, 1); continue; }
        cursor->next = page.first + (time_t)position * page.cadence;
        if(physical->incremental && !cursor->projection_started) {
            bool native_counter = rrdeng_page_type_is_counter(pgd_type(page.data));
            physical->anchored = native_counter;
            physical->basis_selected = true;
            cursor->counter = 0.0;
            if(native_counter) {
                if(!rrdeng_reader_counter_before(cursor->reader, position, &cursor->counter, &operation, &failure))
                    return reader_error(cursor, &frame, failure, error);
                if(position) counted(owner, &owner->storage_points[span.tier], 1);
            }
            cursor->projection_started = true;
        }
        if(physical->incremental && (new_page || !physical->checkpoints)) {
            int cause;
            if(!checkpoint_add(cursor, &frame, &cause)) {
                cursor->error_status = temporary_failure(owner, &frame, cause, error);
                return cursor->error_status;
            }
        }
        for(; !cursor->ended && position < page.entries && count < PROMQL_STORAGE_PAGE_SAMPLES; position++) {
            time_t timestamp = page.first + (time_t)position * page.cadence;
            if(timestamp > span.last || timestamp > cursor->last) break;
            STORAGE_POINT point = STORAGE_POINT_UNSET;
            if(!rrdeng_reader_point(cursor->reader, position, &point, &operation, &failure))
                return reader_error(cursor, &frame, failure, error);
            counted(owner, &owner->storage_points[span.tier], 1);
            double value;
            if(physical->incremental) {
                bool native_counter = rrdeng_page_type_is_counter(pgd_type(page.data));
                bool positive = native_counter || owner->absolute;
                bool valid = native_counter ? storage_counter_fold(&cursor->counter, &point, positive) :
                                                 storage_counter_fold_legacy(&cursor->counter, &point, positive);
                counted(owner, &owner->reconstruction_points, 1);
                if(!valid) return reader_error(cursor, &frame,
                    (RRDENG_READ_ERROR){.code = RRDENG_READ_PAYLOAD, .slot = position}, error);
                value = cursor->counter;
            }
            else value = point.count ? (double)point.sum / point.count : NAN;
            cursor_advance(cursor, timestamp, page.cadence);
            if(timestamp >= cursor->wanted)
                cursor->samples[count++] = sample(timestamp, isnan(point.sum) ? PROMQL_STALE_BITS : netdata_promql_double_bits(value));
        }
        if(!cursor->ended) {
            if(position == page.entries) cursor_advance(cursor, page.last, 1);
            else if(cursor->next > span.last) cursor_advance(cursor, span.last, 1);
        }
        if(count) break;
    }
    if(!count) { cursor->ended = true; return NETDATA_PROMQL_END; }
    out->samples = (netdata_promql_samples_t){cursor->samples, count};
    return NETDATA_PROMQL_OK;
}

static uint32_t temporary_failure(PROMQL_PROVIDER_CONTEXT *context, PROMQL_READ_FRAME *frame, int cause,
                                   netdata_promql_bytes_t *error) {
    if(cause == ECANCELED && frame->interrupted != NETDATA_PROMQL_OK) return frame->interrupted;
    snprintfz(context->error, sizeof(context->error) - 1, "PromQL temporary backing: %s", strerror(cause));
    *error = promql_bytes(context->error);
    return NETDATA_PROMQL_PROVIDER_ERROR;
}

static uint32_t temporary_create(void *context, void *query __maybe_unused, uint64_t values,
                                  const netdata_promql_callback_control_t *control, void **out,
                                  netdata_promql_bytes_t *error) {
    PROMQL_READ_FRAME frame;
    promql_read_operation(&frame, control);
    if(!promql_read_poll(&frame)) return frame.interrupted;
    if(values > UINT64_MAX / sizeof(uint64_t)) return temporary_failure(context, &frame, EOVERFLOW, error);
    int cause;
    PROMQL_FILE *file = promql_file_open(((PROMQL_PROVIDER_CONTEXT *)context)->backing_directory,
                                        frame.account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, &cause);
    if(!file) return temporary_failure(context, &frame, cause, error);
    if(!promql_file_size(file, values * sizeof(uint64_t), &cause)) {
        promql_file_close(file);
        return temporary_failure(context, &frame, cause, error);
    }
    *out = file;
    return NETDATA_PROMQL_OK;
}

static uint32_t temporary_read(void *context, void *handle, uint64_t offset, uint64_t *data, uint64_t values,
                                const netdata_promql_callback_control_t *control, uint64_t *transferred,
                                netdata_promql_bytes_t *error) {
    *transferred = 0;
    PROMQL_READ_FRAME frame;
    promql_read_operation(&frame, control);
    if(offset > UINT64_MAX / sizeof(uint64_t) || values > SIZE_MAX / sizeof(uint64_t))
        return temporary_failure(context, &frame, EOVERFLOW, error);
    int cause;
    if(!promql_file_read(handle, offset * sizeof(uint64_t), data, (size_t)values * sizeof(uint64_t), &frame, &cause))
        return temporary_failure(context, &frame, cause, error);
    *transferred = values;
    return NETDATA_PROMQL_OK;
}

static uint32_t temporary_write(void *context, void *handle, uint64_t offset, const uint64_t *data, uint64_t values,
                                 const netdata_promql_callback_control_t *control, uint64_t *transferred,
                                 netdata_promql_bytes_t *error) {
    *transferred = 0;
    PROMQL_READ_FRAME frame;
    promql_read_operation(&frame, control);
    if(offset > UINT64_MAX / sizeof(uint64_t) || values > SIZE_MAX / sizeof(uint64_t))
        return temporary_failure(context, &frame, EOVERFLOW, error);
    int cause;
    if(!promql_file_write(handle, offset * sizeof(uint64_t), data, (size_t)values * sizeof(uint64_t), &frame, &cause))
        return temporary_failure(context, &frame, cause, error);
    *transferred = values;
    return NETDATA_PROMQL_OK;
}

static void temporary_close(void *context __maybe_unused, void *handle,
                             const netdata_promql_accounting_context_t *account __maybe_unused) {
    promql_file_close(handle);
}

netdata_promql_provider_t promql_provider(PROMQL_PROVIDER_CONTEXT *context) {
    return (netdata_promql_provider_t){
        .header = {NETDATA_PROMQL_ABI_VERSION, sizeof(netdata_promql_provider_t)}, .context = context,
        .query_open = query_open, .query_close = query_close, .query_resources = resources,
        .selection_open = selection_open, .selection_close = selection_close, .selection_resources = resources,
        .selection_next = selection_next, .series_close = series_close, .series_resources = resources,
        .cursor_open = cursor_open, .cursor_close = cursor_close, .cursor_resources = resources,
        .cursor_seek = cursor_seek, .cursor_next = cursor_next,
        .temporary_create = temporary_create, .temporary_read = temporary_read, .temporary_write = temporary_write,
        .temporary_resources = resources, .temporary_close = temporary_close,
    };
}
