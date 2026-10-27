// SPDX-License-Identifier: GPL-3.0-or-later

#include "api-internal.h"
#include "keys.h"
#include "notes.h"

typedef struct { uint64_t key, length, hash, json, json_length; } DISCOVERY_ENTRY;
typedef struct { uint64_t json_name_length, key_name_length; } DESCRIPTION_RECORD;
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
    int64_t metric_limit, description_limit;
    uint32_t status;
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

static bool create_records(DISCOVERY_OUTPUT *output, PROMQL_READ_FRAME *frame) {
    output->records = promql_body_create(frame->account, output->request->directory, NETDATA_PROMQL_MEMORY_RESULT, &output->failure);
    output->body = promql_body_create(frame->account, output->request->directory, NETDATA_PROMQL_MEMORY_RESULT, &output->failure);
    output->window = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_WORKSPACE, PROMQL_BODY_WINDOW);
    if(output->records && output->body && output->window) return true;
    if(!output->failure) output->failure = ENOMEM;
    return false;
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

static bool description_record(DISCOVERY_OUTPUT *output, PROMQL_KEY_VIEW view, PROMQL_READ_FRAME *frame) {
    netdata_promql_bytes_t name = view.labels.data[0].name;
    DESCRIPTION_RECORD header = {0};
    uint64_t hash, offset = promql_body_length(output->records);
    if(!promql_key_measure((PROMQL_KEY_VIEW){.single = true, .field = name}, &header.key_name_length, &hash,
        frame, &output->failure) || !promql_body_append(output->records, &header, sizeof(header), frame, &output->failure) ||
       !promql_json_string(output->records, name, PROMQL_JSON_ENVELOPE, frame, &output->failure)) return false;
    header.json_name_length = promql_body_length(output->records) - offset - sizeof(header);
    return promql_body_write(output->records, offset, &header, sizeof(header), frame, &output->failure) &&
        promql_json_text(output->records, "{\"type\":", frame, &output->failure) &&
        promql_json_string(output->records, view.labels.data[0].value, PROMQL_JSON_ENVELOPE, frame, &output->failure) &&
        promql_json_text(output->records, ",\"help\":", frame, &output->failure) &&
        promql_json_string(output->records, view.labels.data[1].name, PROMQL_JSON_ENVELOPE, frame, &output->failure) &&
        promql_json_text(output->records, ",\"unit\":", frame, &output->failure) &&
        promql_json_string(output->records, view.labels.data[1].value, PROMQL_JSON_ENVELOPE, frame, &output->failure) &&
        promql_json_text(output->records, "}", frame, &output->failure);
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
                if(!promql_key_equal(output->records, entry->key, entry->length, view, output->window, frame, &output->failure, &equal)) return false;
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
    bool valid = output->kind == PROMQL_API_METADATA ? description_record(output, view, frame) :
        view.single ? promql_json_string(output->records, view.field, PROMQL_JSON_ENVELOPE, frame, &output->failure) :
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
    if(!create_records(output, &frame)) return output_error(output, &frame, error);
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

static bool compare_keys(DISCOVERY_OUTPUT *output, uint64_t left, uint64_t right,
                          PROMQL_READ_FRAME *frame, int32_t *ordering) {
    DISCOVERY_ENTRY *a = &output->entries[left], *b = &output->entries[right];
    return promql_body_compare(output->records, a->key, a->length, b->key, b->length,
        output->window, frame, &output->failure, ordering);
}

static uint32_t compare_entries(void *context, uint64_t left, uint64_t right,
                                 const netdata_promql_callback_control_t *control, int32_t *ordering,
                                 netdata_promql_bytes_t *error) {
    DISCOVERY_OUTPUT *output = context;
    PROMQL_READ_FRAME frame; promql_read_operation(&frame, control);
    if(!compare_keys(output, left, right, &frame, ordering)) return output_error(output, &frame, error);
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
    request->context.names_only = kind == PROMQL_API_VALUES &&
        request->label_name.len == sizeof("__name__") - 1 &&
        !memcmp(request->label_name.data, "__name__", sizeof("__name__") - 1);
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

static bool description_heap(DISCOVERY_OUTPUT *output, uint64_t *order, size_t root, size_t count,
                               PROMQL_READ_FRAME *frame) {
    while(root < count / 2) {
        size_t child = root * 2 + 1;
        int32_t comparison;
        if(child + 1 < count) {
            if(!compare_keys(output, order[child], order[child + 1], frame, &comparison)) return false;
            if(comparison < 0) child++;
        }
        if(!compare_keys(output, order[root], order[child], frame, &comparison)) return false;
        if(comparison >= 0) break;
        SWAP(order[root], order[child]);
        root = child;
    }
    return true;
}

static bool description_sort(DISCOVERY_OUTPUT *output, uint64_t *order, PROMQL_READ_FRAME *frame) {
    for(size_t i = 0; i < output->count; i++) {
        if(!(i % 1024) && !promql_read_poll(frame)) { output->failure = ECANCELED; return false; }
        order[i] = i;
    }
    for(size_t root = output->count / 2; root; root--)
        if(!description_heap(output, order, root - 1, output->count, frame)) return false;
    for(size_t remaining = output->count; remaining > 1; remaining--) {
        SWAP(order[0], order[remaining - 1]);
        if(!description_heap(output, order, 0, remaining - 1, frame)) return false;
    }
    return true;
}

static bool descriptions_write(DISCOVERY_OUTPUT *output, const uint64_t *order, PROMQL_READ_FRAME *frame) {
    if(!promql_json_text(output->body, "{\"status\":\"success\",\"data\":{", frame, &output->failure)) return false;
    uint64_t groups = 0, values = 0;
    DISCOVERY_ENTRY *previous = NULL;
    DESCRIPTION_RECORD previous_header = {0};
    for(size_t i = 0; i < output->count; i++) {
        if(!promql_read_poll(frame)) { output->failure = ECANCELED; return false; }
        DISCOVERY_ENTRY *entry = &output->entries[order[i]];
        DESCRIPTION_RECORD header;
        if(entry->json_length < sizeof(header)) { output->failure = EINVAL; return false; }
        if(!promql_body_read(output->records, entry->json, &header, sizeof(header), frame, &output->failure)) return false;
        if(header.json_name_length > entry->json_length - sizeof(header) || header.key_name_length > entry->length) {
            output->failure = EINVAL; return false;
        }
        int32_t comparison = 1;
        if(previous && !promql_body_compare(output->records, previous->key, previous_header.key_name_length,
            entry->key, header.key_name_length, output->window, frame, &output->failure, &comparison)) return false;
        if(comparison) {
            if(output->metric_limit >= 0 && groups >= (uint64_t)output->metric_limit) break;
            if((groups && !promql_json_text(output->body, "],", frame, &output->failure)) ||
               !promql_body_copy(output->records, entry->json + sizeof(header), header.json_name_length,
                   output->body, output->window, frame, &output->failure) ||
               !promql_json_text(output->body, ":[", frame, &output->failure)) return false;
            groups++; values = 0; previous = entry; previous_header = header;
        }
        if(output->description_limit > 0 && values >= (uint64_t)output->description_limit) continue;
        uint64_t prefix = sizeof(header) + header.json_name_length;
        if((values && !promql_json_text(output->body, ",", frame, &output->failure)) ||
           !promql_body_copy(output->records, entry->json + prefix, entry->json_length - prefix,
               output->body, output->window, frame, &output->failure)) return false;
        values++;
    }
    return (!groups || promql_json_text(output->body, "]", frame, &output->failure)) &&
        promql_json_text(output->body, "}}", frame, &output->failure) &&
        promql_body_seal(output->body, frame, &output->failure);
}

static uint32_t description_visit(void *context, const PROMQL_DESCRIPTION *description,
                                   PROMQL_READ_FRAME *frame, netdata_promql_bytes_t *error) {
    DISCOVERY_OUTPUT *output = context;
    if(output->metric_limit == 0) return NETDATA_PROMQL_OK;
    // The key encoder preserves all four byte fields; these pairs are not series labels.
    netdata_promql_label_t fields[] = {{description->name, description->type}, {description->help, description->unit}};
    if(!intern(output, (PROMQL_KEY_VIEW){.labels = {fields, 2}}, frame)) return output_error(output, frame, error);
    return NETDATA_PROMQL_OK;
}

static void prepare_descriptions(void *context, void *handle __maybe_unused,
                                  const netdata_promql_accounting_context_t *account) {
    DISCOVERY_OUTPUT *output = context;
    PROMQL_API_REQUEST *request = output->request;
    PROMQL_READ_FRAME frame = {.account = account, .call_control = request->operation.control};
    netdata_promql_bytes_t message = {0};
    uint64_t *order = NULL;
    if(!create_records(output, &frame)) goto failed;
    output->status = promql_metadata_describe(&request->context, promql_form_value(request->form, PROMQL_PARAM_METRIC),
        &frame, description_visit, output, &message);
    if(output->status != NETDATA_PROMQL_OK) goto failed;
    if(output->count) {
        if(output->count > SIZE_MAX / sizeof(*order)) { output->failure = EOVERFLOW; goto failed; }
        order = promql_malloc(account, NETDATA_PROMQL_MEMORY_WORKSPACE, output->count * sizeof(*order));
        if(!order) { output->failure = ENOMEM; goto failed; }
        if(!description_sort(output, order, &frame)) goto failed;
    }
    if(!descriptions_write(output, order, &frame)) goto failed;
    if(request->gzip) {
        PROMQL_BODY *encoded = promql_body_gzip(output->body, &frame, web_gzip_level, web_gzip_strategy, &output->failure);
        if(!encoded) goto failed;
        promql_body_release(output->body); output->body = encoded;
    }
    if(!promql_read_poll(&frame)) { output->failure = ECANCELED; goto failed; }
    promql_free(order);
    release_records(output);
    output->committed = true;
    return;
failed:
    if(output->status == NETDATA_PROMQL_OK) output->status = output_error(output, &frame, &message);
    if(message.len) {
        // Direct accounting callbacks do not copy the message into a Rust report.
        size_t length = (size_t)MIN(message.len, sizeof(request->context.error) - 1);
        memmove(request->context.error, message.data, length);
        request->context.error[length] = 0;
    }
    promql_free(order);
    abort_output(output, account);
}

static bool description_limit(PROMQL_API_REQUEST *request, PROMQL_PARAMETER parameter, const char *error,
                                int64_t *limit) {
    netdata_promql_bytes_t text = promql_form_value(request->form, parameter);
    *limit = -1;
    if(!text.len) return true;
    size_t i = 0;
    bool negative = text.data[0] == '-';
    if(negative || text.data[0] == '+') i++;
    if(i == text.len) goto invalid;
    uint64_t value = 0, maximum = negative ? (uint64_t)INT64_MAX + 1 : (uint64_t)INT64_MAX;
    PROMQL_READ_FRAME frame = {.call_control = request->operation.control};
    for(; i < text.len; i++) {
        if(!(i % 1024) && !promql_read_poll(&frame)) {
            promql_api_report_failure(request, frame.interrupted, NULL, (netdata_promql_bytes_t){0}); return false;
        }
        uint8_t digit = text.data[i] - '0';
        if(digit > 9 || value > (maximum - digit) / 10) goto invalid;
        value = value * 10 + digit;
    }
    *limit = negative ? value == (uint64_t)INT64_MAX + 1 ? INT64_MIN : -(int64_t)value : (int64_t)value;
    return true;
invalid:
    promql_api_bad_data(request, NULL, error);
    return false;
}

bool promql_api_metadata(PROMQL_API_REQUEST *request) {
    DISCOVERY_OUTPUT output = {.request = request, .kind = PROMQL_API_METADATA};
    if(!description_limit(request, PROMQL_PARAM_LIMIT, "limit must be a number", &output.metric_limit) ||
       !description_limit(request, PROMQL_PARAM_LIMIT_PER_METRIC, "limit_per_metric must be a number", &output.description_limit))
        return false;
    uint32_t status = netdata_promql_report_accounted(request->report, prepare_descriptions, &output, NULL);
    if(status == NETDATA_PROMQL_OK) status = output.status;
    if(status == NETDATA_PROMQL_OK && output.committed) { request->body = output.body; output.body = NULL; }
    else {
        promql_api_report_failure(request, status == NETDATA_PROMQL_OK ? NETDATA_PROMQL_INTERNAL_ERROR : status,
            NULL, (netdata_promql_bytes_t){0});
        if(request->context.error[0]) request->failure.message = promql_bytes(request->context.error);
    }
    release_records(&output);
    promql_body_release(output.body);
    return request->body != NULL;
}

#ifdef NETDATA_INTERNAL_CHECKS
typedef struct {
    unsigned length;
    bool labels;
    netdata_promql_control_t *control;
    int errors;
    uint64_t encoded_bytes, read_calls;
} DESCRIPTION_KEY_PROBE;

static void description_key_probe(void *context, void *handle __maybe_unused,
                                   const netdata_promql_accounting_context_t *account) {
    DESCRIPTION_KEY_PROBE *probe = context;
    PROMQL_READ_FRAME frame = {.account = account, .call_control = probe->control};
    int error = 0;
    PROMQL_BODY *body = promql_body_create(account, ".", NETDATA_PROMQL_MEMORY_RESULT, &error);
    uint8_t *data = promql_malloc(account, NETDATA_PROMQL_MEMORY_WORKSPACE, MAX(probe->length, 1));
    uint8_t *window = promql_malloc(account, NETDATA_PROMQL_MEMORY_WORKSPACE, PROMQL_BODY_WINDOW);
    if(!body || !data || !window) { probe->errors++; goto done; }
    memset(data, 'q', probe->length);
    if(probe->length) data[0] = data[probe->length / 2] = data[probe->length - 1] = 0;
    netdata_promql_label_t labels[] = {
        {{data, probe->length / 2}, {data + probe->length / 2, probe->length - probe->length / 2}},
        {{NULL, 0}, {(const uint8_t *)"unit", 4}},
    };
    PROMQL_KEY_VIEW view = {.single = !probe->labels, .field = {data, probe->length}, .labels = {labels, 2}};
    uint64_t length, hash;
    memset(window, 'p', PROMQL_BODY_WINDOW);
    if(!promql_body_append(body, window, PROMQL_BODY_WINDOW, &frame, &error)) { probe->errors++; goto done; }
    uint64_t offset = promql_body_length(body);
    if(!promql_key_measure(view, &length, &hash, &frame, &error) ||
       !promql_key_write(body, view, &frame, &error) ||
       !promql_body_append(body, window, PROMQL_BODY_WINDOW, &frame, &error)) { probe->errors++; goto done; }
    probe->encoded_bytes = length;
    PROMQL_FILE_OBSERVATIONS before = promql_body_file_observations(body);
    bool equal = false;
    probe->errors += !promql_key_equal(body, offset, length, view, window, &frame, &error, &equal) || !equal || error;
    PROMQL_FILE_OBSERVATIONS after = promql_body_file_observations(body);
    probe->read_calls = after.read_calls - before.read_calls;
    probe->errors += probe->read_calls != (length + PROMQL_BODY_WINDOW - 1) / PROMQL_BODY_WINDOW ||
        after.read_bytes - before.read_bytes != length;
    if(probe->length > 2) {
        data[1] = 'r';
        probe->errors += !promql_key_equal(body, offset, length, view, window, &frame, &error, &equal) || equal || error;
        data[1] = 'q';
    }
    probe->errors += !promql_key_equal(body, offset, length - 1, view, window, &frame, &error, &equal) || equal || error;
    probe->errors += !promql_key_equal(body, offset, length + 1, view, window, &frame, &error, &equal) || equal || error;
    probe->errors += promql_key_equal(body, promql_body_length(body) + 1, length, view, window, &frame, &error, &equal) || error != EINVAL;
    error = 0;
    probe->errors += netdata_promql_control_cancel(probe->control) != NETDATA_PROMQL_OK;
    probe->errors += promql_key_equal(body, offset, length, view, window, &frame, &error, &equal) ||
        error != ECANCELED || frame.interrupted != NETDATA_PROMQL_CANCELLED;
done:
    promql_body_release(body);
    promql_free(data); promql_free(window);
}

static int description_key_fixture(netdata_promql_engine_t *engine) {
    const unsigned lengths[] = {0, 31, 8190, 8193, 24577};
    int errors = 0;
    for(size_t variant = 0; variant < 2 * sizeof(lengths) / sizeof(lengths[0]); variant++) {
        DESCRIPTION_KEY_PROBE probe = {.length = lengths[variant / 2], .labels = variant % 2};
        netdata_promql_report_t *report = NULL;
        netdata_promql_query_observations_t before = {0}, after = {0};
        int64_t limit;
        probe.errors += netdata_promql_control_create(engine, NULL, &probe.control) != NETDATA_PROMQL_OK;
        probe.errors += netdata_promql_api_parse_limit(engine, (netdata_promql_bytes_t){0}, NULL, &limit, &report) != NETDATA_PROMQL_OK;
        if(report && probe.control) {
            probe.errors += netdata_promql_report_live_observations(report, &before) != NETDATA_PROMQL_OK;
            probe.errors += netdata_promql_report_accounted(report, description_key_probe, &probe, NULL) != NETDATA_PROMQL_OK;
            probe.errors += netdata_promql_report_live_observations(report, &after) != NETDATA_PROMQL_OK;
            for(size_t kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++)
                if(kind != NETDATA_PROMQL_MEMORY_ACCOUNTING) probe.errors += after.memory[kind].current != before.memory[kind].current;
        }
        if(report) netdata_promql_report_release(report);
        if(probe.control) netdata_promql_control_release(probe.control);
        printf("{\"metadata_key_case\":\"%s-%u\",\"errors\":%d,\"encoded_bytes\":%"PRIu64",\"read_calls\":%"PRIu64"}\n",
            probe.labels ? "labels" : "field", probe.length, probe.errors, probe.encoded_bytes, probe.read_calls);
        errors += probe.errors;
    }
    return errors;
}

typedef struct {
    unsigned names, passes;
    int64_t limit;
    netdata_promql_control_t *control;
    int errors;
    uint64_t entries, records_bytes, output_bytes, elapsed[3];
    PROMQL_FILE_OBSERVATIONS io[3];
} DESCRIPTION_COST_PROBE;

static PROMQL_FILE_OBSERVATIONS description_cost_io(const DISCOVERY_OUTPUT *output) {
    PROMQL_FILE_OBSERVATIONS records = promql_body_file_observations(output->records);
    PROMQL_FILE_OBSERVATIONS body = promql_body_file_observations(output->body);
    return (PROMQL_FILE_OBSERVATIONS){records.read_calls + body.read_calls, records.read_bytes + body.read_bytes,
        records.write_calls + body.write_calls, records.write_bytes + body.write_bytes};
}

static bool description_cost_text(PROMQL_BODY *body, uint64_t *offset, const char *expected,
                                   PROMQL_READ_FRAME *frame, int *error) {
    char bytes[256];
    size_t length = strlen(expected);
    if(length > sizeof(bytes) || !promql_body_read(body, *offset, bytes, length, frame, error) ||
       memcmp(bytes, expected, length)) return false;
    *offset += length;
    return true;
}

static void description_cost_probe(void *context, void *handle __maybe_unused,
                                    const netdata_promql_accounting_context_t *account) {
    DESCRIPTION_COST_PROBE *probe = context;
    PROMQL_API_REQUEST request = {.directory = "."};
    DISCOVERY_OUTPUT output = {.request = &request, .kind = PROMQL_API_METADATA,
        .metric_limit = probe->limit, .description_limit = -1};
    PROMQL_READ_FRAME frame = {.account = account, .call_control = probe->control};
    uint64_t *order = NULL;
    netdata_promql_bytes_t error = {0};
    if(!create_records(&output, &frame)) { probe->errors++; goto done; }
    usec_t started = now_monotonic_usec();
    // Separate catalogue passes force duplicate checks against earlier spilled records.
    for(unsigned pass = 0; pass < probe->passes; pass++) {
        for(unsigned i = 0; i < probe->names; i++) {
            char name[32];
            snprintfz(name, sizeof(name) - 1, "fixture_%06u", probe->names - i - 1);
            PROMQL_DESCRIPTION description = {.name = promql_bytes(name), .type = promql_bytes("gauge"),
                .help = promql_bytes("Fixture requests"), .unit = promql_bytes("requests/s")};
            if(description_visit(&output, &description, &frame, &error) != NETDATA_PROMQL_OK) {
                probe->errors++; goto done;
            }
        }
    }
    probe->elapsed[0] = now_monotonic_usec() - started;
    probe->io[0] = description_cost_io(&output);
    probe->entries = output.count;
    probe->records_bytes = promql_body_length(output.records);
    probe->errors += output.count != (probe->limit == 0 ? 0 : probe->names);
    started = now_monotonic_usec();
    if(output.count) {
        order = promql_malloc(account, NETDATA_PROMQL_MEMORY_WORKSPACE, output.count * sizeof(*order));
        if(!order || !description_sort(&output, order, &frame)) { probe->errors++; goto done; }
        for(size_t i = 0; i < output.count; i++) probe->errors += order[i] != output.count - i - 1;
    }
    probe->elapsed[1] = now_monotonic_usec() - started;
    probe->io[1] = description_cost_io(&output);
    started = now_monotonic_usec();
    if(!descriptions_write(&output, order, &frame)) { probe->errors++; goto done; }
    probe->elapsed[2] = now_monotonic_usec() - started;
    probe->io[2] = description_cost_io(&output);
    probe->output_bytes = promql_body_length(output.body);
    uint64_t offset = 0;
    bool valid = promql_body_is_sealed(output.body) &&
        description_cost_text(output.body, &offset, "{\"status\":\"success\",\"data\":{", &frame, &output.failure);
    unsigned names = probe->limit < 0 ? probe->names : MIN((uint64_t)probe->limit, probe->names);
    for(unsigned i = 0; valid && i < names; i++) {
        char expected[256];
        snprintfz(expected, sizeof(expected) - 1,
            "%s\"fixture_%06u\":[{\"type\":\"gauge\",\"help\":\"Fixture requests\",\"unit\":\"requests/s\"}]", i ? "," : "", i);
        valid = description_cost_text(output.body, &offset, expected, &frame, &output.failure);
    }
    probe->errors += !valid || !description_cost_text(output.body, &offset, "}}", &frame, &output.failure) ||
        offset != probe->output_bytes;
done:
    promql_free(order);
    abort_output(&output, account);
}

static int description_cost_fixture(netdata_promql_engine_t *engine) {
    const struct { const char *name; unsigned names, passes; int64_t limit; } cases[] = {
        {"in-memory-duplicates", 16, 16, -1}, {"distinct-catalogue", 5000, 1, -1},
        {"duplicate-catalogue", 5000, 16, -1}, {"duplicate-one", 5000, 16, 1},
        {"duplicate-zero", 5000, 16, 0},
    };
    int errors = 0;
    for(size_t variant = 0; variant < 2 * sizeof(cases) / sizeof(cases[0]); variant++) {
        size_t i = variant / 2;
        bool controlled = variant % 2;
        DESCRIPTION_COST_PROBE probe = {.names = cases[i].names, .passes = cases[i].passes, .limit = cases[i].limit};
        netdata_promql_report_t *report = NULL;
        netdata_promql_query_observations_t before = {0}, observed = {0};
        int64_t limit;
        if(controlled) probe.errors += netdata_promql_control_create(engine, NULL, &probe.control) != NETDATA_PROMQL_OK;
        probe.errors += netdata_promql_api_parse_limit(engine, (netdata_promql_bytes_t){0}, NULL, &limit, &report) != NETDATA_PROMQL_OK;
        if(report) {
            probe.errors += netdata_promql_report_live_observations(report, &before) != NETDATA_PROMQL_OK;
            probe.errors += netdata_promql_report_accounted(report, description_cost_probe, &probe, NULL) != NETDATA_PROMQL_OK;
            probe.errors += netdata_promql_report_live_observations(report, &observed) != NETDATA_PROMQL_OK;
            for(size_t kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++)
                if(kind != NETDATA_PROMQL_MEMORY_ACCOUNTING)
                    probe.errors += observed.memory[kind].current != before.memory[kind].current;
            netdata_promql_report_release(report);
        }
        if(probe.control) netdata_promql_control_release(probe.control);
        printf("{\"metadata_cost_case\":\"%s%s\",\"errors\":%d,\"candidates\":%u,\"entries\":%"PRIu64
            ",\"records_bytes\":%"PRIu64",\"output_bytes\":%"PRIu64",\"phases\":[",
            cases[i].name, controlled ? "-controlled" : "", probe.errors,
            probe.names * probe.passes, probe.entries, probe.records_bytes, probe.output_bytes);
        PROMQL_FILE_OBSERVATIONS previous = {0};
        for(size_t phase = 0; phase < 3; phase++) {
            PROMQL_FILE_OBSERVATIONS current = probe.io[phase];
            const char *names[] = {"prepare", "sort", "emit"};
            printf("%s{\"name\":\"%s\",\"microseconds\":%"PRIu64",\"read_calls\":%"PRIu64",\"read_bytes\":%"PRIu64
                ",\"write_calls\":%"PRIu64",\"write_bytes\":%"PRIu64"}", phase ? "," : "", names[phase], probe.elapsed[phase],
                current.read_calls - previous.read_calls, current.read_bytes - previous.read_bytes,
                current.write_calls - previous.write_calls, current.write_bytes - previous.write_bytes);
            previous = current;
        }
        printf("]}\n");
        errors += probe.errors;
    }
    return errors;
}

typedef struct {
    netdata_promql_engine_t *engine;
    unsigned count, mode;
    uint64_t records_bytes, entries;
    bool reverse;
    int errors;
} DESCRIPTION_SORT_PROBE;

static void description_sort_probe(void *context, void *handle __maybe_unused,
                                    const netdata_promql_accounting_context_t *account) {
    DESCRIPTION_SORT_PROBE *probe = context;
    PROMQL_API_REQUEST request = {0};
    DISCOVERY_OUTPUT output = {.request = &request, .kind = PROMQL_API_METADATA, .metric_limit = probe->mode == 5 ? 0 : -1, .description_limit = -1};
    PROMQL_READ_FRAME frame = {.account = account};
    uint64_t *order = NULL;
    netdata_promql_control_t *control = NULL;
    netdata_promql_bytes_t error = {0};
    output.records = promql_body_create(account, ".", NETDATA_PROMQL_MEMORY_RESULT, &output.failure);
    output.body = promql_body_create(account, ".", NETDATA_PROMQL_MEMORY_RESULT, &output.failure);
    output.window = promql_malloc(account, NETDATA_PROMQL_MEMORY_WORKSPACE, PROMQL_BODY_WINDOW);
    if(!output.records || !output.body || !output.window) { probe->errors++; goto done; }
    char help[9020];
    memset(help, 'x', 9000);
    for(unsigned i = 0; i < probe->count; i++) {
        unsigned key = probe->reverse ? probe->count - i - 1 : i;
        snprintfz(help + 9000, sizeof(help) - 9001, "%06u", key);
        PROMQL_DESCRIPTION description = {.name = promql_bytes("fixture"), .type = promql_bytes("gauge"),
            .help = promql_bytes(help), .unit = promql_bytes("KiB")};
        for(unsigned duplicate = 0; duplicate < 2; duplicate++)
            if(description_visit(&output, &description, &frame, &error) != NETDATA_PROMQL_OK) { probe->errors++; goto done; }
    }
    probe->records_bytes = promql_body_length(output.records);
    probe->entries = output.count;
    probe->errors += output.count != (probe->mode == 5 ? 0 : probe->count);
    if(probe->mode == 5)
        probe->errors += output.entries != NULL || output.slots != NULL || probe->records_bytes != 0;
    order = promql_malloc(account, NETDATA_PROMQL_MEMORY_WORKSPACE, MAX(output.count, 1) * sizeof(*order));
    if(!order) { probe->errors++; goto done; }
    if(probe->mode >= 1 && probe->mode <= 3) {
        probe->errors += netdata_promql_control_create(probe->engine, NULL, &control) != NETDATA_PROMQL_OK;
        if(!control) goto done;
        probe->errors += netdata_promql_control_cancel(control) != NETDATA_PROMQL_OK;
        frame.call_control = control;
        for(size_t i = 0; i < output.count; i++) order[i] = i;
        bool success = probe->mode == 1 ? description_sort(&output, order, &frame) :
            probe->mode == 2 ? description_heap(&output, order, 0, output.count, &frame) :
            descriptions_write(&output, order, &frame);
        probe->errors += success || output.failure != ECANCELED || frame.interrupted != NETDATA_PROMQL_CANCELLED;
    }
    else if(probe->mode == 4) {
        for(size_t i = 0; i < output.count; i++) output.entries[i].key = promql_body_length(output.records) + 1;
        probe->errors += description_sort(&output, order, &frame) || output.failure != EINVAL;
    }
    else {
        probe->errors += !description_sort(&output, order, &frame);
        for(size_t i = 0; i < output.count; i++)
            probe->errors += order[i] != (probe->reverse ? output.count - i - 1 : i);
        probe->errors += !descriptions_write(&output, order, &frame) || !promql_body_is_sealed(output.body);
    }
done:
    promql_free(order);
    abort_output(&output, account);
    if(control) netdata_promql_control_release(control);
}

int promql_description_sort_fixture(netdata_promql_engine_t *engine) {
    const struct { const char *name; unsigned count, mode; bool reverse; } cases[] = {
        {"empty", 0, 0, false}, {"single", 1, 0, false}, {"sorted-long", 37, 0, false},
        {"reverse-long", 37, 0, true}, {"cancel-initialization", 37, 1, true},
        {"cancel-comparison", 37, 2, true}, {"cancel-emission", 37, 3, true}, {"read-failure", 37, 4, true},
        {"zero-limit-long", 37, 5, true},
    };
    int errors = 0;
    for(size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        DESCRIPTION_SORT_PROBE probe = {.engine = engine, .count = cases[i].count, .mode = cases[i].mode, .reverse = cases[i].reverse};
        netdata_promql_report_t *report = NULL;
        int64_t limit;
        probe.errors += netdata_promql_api_parse_limit(engine, (netdata_promql_bytes_t){0}, NULL, &limit, &report) != NETDATA_PROMQL_OK;
        if(report) {
            probe.errors += netdata_promql_report_accounted(report, description_sort_probe, &probe, NULL) != NETDATA_PROMQL_OK;
            netdata_promql_report_release(report);
        }
        netdata_promql_engine_observations_t observed = {0};
        probe.errors += netdata_promql_engine_observations(engine, &observed) != NETDATA_PROMQL_OK;
        for(size_t kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++)
            if(kind != NETDATA_PROMQL_MEMORY_ACCOUNTING) probe.errors += observed.memory[kind].current != 0;
        printf("{\"metadata_sort_case\":\"%s\",\"errors\":%d,\"entries\":%"PRIu64",\"records_bytes\":%"PRIu64"}\n",
            cases[i].name, probe.errors, probe.entries, probe.records_bytes);
        errors += probe.errors;
    }
    return errors + description_cost_fixture(engine) + description_key_fixture(engine);
}
#endif
