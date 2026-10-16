// SPDX-License-Identifier: GPL-3.0-or-later

// Isolated I/O tests do not stand in for real callback-accounting qualification.
#ifndef NETDATA_PROMQL_PROVIDER_MEMORY_H
#define NETDATA_PROMQL_PROVIDER_MEMORY_H
#include "netdata-promql.h"
#include <stdbool.h>
#include <stdint.h>
#include <inttypes.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <stdarg.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#define MIN(a, b) ((a) < (b) ? (a) : (b))
#define MAX(a, b) ((a) > (b) ? (a) : (b))
#define __maybe_unused __attribute__((unused))
typedef struct {
    const netdata_promql_callback_control_t *control;
    const netdata_promql_accounting_context_t *account;
    uint32_t interrupted;
    bool ready;
} PROMQL_READ_FRAME;
void *output_test_allocate(size_t);
size_t output_test_size(void *);
void output_test_free(void *);
bool output_test_poll(PROMQL_READ_FRAME *);
static inline void *promql_malloc(const netdata_promql_accounting_context_t *account,
                                 uint32_t kind, size_t length) {
    (void)account; (void)kind;
    return output_test_allocate(length);
}
static inline void promql_free(void *data) { output_test_free(data); }
static inline void *promql_resize(const netdata_promql_accounting_context_t *account, void *data, size_t length) {
    void *replacement = promql_malloc(account, 0, length);
    if(replacement) { memcpy(replacement, data, MIN(length, output_test_size(data))); promql_free(data); }
    return replacement;
}
static inline char *promql_strdup(const netdata_promql_accounting_context_t *account, const char *text) {
    if(!text) text = "";
    size_t length = strlen(text) + 1;
    char *copy = promql_malloc(account, 0, length);
    if(copy) memcpy(copy, text, length);
    return copy;
}
static inline netdata_promql_bytes_t promql_bytes(const char *text) {
    return (netdata_promql_bytes_t){(const uint8_t *)text, text ? strlen(text) : 0};
}
static inline bool promql_read_poll(PROMQL_READ_FRAME *frame) { return output_test_poll(frame); }
static inline void promql_read_operation(PROMQL_READ_FRAME *frame,
                                        const netdata_promql_callback_control_t *control) {
    *frame = (PROMQL_READ_FRAME){.control = control, .ready = true};
}
static inline int snprintfz(char *out, size_t length, const char *format, ...) {
    va_list args;
    va_start(args, format);
    int result = vsnprintf(out, length, format, args);
    va_end(args);
    return result;
}
#endif
