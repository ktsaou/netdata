// SPDX-License-Identifier: GPL-3.0-or-later

#include "output.h"
#include "keys.h"
#include "notes.h"
#include "web/server/web_client.h"

#define NO_RECORD UINT64_MAX

typedef struct { uint64_t next, length; } POINT_RECORD;
typedef struct { uint64_t first, last, count; } POINT_CHAIN;
typedef struct {
    uint64_t id, key, key_length, metric, metric_length;
    POINT_CHAIN floats, histograms;
} OUTPUT_SERIES;

struct promql_output {
    char *directory;
    uint64_t limit;
    bool stats, gzip, begun, committed;
    int gzip_level, gzip_strategy;
    netdata_promql_output_header_t header;
    PROMQL_BODY *records, *body;
    PROMQL_NOTES notes;
    netdata_promql_bytes_t stats_parameter;
    OUTPUT_SERIES *series;
    size_t count, capacity, *slots, slot_count;
    uint8_t *window;
    netdata_promql_report_t *child_report;
    int failure;
    char error[192];
};

static uint32_t output_failure(PROMQL_OUTPUT *output, PROMQL_READ_FRAME *frame, netdata_promql_bytes_t *error) {
    if(output->failure == ECANCELED && frame->interrupted != NETDATA_PROMQL_OK) return frame->interrupted;
    snprintfz(output->error, sizeof(output->error) - 1, "PromQL response backing: %s", strerror(output->failure));
    *error = promql_bytes(output->error);
    return NETDATA_PROMQL_OUTPUT_ERROR;
}

static void provisional_release(PROMQL_OUTPUT *output) {
    promql_body_release(output->records); output->records = NULL;
    promql_notes_destroy(&output->notes);
    promql_free(output->series); output->series = NULL;
    promql_free(output->slots); output->slots = NULL;
    promql_free(output->window); output->window = NULL;
    if(output->child_report) { netdata_promql_report_release(output->child_report); output->child_report = NULL; }
}

void promql_output_destroy(PROMQL_OUTPUT *output) {
    if(!output) return;
    provisional_release(output);
    promql_body_release(output->body);
    promql_free(output->directory);
    promql_free(output);
}

PROMQL_OUTPUT *promql_output_create(const netdata_promql_accounting_context_t *account,
                                    const char *directory, uint64_t limit, bool stats, bool gzip, int *error) {
    *error = 0;
    PROMQL_OUTPUT *output = promql_malloc(account, NETDATA_PROMQL_MEMORY_RESULT, sizeof(*output));
    if(!output) { *error = ENOMEM; return NULL; }
    output->directory = promql_strdup(account, directory);
    if(!output->directory) { *error = ENOMEM; promql_output_destroy(output); return NULL; }
    output->limit = limit; output->stats = stats; output->gzip = gzip;
    output->gzip_level = web_gzip_level;
    output->gzip_strategy = web_gzip_strategy;
    return output;
}

PROMQL_BODY *promql_output_take_body(PROMQL_OUTPUT *output) {
    if(!output || !output->committed) return NULL;
    PROMQL_BODY *body = output->body;
    output->body = NULL;
    return body;
}

void promql_output_stats_parameter(PROMQL_OUTPUT *output, netdata_promql_bytes_t value) {
    // The decoded form remains alive until synchronous output commit or abort.
    output->stats_parameter = value;
}

static bool copy_blob(PROMQL_OUTPUT *output, PROMQL_BODY *source, uint64_t offset, uint64_t length,
                       PROMQL_BODY *target, PROMQL_READ_FRAME *frame) {
    return promql_body_copy(source, offset, length, target, output->window, frame, &output->failure);
}

static bool key_field(PROMQL_OUTPUT *output, netdata_promql_bytes_t field, PROMQL_READ_FRAME *frame) {
    return promql_key_write(output->records, (PROMQL_KEY_VIEW){.single = true, .field = field}, frame, &output->failure);
}

static size_t id_hash(uint64_t id) {
    id ^= id >> 33; id *= UINT64_C(0xff51afd7ed558ccd); id ^= id >> 33;
    return (size_t)(id ^ (id >> 32));
}

static bool series_capacity(PROMQL_OUTPUT *output, PROMQL_READ_FRAME *frame) {
    if(output->count == output->capacity) {
        size_t capacity = output->capacity ? output->capacity * 2 : 16;
        if(capacity < output->capacity || capacity > SIZE_MAX / sizeof(*output->series)) goto overflow;
        void *series = output->series ? promql_resize(frame->account, output->series, capacity * sizeof(*output->series)) :
            promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_RESULT, capacity * sizeof(*output->series));
        if(!series) { output->failure = ENOMEM; return false; }
        output->series = series; output->capacity = capacity;
    }
    if(output->count >= output->slot_count / 2) {
        size_t count = output->slot_count ? output->slot_count * 2 : 32;
        if(count < output->slot_count || count > SIZE_MAX / sizeof(*output->slots)) goto overflow;
        size_t *slots = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_RESULT, count * sizeof(*slots));
        if(!slots) { output->failure = ENOMEM; return false; }
        for(size_t i = 0; i < output->count; i++) {
            if(i % 1024 == 0 && !promql_read_poll(frame)) {
                promql_free(slots); output->failure = ECANCELED; return false;
            }
            size_t slot = id_hash(output->series[i].id) & (count - 1);
            while(slots[slot]) slot = (slot + 1) & (count - 1);
            slots[slot] = i + 1;
        }
        promql_free(output->slots); output->slots = slots; output->slot_count = count;
    }
    return true;
overflow:
    output->failure = EOVERFLOW;
    return false;
}

static OUTPUT_SERIES *intern_series(PROMQL_OUTPUT *output, const netdata_promql_series_chunk_t *chunk,
                                     PROMQL_READ_FRAME *frame) {
    size_t slot;
    if(output->slot_count) {
        slot = id_hash(chunk->series_id) & (output->slot_count - 1);
        while(output->slots[slot]) {
            OUTPUT_SERIES *series = &output->series[output->slots[slot] - 1];
            if(series->id == chunk->series_id) return series;
            slot = (slot + 1) & (output->slot_count - 1);
        }
    }
    if(!series_capacity(output, frame)) return NULL;
    slot = id_hash(chunk->series_id) & (output->slot_count - 1);
    while(output->slots[slot]) slot = (slot + 1) & (output->slot_count - 1);
    OUTPUT_SERIES *series = &output->series[output->count];
    *series = (OUTPUT_SERIES){.id = chunk->series_id, .key = promql_body_length(output->records),
        .floats = {NO_RECORD, NO_RECORD, 0}, .histograms = {NO_RECORD, NO_RECORD, 0}};
    for(uint64_t i = 0; i < chunk->labels.len; i++)
        if(!key_field(output, chunk->labels.data[i].name, frame) || !key_field(output, chunk->labels.data[i].value, frame))
            return NULL;
    series->key_length = promql_body_length(output->records) - series->key;
    series->metric = promql_body_length(output->records);
    if(!promql_json_labels(output->records, chunk->labels, PROMQL_JSON_METRIC, frame, &output->failure)) return NULL;
    series->metric_length = promql_body_length(output->records) - series->metric;
    output->slots[slot] = ++output->count;
    return series;
}

typedef struct { PROMQL_OUTPUT *output; PROMQL_READ_FRAME *frame; bool buckets; } HISTOGRAM_JSON;

static uint32_t histogram_bucket(void *context, const netdata_promql_histogram_bucket_t *bucket,
                                  const netdata_promql_callback_control_t *control __maybe_unused,
                                  netdata_promql_bytes_t *error) {
    HISTOGRAM_JSON *json = context;
    if(netdata_promql_double_from_bits(bucket->count_bits) == 0.0) return NETDATA_PROMQL_OK;
    PROMQL_OUTPUT *output = json->output;
    if(!promql_json_text(output->records, json->buckets ? ",[" : ",\"buckets\":[[", json->frame, &output->failure))
        return output_failure(output, json->frame, error);
    json->buckets = true;
    unsigned code = bucket->lower_inclusive ? (bucket->upper_inclusive ? 3 : 1) : (bucket->upper_inclusive ? 0 : 2);
    if(!promql_json_unsigned(output->records, code, json->frame, &output->failure) ||
       !promql_json_text(output->records, ",", json->frame, &output->failure) ||
       !promql_json_float(output->records, bucket->lower_bits, false, json->frame, &output->failure) ||
       !promql_json_text(output->records, ",", json->frame, &output->failure) ||
       !promql_json_float(output->records, bucket->upper_bits, false, json->frame, &output->failure) ||
       !promql_json_text(output->records, ",", json->frame, &output->failure) ||
       !promql_json_float(output->records, bucket->count_bits, false, json->frame, &output->failure) ||
       !promql_json_text(output->records, "]", json->frame, &output->failure)) return output_failure(output, json->frame, error);
    return NETDATA_PROMQL_OK;
}

static uint32_t point_append(PROMQL_OUTPUT *output, OUTPUT_SERIES *series, const netdata_promql_sample_t *point,
                              PROMQL_READ_FRAME *frame, netdata_promql_bytes_t *error) {
    bool histogram = point->value.kind == NETDATA_PROMQL_VALUE_FLOAT_HISTOGRAM;
    if(!histogram && point->value.kind != NETDATA_PROMQL_VALUE_FLOAT) {
        *error = promql_bytes("invalid normalized output value"); return NETDATA_PROMQL_OUTPUT_ERROR;
    }
    POINT_CHAIN *chain = histogram ? &series->histograms : &series->floats;
    uint64_t offset = promql_body_length(output->records);
    POINT_RECORD record = {.next = NO_RECORD};
    if(!promql_body_append(output->records, &record, sizeof(record), frame, &output->failure) ||
       !promql_json_text(output->records, "[", frame, &output->failure) ||
       !promql_json_time(output->records, point->time_ms, false, frame, &output->failure) ||
       !promql_json_text(output->records, ",", frame, &output->failure)) return output_failure(output, frame, error);
    if(histogram) {
        const netdata_promql_float_histogram_t *value = point->value.payload.float_histogram;
        if(!promql_json_text(output->records, "{\"count\":", frame, &output->failure) ||
           !promql_json_float(output->records, value->count_bits, false, frame, &output->failure) ||
           !promql_json_text(output->records, ",\"sum\":", frame, &output->failure) ||
           !promql_json_float(output->records, value->sum_bits, false, frame, &output->failure))
            return output_failure(output, frame, error);
        HISTOGRAM_JSON json = {.output = output, .frame = frame};
        uint32_t status = netdata_promql_callback_histogram_buckets(value, frame->control, histogram_bucket, &json,
                                                                   &output->child_report);
        if(status != NETDATA_PROMQL_OK) {
            netdata_promql_error_t child;
            if(output->child_report && netdata_promql_report_error(output->child_report, 0, &child) == NETDATA_PROMQL_OK)
                *error = child.message;
            return status;
        }
        netdata_promql_report_release(output->child_report); output->child_report = NULL;
        if((json.buckets && !promql_json_text(output->records, "]", frame, &output->failure)) ||
           !promql_json_text(output->records, "}", frame, &output->failure)) return output_failure(output, frame, error);
    }
    else if(!promql_json_float(output->records, point->value.payload.float_bits, false, frame, &output->failure))
        return output_failure(output, frame, error);
    if(!promql_json_text(output->records, "]", frame, &output->failure)) return output_failure(output, frame, error);
    record.length = promql_body_length(output->records) - offset - sizeof(record);
    if(!promql_body_write(output->records, offset, &record, sizeof(record), frame, &output->failure) ||
       (chain->last != NO_RECORD && !promql_body_write(output->records, chain->last, &offset, sizeof(offset), frame, &output->failure)))
        return output_failure(output, frame, error);
    if(chain->first == NO_RECORD) chain->first = offset;
    chain->last = offset; chain->count++;
    return NETDATA_PROMQL_OK;
}

static uint32_t begin(void *context, const netdata_promql_output_header_t *header,
                       const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    PROMQL_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    output->header = *header; output->begun = true;
    output->body = promql_body_create(frame.account, output->directory, NETDATA_PROMQL_MEMORY_RESULT, &output->failure);
    output->records = promql_body_create(frame.account, output->directory, NETDATA_PROMQL_MEMORY_RESULT, &output->failure);
    output->window = promql_malloc(frame.account, NETDATA_PROMQL_MEMORY_WORKSPACE, PROMQL_BODY_WINDOW);
    if(!output->body || !output->records || !output->window) { output->failure = ENOMEM; return output_failure(output, &frame, error); }
    return NETDATA_PROMQL_OK;
}

static bool annotation_append(PROMQL_OUTPUT *output, unsigned severity, netdata_promql_bytes_t message,
                               PROMQL_READ_FRAME *frame) {
    return promql_notes_append(&output->notes, severity, message, output->directory, frame, &output->failure);
}

static uint32_t chunk(void *context, const netdata_promql_chunk_t *chunk,
                       const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    PROMQL_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    switch(chunk->kind) {
        case NETDATA_PROMQL_CHUNK_ANNOTATIONS:
            for(uint64_t i = 0; i < chunk->payload.annotations.len; i++) {
                const netdata_promql_annotation_t *annotation = &chunk->payload.annotations.data[i];
                if(!annotation_append(output, annotation->severity, annotation->message, &frame)) return output_failure(output, &frame, error);
            }
            return NETDATA_PROMQL_OK;
        case NETDATA_PROMQL_CHUNK_SCALAR:
            if(!promql_json_text(output->records, "[", &frame, &output->failure) ||
               !promql_json_time(output->records, chunk->payload.scalar.time_ms, true, &frame, &output->failure) ||
               !promql_json_text(output->records, ",", &frame, &output->failure) ||
               !promql_json_float(output->records, chunk->payload.scalar.value_bits, true, &frame, &output->failure) ||
               !promql_json_text(output->records, "]", &frame, &output->failure)) return output_failure(output, &frame, error);
            return NETDATA_PROMQL_OK;
        case NETDATA_PROMQL_CHUNK_STRING:
            if(!promql_json_text(output->records, "[", &frame, &output->failure) ||
               !promql_json_time(output->records, chunk->payload.string.time_ms, true, &frame, &output->failure) ||
               !promql_json_text(output->records, ",", &frame, &output->failure) ||
               !promql_json_string(output->records, chunk->payload.string.value, PROMQL_JSON_SCALAR, &frame, &output->failure) ||
               !promql_json_text(output->records, "]", &frame, &output->failure)) return output_failure(output, &frame, error);
            return NETDATA_PROMQL_OK;
        case NETDATA_PROMQL_CHUNK_SERIES: {
            OUTPUT_SERIES *series = intern_series(output, &chunk->payload.series, &frame);
            if(!series) return output_failure(output, &frame, error);
            for(uint64_t i = 0; i < chunk->payload.series.points.len; i++) {
                uint32_t status = point_append(output, series, &chunk->payload.series.points.data[i], &frame, error);
                if(status != NETDATA_PROMQL_OK) return status;
            }
            return NETDATA_PROMQL_OK;
        }
        default: *error = promql_bytes("invalid output chunk"); return NETDATA_PROMQL_OUTPUT_ERROR;
    }
}

static uint32_t compare_series(void *context, uint64_t left, uint64_t right,
                                const netdata_promql_callback_control_t *control,
                                int32_t *ordering, netdata_promql_bytes_t *error) {
    PROMQL_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    const OUTPUT_SERIES *a = &output->series[left], *b = &output->series[right];
    if(!promql_body_compare(output->records, a->key, a->key_length, b->key, b->key_length,
        output->window, &frame, &output->failure, ordering)) return output_failure(output, &frame, error);
    return NETDATA_PROMQL_OK;
}

typedef struct { uint64_t offset; size_t length; } POINT_WINDOW;

static const uint8_t *chain_bytes(PROMQL_OUTPUT *output, POINT_WINDOW *window, uint64_t offset,
                                  size_t length, bool read_ahead, PROMQL_READ_FRAME *frame) {
    uint64_t total = promql_body_length(output->records);
    if(length > PROMQL_BODY_WINDOW || offset > total || length > total - offset) {
        output->failure = EINVAL;
        return NULL;
    }
    if(length > window->length || offset < window->offset ||
       offset - window->offset > window->length - length) {
        size_t count = read_ahead ? (size_t)MIN(total - offset, (uint64_t)PROMQL_BODY_WINDOW) : length;
        if(!promql_body_read(output->records, offset, output->window, count, frame, &output->failure)) return NULL;
        window->offset = offset;
        window->length = count;
    }
    else if(!promql_read_poll(frame)) {
        output->failure = ECANCELED;
        return NULL;
    }
    return output->window + (size_t)(offset - window->offset);
}

static bool write_chain(PROMQL_OUTPUT *output, POINT_CHAIN chain, bool matrix, PROMQL_READ_FRAME *frame) {
    // Other output phases reuse the same scratch bytes, so reader state belongs to this chain only.
    POINT_WINDOW window = {0};
    // One series of one value kind has no other point chain interleaved in its records.
    bool read_ahead = output->count == 1 &&
        (!output->series[0].floats.count || !output->series[0].histograms.count);
    uint64_t position = chain.first;
    if(matrix && !promql_json_text(output->body, "[", frame, &output->failure)) return false;
    for(uint64_t i = 0; i < chain.count; i++) {
        POINT_RECORD record;
        if(position == NO_RECORD) return false;
        const uint8_t *bytes = chain_bytes(output, &window, position, sizeof(record), read_ahead, frame);
        if(!bytes) return false;
        memcpy(&record, bytes, sizeof(record));
        if(i && !promql_json_text(output->body, ",", frame, &output->failure)) return false;
        uint64_t offset = position + sizeof(record);
        while(record.length) {
            size_t count = (size_t)MIN(record.length, (uint64_t)PROMQL_BODY_WINDOW);
            if(offset >= window.offset && offset - window.offset < window.length)
                count = MIN(count, window.length - (size_t)(offset - window.offset));
            bytes = chain_bytes(output, &window, offset, count, read_ahead, frame);
            if(!bytes || !promql_body_append(output->body, bytes, count, frame, &output->failure)) return false;
            offset += count;
            record.length -= count;
        }
        position = record.next;
    }
    if(position != NO_RECORD) { output->failure = EINVAL; return false; }
    return !matrix || promql_json_text(output->body, "]", frame, &output->failure);
}

static bool write_annotations(PROMQL_OUTPUT *output, PROMQL_READ_FRAME *frame) {
    return promql_notes_write(&output->notes, output->body, output->window, frame, &output->failure);
}

static bool write_stats(PROMQL_OUTPUT *output, const netdata_promql_query_observations_t *observations,
                         PROMQL_READ_FRAME *frame) {
    if(!output->stats) return true;
    const netdata_promql_statistics_timings_t *times = &observations->statistics.timings;
    const char *names[] = {"evalTotalTime", "resultSortTime", "queryPreparationTime", "innerEvalTime", "execQueueTime", "execTotalTime"};
    netdata_promql_duration_t values[] = {times->eval_total_time, times->result_sort_time, times->query_preparation_time,
                                        times->inner_eval_time, times->exec_queue_time, times->exec_total_time};
    if(!promql_json_text(output->body, ",\"stats\":{\"timings\":{", frame, &output->failure)) return false;
    for(size_t i = 0; i < sizeof(values) / sizeof(values[0]); i++) {
        netdata_promql_numeric_text_t text;
        double seconds = (double)values[i].seconds + (double)values[i].nanoseconds / 1e9;
        if(netdata_promql_api_float_text(netdata_promql_double_bits(seconds), &text) != NETDATA_PROMQL_OK) { output->failure = EINVAL; return false; }
        if((i && !promql_json_text(output->body, ",", frame, &output->failure)) ||
           !promql_json_string(output->body, promql_bytes(names[i]), PROMQL_JSON_ENVELOPE, frame, &output->failure) ||
           !promql_json_text(output->body, ":", frame, &output->failure) ||
           !promql_body_append(output->body, text.bytes, text.len, frame, &output->failure)) return false;
    }
    return promql_json_text(output->body, "},\"samples\":{\"totalQueryableSamples\":", frame, &output->failure) &&
        promql_json_unsigned(output->body, observations->statistics.samples.total_queryable_samples, frame, &output->failure) &&
        promql_json_text(output->body, ",\"peakSamples\":", frame, &output->failure) &&
        promql_json_unsigned(output->body, observations->statistics.samples.peak_samples, frame, &output->failure) &&
        promql_json_text(output->body, "}}", frame, &output->failure);
}

static uint32_t commit(void *context, const netdata_promql_query_observations_t *observations,
                        const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    PROMQL_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    static const char *const types[] = {NULL, "scalar", "vector", "matrix", "string"};
    uint32_t type = output->header.value_type;
    if(type < NETDATA_PROMQL_TYPE_SCALAR || type > NETDATA_PROMQL_TYPE_STRING) {
        *error = promql_bytes("invalid output type"); return NETDATA_PROMQL_OUTPUT_ERROR;
    }
    uint64_t *order = NULL;
    if(type == NETDATA_PROMQL_TYPE_MATRIX && output->count) {
        if(output->count > SIZE_MAX / sizeof(*order)) { output->failure = EOVERFLOW; return output_failure(output, &frame, error); }
        order = promql_malloc(frame.account, NETDATA_PROMQL_MEMORY_WORKSPACE, output->count * sizeof(*order));
        if(!order) { output->failure = ENOMEM; return output_failure(output, &frame, error); }
        for(size_t i = 0; i < output->count; i++) order[i] = i;
        uint32_t status = netdata_promql_callback_sort_indices(order, output->count, control, compare_series, output, &output->child_report);
        if(status != NETDATA_PROMQL_OK) {
            netdata_promql_error_t child;
            if(output->child_report && netdata_promql_report_error(output->child_report, 0, &child) == NETDATA_PROMQL_OK) *error = child.message;
            promql_free(order); return status;
        }
        netdata_promql_report_release(output->child_report); output->child_report = NULL;
    }
    bool valid = promql_json_text(output->body, "{\"status\":\"success\",\"data\":{\"resultType\":", &frame, &output->failure) &&
        promql_json_string(output->body, promql_bytes(types[type]), PROMQL_JSON_ENVELOPE, &frame, &output->failure) &&
        promql_json_text(output->body, ",\"result\":", &frame, &output->failure);
    if(valid && (type == NETDATA_PROMQL_TYPE_SCALAR || type == NETDATA_PROMQL_TYPE_STRING))
        valid = copy_blob(output, output->records, 0, promql_body_length(output->records), output->body, &frame);
    else if(valid) {
        size_t count = output->limit && output->limit < output->count ? (size_t)output->limit : output->count;
        valid = promql_json_text(output->body, "[", &frame, &output->failure);
        for(size_t i = 0; valid && i < count; i++) {
            OUTPUT_SERIES *series = &output->series[order ? (size_t)order[i] : i];
            if(type == NETDATA_PROMQL_TYPE_VECTOR &&
               (series->floats.count > 1 || series->histograms.count > 1 ||
                series->floats.count + series->histograms.count != 1)) { output->failure = EINVAL; valid = false; break; }
            valid = (!i || promql_json_text(output->body, ",", &frame, &output->failure)) &&
                promql_json_text(output->body, "{\"metric\":", &frame, &output->failure) &&
                copy_blob(output, output->records, series->metric, series->metric_length, output->body, &frame);
            if(valid && series->floats.count)
                valid = promql_json_text(output->body, type == NETDATA_PROMQL_TYPE_MATRIX ? ",\"values\":" : ",\"value\":", &frame, &output->failure) &&
                    write_chain(output, series->floats, type == NETDATA_PROMQL_TYPE_MATRIX, &frame);
            if(valid && series->histograms.count)
                valid = promql_json_text(output->body, type == NETDATA_PROMQL_TYPE_MATRIX ? ",\"histograms\":" : ",\"histogram\":", &frame, &output->failure) &&
                    write_chain(output, series->histograms, type == NETDATA_PROMQL_TYPE_MATRIX, &frame);
            valid = valid && promql_json_text(output->body, "}", &frame, &output->failure);
        }
        valid = valid && promql_json_text(output->body, "]", &frame, &output->failure);
        if(valid && count < output->count)
            valid = annotation_append(output, 1, promql_bytes("results truncated due to limit"), &frame);
    }
    promql_free(order);
    valid = valid && promql_notes_stats(&output->notes, output->stats_parameter, output->directory, &frame, &output->failure) &&
        write_stats(output, observations, &frame) &&
        promql_json_text(output->body, "}", &frame, &output->failure) && write_annotations(output, &frame) &&
        promql_json_text(output->body, "}", &frame, &output->failure) && promql_body_seal(output->body, &frame, &output->failure);
    if(!valid) return output_failure(output, &frame, error);
    if(output->gzip) {
        PROMQL_BODY *encoded = promql_body_gzip(output->body, &frame, output->gzip_level,
                                                output->gzip_strategy, &output->failure);
        if(!encoded) return output_failure(output, &frame, error);
        promql_body_release(output->body); output->body = encoded;
    }
    provisional_release(output);
    output->committed = true;
    return NETDATA_PROMQL_OK;
}

static void abort_output(void *context, const netdata_promql_accounting_context_t *account __maybe_unused) {
    PROMQL_OUTPUT *output = context;
    provisional_release(output);
    promql_body_release(output->body); output->body = NULL;
}

netdata_promql_output_t promql_output_callbacks(PROMQL_OUTPUT *output) {
    return (netdata_promql_output_t){.header = {NETDATA_PROMQL_ABI_VERSION, sizeof(netdata_promql_output_t)},
        .context = output, .begin = begin, .chunk = chunk, .commit = commit, .abort = abort_output};
}
