// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_BODY_H
#define NETDATA_PROMQL_BODY_H

#include "backing.h"

#define PROMQL_BODY_WINDOW 8192U

typedef struct promql_body PROMQL_BODY;

#ifdef NETDATA_INTERNAL_CHECKS
PROMQL_FILE_OBSERVATIONS promql_body_file_observations(const PROMQL_BODY *body);
#endif

PROMQL_BODY *promql_body_create(const netdata_promql_accounting_context_t *account, const char *directory,
                                uint32_t kind, int *error);
bool promql_body_acquire(PROMQL_BODY *body);
void promql_body_release(PROMQL_BODY *body);
uint64_t promql_body_length(const PROMQL_BODY *body);
uint64_t promql_body_plain_length(const PROMQL_BODY *body);
const char *promql_body_directory(const PROMQL_BODY *body);
bool promql_body_append(PROMQL_BODY *body, const void *bytes, size_t length, PROMQL_READ_FRAME *frame, int *error);
bool promql_body_write(PROMQL_BODY *body, uint64_t offset, const void *bytes, size_t length,
                       PROMQL_READ_FRAME *frame, int *error);
bool promql_body_read(PROMQL_BODY *body, uint64_t offset, void *bytes, size_t length,
                      PROMQL_READ_FRAME *frame, int *error);
bool promql_body_copy(PROMQL_BODY *source, uint64_t offset, uint64_t length, PROMQL_BODY *target,
                      uint8_t window[PROMQL_BODY_WINDOW], PROMQL_READ_FRAME *frame, int *error);
bool promql_body_compare(PROMQL_BODY *body, uint64_t left, uint64_t left_length, uint64_t right,
                         uint64_t right_length, uint8_t window[PROMQL_BODY_WINDOW],
                         PROMQL_READ_FRAME *frame, int *error, int32_t *ordering);
bool promql_body_seal(PROMQL_BODY *body, PROMQL_READ_FRAME *frame, int *error);
bool promql_body_is_sealed(const PROMQL_BODY *body);
bool promql_body_is_gzip(const PROMQL_BODY *body);
bool promql_body_attach_report(PROMQL_BODY *body, netdata_promql_report_t *report);
uint32_t promql_body_accounted(PROMQL_BODY *body, netdata_promql_close_t callback, void *context, void *handle);
PROMQL_BODY *promql_body_gzip(PROMQL_BODY *plain, PROMQL_READ_FRAME *frame, int level, int strategy, int *error);

#endif
