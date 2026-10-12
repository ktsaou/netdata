// SPDX-License-Identifier: GPL-3.0-or-later

#include "api-internal.h"
#include "keys.h"
#include "notes.h"

typedef struct { uint64_t key, length, hash, json, json_length; } DISCOVERY_ENTRY;
typedef struct {
    PROMQL_API_REQUEST *request;
    PROMQL_API_KIND kind;
    PROMQL_BODY *records, *body;
    PROMQL_NOTES notes;
    DISCOVERY_ENTRY *entries;
    size_t count, capacity, *slots, slot_count;
    uint8_t *window;
    netdata_promql_report_t *sort_report;
    bool committed;
    int failure;
    char error[192];
} DISCOVERY_OUTPUT;

static void release_records(DISCOVERY_OUTPUT *output) {
    promql_body_release(output->records); output->records = NULL;
    promql_notes_destroy(&output->notes);
    promql_free(output->entries); output->entries = NULL;
    promql_free(output->slots); output->slots = NULL;
    promql_free(output->window); output->window = NULL;
    if(output->sort_report) { netdata_promql_report_release(output->sort_report); output->sort_report = NULL; }
}

static uint32_t output_error(DISCOVERY_OUTPUT *output, PROMQL_READ_FRAME *frame, netdata_promql_bytes_t *error) {
    if(output->failure == ECANCELED && frame->interrupted != NETDATA_PROMQL_OK) return frame->interrupted;
    snprintfz(output->error, sizeof(output->error) - 1, "PromQL metadata response backing: %s", strerror(output->failure));
    *error = promql_bytes(output->error);
    return NETDATA_PROMQL_OUTPUT_ERROR;
}

static bool reserve(DISCOVERY_OUTPUT *output, PROMQL_READ_FRAME *frame) {
    if(output->count == output->capacity) {
        size_t capacity = output->capacity ? output->capacity * 2 : 16;
        if(capacity < output->capacity || capacity > SIZE_MAX / sizeof(*output->entries)) goto overflow;
        void *entries = output->entries ? promql_resize(frame->account, output->entries, capacity * sizeof(*output->entries)) :
            promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_RESULT, capacity * sizeof(*output->entries));
        if(!entries) { output->failure = ENOMEM; return false; }
        output->entries = entries; output->capacity = capacity;
    }
    if(output->count >= output->slot_count / 2) {
        size_t capacity = output->slot_count ? output->slot_count * 2 : 32;
        if(capacity < output->slot_count || capacity > SIZE_MAX / sizeof(*output->slots)) goto overflow;
        size_t *slots = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_WORKSPACE, capacity * sizeof(*slots));
        if(!slots) { output->failure = ENOMEM; return false; }
        for(size_t i = 0; i < output->count; i++) {
            if(!(i % 1024) && !promql_read_poll(frame)) { promql_free(slots); output->failure = ECANCELED; return false; }
            size_t slot = (size_t)output->entries[i].hash & (capacity - 1);
            while(slots[slot]) slot = (slot + 1) & (capacity - 1);
            slots[slot] = i + 1;
        }
        promql_free(output->slots); output->slots = slots; output->slot_count = capacity;
    }
    return true;
overflow:
    output->failure = EOVERFLOW;
    return false;
}

static bool intern(DISCOVERY_OUTPUT *output, PROMQL_KEY_VIEW view, PROMQL_READ_FRAME *frame) {
    uint64_t length, hash;
    if(!promql_key_measure(view, &length, &hash, frame, &output->failure)) return false;
    if(output->slot_count) {
        size_t slot = (size_t)hash & (output->slot_count - 1);
        while(output->slots[slot]) {
            if(!promql_read_poll(frame)) { output->failure = ECANCELED; return false; }
            DISCOVERY_ENTRY *entry = &output->entries[output->slots[slot] - 1];
            if(entry->hash == hash && entry->length == length) {
                bool equal;
                if(!promql_key_equal(output->records, entry->key, view, output->window, frame, &output->failure, &equal)) return false;
                if(equal) return true;
            }
            slot = (slot + 1) & (output->slot_count - 1);
        }
    }
    if(!reserve(output, frame)) return false;
    size_t slot = (size_t)hash & (output->slot_count - 1);
    while(output->slots[slot]) slot = (slot + 1) & (output->slot_count - 1);
    DISCOVERY_ENTRY *entry = &output->entries[output->count];
    *entry = (DISCOVERY_ENTRY){.key = promql_body_length(output->records), .length = length, .hash = hash};
    if(!promql_key_write(output->records, view, frame, &output->failure)) return false;
    entry->json = promql_body_length(output->records);
    bool valid = view.single ? promql_json_string(output->records, view.field, PROMQL_JSON_ENVELOPE, frame, &output->failure) :
        promql_json_labels(output->records, view.labels, PROMQL_JSON_METRIC, frame, &output->failure);
    if(!valid) return false;
    entry->json_length = promql_body_length(output->records) - entry->json;
    output->slots[slot] = ++output->count;
    return true;
}

static uint32_t begin(void *context, const netdata_promql_output_header_t *header,
                       const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    DISCOVERY_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    if(header->value_type != NETDATA_PROMQL_TYPE_VECTOR) { *error = promql_bytes("invalid discovery header"); return NETDATA_PROMQL_OUTPUT_ERROR; }
    output->records = promql_body_create(frame.account, output->request->directory, NETDATA_PROMQL_MEMORY_RESULT, &output->failure);
    output->body = promql_body_create(frame.account, output->request->directory, NETDATA_PROMQL_MEMORY_RESULT, &output->failure);
    output->window = promql_malloc(frame.account, NETDATA_PROMQL_MEMORY_WORKSPACE, PROMQL_BODY_WINDOW);
    if(!output->records || !output->body || !output->window) {
        if(!output->failure) output->failure = ENOMEM;
        return output_error(output, &frame, error);
    }
    return NETDATA_PROMQL_OK;
}

static uint32_t chunk(void *context, const netdata_promql_chunk_t *chunk,
                       const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    DISCOVERY_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    if(chunk->kind == NETDATA_PROMQL_CHUNK_ANNOTATIONS) {
        for(uint64_t i = 0; i < chunk->payload.annotations.len; i++) {
            const netdata_promql_annotation_t *note = &chunk->payload.annotations.data[i];
            if(!promql_notes_append(&output->notes, note->severity, note->message,
                output->request->directory, &frame, &output->failure)) return output_error(output, &frame, error);
        }
        return NETDATA_PROMQL_OK;
    }
    if(chunk->kind != NETDATA_PROMQL_CHUNK_SERIES || chunk->payload.series.points.len) {
        *error = promql_bytes("invalid discovery row"); return NETDATA_PROMQL_OUTPUT_ERROR;
    }
    netdata_promql_labels_t labels = chunk->payload.series.labels;
    if(output->kind == PROMQL_API_SERIES) {
        if(!intern(output, (PROMQL_KEY_VIEW){.labels = labels}, &frame)) return output_error(output, &frame, error);
        if(output->request->selector_count == 1 && output->request->limit && output->count > output->request->limit)
            return NETDATA_PROMQL_END;
    }
    else for(uint64_t i = 0; i < labels.len; i++) {
        netdata_promql_bytes_t field;
        if(output->kind == PROMQL_API_LABELS) field = labels.data[i].name;
        else {
            netdata_promql_bytes_t wanted = output->request->label_name;
            if(wanted.len != labels.data[i].name.len || memcmp(wanted.data, labels.data[i].name.data, (size_t)wanted.len)) continue;
            field = labels.data[i].value;
        }
        if(!intern(output, (PROMQL_KEY_VIEW){.single = true, .field = field}, &frame)) return output_error(output, &frame, error);
    }
    return NETDATA_PROMQL_OK;
}

static uint32_t compare_entries(void *context, uint64_t left, uint64_t right,
                                 const netdata_promql_callback_control_t *control, int32_t *ordering,
                                 netdata_promql_bytes_t *error) {
    DISCOVERY_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    DISCOVERY_ENTRY *a = &output->entries[left], *b = &output->entries[right];
    if(!promql_body_compare(output->records, a->key, a->length, b->key, b->length,
        output->window, &frame, &output->failure, ordering)) return output_error(output, &frame, error);
    return NETDATA_PROMQL_OK;
}

static uint32_t commit(void *context, const netdata_promql_query_observations_t *observations __maybe_unused,
                        const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    DISCOVERY_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    bool sorted = output->kind != PROMQL_API_SERIES || output->request->selector_count > 1;
    uint64_t *order = NULL;
    if(sorted && output->count) {
        if(output->count > SIZE_MAX / sizeof(*order)) { output->failure = EOVERFLOW; return output_error(output, &frame, error); }
        order = promql_malloc(frame.account, NETDATA_PROMQL_MEMORY_WORKSPACE, output->count * sizeof(*order));
        if(!order) { output->failure = ENOMEM; return output_error(output, &frame, error); }
        for(size_t i = 0; i < output->count; i++) order[i] = i;
        uint32_t status = netdata_promql_callback_sort_indices(order, output->count, control, compare_entries, output, &output->sort_report);
        if(status != NETDATA_PROMQL_OK) {
            netdata_promql_error_t child;
            if(output->sort_report && netdata_promql_report_error(output->sort_report, 0, &child) == NETDATA_PROMQL_OK) *error = child.message;
            promql_free(order); return status;
        }
        netdata_promql_report_release(output->sort_report); output->sort_report = NULL;
    }
    size_t count = output->request->limit && output->request->limit < output->count ? (size_t)output->request->limit : output->count;
    bool valid = promql_json_text(output->body, "{\"status\":\"success\",\"data\":[", &frame, &output->failure);
    for(size_t i = 0; valid && i < count; i++) {
        DISCOVERY_ENTRY *entry = &output->entries[order ? (size_t)order[i] : i];
        valid = (!i || promql_json_text(output->body, ",", &frame, &output->failure)) &&
            promql_body_copy(output->records, entry->json, entry->json_length, output->body, output->window, &frame, &output->failure);
    }
    promql_free(order);
    if(valid && count < output->count) valid = promql_notes_append(&output->notes, NETDATA_PROMQL_WARNING,
        promql_bytes("results truncated due to limit"), output->request->directory, &frame, &output->failure);
    valid = valid && promql_json_text(output->body, "]", &frame, &output->failure) &&
        promql_notes_write(&output->notes, output->body, output->window, &frame, &output->failure) &&
        promql_json_text(output->body, "}", &frame, &output->failure) && promql_body_seal(output->body, &frame, &output->failure);
    if(!valid) return output_error(output, &frame, error);
    if(output->request->gzip) {
        PROMQL_BODY *encoded = promql_body_gzip(output->body, &frame, web_gzip_level, web_gzip_strategy, &output->failure);
        if(!encoded) return output_error(output, &frame, error);
        promql_body_release(output->body); output->body = encoded;
    }
    release_records(output);
    output->committed = true;
    return NETDATA_PROMQL_OK;
}

static void abort_output(void *context, const netdata_promql_accounting_context_t *account __maybe_unused) {
    DISCOVERY_OUTPUT *output = context;
    release_records(output);
    promql_body_release(output->body); output->body = NULL;
}

static void prepare_selectors(void *context, void *handle __maybe_unused,
                               const netdata_promql_accounting_context_t *account) {
    PROMQL_API_REQUEST *request = context;
    if(request->selector_count > SIZE_MAX / sizeof(*request->selectors)) { request->error = EOVERFLOW; return; }
    request->selectors = promql_malloc(account, NETDATA_PROMQL_MEMORY_WORKSPACE,
        request->selector_count * sizeof(*request->selectors));
    if(!request->selectors) request->error = ENOMEM;
}

bool promql_api_discovery(PROMQL_API_REQUEST *request, PROMQL_API_KIND kind) {
    if(kind == PROMQL_API_SERIES && request->form->error.message) {
        PROMQL_FORM_ERROR *error = &request->form->error;
        request->failure = (PROMQL_API_ERROR){.code = HTTP_RESP_BAD_REQUEST, .type = "bad_data", .prefix = "error parsing form values: ",
            .message = promql_bytes(error->message)};
        if(error->escape_length) {
            request->failure.prefix = "error parsing form values: invalid URL escape ";
            request->failure.message = (netdata_promql_bytes_t){error->escape, error->escape_length};
            request->failure.quote = true;
        }
        return false;
    }
    request->selector_count = promql_form_count(request->form, PROMQL_PARAM_MATCH);
    if(kind == PROMQL_API_SERIES && !request->selector_count) {
        promql_api_bad_data(request, NULL, "no match[] parameter provided"); return false;
    }
    if(!promql_api_limit(request)) return false;
    netdata_promql_timestamp_t start = {.unix_seconds = INT64_C(-9223309901257974)};
    netdata_promql_timestamp_t end = {.unix_seconds = INT64_C(9223309901257974), .nanoseconds = UINT32_C(999999999)};
    if(!promql_api_time(request, PROMQL_PARAM_START, "start", true, start, &start) ||
       !promql_api_time(request, PROMQL_PARAM_END, "end", true, end, &end)) return false;
    if(request->selector_count) {
        uint32_t status = netdata_promql_report_accounted(request->report, prepare_selectors, request, NULL);
        if(status != NETDATA_PROMQL_OK || !request->selectors) {
            promql_api_report_failure(request, NETDATA_PROMQL_INTERNAL_ERROR, NULL, (netdata_promql_bytes_t){0}); return false;
        }
    }
    size_t selected = 0;
    for(size_t i = 0; i < request->form->count; i++) {
        PROMQL_FORM_FIELD *field = &request->form->fields[i];
        if(field->parameter != PROMQL_PARAM_MATCH) continue;
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_selector_compile(request->engine, field->value, request->operation.control,
            &request->selectors[selected++], &report);
        promql_api_replace_report(request, report);
        if(status != NETDATA_PROMQL_OK) {
            promql_api_report_failure(request, status, kind == PROMQL_API_SERIES ? "match[]" : NULL, field->value); return false;
        }
    }
    for(size_t i = 0; i < request->selector_count; i++) {
        uint32_t nonempty;
        uint32_t status = netdata_promql_selector_nonempty(request->selectors[i], request->operation.control, &nonempty);
        if(status != NETDATA_PROMQL_OK) { promql_api_report_failure(request, status, NULL, (netdata_promql_bytes_t){0}); return false; }
        if(!nonempty) { promql_api_bad_data(request, kind == PROMQL_API_SERIES ? "match[]" : NULL,
            "match[] must contain at least one non-empty matcher"); return false; }
    }
    netdata_promql_bytes_t options = promql_form_value(request->form, PROMQL_PARAM_OPTIONS);
    if(options.len && (options.len != 8 || memcmp(options.data, "absolute", 8))) {
        promql_api_bad_data(request, "options", "unsupported PromQL option; supported value is absolute"); return false;
    }
    DISCOVERY_OUTPUT output = {.request = request, .kind = kind};
    netdata_promql_output_t callbacks = {.header = {NETDATA_PROMQL_ABI_VERSION, sizeof(callbacks)}, .context = &output,
        .begin = begin, .chunk = chunk, .commit = commit, .abort = abort_output};
    netdata_promql_provider_t provider = promql_provider(&request->context);
    netdata_promql_bounds_t bounds = {.start_ms = promql_api_milliseconds(start), .end_ms = promql_api_milliseconds(end)};
    netdata_promql_report_t *report = NULL;
    uint32_t status = netdata_promql_discover(request->engine, &provider, bounds,
        (const netdata_promql_selector_t *const *)request->selectors, request->selector_count,
        request->operation.control, &callbacks, &report);
    promql_api_replace_report(request, report);
    if(status == NETDATA_PROMQL_OK && output.committed) { request->body = output.body; output.body = NULL; }
    else promql_api_report_failure(request, status == NETDATA_PROMQL_OK ? NETDATA_PROMQL_INTERNAL_ERROR : status,
        NULL, (netdata_promql_bytes_t){0});
    release_records(&output);
    promql_body_release(output.body);
    return request->body != NULL;
}
