// SPDX-License-Identifier: GPL-3.0-or-later

#include "aclk-source.h"
#include "json.h"

typedef struct {
    struct mqtt_payload_source source;
    PROMQL_BODY *prefix, *body;
    netdata_promql_allocation_handle_t *header_view;
    uint8_t window[PROMQL_BODY_WINDOW];
} ACLK_SOURCE;

static void source_release(void *context) {
    ACLK_SOURCE *source = context;
    PROMQL_BODY *body = source->body;
    promql_body_release(source->prefix);
    if(source->header_view) netdata_promql_allocation_release(source->header_view);
    promql_free(source);
    promql_body_release(body);
}

static bool source_view(void *context, uint64_t bytes) {
    return netdata_promql_allocation_resize(((ACLK_SOURCE *)context)->header_view, bytes) == NETDATA_PROMQL_OK;
}

static bool source_read(void *context, uint64_t offset, void *bytes, size_t length) {
    ACLK_SOURCE *source = context;
    if(offset > source->source.length || length > source->source.length - offset || (length && !bytes)) return false;
    uint64_t prefix = promql_body_length(source->prefix);
    int error = 0;
    if(offset < prefix) {
        size_t count = (size_t)MIN((uint64_t)length, prefix - offset);
        if(!promql_body_read(source->prefix, offset, bytes, count, NULL, &error)) return false;
        offset += count; bytes = (uint8_t *)bytes + count; length -= count;
    }
    return !length || promql_body_read(source->body, offset - prefix, bytes, length, NULL, &error);
}

static bool unsigned_bits(PROMQL_BODY *body, usec_t value, PROMQL_READ_FRAME *frame, int *error) {
    // Existing json-c int64 fields preserve the signed interpretation of these bits.
    int64_t signed_value;
    static_assert(sizeof(value) == sizeof(signed_value), "ACLK timestamp width");
    memcpy(&signed_value, &value, sizeof(value));
    return promql_json_signed(body, signed_value, frame, error);
}

typedef struct {
    PROMQL_BODY *body;
    const PROMQL_ACLK_ENVELOPE *envelope;
    ACLK_SOURCE *source;
    int error;
} SOURCE_PREPARATION;

static void prepare_source(void *context, void *handle __maybe_unused,
                            const netdata_promql_accounting_context_t *account) {
    SOURCE_PREPARATION *preparation = context;
    const PROMQL_ACLK_ENVELOPE *envelope = preparation->envelope;
    ACLK_SOURCE *source = promql_malloc(account, NETDATA_PROMQL_MEMORY_RESULT, sizeof(*source));
    if(!source) { preparation->error = ENOMEM; return; }
    source->prefix = promql_body_create(account, promql_body_directory(preparation->body),
                                         NETDATA_PROMQL_MEMORY_RESULT, &preparation->error);
    if(!source->prefix || !promql_body_acquire(preparation->body)) {
        if(!preparation->error) preparation->error = EOVERFLOW;
        source_release(source); return;
    }
    source->body = preparation->body;
    PROMQL_READ_FRAME frame = {.account = account};
    PROMQL_BODY *prefix = source->prefix;
    bool valid = netdata_promql_allocation_acquire(account, NETDATA_PROMQL_MEMORY_BORROWED_VIEW, 0,
        &source->header_view) == NETDATA_PROMQL_OK;
    if(!valid) preparation->error = ENOMEM;
    valid = valid &&
        promql_json_text(prefix, "{\"type\":\"http\",\"msg-id\":", &frame, &preparation->error) &&
        promql_json_string(prefix, promql_bytes(envelope->message_id), PROMQL_JSON_NATIVE, &frame, &preparation->error) &&
        promql_json_text(prefix, ",\"timestamp\":", &frame, &preparation->error) &&
        unsigned_bits(prefix, envelope->timestamp / USEC_PER_SEC, &frame, &preparation->error) &&
        promql_json_text(prefix, ",\"timestamp-offset-usec\":", &frame, &preparation->error) &&
        unsigned_bits(prefix, envelope->timestamp % USEC_PER_SEC, &frame, &preparation->error) &&
        promql_json_text(prefix, ",\"connect\":", &frame, &preparation->error) &&
        unsigned_bits(prefix, envelope->session / USEC_PER_SEC, &frame, &preparation->error) &&
        promql_json_text(prefix, ",\"connect-offset-usec\":", &frame, &preparation->error) &&
        unsigned_bits(prefix, envelope->session % USEC_PER_SEC, &frame, &preparation->error) &&
        promql_json_text(prefix, ",\"version\":", &frame, &preparation->error) &&
        promql_json_signed(prefix, envelope->version, &frame, &preparation->error) &&
        promql_json_text(prefix, ",\"t-exec\":", &frame, &preparation->error) &&
        unsigned_bits(prefix, envelope->execution_time, &frame, &preparation->error) &&
        promql_json_text(prefix, ",\"t-rx\":", &frame, &preparation->error) &&
        unsigned_bits(prefix, envelope->received_time, &frame, &preparation->error) &&
        promql_json_text(prefix, ",\"http-code\":", &frame, &preparation->error) &&
        promql_json_signed(prefix, envelope->code, &frame, &preparation->error) &&
        promql_json_text(prefix, "}", &frame, &preparation->error) &&
        (!(promql_body_length(source->body) || (envelope->headers && envelope->headers_length)) ||
         promql_json_text(prefix, "\r\n\r\n", &frame, &preparation->error)) &&
        promql_body_append(prefix, envelope->headers, envelope->headers ? envelope->headers_length : 0,
                             &frame, &preparation->error) && promql_body_seal(prefix, &frame, &preparation->error);
    uint64_t prefix_length = promql_body_length(prefix), length = promql_body_length(source->body);
    if(valid && length > UINT64_MAX - prefix_length) { valid = false; preparation->error = EOVERFLOW; }
    if(!valid) { source_release(source); return; }
    source->source = (struct mqtt_payload_source){.context = source, .length = prefix_length + length,
        .window = source->window, .window_size = sizeof(source->window), .read = source_read,
        .view = source_view, .release = source_release};
    preparation->source = source;
}

struct mqtt_payload_source *promql_aclk_source(PROMQL_BODY *body, const PROMQL_ACLK_ENVELOPE *envelope, int *error) {
    *error = 0;
    if(!promql_body_is_sealed(body) || !envelope || !envelope->message_id) { *error = EINVAL; return NULL; }
    SOURCE_PREPARATION preparation = {.body = body, .envelope = envelope};
    uint32_t status = promql_body_accounted(body, prepare_source, &preparation, NULL);
    *error = status == NETDATA_PROMQL_OK ? preparation.error : EINVAL;
    return preparation.source ? &preparation.source->source : NULL;
}
