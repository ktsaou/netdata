// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_RTC_BODY_H
#define NETDATA_PROMQL_RTC_BODY_H

#include "body.h"

#define PROMQL_RTC_HEADER_SIZE 200U

typedef struct {
    void *context;
    bool (*ready)(void *context);
    bool (*send)(void *context, const char *text, size_t length);
} PROMQL_RTC_WRITER;

bool promql_rtc_body_send(PROMQL_BODY *body, int code, const char *content_type, size_t max_message_size,
                           const PROMQL_RTC_WRITER *writer, uint64_t *sent, int *error);

#endif
