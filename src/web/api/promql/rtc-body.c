// SPDX-License-Identifier: GPL-3.0-or-later

#include "rtc-body.h"
#include "json.h"

typedef struct {
    netdata_promql_allocation_handle_t *exposure;
    char bytes[PROMQL_RTC_HEADER_SIZE + PROMQL_BODY_WINDOW + 4];
} RTC_FRAME;

typedef struct { RTC_FRAME *frame; int error; } FRAME_PREPARATION;

static void prepare_frame(void *context, void *handle __maybe_unused,
                           const netdata_promql_accounting_context_t *account) {
    FRAME_PREPARATION *preparation = context;
    preparation->frame = promql_malloc(account, NETDATA_PROMQL_MEMORY_RESULT, sizeof(*preparation->frame));
    if(!preparation->frame) { preparation->error = ENOMEM; return; }
    if(netdata_promql_allocation_acquire(account, NETDATA_PROMQL_MEMORY_BORROWED_VIEW, 0,
        &preparation->frame->exposure) != NETDATA_PROMQL_OK) {
        promql_free(preparation->frame); preparation->frame = NULL; preparation->error = ENOMEM;
    }
}

static size_t text_unit(const uint8_t *bytes, size_t length) {
    uint32_t rune;
    size_t width = promql_utf8_rune(bytes, length, &rune);
    if(width) return width;
    uint8_t lead = bytes[0];
    size_t expected = lead >= 0xc2 && lead <= 0xdf ? 2 : lead >= 0xe0 && lead <= 0xef ? 3 :
        lead >= 0xf0 && lead <= 0xf4 ? 4 : 1;
    size_t prefix = 1;
    // Preserve incomplete UTF-8 subparts together, as a whole-stream text decoder would consume them.
    while(prefix < expected && prefix < length) {
        uint8_t byte = bytes[prefix];
        if(byte < 0x80 || byte > 0xbf || (prefix == 1 &&
            ((lead == 0xe0 && byte < 0xa0) || (lead == 0xed && byte >= 0xa0) ||
             (lead == 0xf0 && byte < 0x90) || (lead == 0xf4 && byte >= 0x90)))) break;
        prefix++;
    }
    return prefix;
}

static size_t body_chunk(PROMQL_BODY *body, uint64_t offset, size_t capacity, RTC_FRAME *frame, int *error) {
    uint64_t remaining = promql_body_length(body) - offset;
    size_t available = (size_t)MIN(remaining, (uint64_t)capacity + 3);
    uint8_t *bytes = (uint8_t *)frame->bytes + PROMQL_RTC_HEADER_SIZE;
    if(!promql_body_read(body, offset, bytes, available, NULL, error)) return 0;
    size_t used = 0;
    while(used < available) {
        if(!bytes[used]) { *error = EINVAL; return 0; }
        size_t width = text_unit(bytes + used, available - used);
        if(width > capacity - used) break;
        used += width;
        if(used == capacity) break;
    }
    if(!used && remaining) *error = EMSGSIZE;
    return used;
}

bool promql_rtc_body_send(PROMQL_BODY *body, int code, const char *content_type, size_t max_message_size,
                           const PROMQL_RTC_WRITER *writer, uint64_t *sent, int *error) {
    *sent = 0; *error = 0;
    if(!promql_body_is_sealed(body) || promql_body_is_gzip(body) || !content_type || !writer || !writer->ready || !writer->send) {
        *error = EINVAL; return false;
    }
    if(max_message_size <= PROMQL_RTC_HEADER_SIZE + 1) { *error = EMSGSIZE; return false; }
    size_t capacity = MIN((size_t)PROMQL_BODY_WINDOW, max_message_size - PROMQL_RTC_HEADER_SIZE - 1);
    FRAME_PREPARATION preparation = {0};
    uint32_t status = promql_body_accounted(body, prepare_frame, &preparation, NULL);
    if(status != NETDATA_PROMQL_OK || !preparation.frame) {
        *error = preparation.error ? preparation.error : EINVAL; return false;
    }
    RTC_FRAME *frame = preparation.frame;
    uint64_t length = promql_body_length(body), total = 0, chunk = 0;
    for(uint64_t offset = 0; !*error && offset < length;) {
        if(!writer->ready(writer->context)) { *error = ECANCELED; break; }
        size_t count = body_chunk(body, offset, capacity, frame, error);
        if(!count) break;
        offset += count; total++;
    }
    for(uint64_t offset = 0; !*error && offset < length;) {
        if(!writer->ready(writer->context)) { *error = ECANCELED; break; }
        if(netdata_promql_allocation_resize(frame->exposure, 0) != NETDATA_PROMQL_OK) { *error = ENOMEM; break; }
        size_t count = body_chunk(body, offset, capacity, frame, error);
        if(!count) break;
        int header = snprintf(frame->bytes, PROMQL_RTC_HEADER_SIZE, "%d PLAIN %zu %"PRIu64" %"PRIu64" %s\r\n",
                                code, count, ++chunk, total, content_type);
        if(header <= 0 || (size_t)header >= PROMQL_RTC_HEADER_SIZE ||
           (size_t)header + count >= max_message_size || (uint64_t)header + count > UINT64_MAX - *sent) {
            *error = EOVERFLOW; break;
        }
        memmove(frame->bytes + header, frame->bytes + PROMQL_RTC_HEADER_SIZE, count);
        frame->bytes[header + count] = 0;
        size_t bytes = (size_t)header + count;
        if(netdata_promql_allocation_resize(frame->exposure, bytes) != NETDATA_PROMQL_OK) { *error = ENOMEM; break; }
        if(!writer->send(writer->context, frame->bytes, bytes)) { *error = EIO; break; }
        *sent += bytes; offset += count;
    }
    if(!*error && !writer->ready(writer->context)) *error = ECANCELED;
    netdata_promql_allocation_release(frame->exposure);
    promql_free(frame);
    return *error == 0;
}
