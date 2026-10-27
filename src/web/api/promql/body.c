// SPDX-License-Identifier: GPL-3.0-or-later

#include "body.h"
#include <zlib.h>

struct promql_body {
    uint32_t references, kind;
    uint64_t length, plain_length;
    size_t pending;
    bool sealed, gzip;
    char *directory;
    PROMQL_FILE *file;
    netdata_promql_report_t *report;
    uint8_t bytes[PROMQL_BODY_WINDOW];
};

PROMQL_BODY *promql_body_create(const netdata_promql_accounting_context_t *account, const char *directory,
                                uint32_t kind, int *error) {
    *error = 0;
    PROMQL_BODY *body = promql_malloc(account, kind, sizeof(*body));
    if(!body) { *error = ENOMEM; return NULL; }
    size_t length = directory ? strlen(directory) : 0;
    body->directory = promql_malloc(account, kind, length + 1);
    if(!body->directory) { promql_free(body); *error = ENOMEM; return NULL; }
    if(length) memcpy(body->directory, directory, length);
    body->references = 1;
    body->kind = kind;
    return body;
}

bool promql_body_acquire(PROMQL_BODY *body) {
    uint32_t references = __atomic_load_n(&body->references, __ATOMIC_RELAXED);
    do {
        if(!references || references == UINT32_MAX) return false;
    } while(!__atomic_compare_exchange_n(&body->references, &references, references + 1,
                                        false, __ATOMIC_RELAXED, __ATOMIC_RELAXED));
    return true;
}

void promql_body_release(PROMQL_BODY *body) {
    if(!body || __atomic_sub_fetch(&body->references, 1, __ATOMIC_ACQ_REL)) return;
    netdata_promql_report_t *report = body->report;
    promql_file_close(body->file);
    promql_free(body->directory);
    promql_free(body);
    if(report) netdata_promql_report_release(report);
}

uint64_t promql_body_length(const PROMQL_BODY *body) { return body ? body->length : 0; }
#ifdef NETDATA_INTERNAL_CHECKS
PROMQL_FILE_OBSERVATIONS promql_body_file_observations(const PROMQL_BODY *body) {
    return body && body->file ? body->file->observations : (PROMQL_FILE_OBSERVATIONS){0};
}
#endif
const char *promql_body_directory(const PROMQL_BODY *body) { return body ? body->directory : NULL; }
uint64_t promql_body_plain_length(const PROMQL_BODY *body) {
    return body ? (body->gzip ? body->plain_length : body->length) : 0;
}
bool promql_body_is_sealed(const PROMQL_BODY *body) { return body && body->sealed; }
bool promql_body_is_gzip(const PROMQL_BODY *body) { return body && body->gzip; }

static bool body_flush(PROMQL_BODY *body, PROMQL_READ_FRAME *frame, int *error) {
    if(!body->pending) return true;
    if(!body->file) {
        if(!frame || !frame->account) { *error = EINVAL; return false; }
        body->file = promql_file_open(body->directory, frame->account, body->kind, error);
        if(!body->file) return false;
    }
    if(!promql_file_write(body->file, body->file->length, body->bytes, body->pending, frame, error)) return false;
    body->pending = 0;
    return true;
}

bool promql_body_append(PROMQL_BODY *body, const void *bytes, size_t length, PROMQL_READ_FRAME *frame, int *error) {
    *error = 0;
    if(body->sealed || length > UINT64_MAX - body->length) { *error = body->sealed ? EINVAL : EOVERFLOW; return false; }
    const uint8_t *source = bytes;
    while(length) {
        if(frame && !promql_read_poll(frame)) { *error = ECANCELED; return false; }
        if(body->pending == sizeof(body->bytes) && !body_flush(body, frame, error)) return false;
        size_t count = MIN(length, sizeof(body->bytes) - body->pending);
        memcpy(body->bytes + body->pending, source, count);
        body->pending += count;
        body->length += count;
        source += count;
        length -= count;
    }
    return true;
}

bool promql_body_read(PROMQL_BODY *body, uint64_t offset, void *bytes, size_t length,
                      PROMQL_READ_FRAME *frame, int *error) {
    *error = 0;
    if(offset > body->length || length > body->length - offset) { *error = EINVAL; return false; }
    if(frame && !promql_read_poll(frame)) { *error = ECANCELED; return false; }
    uint64_t flushed = body->file ? body->file->length : 0;
    if(offset < flushed) {
        size_t count = (size_t)MIN((uint64_t)length, flushed - offset);
        if(!promql_file_read(body->file, offset, bytes, count, frame, error)) return false;
        bytes = (uint8_t *)bytes + count;
        offset += count;
        length -= count;
    }
    if(length) memcpy(bytes, body->bytes + (size_t)(offset - flushed), length);
    return true;
}

bool promql_body_write(PROMQL_BODY *body, uint64_t offset, const void *bytes, size_t length,
                       PROMQL_READ_FRAME *frame, int *error) {
    *error = 0;
    if(body->sealed || offset > body->length || length > body->length - offset) { *error = EINVAL; return false; }
    if(frame && !promql_read_poll(frame)) { *error = ECANCELED; return false; }
    uint64_t flushed = body->file ? body->file->length : 0;
    if(offset < flushed) {
        size_t count = (size_t)MIN((uint64_t)length, flushed - offset);
        if(!promql_file_write(body->file, offset, bytes, count, frame, error)) return false;
        bytes = (const uint8_t *)bytes + count;
        offset += count;
        length -= count;
    }
    if(length) memcpy(body->bytes + (size_t)(offset - flushed), bytes, length);
    return true;
}

bool promql_body_copy(PROMQL_BODY *source, uint64_t offset, uint64_t length, PROMQL_BODY *target,
                      uint8_t window[PROMQL_BODY_WINDOW], PROMQL_READ_FRAME *frame, int *error) {
    while(length) {
        size_t count = (size_t)MIN(length, (uint64_t)PROMQL_BODY_WINDOW);
        if(!promql_body_read(source, offset, window, count, frame, error) ||
           !promql_body_append(target, window, count, frame, error)) return false;
        offset += count; length -= count;
    }
    return true;
}

bool promql_body_compare(PROMQL_BODY *body, uint64_t left, uint64_t left_length, uint64_t right,
                         uint64_t right_length, uint8_t window[PROMQL_BODY_WINDOW],
                         PROMQL_READ_FRAME *frame, int *error, int32_t *ordering) {
    uint64_t position = 0, length = MIN(left_length, right_length);
    while(position < length) {
        size_t count = (size_t)MIN(length - position, (uint64_t)PROMQL_BODY_WINDOW / 2);
        if(!promql_body_read(body, left + position, window, count, frame, error) ||
           !promql_body_read(body, right + position, window + PROMQL_BODY_WINDOW / 2, count, frame, error)) return false;
        int difference = memcmp(window, window + PROMQL_BODY_WINDOW / 2, count);
        if(difference) { *ordering = difference < 0 ? -1 : 1; return true; }
        position += count;
    }
    *ordering = left_length < right_length ? -1 : left_length > right_length ? 1 : 0;
    return true;
}

bool promql_body_seal(PROMQL_BODY *body, PROMQL_READ_FRAME *frame, int *error) {
    *error = 0;
    if(body->sealed) return true;
    if(frame && !promql_read_poll(frame)) { *error = ECANCELED; return false; }
    if(body->file && !body_flush(body, frame, error)) return false;
    body->sealed = true;
    return true;
}

bool promql_body_attach_report(PROMQL_BODY *body, netdata_promql_report_t *report) {
    if(!body || !body->sealed || body->report || !report) return false;
    body->report = report;
    return true;
}

uint32_t promql_body_accounted(PROMQL_BODY *body, netdata_promql_close_t callback, void *context, void *handle) {
    if(!body || !body->report) return NETDATA_PROMQL_INVALID_ARGUMENT;
    return netdata_promql_report_accounted(body->report, callback, context, handle);
}

static voidpf gzip_allocate(voidpf opaque, uInt count, uInt width) {
    PROMQL_READ_FRAME *frame = opaque;
    if(width && count > SIZE_MAX / width) return NULL;
    return promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_RESULT, (size_t)count * width);
}

static void gzip_free(voidpf opaque __maybe_unused, voidpf memory) { promql_free(memory); }

PROMQL_BODY *promql_body_gzip(PROMQL_BODY *plain, PROMQL_READ_FRAME *frame, int level, int strategy, int *error) {
    *error = 0;
    if(!plain || !plain->sealed || !frame || !frame->account) { *error = EINVAL; return NULL; }
    PROMQL_BODY *encoded = promql_body_create(frame->account, plain->directory, NETDATA_PROMQL_MEMORY_RESULT, error);
    if(!encoded) return NULL;
    uint8_t *windows = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_RESULT, 2 * PROMQL_BODY_WINDOW);
    if(!windows) { *error = ENOMEM; promql_body_release(encoded); return NULL; }
    z_stream stream = {.zalloc = gzip_allocate, .zfree = gzip_free, .opaque = frame};
    bool initialized = deflateInit2(&stream, level, Z_DEFLATED, 15 + 16, 8, strategy) == Z_OK;
    if(!initialized) *error = ENOMEM;
    uint64_t offset = 0;
    bool finished = false;
    while(initialized && !*error && !finished) {
        if(!promql_read_poll(frame)) { *error = ECANCELED; break; }
        if(!stream.avail_in && offset < plain->length) {
            size_t count = (size_t)MIN((uint64_t)PROMQL_BODY_WINDOW, plain->length - offset);
            if(!promql_body_read(plain, offset, windows, count, frame, error)) break;
            offset += count;
            stream.next_in = windows;
            stream.avail_in = (uInt)count;
        }
        stream.next_out = windows + PROMQL_BODY_WINDOW;
        stream.avail_out = PROMQL_BODY_WINDOW;
        int status = deflate(&stream, offset == plain->length ? Z_FINISH : Z_NO_FLUSH);
        if(status != Z_OK && status != Z_STREAM_END) { *error = EIO; break; }
        size_t produced = PROMQL_BODY_WINDOW - stream.avail_out;
        if(!promql_body_append(encoded, windows + PROMQL_BODY_WINDOW, produced, frame, error)) break;
        finished = status == Z_STREAM_END;
    }
    if(initialized) deflateEnd(&stream);
    promql_free(windows);
    if(!*error && (!finished || !promql_body_seal(encoded, frame, error))) {
        if(!*error) *error = EIO;
    }
    if(*error) { promql_body_release(encoded); return NULL; }
    encoded->gzip = true;
    encoded->plain_length = plain->length;
    return encoded;
}
