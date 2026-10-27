// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_KEYS_H
#define NETDATA_PROMQL_KEYS_H

#include "body.h"

typedef struct {
    netdata_promql_labels_t labels;
    netdata_promql_bytes_t field;
    bool single;
} PROMQL_KEY_VIEW;

bool promql_key_measure(PROMQL_KEY_VIEW view, uint64_t *length, uint64_t *hash,
                        PROMQL_READ_FRAME *frame, int *error);
bool promql_key_write(PROMQL_BODY *body, PROMQL_KEY_VIEW view, PROMQL_READ_FRAME *frame, int *error);
bool promql_key_equal(PROMQL_BODY *body, uint64_t offset, uint64_t length, PROMQL_KEY_VIEW view,
                      uint8_t window[PROMQL_BODY_WINDOW], PROMQL_READ_FRAME *frame, int *error, bool *equal);

#endif
