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
    netdata_promql_engine_t *accounting_engine;
    unsigned accounting_phases;
    void *query_buffer;
    netdata_promql_allocation_handle_t *query_allocation;
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
static void *foreign_buffer(size_t size) {
    volatile uint8_t *value = (volatile uint8_t *)malloc(size);
    assert(value && size);
    value[0] = 1;
    value[size - 1] = 2;
    return (void *)value;
}
static netdata_promql_allocation_handle_t *foreign_allocation(
        const netdata_promql_accounting_context_t *account, uint32_t kind, uint64_t size) {
    netdata_promql_allocation_handle_t *value = NULL;
    assert(netdata_promql_allocation_acquire(account, kind, size, &value) == NETDATA_PROMQL_OK);
    assert(value);
    return value;
}
static netdata_promql_engine_observations_t engine_observations(netdata_promql_engine_t *engine) {
    netdata_promql_engine_observations_t value;
    assert(netdata_promql_engine_observations(engine, &value) == NETDATA_PROMQL_OK);
    return value;
}
static void foreign_accounting_probe(netdata_promql_engine_t *engine,
                                     const netdata_promql_accounting_context_t *account) {
    const uint64_t unit = UINT64_C(1048576);
    netdata_promql_engine_observations_t before = engine_observations(engine);
    void *first = foreign_buffer((size_t)unit);
    netdata_promql_allocation_handle_t *a = foreign_allocation(account, NETDATA_PROMQL_MEMORY_WORKSPACE, unit);
    netdata_promql_engine_observations_t one = engine_observations(engine);
    assert(one.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current == before.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current + unit);
    uint64_t wrapper = one.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current - before.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current;
    assert(wrapper > 0);
    void *second = foreign_buffer((size_t)(2 * unit));
    netdata_promql_allocation_handle_t *b = foreign_allocation(account, NETDATA_PROMQL_MEMORY_WORKSPACE, 2 * unit);
    void *replacement = foreign_buffer((size_t)(4 * unit));
    netdata_promql_allocation_handle_t *c = foreign_allocation(account, NETDATA_PROMQL_MEMORY_WORKSPACE, 4 * unit);
    netdata_promql_engine_observations_t overlap = engine_observations(engine);
    assert(overlap.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current == before.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current + 7 * unit);
    assert(overlap.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak >= overlap.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current);
    assert(overlap.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current == before.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current + 3 * wrapper);
    free(second);
    assert(netdata_promql_allocation_release(b) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t smaller = engine_observations(engine);
    assert(smaller.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current == before.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current + 5 * unit);
    free(first);
    assert(netdata_promql_allocation_resize(a, 0) == NETDATA_PROMQL_OK);
    first = foreign_buffer(4096);
    assert(netdata_promql_allocation_resize(a, 4096) == NETDATA_PROMQL_OK);
    smaller = engine_observations(engine);
    assert(smaller.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current == before.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current + 4 * unit + 4096);
    assert(smaller.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current == before.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current + 2 * wrapper);
    free(first);
    assert(netdata_promql_allocation_resize(a, 0) == NETDATA_PROMQL_OK);
    assert(netdata_promql_allocation_release(a) == NETDATA_PROMQL_OK);
    free(replacement);
    assert(netdata_promql_allocation_release(c) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t after = engine_observations(engine);
    assert(after.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current == before.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current);
    assert(after.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current == before.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current);
}
static void account_fixture(fixture *f, const netdata_promql_accounting_context_t *account, unsigned phase) {
    assert(account);
    if (!f->accounting_engine) return;
    f->accounting_phases |= 1u << phase;
    foreign_accounting_probe(f->accounting_engine, account);
}
static netdata_promql_resources_t no_resources(void *context, void *handle,
                                             const netdata_promql_accounting_context_t *account) {
    fixture *f = (fixture *)context;
    (void)handle;
    assert(account != NULL);
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
static const netdata_promql_accounting_context_t *callback_account(const netdata_promql_callback_control_t *control) {
    const netdata_promql_accounting_context_t *account = NULL;
    assert(netdata_promql_callback_accounting(control, &account) == NETDATA_PROMQL_OK);
    assert(account != NULL);
    return account;
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
    if (f->accounting_engine) account_fixture(f, callback_account(control), 0);
    f->opened[0]++;
    status = returned(f, 1, error);
    if (status != NETDATA_PROMQL_OK) return status;
    if (f->accounting_engine) {
        f->query_buffer = foreign_buffer(4096);
        f->query_allocation = foreign_allocation(callback_account(control), NETDATA_PROMQL_MEMORY_PROVIDER_HELD, 4096);
    }
    *out = NULL;
    return NETDATA_PROMQL_OK;
}
static void query_close(void *context, void *handle, const netdata_promql_accounting_context_t *account) {
    fixture *f = (fixture *)context;
    assert(account != NULL);
    account_fixture(f, account, 2);
    if (f->query_allocation) {
        free(f->query_buffer);
        f->query_buffer = NULL;
        assert(netdata_promql_allocation_release(f->query_allocation) == NETDATA_PROMQL_OK);
        f->query_allocation = NULL;
    }
    (void)handle;
    f->closed[0]++;
    memset(f->error, 'X', sizeof(f->error));
}
static netdata_promql_resources_t query_resources(void *context, void *handle,
                                                 const netdata_promql_accounting_context_t *account) {
    account_fixture((fixture *)context, account, 1);
    /* The retained query buffer is already charged through its explicit allocation handle. */
    return no_resources(context, handle, account);
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
static void selection_close(void *context, void *handle, const netdata_promql_accounting_context_t *account) {
    fixture *f = (fixture *)context;
    assert(account != NULL);
    account_fixture(f, account, 4);
    f->closed[1]++;
    free(handle);
    memset(f->error, 'X', sizeof(f->error));
}
static netdata_promql_resources_t selection_resources(void *context, void *handle,
                                                    const netdata_promql_accounting_context_t *account) {
    account_fixture((fixture *)context, account, 3);
    netdata_promql_resources_t result = no_resources(context, handle, account);
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
static void series_close(void *context, void *handle, const netdata_promql_accounting_context_t *account) {
    fixture *f = (fixture *)context;
    assert(account != NULL);
    account_fixture(f, account, 6);
    (void)handle;
    f->closed[2]++;
    memset(f->error, 'X', sizeof(f->error));
}
static netdata_promql_resources_t series_resources(void *context, void *handle,
                                                  const netdata_promql_accounting_context_t *account) {
    account_fixture((fixture *)context, account, 5);
    return no_resources(context, handle, account);
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
static void cursor_close(void *context, void *handle, const netdata_promql_accounting_context_t *account) {
    fixture *f = (fixture *)context;
    cursor *c = (cursor *)handle;
    assert(account != NULL);
    account_fixture(f, account, 8);
    f->closed[3]++;
    free(c->page);
    free(c);
    memset(f->error, 'X', sizeof(f->error));
}
static netdata_promql_resources_t cursor_resources(void *context, void *handle,
                                                 const netdata_promql_accounting_context_t *account) {
    account_fixture((fixture *)context, account, 7);
    cursor *c = (cursor *)handle;
    netdata_promql_resources_t result = no_resources(context, handle, account);
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
    *resources = cursor_resources(context, handle, callback_account(control));
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    status = returned(f, 6, error);
    if (status != NETDATA_PROMQL_OK) return status;
    uint64_t count = 0;
    while (count < c->capacity && c->index < f->rows) {
        int64_t time_ms = (int64_t)(c->index + 1) * 1000;
        if (time_ms > c->bounds.end_ms) break;
        if (time_ms < c->bounds.start_ms) {
            c->index++;
            continue;
        }
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
    p.query_open = query_open; p.query_close = query_close; p.query_resources = query_resources;
    p.selection_open = selection_open; p.selection_close = selection_close;
    p.selection_resources = selection_resources; p.selection_next = selection_next;
    p.series_close = series_close; p.series_resources = series_resources;
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
        /* A matrix requires a physical Seek; this instant lookup's Seek is a no-op. */
        netdata_promql_query_t *query = compile(engine, stage == 5 ? "m[1s]" : "m");
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
    netdata_promql_engine_t *accounting_engine;
    uint32_t accounting_callbacks, null_accounting, retain_output;
    uint64_t result_before;
    void *buffer;
    netdata_promql_allocation_handle_t *allocation;
} delivery;
static uint32_t output_status(delivery *d, uint32_t phase,
                              const netdata_promql_callback_control_t *control,
                              netdata_promql_bytes_t *error) {
    assert(checked_control(control) == NETDATA_PROMQL_OK);
    if (d->accounting_engine) {
        const netdata_promql_accounting_context_t *account = callback_account(control);
        foreign_accounting_probe(d->accounting_engine, account);
        d->accounting_callbacks++;
        if (d->retain_output && phase == 1) {
            d->result_before = engine_observations(d->accounting_engine).memory[NETDATA_PROMQL_MEMORY_RESULT].current;
            d->buffer = foreign_buffer(4096);
            d->allocation = foreign_allocation(account, NETDATA_PROMQL_MEMORY_RESULT, 4096);
        }
    }
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
static void output_abort(void *context, const netdata_promql_accounting_context_t *account) {
    delivery *d = (delivery *)context;
    if (d->accounting_engine) {
        if (account) {
            foreign_accounting_probe(d->accounting_engine, account);
            d->accounting_callbacks++;
        } else d->null_accounting++;
    } else assert(account);
    d->calls[3]++;
    if (d->allocation) {
        free(d->buffer);
        d->buffer = NULL;
        assert(netdata_promql_allocation_release(d->allocation) == NETDATA_PROMQL_OK);
        d->allocation = NULL;
    }
    memset(d->error, 'X', sizeof(d->error));
}
static netdata_promql_output_t delivery_table(delivery *d) {
    netdata_promql_output_t output;
    memset(&output, 0, sizeof(output));
    output.header.abi_version = NETDATA_PROMQL_ABI_VERSION;
    output.header.struct_size = sizeof(output);
    output.context = d;
    output.begin = output_begin; output.chunk = output_chunk;
    output.commit = output_commit; output.abort = output_abort;
    return output;
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
        netdata_promql_output_t output = delivery_table(&d);
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
static void foreign_provider_accounting(netdata_promql_engine_t *engine) {
    netdata_promql_query_t *query = compile(engine, "m[1s]");
    fixture f;
    init_fixture(&f, 3, 1);
    f.accounting_engine = engine;
    netdata_promql_provider_t p = provider(&f);
    netdata_promql_request_t request = instant(3000);
    netdata_promql_result_t *result = NULL;
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_collect(query, &p, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
    released(&f);
    assert(f.accounting_phases == 511 && !f.query_buffer && !f.query_allocation);
    netdata_promql_query_observations_t observed;
    assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current == 0);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].peak >= 4096);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].peak < 8192);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak >= UINT64_C(7340032));
    assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    puts("{\"foreign_accounting\":\"pass\",\"callback_phases\":9,\"overlap_bytes\":7340032,\"retained_bytes\":4096}");
}
static void foreign_early_abort(netdata_promql_engine_t *engine) {
    for (unsigned test = 0; test < 7; test++) {
        netdata_promql_query_t *query = compile(engine, test == 5 ? "m[step()]" : "m");
        fixture f;
        init_fixture(&f, 3, 1);
        netdata_promql_provider_t p = provider(&f);
        if (test == 3) p.header.abi_version = 1;
        netdata_promql_request_t request = instant(3000);
        if (test == 2) request.kind = UINT32_MAX;
        delivery d;
        memset(&d, 0, sizeof(d));
        d.accounting_engine = engine;
        netdata_promql_output_t output = delivery_table(&d);
        if (test == 6) output.header.abi_version = 1;
        if (test == 4) {
            assert(netdata_promql_control_create(engine, NULL, &d.control) == NETDATA_PROMQL_OK);
            assert(netdata_promql_control_cancel(d.control) == NETDATA_PROMQL_OK);
        }
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_execute(test == 0 ? NULL : query, &p,
                test == 1 ? NULL : &request, d.control, &output, &report);
        assert(status == (test == 4 ? NETDATA_PROMQL_CANCELLED :
                          test == 5 ? NETDATA_PROMQL_EVALUATION_ERROR : NETDATA_PROMQL_INVALID_ARGUMENT));
        assert(!d.calls[0] && !d.calls[1] && !d.calls[2]);
        assert(d.calls[3] == (test != 6));
        assert(d.null_accounting == (test == 0));
        assert(d.accounting_callbacks == (test > 0 && test < 6));
        assert(!f.opened[0]);
        released(&f);
        if (report) assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        if (d.control) assert(netdata_promql_control_release(d.control) == NETDATA_PROMQL_OK);
        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    }
    puts("{\"foreign_early_abort\":\"pass\",\"cases\":7}");
}
static void foreign_allocation_lifetime(void) {
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(NULL, &engine) == NETDATA_PROMQL_OK);
    netdata_promql_query_t *query = compile(engine, "m");
    fixture f;
    init_fixture(&f, 3, 1);
    netdata_promql_provider_t p = provider(&f);
    netdata_promql_request_t request = instant(3000);
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    assert(idle.memory[NETDATA_PROMQL_MEMORY_RESULT].current == 0);
    for (unsigned phase = 1; phase <= 3; phase++) {
        delivery failed;
        memset(&failed, 0, sizeof(failed));
        failed.accounting_engine = engine;
        failed.retain_output = 1;
        failed.failure = phase;
        netdata_promql_output_t output = delivery_table(&failed);
        netdata_promql_report_t *report = NULL;
        assert(netdata_promql_execute(query, &p, &request, NULL, &output, &report) == NETDATA_PROMQL_OUTPUT_ERROR);
        assert(failed.calls[3] == 1 && !failed.buffer && !failed.allocation);
        assert(failed.accounting_callbacks == phase + 1);
        netdata_promql_query_observations_t observed;
        assert(netdata_promql_report_live_observations(report, &observed) == NETDATA_PROMQL_OK);
        assert(observed.memory[NETDATA_PROMQL_MEMORY_RESULT].current == failed.result_before);
        assert(observed.memory[NETDATA_PROMQL_MEMORY_RESULT].peak == failed.result_before + 4096);
        released(&f);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        balanced(engine, &idle);
    }
    delivery d;
    memset(&d, 0, sizeof(d));
    d.accounting_engine = engine;
    d.retain_output = 1;
    netdata_promql_output_t output = delivery_table(&d);
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_execute(query, &p, &request, NULL, &output, &report) == NETDATA_PROMQL_OK);
    assert(d.accounting_callbacks == 3 && !d.calls[3] && d.buffer && d.allocation);
    released(&f);
    netdata_promql_query_observations_t fixed, live;
    assert(netdata_promql_report_observations(report, &fixed) == NETDATA_PROMQL_OK);
    assert(fixed.memory[NETDATA_PROMQL_MEMORY_RESULT].current == d.result_before + 4096);
    free(d.buffer);
    assert(netdata_promql_allocation_resize(d.allocation, 0) == NETDATA_PROMQL_OK);
    d.buffer = foreign_buffer(8192);
    assert(netdata_promql_allocation_resize(d.allocation, 8192) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_live_observations(report, &live) == NETDATA_PROMQL_OK);
    assert(live.memory[NETDATA_PROMQL_MEMORY_RESULT].current == d.result_before + 8192);
    assert(netdata_promql_report_observations(report, &live) == NETDATA_PROMQL_OK);
    assert(live.memory[NETDATA_PROMQL_MEMORY_RESULT].current == fixed.memory[NETDATA_PROMQL_MEMORY_RESULT].current);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    free(d.buffer);
    assert(netdata_promql_allocation_resize(d.allocation, 0) == NETDATA_PROMQL_OK);
    d.buffer = foreign_buffer(16384);
    assert(netdata_promql_allocation_resize(d.allocation, 16384) == NETDATA_PROMQL_OK);
    assert(((volatile uint8_t *)d.buffer)[16383] == 2);
    free(d.buffer);
    assert(netdata_promql_allocation_release(d.allocation) == NETDATA_PROMQL_OK);
    puts("{\"foreign_allocation_lifetime\":\"pass\",\"bytes\":16384,\"abort_cases\":3}");
}
static void label_field_boundary(netdata_promql_engine_t *engine) {
    const size_t boundary = ((size_t)1) << 24;
    static const char message[] = "String too long to encode as label.";
    uint8_t *field = (uint8_t *)malloc(boundary + 1);
    assert(field);
    memset(field, 'a', boundary + 1);
    netdata_promql_query_t *query = compile(engine, "m");
    for (unsigned name = 0; name < 2; name++) {
        for (unsigned offset = 0; offset < 3; offset++) {
            size_t length = boundary - 1 + offset;
            fixture f;
            init_fixture(&f, 1, 1);
            if (name) f.labels[1].name = bytes(field, length);
            else f.labels[1].value = bytes(field, length);
            netdata_promql_provider_t p = provider(&f);
            netdata_promql_request_t request = instant(1000);
            netdata_promql_result_t *result = NULL;
            netdata_promql_report_t *report = NULL;
            uint32_t status = netdata_promql_collect(query, &p, &request, NULL, &result, &report);
            assert(report);
            if (offset == 0) {
                assert(status == NETDATA_PROMQL_OK && result);
                netdata_promql_label_t label;
                assert(netdata_promql_result_label(result, 0, 1, &label) == NETDATA_PROMQL_OK);
                netdata_promql_bytes_t value = name ? label.name : label.value;
                assert(value.len == length && memcmp(value.data, field, length) == 0);
                assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
            } else {
                assert(status == NETDATA_PROMQL_EVALUATION_ERROR && !result);
                netdata_promql_error_t error;
                assert(netdata_promql_report_error(report, 0, &error) == NETDATA_PROMQL_OK);
                assert(error.kind == NETDATA_PROMQL_ERROR_STORAGE_INVALID);
                assert(error.message.len == sizeof(message) - 1);
                assert(memcmp(error.message.data, message, sizeof(message) - 1) == 0);
                assert(f.opened[3] == 0);
            }
            released(&f);
            assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        }
    }
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    free(field);
    puts("{\"label_field_boundary\":\"pass\",\"cases\":6}");
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
    foreign_provider_accounting(engine);
    balanced(engine, &idle);
    foreign_early_abort(engine);
    balanced(engine, &idle);
    label_field_boundary(engine);
    balanced(engine, &idle);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    foreign_allocation_lifetime();
    puts("{\"native_embedding\":\"pass\"}");
    return 0;
}
