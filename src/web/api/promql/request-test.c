// SPDX-License-Identifier: GPL-3.0-or-later

#include "request.h"

typedef struct {
    const char *name, *target, *payload, *content_type, *query, *message;
    size_t query_length, matchers;
    bool post;
} REQUEST_CASE;

typedef struct { const REQUEST_CASE *test; PROMQL_FORM *form; int error; } REQUEST_PREPARATION;

static void prepare_form(void *context, void *handle __maybe_unused,
                          const netdata_promql_accounting_context_t *account) {
    REQUEST_PREPARATION *preparation = context;
    const REQUEST_CASE *test = preparation->test;
    PROMQL_READ_FRAME frame = {.account = account};
    preparation->form = promql_form_create(promql_bytes(test->target), promql_bytes(test->payload),
        test->content_type, test->post, &frame, &preparation->error);
}

int promql_request_fixture(netdata_promql_engine_t *engine) {
    static const REQUEST_CASE tests[] = {
        {"body-first", "/api/v1/series?query=url&match[]=u", "query=body&match[]=a&match[]=b",
         "application/x-www-form-urlencoded", "body", NULL, 4, 3, true},
        {"escaped-separators", "/api/v1/query?query=a%26b%3Dc+%2B", NULL, NULL, "a&b=c +", NULL, 7, 0, false},
        {"embedded-nul", "/api/v1/query?query=a%00b", NULL, NULL, "a\0b", NULL, 3, 0, false},
        {"empty-first", "/api/v1/query?query=&query=next", NULL, NULL, "", NULL, 0, 0, false},
        {"invalid-field", "/api/v1/query?query=%QQ&query=ok", NULL, NULL, "ok", "invalid URL escape ", 2, 0, false},
        {"semicolon-order", "/api/v1/query?query=%QQ&unused=x;y&query=ok", NULL, NULL,
         "ok", "invalid semicolon separator in query", 2, 0, false},
        {"escaped-semicolon", "/api/v1/query?%71uery=a%3Bb", NULL, NULL, "a;b", NULL, 3, 0, false},
        {"non-form-body", "/api/v1/query?query=url", "query=body", "application/json", "url", NULL, 3, 0, true},
        {"mime-error-first", "/api/v1/series?match[]=m", "unused=x;y&query=body",
         "application/x-www-form-urlencoded; broken", "body", "mime: invalid media parameter", 4, 1, true},
        {"mime-duplicate-same", "/api/v1/query?query=url", "query=body",
         "application/x-www-form-urlencoded; X=ok; x=\"ok\"", "body", NULL, 4, 0, true},
        {"mime-duplicate-different", "/api/v1/query?query=url", "query=body",
         "application/x-www-form-urlencoded; X=a; x=b", "url", "mime: duplicate parameter name", 3, 0, true},
        {"body-error-before-url", "/api/v1/query?query=%QQ", "query=%XX&query=body",
         "application/x-www-form-urlencoded", "body", "invalid URL escape ", 4, 0, true},
        {"mime-unicode-i", "/api/v1/query?query=url", "query=body",
         "APPL\xc4\xb0" "CATION/X-WWW-FORM-URLENCODED", "body", NULL, 4, 0, true},
        {"mime-unicode-k", "/api/v1/series?match[]=m", "query=body",
         "\xe2\x84\xaa", "", NULL, 0, 1, true},
    };
    int errors = 0;
    for(size_t i = 0; i < sizeof(tests) / sizeof(tests[0]); i++) {
        const REQUEST_CASE *test = &tests[i];
        netdata_promql_report_t *report = NULL;
        int64_t limit = -1;
        uint32_t status = netdata_promql_api_parse_limit(engine, (netdata_promql_bytes_t){0}, NULL, &limit, &report);
        REQUEST_PREPARATION preparation = {.test = test};
        int failed = status != NETDATA_PROMQL_OK || limit != 0 || !report;
        if(!failed) failed += netdata_promql_report_accounted(report, prepare_form, &preparation, NULL) != NETDATA_PROMQL_OK;
        PROMQL_FORM *form = preparation.form;
        failed += !form || preparation.error;
        if(form) {
            netdata_promql_bytes_t query = promql_form_value(form, PROMQL_PARAM_QUERY);
            failed += query.len != test->query_length ||
                (query.len == test->query_length && query.len && memcmp(query.data, test->query, (size_t)query.len));
            failed += promql_form_count(form, PROMQL_PARAM_MATCH) != test->matchers;
            failed += test->message ? !form->error.message || strcmp(form->error.message, test->message) : form->error.message != NULL;
            if(failed) fprintf(stderr, "request fixture %s: error %s\n", test->name, form->error.message ? form->error.message : "none");
        }
        promql_form_destroy(form);
        if(report) {
            netdata_promql_query_observations_t observed;
            failed += netdata_promql_report_live_observations(report, &observed) != NETDATA_PROMQL_OK;
            failed += observed.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current != 0 ||
                observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current != 0;
            netdata_promql_report_release(report);
        }
        printf("{\"request_case\":\"%s\",\"errors\":%d}\n", test->name, failed);
        errors += failed;
    }
    return errors;
}
