// SPDX-License-Identifier: GPL-3.0-or-later

#include "keys.h"

typedef bool (*KEY_PART)(void *, const uint8_t *, size_t, PROMQL_READ_FRAME *, int *);

static bool key_field(netdata_promql_bytes_t field, KEY_PART part, void *context,
                       PROMQL_READ_FRAME *frame, int *error) {
    if(field.len > SIZE_MAX || (field.len && !field.data)) { *error = EOVERFLOW; return false; }
    size_t position = 0;
    while(position < (size_t)field.len) {
        if(!promql_read_poll(frame)) { *error = ECANCELED; return false; }
        size_t count = (size_t)MIN(field.len - position, (uint64_t)PROMQL_BODY_WINDOW);
        const uint8_t *zero = memchr(field.data + position, 0, count);
        if(zero) count = (size_t)(zero - field.data - position);
        if(count && !part(context, field.data + position, count, frame, error)) return false;
        position += count;
        if(zero) {
            if(!part(context, (const uint8_t *)"\0\1", 2, frame, error)) return false;
            position++;
        }
    }
    return part(context, (const uint8_t *)"\0\0", 2, frame, error);
}

static bool visit_key(PROMQL_KEY_VIEW view, KEY_PART part, void *context, PROMQL_READ_FRAME *frame, int *error) {
    if(view.single) return key_field(view.field, part, context, frame, error);
    if(view.labels.len > SIZE_MAX / sizeof(*view.labels.data) || (view.labels.len && !view.labels.data)) {
        *error = EOVERFLOW; return false;
    }
    for(uint64_t i = 0; i < view.labels.len; i++)
        if(!key_field(view.labels.data[i].name, part, context, frame, error) ||
           !key_field(view.labels.data[i].value, part, context, frame, error)) return false;
    return true;
}

typedef struct { uint64_t length, hash; } KEY_MEASURE;

static bool measure_part(void *context, const uint8_t *bytes, size_t length,
                          PROMQL_READ_FRAME *frame __maybe_unused, int *error) {
    KEY_MEASURE *measure = context;
    if(length > UINT64_MAX - measure->length) { *error = EOVERFLOW; return false; }
    measure->length += length;
    for(size_t i = 0; i < length; i++) {
        measure->hash ^= bytes[i];
        measure->hash *= UINT64_C(1099511628211);
    }
    return true;
}

bool promql_key_measure(PROMQL_KEY_VIEW view, uint64_t *length, uint64_t *hash,
                        PROMQL_READ_FRAME *frame, int *error) {
    KEY_MEASURE measure = {.hash = UINT64_C(14695981039346656037)};
    if(!visit_key(view, measure_part, &measure, frame, error)) return false;
    *length = measure.length; *hash = measure.hash;
    return true;
}

static bool write_part(void *context, const uint8_t *bytes, size_t length, PROMQL_READ_FRAME *frame, int *error) {
    return promql_body_append(context, bytes, length, frame, error);
}

bool promql_key_write(PROMQL_BODY *body, PROMQL_KEY_VIEW view, PROMQL_READ_FRAME *frame, int *error) {
    return visit_key(view, write_part, body, frame, error);
}

typedef struct { PROMQL_BODY *body; uint64_t offset; uint8_t *window; bool equal; } KEY_EQUAL;

static bool equal_part(void *context, const uint8_t *bytes, size_t length, PROMQL_READ_FRAME *frame, int *error) {
    KEY_EQUAL *comparison = context;
    if(!promql_body_read(comparison->body, comparison->offset, comparison->window, length, frame, error)) return false;
    comparison->equal = memcmp(comparison->window, bytes, length) == 0;
    comparison->offset += length;
    return comparison->equal;
}

bool promql_key_equal(PROMQL_BODY *body, uint64_t offset, PROMQL_KEY_VIEW view,
                      uint8_t window[PROMQL_BODY_WINDOW], PROMQL_READ_FRAME *frame, int *error, bool *equal) {
    KEY_EQUAL comparison = {.body = body, .offset = offset, .window = window, .equal = true};
    bool success = visit_key(view, equal_part, &comparison, frame, error);
    *equal = comparison.equal;
    return success || (!comparison.equal && !*error);
}
