// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_REQUEST_H
#define NETDATA_PROMQL_REQUEST_H

#include "json.h"

typedef enum {
    PROMQL_PARAM_QUERY, PROMQL_PARAM_TIME, PROMQL_PARAM_TIMEOUT, PROMQL_PARAM_LOOKBACK,
    PROMQL_PARAM_LIMIT, PROMQL_PARAM_STATS, PROMQL_PARAM_START, PROMQL_PARAM_END, PROMQL_PARAM_STEP,
    PROMQL_PARAM_MATCH, PROMQL_PARAM_OPTIONS, PROMQL_PARAM_COUNT
} PROMQL_PARAMETER;

typedef struct {
    PROMQL_PARAMETER parameter;
    netdata_promql_bytes_t value;
} PROMQL_FORM_FIELD;

typedef struct {
    const char *message;
    uint8_t escape[3];
    size_t escape_length;
} PROMQL_FORM_ERROR;

typedef struct {
    PROMQL_FORM_FIELD *fields;
    size_t count, capacity;
    netdata_promql_allocation_handle_t *input_view;
    PROMQL_FORM_ERROR error;
} PROMQL_FORM;

PROMQL_FORM *promql_form_create(netdata_promql_bytes_t target, netdata_promql_bytes_t payload,
                                const char *content_type, bool post, PROMQL_READ_FRAME *frame, int *error);
void promql_form_destroy(PROMQL_FORM *form);
netdata_promql_bytes_t promql_form_value(const PROMQL_FORM *form, PROMQL_PARAMETER parameter);
size_t promql_form_count(const PROMQL_FORM *form, PROMQL_PARAMETER parameter);
int promql_hex_digit(uint8_t byte);

#endif
