// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_ACLK_SOURCE_H
#define NETDATA_PROMQL_ACLK_SOURCE_H

#include "body.h"
#include "aclk/mqtt_websockets/common_public.h"

typedef struct {
    const char *message_id, *headers;
    size_t headers_length;
    usec_t timestamp, session, execution_time, received_time;
    int version, code;
} PROMQL_ACLK_ENVELOPE;

/* Borrows inputs during construction; the returned source owns a body reference and its copied prefix. */
struct mqtt_payload_source *promql_aclk_source(PROMQL_BODY *body, const PROMQL_ACLK_ENVELOPE *envelope, int *error);

#endif
