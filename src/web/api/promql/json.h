// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_JSON_H
#define NETDATA_PROMQL_JSON_H

#include "body.h"

typedef enum { PROMQL_JSON_METRIC, PROMQL_JSON_ENVELOPE, PROMQL_JSON_SCALAR, PROMQL_JSON_NATIVE } PROMQL_JSON_MODE;

bool promql_json_text(PROMQL_BODY *body, const char *text, PROMQL_READ_FRAME *frame, int *error);
bool promql_json_string(PROMQL_BODY *body, netdata_promql_bytes_t bytes, PROMQL_JSON_MODE mode,
                         PROMQL_READ_FRAME *frame, int *error);
bool promql_json_escape(PROMQL_BODY *body, netdata_promql_bytes_t bytes, PROMQL_JSON_MODE mode,
                         PROMQL_READ_FRAME *frame, int *error);
bool promql_json_go_quote(PROMQL_BODY *body, netdata_promql_bytes_t bytes, PROMQL_READ_FRAME *frame, int *error);
size_t promql_utf8_rune(const uint8_t *bytes, size_t length, uint32_t *rune);
bool promql_json_float(PROMQL_BODY *body, uint64_t bits, bool scalar, PROMQL_READ_FRAME *frame, int *error);
bool promql_json_time(PROMQL_BODY *body, int64_t milliseconds, bool scalar, PROMQL_READ_FRAME *frame, int *error);
bool promql_json_unsigned(PROMQL_BODY *body, uint64_t number, PROMQL_READ_FRAME *frame, int *error);
bool promql_json_signed(PROMQL_BODY *body, int64_t number, PROMQL_READ_FRAME *frame, int *error);
bool promql_json_labels(PROMQL_BODY *body, netdata_promql_labels_t labels, PROMQL_JSON_MODE mode,
                         PROMQL_READ_FRAME *frame, int *error);

#endif
