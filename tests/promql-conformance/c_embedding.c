/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Real C/C++ caller: generated source samples retain one page, never a complete history. */
#include "netdata-promql.h"
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    uint64_t rows, maximum, opened[4], closed[4], pages_returned, samples_returned;
    netdata_promql_label_t labels[2];
    unsigned failure, cancel_at;
    netdata_promql_control_t *control;
    uint8_t error[32];
} fixture;

typedef struct {
    fixture *owner;
    netdata_promql_bounds_t bounds;
    uint64_t index, capacity;
    netdata_promql_sample_t *page;
} cursor;

static netdata_promql_bytes_t bytes(const void *data, uint64_t len) {
    netdata_promql_bytes_t value;
    value.data = (const uint8_t *)data;
    value.len = len;
    return value;
}
static netdata_promql_resources_t no_resources(void *context, void *handle) {
    fixture *f = (fixture *)context;
    (void)handle;
    memset(f->error, 'X', sizeof(f->error));
    netdata_promql_resources_t result;
    memset(&result, 0, sizeof(result));
    return result;
}
static uint32_t checked_control(const netdata_promql_callback_control_t *control) {
    netdata_promql_control_state_t state;
    uint32_t status = netdata_promql_callback_control_state(control, &state);
    return status == NETDATA_PROMQL_OK ? state.status : status;
}
static uint32_t returned(fixture *f, unsigned stage, netdata_promql_bytes_t *error) {
    static const uint8_t message[] = {'p','r','o','v','i','d','e','r',0,255,'f','a','i','l','u','r','e'};
    if (f->cancel_at == stage) {
        uint32_t status = netdata_promql_control_cancel(f->control);
        if (status != NETDATA_PROMQL_OK) return status;
    }
    if (f->failure != stage) return NETDATA_PROMQL_OK;
    memcpy(f->error, message, sizeof(message));
    *error = bytes(f->error, sizeof(message));
    return NETDATA_PROMQL_PROVIDER_ERROR;
}
static uint32_t query_open(void *context, netdata_promql_bounds_t bounds,
                           const netdata_promql_callback_control_t *control, void **out,
                           netdata_promql_bytes_t *error) {
    fixture *f = (fixture *)context;
    (void)bounds;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    f->opened[0]++;
    status = returned(f, 1, error);
    if (status != NETDATA_PROMQL_OK) return status;
    *out = NULL;
    return NETDATA_PROMQL_OK;
}
static void query_close(void *context, void *handle) {
    fixture *f = (fixture *)context;
    (void)handle;
    f->closed[0]++;
    memset(f->error, 'X', sizeof(f->error));
}
static uint32_t selection_open(void *context, void *query,
                               const netdata_promql_selection_request_t *request,
                               const netdata_promql_callback_control_t *control, void **out,
                               netdata_promql_bytes_t *error) {
    fixture *f = (fixture *)context;
    (void)query;
    (void)request;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    f->opened[1]++;
    status = returned(f, 2, error);
    if (status != NETDATA_PROMQL_OK) return status;
    *out = calloc(1, 1);
    return *out ? NETDATA_PROMQL_OK : NETDATA_PROMQL_PROVIDER_ERROR;
}
static void selection_close(void *context, void *handle) {
    fixture *f = (fixture *)context;
    f->closed[1]++;
    free(handle);
    memset(f->error, 'X', sizeof(f->error));
}
static netdata_promql_resources_t selection_resources(void *context, void *handle) {
    netdata_promql_resources_t result = no_resources(context, handle);
    result.held_bytes = 1;
    return result;
}
static uint32_t selection_next(void *context, void *handle,
                               const netdata_promql_callback_control_t *control,
                               netdata_promql_series_open_t *out,
                               netdata_promql_provider_diagnostics_t *diagnostics,
                               netdata_promql_bytes_t *error) {
    fixture *f = (fixture *)context;
    (void)diagnostics;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    status = returned(f, 3, error);
    if (status != NETDATA_PROMQL_OK) return status;
    uint8_t *sent = (uint8_t *)handle;
    if (*sent) return NETDATA_PROMQL_END;
    *sent = 1;
    f->opened[2]++;
    out->handle = NULL;
    out->labels.data = f->labels;
    out->labels.len = 2;
    return NETDATA_PROMQL_OK;
}
static void series_close(void *context, void *handle) {
    fixture *f = (fixture *)context;
    (void)handle;
    f->closed[2]++;
    memset(f->error, 'X', sizeof(f->error));
}
static uint32_t cursor_open(void *context, void *series, netdata_promql_bounds_t bounds,
                            const netdata_promql_callback_control_t *control,
                            netdata_promql_cursor_open_t *out, netdata_promql_bytes_t *error) {
    fixture *f = (fixture *)context;
    (void)series;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    f->opened[3]++;
    status = returned(f, 4, error);
    if (status != NETDATA_PROMQL_OK) return status;
    cursor *c = (cursor *)calloc(1, sizeof(*c));
    if (!c) return NETDATA_PROMQL_PROVIDER_ERROR;
    c->page = (netdata_promql_sample_t *)calloc((size_t)f->maximum, sizeof(*c->page));
    if (!c->page) { free(c); return NETDATA_PROMQL_PROVIDER_ERROR; }
    c->capacity = f->maximum;
    c->owner = f;
    c->bounds = bounds;
    out->handle = c;
    out->max_page_samples = c->capacity;
    return NETDATA_PROMQL_OK;
}
static void cursor_close(void *context, void *handle) {
    fixture *f = (fixture *)context;
    cursor *c = (cursor *)handle;
    f->closed[3]++;
    free(c->page);
    free(c);
    memset(f->error, 'X', sizeof(f->error));
}
static netdata_promql_resources_t cursor_resources(void *context, void *handle) {
    cursor *c = (cursor *)handle;
    netdata_promql_resources_t result = no_resources(context, handle);
    result.held_bytes = sizeof(*c) + c->capacity * sizeof(*c->page);
    result.shared_cache_bytes = 7;
    return result;
}
static uint32_t cursor_seek(void *context, void *handle, int64_t time_ms,
                            const netdata_promql_callback_control_t *control,
                            netdata_promql_bytes_t *error) {
    fixture *f = (fixture *)context;
    cursor *c = (cursor *)handle;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    status = returned(f, 5, error);
    if (status != NETDATA_PROMQL_OK) return status;
    c->index = time_ms <= 1000 ? 0 : (uint64_t)((time_ms - 1) / 1000);
    return NETDATA_PROMQL_OK;
}
static uint32_t cursor_next(void *context, void *handle,
                            const netdata_promql_callback_control_t *control,
                            netdata_promql_page_t *out, netdata_promql_resources_t *resources,
                            netdata_promql_bytes_t *error) {
    fixture *f = (fixture *)context;
    cursor *c = (cursor *)handle;
    *resources = cursor_resources(context, handle);
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    status = returned(f, 6, error);
    if (status != NETDATA_PROMQL_OK) return status;
    uint64_t count = 0;
    while (count < c->capacity && c->index < f->rows) {
        int64_t time_ms = (int64_t)(c->index + 1) * 1000;
        if (time_ms > c->bounds.end_ms) break;
        netdata_promql_sample_t *sample = &c->page[count++];
        memset(sample, 0, sizeof(*sample));
        sample->time_ms = time_ms;
        sample->value.kind = NETDATA_PROMQL_VALUE_FLOAT;
        sample->value.payload.float_bits = netdata_promql_double_bits((double)(++c->index));
    }
    if (!count) return NETDATA_PROMQL_END;
    out->samples.data = c->page;
    out->samples.len = count;
    out->resources = *resources;
    f->pages_returned++;
    f->samples_returned += count;
    return NETDATA_PROMQL_OK;
}
static void init_fixture(fixture *f, uint64_t rows, uint64_t maximum) {
    static const uint8_t name[] = "__name__", metric[] = "m", zone[] = "zone";
    static const uint8_t value[] = {'v','a','l','u','e',0,255};
    memset(f, 0, sizeof(*f));
    f->rows = rows;
    f->maximum = maximum;
    f->labels[0].name = bytes(name, sizeof(name) - 1);
    f->labels[0].value = bytes(metric, sizeof(metric) - 1);
    f->labels[1].name = bytes(zone, sizeof(zone) - 1);
    f->labels[1].value = bytes(value, sizeof(value));
}
static netdata_promql_provider_t provider(fixture *f) {
    netdata_promql_provider_t p;
    memset(&p, 0, sizeof(p));
    p.header.abi_version = NETDATA_PROMQL_ABI_VERSION;
    p.header.struct_size = sizeof(p);
    p.context = f;
    p.query_open = query_open; p.query_close = query_close; p.query_resources = no_resources;
    p.selection_open = selection_open; p.selection_close = selection_close;
    p.selection_resources = selection_resources; p.selection_next = selection_next;
    p.series_close = series_close; p.series_resources = no_resources;
    p.cursor_open = cursor_open; p.cursor_close = cursor_close; p.cursor_resources = cursor_resources;
    p.cursor_seek = cursor_seek; p.cursor_next = cursor_next;
    return p;
}
static void released(const fixture *f) {
    for (unsigned i = 0; i < 4; i++) {
        uint64_t failed = (f->failure == 1 && i == 0) || (f->failure == 2 && i == 1) || (f->failure == 4 && i == 3);
        assert(f->opened[i] == f->closed[i] + failed);
    }
}
static netdata_promql_query_t *compile(netdata_promql_engine_t *engine, const char *text) {
    netdata_promql_query_t *query = NULL;
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_compile(engine, bytes(text, strlen(text)), NULL, &query, &report) == NETDATA_PROMQL_OK);
    assert(query && report);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    return query;
}
static netdata_promql_request_t instant(int64_t time_ms) {
    netdata_promql_request_t value;
    memset(&value, 0, sizeof(value));
    value.header.abi_version = NETDATA_PROMQL_ABI_VERSION;
    value.header.struct_size = sizeof(value);
    value.kind = NETDATA_PROMQL_REQUEST_INSTANT;
    value.start_ms = time_ms;
    return value;
}
static void balanced(netdata_promql_engine_t *engine, const netdata_promql_engine_observations_t *idle) {
    netdata_promql_engine_observations_t observed;
    assert(netdata_promql_engine_observations(engine, &observed) == NETDATA_PROMQL_OK);
    assert(observed.active_executions == 0);
    for (unsigned i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++) {
        assert(observed.memory[i].current == idle->memory[i].current);
        assert(!observed.memory[i].overflowed);
        assert(observed.memory[i].acquired - observed.memory[i].released == observed.memory[i].current);
    }
    assert(observed.cancellation_references.engine_owned.current == 0);
    assert(observed.cancellation_references.external.current == 0);
}
static uint64_t long_window(netdata_promql_engine_t *engine, uint64_t rows, uint64_t maximum) {
    char text[80];
    assert(snprintf(text, sizeof(text), "sum_over_time(m[%" PRIu64 "s])", rows) > 0);
    netdata_promql_query_t *query = compile(engine, text);
    fixture f;
    init_fixture(&f, rows, maximum);
    netdata_promql_provider_t p = provider(&f);
    netdata_promql_request_t request = instant((int64_t)rows * 1000);
    netdata_promql_result_t *result = NULL;
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_collect(query, &p, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
    released(&f);
    netdata_promql_query_observations_t observed;
    assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
    /* Physical reads include replay; every successfully returned page must be charged once. */
    assert(f.samples_returned >= rows);
    assert(observed.samples_fetched == f.samples_returned);
    assert(observed.pages == f.pages_returned);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].peak == maximum * sizeof(netdata_promql_sample_t));
    assert(observed.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current == 0);
    netdata_promql_point_info_t point;
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_result_point(result, 0, 0, &point) == NETDATA_PROMQL_OK);
    assert(point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
    assert(point.value_bits == netdata_promql_double_bits((double)(rows * (rows + 1) / 2)));
    assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
    printf("{\"rows\":%" PRIu64 ",\"page_size\":%" PRIu64 ",\"workspace_peak\":%" PRIu64 ",\"borrowed_peak\":%" PRIu64
           ",\"samples_fetched\":%" PRIu64 ",\"pages\":%" PRIu64 "}\n",
           rows, maximum, observed.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak,
           observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].peak, observed.samples_fetched, observed.pages);
    return observed.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak;
}
static void error_phases(netdata_promql_engine_t *engine) {
    for (unsigned stage = 1; stage <= 6; stage++) {
        netdata_promql_query_t *query = compile(engine, "m");
        fixture f;
        init_fixture(&f, 5, 1);
        f.failure = f.cancel_at = stage;
        assert(netdata_promql_control_create(engine, NULL, &f.control) == NETDATA_PROMQL_OK);
        netdata_promql_provider_t p = provider(&f);
        netdata_promql_request_t request = instant(3000);
        netdata_promql_result_t *result = NULL;
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_collect(query, &p, &request, f.control, &result, &report);
        assert(status != NETDATA_PROMQL_OK && result == NULL && report != NULL);
        assert((status == NETDATA_PROMQL_CANCELLED) == (stage == 2));
        released(&f);
        netdata_promql_error_t error;
        assert(netdata_promql_report_error(report, 0, &error) == NETDATA_PROMQL_OK);
        if (stage != 2) {
            static const uint8_t expected[] = {'p','r','o','v','i','d','e','r',0,255,'f','a','i','l','u','r','e'};
            assert(error.message.len >= sizeof(expected));
            assert(memcmp(error.message.data + error.message.len - sizeof(expected), expected, sizeof(expected)) == 0);
        }
        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
        assert(netdata_promql_control_release(f.control) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    }
}
static void range_result(netdata_promql_engine_t *engine) {
    netdata_promql_query_t *query = compile(engine, "m");
    fixture f;
    init_fixture(&f, 5, 7);
    netdata_promql_provider_t p = provider(&f);
    netdata_promql_request_t request = instant(2000);
    request.kind = NETDATA_PROMQL_REQUEST_RANGE;
    request.end_ms = 4000;
    request.step_ms = 1000;
    netdata_promql_result_t *result = NULL;
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_collect(query, &p, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
    released(&f);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    netdata_promql_result_info_t info;
    assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
    assert(info.header.value_type == NETDATA_PROMQL_TYPE_MATRIX && info.series_count == 1);
    netdata_promql_series_info_t series;
    assert(netdata_promql_result_series_info(result, 0, &series) == NETDATA_PROMQL_OK);
    assert(series.label_count == 2 && series.point_count == 3);
    for (uint64_t i = 0; i < 3; i++) {
        netdata_promql_point_info_t point;
        assert(netdata_promql_result_point(result, 0, i, &point) == NETDATA_PROMQL_OK);
        assert(point.time_ms == (int64_t)(i + 2) * 1000);
        assert(point.value_bits == netdata_promql_double_bits((double)(i + 2)));
    }
    assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
}
typedef struct {
    uint32_t failure, cancel_at, calls[4];
    netdata_promql_control_t *control;
    uint8_t error[5];
} delivery;
static uint32_t output_status(delivery *d, uint32_t phase,
                              const netdata_promql_callback_control_t *control,
                              netdata_promql_bytes_t *error) {
    assert(checked_control(control) == NETDATA_PROMQL_OK);
    d->calls[phase - 1]++;
    if (d->cancel_at == phase) assert(netdata_promql_control_cancel(d->control) == NETDATA_PROMQL_OK);
    if (d->failure != phase) return NETDATA_PROMQL_OK;
    const uint8_t message[] = {'o','u','t',0,255};
    memcpy(d->error, message, sizeof(message));
    *error = bytes(d->error, sizeof(message));
    return NETDATA_PROMQL_OUTPUT_ERROR;
}
static uint32_t output_begin(void *context, const netdata_promql_output_header_t *header,
                             const netdata_promql_callback_control_t *control,
                             netdata_promql_bytes_t *error) {
    assert(header->value_type == NETDATA_PROMQL_TYPE_VECTOR && header->reserved == 0);
    assert(header->start_ms == 3000 && header->end_ms == 3000 && header->step_ms == 0);
    return output_status((delivery *)context, 1, control, error);
}
static uint32_t output_chunk(void *context, const netdata_promql_chunk_t *chunk,
                             const netdata_promql_callback_control_t *control,
                             netdata_promql_bytes_t *error) {
    static const uint8_t label[] = {'v','a','l','u','e',0,255};
    assert(chunk->kind == NETDATA_PROMQL_CHUNK_SERIES && chunk->reserved == 0);
    const netdata_promql_series_chunk_t *series = &chunk->payload.series;
    assert(series->series_id == 0 && series->labels.len == 2 && series->points.len == 1);
    assert(series->labels.data[1].value.len == sizeof(label));
    assert(memcmp(series->labels.data[1].value.data, label, sizeof(label)) == 0);
    assert(series->points.data[0].time_ms == 3000);
    assert(series->points.data[0].value.kind == NETDATA_PROMQL_VALUE_FLOAT);
    assert(series->points.data[0].value.payload.float_bits == netdata_promql_double_bits(3));
    return output_status((delivery *)context, 2, control, error);
}
static uint32_t output_commit(void *context, const netdata_promql_query_observations_t *observations,
                              const netdata_promql_callback_control_t *control,
                              netdata_promql_bytes_t *error) {
    assert(observations->output_samples == 1);
    assert(observations->memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    return output_status((delivery *)context, 3, control, error);
}
static void output_abort(void *context) {
    delivery *d = (delivery *)context;
    d->calls[3]++;
    memset(d->error, 'X', sizeof(d->error));
}
static void output_delivery(netdata_promql_engine_t *engine) {
    for (unsigned test = 0; test < 6; test++) {
        netdata_promql_query_t *query = compile(engine, "m");
        fixture f;
        init_fixture(&f, 5, 7);
        netdata_promql_provider_t p = provider(&f);
        netdata_promql_request_t request = instant(3000);
        delivery d;
        memset(&d, 0, sizeof(d));
        d.failure = test < 4 ? test : 0;
        d.cancel_at = test < 4 ? (test ? test : 3) : test - 3;
        assert(netdata_promql_control_create(engine, NULL, &d.control) == NETDATA_PROMQL_OK);
        netdata_promql_output_t output;
        memset(&output, 0, sizeof(output));
        output.header.abi_version = NETDATA_PROMQL_ABI_VERSION;
        output.header.struct_size = sizeof(output);
        output.context = &d;
        output.begin = output_begin; output.chunk = output_chunk;
        output.commit = output_commit; output.abort = output_abort;
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_execute(query, &p, &request, d.control, &output, &report);
        assert(status == (d.failure ? NETDATA_PROMQL_OUTPUT_ERROR : test ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_OK));
        assert(report && d.calls[0] == 1 && d.calls[3] == (test != 0));
        assert(d.calls[1] == (d.failure != 1 && d.cancel_at != 1));
        assert(d.calls[2] == (test == 0 || d.failure == 3));
        released(&f);
        if (d.failure) {
            static const uint8_t expected[] = {'o','u','t',0,255};
            netdata_promql_error_t error;
            assert(netdata_promql_report_error(report, 0, &error) == NETDATA_PROMQL_OK);
            assert(error.kind == NETDATA_PROMQL_ERROR_OUTPUT && error.message.len == sizeof(expected));
            assert(memcmp(error.message.data, expected, sizeof(expected)) == 0);
        }
        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
        assert(netdata_promql_control_release(d.control) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    }
}
int main(void) {
    assert(netdata_promql_abi_version() == NETDATA_PROMQL_ABI_VERSION);
    netdata_promql_engine_options_t options;
    assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
    assert(options.header.struct_size == sizeof(options));
    assert(options.lookback_ms == 300000 && options.no_step_subquery_interval_ms == 60000);
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t idle;
    assert(netdata_promql_engine_observations(engine, &idle) == NETDATA_PROMQL_OK);
    const uint64_t sizes[] = {1, 7, 1024};
    for (unsigned i = 0; i < 3; i++) {
        uint64_t first = long_window(engine, 8192, sizes[i]);
        balanced(engine, &idle);
        uint64_t second = long_window(engine, 65536, sizes[i]);
        assert(first == second);
        balanced(engine, &idle);
    }
    range_result(engine);
    balanced(engine, &idle);
    error_phases(engine);
    balanced(engine, &idle);
    output_delivery(engine);
    balanced(engine, &idle);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    puts("{\"native_embedding\":\"pass\"}");
    return 0;
}
