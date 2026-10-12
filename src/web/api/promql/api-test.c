// SPDX-License-Identifier: GPL-3.0-or-later

#include "api-internal.h"
#include "web/server/web_client_cache.h"

typedef struct {
    const char *name, *target, *payload, *content_type, *expected;
    PROMQL_API_KIND kind;
    int code;
    bool post, fragment, fleet;
    usec_t caller_timeout;
    bool caller_expired, cancel_on_register;
} API_CASE;

typedef struct { bool cancel; unsigned begins, ends; } OPERATION_HOOK;

static void operation_hook(void *data, uint64_t id) {
    OPERATION_HOOK *hook = data;
    if(id) { hook->begins++; if(hook->cancel) promql_operations_cancel_id(id); }
    else hook->ends++;
}

static int route_request(struct web_client *client, const char *target) {
    if(!web_client_decode_path_and_query_string(client, (char *)target)) return HTTP_RESP_BAD_REQUEST;
    char *path = strdupz(buffer_tostring(client->url_path_decoded));
    int code = web_client_api_request_with_node_selection(localhost, client, path);
    freez(path);
    return code;
}

static int handler_case(const API_CASE *test, bool routed) {
    size_t native_memory = 0;
    struct web_client *client = web_client_create(&native_memory);
    buffer_strcat(client->url_as_received, test->target);
    client->mode = test->post ? HTTP_REQUEST_MODE_POST : HTTP_REQUEST_MODE_GET;
    if(test->payload) {
        client->payload = buffer_create(strlen(test->payload) + 1, &native_memory);
        buffer_strcat(client->payload, test->payload);
    }
    if(test->content_type) client->request_content_type = strdupz(test->content_type);
    OPERATION_HOOK hook = {.cancel = test->cancel_on_register};
    client->promql_operation_hook = operation_hook;
    client->promql_operation_hook_data = &hook;
    client->timings.timeout_ut = test->caller_timeout;
    now_monotonic_high_precision_timeval(&client->timings.tv_in);
    if(test->caller_expired) client->timings.tv_in.tv_sec--;
    client->acl = client->port_acl = HTTP_ACL_METRICS;
    client->promql_scoped = !routed && !test->fleet;
    if(client->promql_scoped) memcpy(client->promql_host_guid, localhost->machine_guid, sizeof(client->promql_host_guid));
    const int (*handlers[])(RRDHOST *, struct web_client *, char *) = {
        api_v1_promql_query, api_v1_promql_query_range, api_v1_promql_labels, api_v1_promql_label_values, api_v1_promql_series};
    int code = routed ? route_request(client, test->target) : handlers[test->kind](localhost, client, NULL);
    char bytes[PROMQL_BODY_WINDOW + 1];
    PROMQL_BODY *body = client->response.promql_body;
    uint64_t length = body ? promql_body_length(body) : buffer_strlen(client->response.data);
    int errors = code != test->code || client->response.code != test->code || !body || !promql_body_is_sealed(body) ||
        promql_body_is_gzip(body) || length >= sizeof(bytes) || client->promql_operation_id || promql_operations_active();
    errors += hook.begins != 1 || hook.ends != 1;
    int read_error = 0;
    if(length < sizeof(bytes)) {
        if(body) errors += !promql_body_read(body, 0, bytes, (size_t)length, NULL, &read_error);
        else memcpy(bytes, buffer_tostring(client->response.data), (size_t)length);
        bytes[length] = 0;
        errors += test->fragment ? !strstr(bytes, test->expected) : strcmp(bytes, test->expected) != 0;
        if(code >= 400) errors += strstr(bytes, "\"warnings\":") != NULL || strstr(bytes, "\"infos\":") != NULL;
        if(errors) fprintf(stderr, "API handler fixture %s: code%d expected%d body %s\n", test->name, code, test->code, bytes);
    }
    web_client_free(client);
    errors += native_memory != 0;
    netdata_promql_engine_observations_t observed = {0};
    errors += promql_api_observations(&observed) != NETDATA_PROMQL_OK || observed.active_executions;
    for(size_t i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++)
        if(i != NETDATA_PROMQL_MEMORY_ACCOUNTING) errors += observed.memory[i].current != 0;
    printf("{\"%s\":\"%s\",\"errors\":%d,\"code\":%d,\"bytes\":%"PRIu64"}\n",
           routed ? "api_route_case" : "api_handler_case", test->name, errors, code, length);
    return errors;
}

static int cache_release_case(void) {
    struct web_client *client = web_client_get_from_cache();
    buffer_strcat(client->url_as_received, "/api/v1/query?query=1&time=1700002804");
    int errors = api_v1_promql_query(localhost, client, NULL) != 200 || !client->response.promql_body;
    web_client_release_to_cache(client);
    netdata_promql_engine_observations_t observed = {0};
    errors += promql_api_observations(&observed) != NETDATA_PROMQL_OK;
    uint64_t remaining = 0;
    for(size_t i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++)
        if(i != NETDATA_PROMQL_MEMORY_ACCOUNTING) remaining += observed.memory[i].current;
    errors += remaining != 0;
    printf("{\"api_handler_case\":\"pooled-release\",\"errors\":%d,\"remaining_query_memory\":%"PRIu64"}\n", errors, remaining);
    return errors;
}

int promql_api_fixture(void) {
    static const API_CASE tests[] = {
        {"scalar", "/api/v1/query?query=1%2B2&time=1700002804", NULL, NULL,
         "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002804,\"3\"]}}", PROMQL_API_QUERY, 200},
        {"live-gauge", "/api/v1/query?query=scalar(fixture_provider{netdata_dimension_id=\"gauge\"})&time=1700002804", NULL, NULL,
         "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002804,\"5\"]}}", PROMQL_API_QUERY, 200},
        {"counter-absolute", "/api/v1/query?query=scalar(fixture_provider{netdata_dimension_id=\"legacy\"})&time=1700002804&options=absolute", NULL, NULL,
         "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002804,\"8\"]}}", PROMQL_API_QUERY, 200},
        {"counter-legacy", "/api/v1/query?query=scalar(fixture_provider{netdata_dimension_id=\"legacy\"})&time=1700002804", NULL, NULL,
         "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002804,\"-8\"]}}", PROMQL_API_QUERY, 200},
        {"fleet-aliases", "/api/v1/query?query=scalar(sum({__name__=~\"fixture[._]provider\",netdata_dimension_id=\"gauge\"}))&time=1700002804", NULL, NULL,
         "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002804,\"10\"]}}", PROMQL_API_QUERY, 200, false, false, true},
        {"form-body-priority", "/api/v1/query?query=2&time=1700002804", "query=1", "application/x-www-form-urlencoded",
         "{\"status\":\"success\",\"data\":{\"resultType\":\"scalar\",\"result\":[1700002804,\"1\"]}}", PROMQL_API_QUERY, 200, true},
        {"range", "/api/v1/query_range?query=vector(1)&start=1700002801&end=1700002803&step=1", NULL, NULL,
         "{\"status\":\"success\",\"data\":{\"resultType\":\"matrix\",\"result\":[{\"metric\":{},\"values\":[[1700002801,\"1\"],[1700002802,\"1\"],[1700002803,\"1\"]]}]}}", PROMQL_API_RANGE, 200},
        {"range-end-order", "/api/v1/query_range?query=vector(1)&start=2&end=1&step=bad", NULL, NULL,
         "{\"status\":\"error\",\"errorType\":\"bad_data\",\"error\":\"invalid parameter \\\"end\\\": end timestamp must not be before start time\"}", PROMQL_API_RANGE, 400},
        {"range-zero-step", "/api/v1/query_range?query=vector(1)&start=1&end=2&step=0", NULL, NULL,
         "invalid parameter \\\"step\\\": zero or negative query resolution step widths", PROMQL_API_RANGE, 400, false, true},
        {"range-type", "/api/v1/query_range?query=m[1m]&start=1&end=2&step=1", NULL, NULL,
         "invalid parameter \\\"query\\\": invalid expression type \\\"range vector\\\" for range query, must be Scalar or instant Vector", PROMQL_API_RANGE, 400, false, true},
        {"range-submillisecond", "/api/v1/query_range?query=vector(1)&start=1&end=2&step=0.000001", NULL, NULL,
         "\"errorType\":\"execution\"", PROMQL_API_RANGE, 422, false, true},
        {"empty-query", "/api/v1/query?time=1700002804", NULL, NULL,
         "invalid parameter \\\"query\\\": unknown position: parse error: no expression found in input", PROMQL_API_QUERY, 400, false, true},
        {"time-wrapper", "/api/v1/query?query=1&time=bad", NULL, NULL,
         "invalid parameter \\\"time\\\": invalid time value for 'time': cannot parse \\\"bad\\\" to a valid timestamp", PROMQL_API_QUERY, 400, false, true},
        {"limit-before-time", "/api/v1/query?query=1&limit=-1&time=bad", NULL, NULL,
         "{\"status\":\"error\",\"errorType\":\"bad_data\",\"error\":\"invalid parameter \\\"limit\\\": limit must be non-negative\"}", PROMQL_API_QUERY, 400},
        {"timeout-before-lookback", "/api/v1/query?query=1&timeout=bad&lookback_delta=bad", NULL, NULL,
         "invalid parameter \\\"timeout\\\": cannot parse", PROMQL_API_QUERY, 400, false, true},
        {"lookback-unwrapped", "/api/v1/query?query=1&lookback_delta=bad", NULL, NULL,
         "\"error\":\"error parsing lookback delta duration: cannot parse", PROMQL_API_QUERY, 400, false, true},
        {"expired-timeout", "/api/v1/query?query=1&time=1700002804&timeout=0", NULL, NULL,
         "{\"status\":\"error\",\"errorType\":\"timeout\",\"error\":\"query deadline exceeded\"}", PROMQL_API_QUERY, 503},
        {"expired-parse-first", "/api/v1/query?time=1700002804&timeout=0", NULL, NULL,
         "\"errorType\":\"bad_data\"", PROMQL_API_QUERY, 400, false, true},
        {"stats-warning", "/api/v1/query?query=1&time=1700002804&stats=false", NULL, NULL,
         "\"warnings\":[\"value \\\"false\\\" for parameter \\\"stats\\\" is deprecated and will be rejected in the next major release, use \\\"true\\\" or \\\"all\\\"\"]", PROMQL_API_QUERY, 200, false, true},
        {"reject-shaping", "/api/v1/query?query=1&time=1700002804&options=percentage", NULL, NULL,
         "invalid parameter \\\"options\\\": unsupported PromQL option", PROMQL_API_QUERY, 400, false, true},
        {"label-values", "/api/v1/label/netdata_units/values?match[]=fixture_provider", NULL, NULL,
         "{\"status\":\"success\",\"data\":[\"KiB\"]}", PROMQL_API_VALUES, 200},
        {"label-unescaped", "/api/v1/label/U__netdata_5f_units/values?match[]=fixture_provider", NULL, NULL,
         "{\"status\":\"success\",\"data\":[\"KiB\"]}", PROMQL_API_VALUES, 200},
        {"label-name-before-limit", "/api/v1/label//values?limit=-1", NULL, NULL,
         "{\"status\":\"error\",\"errorType\":\"bad_data\",\"error\":\"invalid label name: \\\"\\\"\"}", PROMQL_API_VALUES, 400},
        {"label-nul-rune", "/api/v1/label/U___0_/values?match[]=fixture_provider", NULL, NULL,
         "{\"status\":\"success\",\"data\":[]}", PROMQL_API_VALUES, 200},
        {"label-get-only", "/api/v1/label/netdata_units/values?match[]=fixture_provider", "match[]=fixture_provider",
         "application/x-www-form-urlencoded", "method not allowed", PROMQL_API_VALUES, 405, true, true},
        {"labels-limit", "/api/v1/labels?match[]=fixture_provider&limit=1", NULL, NULL,
         "{\"status\":\"success\",\"data\":[\"__name__\"],\"warnings\":[\"results truncated due to limit\"]}", PROMQL_API_LABELS, 200},
        {"values-union", "/api/v1/label/netdata_context/values?match[]=fixture_provider&match[]=fixture_provider", NULL, NULL,
         "{\"status\":\"success\",\"data\":[\"fixture.provider\"]}", PROMQL_API_VALUES, 200},
        {"values-sorted", "/api/v1/label/netdata_dimension_id/values?match[]=fixture_provider", NULL, NULL,
         "{\"status\":\"success\",\"data\":[\"gauge\",\"legacy\",\"mixed\",\"negative\",\"positive\",\"reset_gap\",\"seam\",\"seam_native\",\"seam_negative\"]}", PROMQL_API_VALUES, 200},
        {"series-requires-match", "/api/v1/series?limit=-1", NULL, NULL,
         "{\"status\":\"error\",\"errorType\":\"bad_data\",\"error\":\"no match[] parameter provided\"}", PROMQL_API_SERIES, 400},
        {"series-limit-before-match", "/api/v1/series?match[]=fixture_provider&limit=-1", NULL, NULL,
         "{\"status\":\"error\",\"errorType\":\"bad_data\",\"error\":\"invalid parameter \\\"limit\\\": limit must be non-negative\"}", PROMQL_API_SERIES, 400},
        {"series-form-error-first", "/api/v1/series?match[]=fixture_provider&limit=-1&unused=%QQ", NULL, NULL,
         "error parsing form values: invalid URL escape \\\"%QQ\\\"", PROMQL_API_SERIES, 400, false, true},
        {"series-empty-guard", "/api/v1/series?match[]={x=\"\"}", NULL, NULL,
         "invalid parameter \\\"match[]\\\": match[] must contain at least one non-empty matcher", PROMQL_API_SERIES, 400, false, true},
        {"labels-empty-guard", "/api/v1/labels?match[]={x=\"\"}", NULL, NULL,
         "\"error\":\"match[] must contain at least one non-empty matcher\"", PROMQL_API_LABELS, 400, false, true},
        {"selector-syntax-before-guard", "/api/v1/labels?match[]={x=\"\"}&match[]=1", NULL, NULL,
         "parse error:", PROMQL_API_LABELS, 400, false, true},
        {"discovery-reversed", "/api/v1/labels?match[]=fixture_provider&start=2&end=1", NULL, NULL,
         "{\"status\":\"success\",\"data\":[]}", PROMQL_API_LABELS, 200},
        {"execution-error-drops-infos", "/api/v1/query?query=rate(fixture_provider[3s])%2Bon(netdata_units)rate(fixture_provider[3s])&time=1700002804", NULL, NULL,
         "found duplicate series for the match group", PROMQL_API_QUERY, 422, false, true},
        {"caller-expired", "/api/v1/query?query=1&time=1700002804", NULL, NULL,
         "\"errorType\":\"timeout\"", PROMQL_API_QUERY, 503, false, true, false, 1, true},
        {"caller-expired-before-parse", "/api/v1/query?time=1700002804", NULL, NULL,
         "\"errorType\":\"timeout\"", PROMQL_API_QUERY, 503, false, true, false, 1, true},
        {"caller-keeps-uri-deadline", "/api/v1/query?query=1&time=1700002804&timeout=0", NULL, NULL,
         "\"errorType\":\"timeout\"", PROMQL_API_QUERY, 503, false, true, false, USEC_PER_SEC * 60},
        {"cancel-on-registration", "/api/v1/query?query=1&time=1700002804", NULL, NULL,
         "\"errorType\":\"canceled\"", PROMQL_API_QUERY, 499, false, true, false, 0, false, true},
    };
    int errors = 0;
    for(size_t i = 0; i < sizeof(tests) / sizeof(tests[0]); i++) {
        errors += handler_case(&tests[i], false);
        errors += handler_case(&tests[i], true);
    }
    errors += cache_release_case();
    return errors;
}
