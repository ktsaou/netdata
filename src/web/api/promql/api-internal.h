// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_API_INTERNAL_H
#define NETDATA_PROMQL_API_INTERNAL_H

#include "api.h"
#include "request.h"
#include "operations.h"
#include "provider.h"
#include "output.h"

typedef enum {
    PROMQL_API_QUERY, PROMQL_API_RANGE, PROMQL_API_LABELS, PROMQL_API_VALUES, PROMQL_API_SERIES, PROMQL_API_METADATA
} PROMQL_API_KIND;

typedef struct {
    int code;
    const char *type, *parameter, *time_parameter, *prefix, *suffix;
    netdata_promql_bytes_t message, source;
    uint64_t position;
    bool position_present, quote;
} PROMQL_API_ERROR;

typedef struct {
    struct web_client *client;
    netdata_promql_engine_t *engine;
    PROMQL_OPERATION operation;
    PROMQL_FORM *form;
    netdata_promql_query_t *query;
    netdata_promql_report_t *report;
    netdata_promql_selector_t **selectors;
    size_t selector_count;
    PROMQL_OUTPUT *output;
    bool execution_started;
    PROMQL_BODY *body;
    netdata_promql_bytes_t label_name;
    PROMQL_PROVIDER_CONTEXT context;
    PROMQL_API_ERROR failure;
    netdata_promql_precise_request_t request;
    uint64_t limit;
    int error;
    bool gzip, engine_acquired, label_name_owned;
    const char *directory;
} PROMQL_API_REQUEST;

bool promql_api_time(PROMQL_API_REQUEST *request, PROMQL_PARAMETER parameter, const char *name,
                     bool optional, netdata_promql_timestamp_t fallback, netdata_promql_timestamp_t *out);
void promql_api_report_failure(PROMQL_API_REQUEST *request, uint32_t status, const char *parameter,
                                netdata_promql_bytes_t source);
void promql_api_bad_data(PROMQL_API_REQUEST *request, const char *parameter, const char *message);
int64_t promql_api_milliseconds(netdata_promql_timestamp_t value);
bool promql_api_before(netdata_promql_timestamp_t a, netdata_promql_timestamp_t b);
bool promql_api_limit(PROMQL_API_REQUEST *request);
void promql_api_replace_report(PROMQL_API_REQUEST *request, netdata_promql_report_t *report);
uint32_t promql_api_observations(netdata_promql_engine_observations_t *out);
bool promql_api_discovery(PROMQL_API_REQUEST *request, PROMQL_API_KIND kind);
bool promql_api_metadata(PROMQL_API_REQUEST *request);

#endif
