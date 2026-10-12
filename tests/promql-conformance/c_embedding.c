/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Real C/C++ caller: generated source samples retain one page, never a complete history. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#ifndef _FILE_OFFSET_BITS
#define _FILE_OFFSET_BITS 64
#endif
#include "netdata-promql.h"
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

#if defined(__cplusplus)
static_assert(sizeof(off_t) == 8, "temporary float storage needs 64-bit file offsets");
#else
_Static_assert(sizeof(off_t) == 8, "temporary float storage needs 64-bit file offsets");
#endif

typedef struct {
    int fd;
    uint64_t count;
    netdata_promql_allocation_handle_t *allocation;
} temporary_store;

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
    unsigned mixed_values, temporary_phases, temporary_failure, temporary_cancel, temporary_null, temporary_wrong_phase;
    int temporary_wrong_length;
    uint64_t temporary_opened, temporary_closed;
    temporary_store *temporary_token;
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
static netdata_promql_bytes_t text(const char *value) {
    return bytes(value, strlen(value));
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
static int duration_order(netdata_promql_duration_t a, netdata_promql_duration_t b) {
    if (a.seconds != b.seconds) return a.seconds < b.seconds ? -1 : 1;
    return a.nanoseconds < b.nanoseconds ? -1 : a.nanoseconds > b.nanoseconds;
}
static void statistics_valid(const netdata_promql_statistics_t *s) {
    const netdata_promql_duration_t phases[] = {
        s->timings.eval_total_time, s->timings.result_sort_time, s->timings.query_preparation_time,
        s->timings.inner_eval_time, s->timings.exec_queue_time, s->timings.exec_total_time,
    };
    for (unsigned i = 0; i < 6; i++) {
        assert(phases[i].nanoseconds < 1000000000 && phases[i].reserved == 0);
    }
    assert(s->timings.exec_queue_time.seconds == 0 && s->timings.exec_queue_time.nanoseconds == 0);
    assert(duration_order(s->timings.exec_total_time, s->timings.eval_total_time) >= 0);
    assert(duration_order(s->timings.eval_total_time, s->timings.query_preparation_time) >= 0);
    assert(duration_order(s->timings.eval_total_time, s->timings.inner_eval_time) >= 0);
    assert(duration_order(s->timings.eval_total_time, s->timings.result_sort_time) >= 0);
}
static void statistics_equal(const netdata_promql_statistics_t *a, const netdata_promql_statistics_t *b) {
    statistics_valid(a); statistics_valid(b);
    const netdata_promql_duration_t left[] = {
        a->timings.eval_total_time, a->timings.result_sort_time, a->timings.query_preparation_time,
        a->timings.inner_eval_time, a->timings.exec_queue_time, a->timings.exec_total_time,
    };
    const netdata_promql_duration_t right[] = {
        b->timings.eval_total_time, b->timings.result_sort_time, b->timings.query_preparation_time,
        b->timings.inner_eval_time, b->timings.exec_queue_time, b->timings.exec_total_time,
    };
    for (unsigned i = 0; i < 6; i++) assert(duration_order(left[i], right[i]) == 0);
    assert(a->samples.total_queryable_samples == b->samples.total_queryable_samples);
    assert(a->samples.samples_read == b->samples.samples_read);
    assert(a->samples.peak_samples == b->samples.peak_samples);
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

    /* Two query exposures share one external backing; neither owns its bytes. */
    static const uint8_t shared_view[4096] = {1};
    assert(shared_view[0] == 1);
    a = foreign_allocation(account, NETDATA_PROMQL_MEMORY_BORROWED_VIEW, 1024);
    b = foreign_allocation(account, NETDATA_PROMQL_MEMORY_BORROWED_VIEW, 1024);
    one = engine_observations(engine);
    assert(one.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == before.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current + 2048);
    assert(one.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current == before.memory[NETDATA_PROMQL_MEMORY_ACCOUNTING].current + 2 * wrapper);
    assert(one.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current == before.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].current);
    assert(one.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current == before.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current);
    assert(netdata_promql_allocation_resize(a, sizeof(shared_view)) == NETDATA_PROMQL_OK);
    overlap = engine_observations(engine);
    assert(overlap.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == before.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current + 5120);
    assert(overlap.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].peak >= overlap.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current);
    assert(netdata_promql_allocation_release(b) == NETDATA_PROMQL_OK);
    assert(netdata_promql_allocation_resize(a, 0) == NETDATA_PROMQL_OK);
    assert(netdata_promql_allocation_release(a) == NETDATA_PROMQL_OK);
    const uint32_t invalid[] = {5, 6, 7, UINT32_MAX};
    for (unsigned i = 0; i < 4; i++) {
        netdata_promql_allocation_handle_t *rejected = NULL;
        assert(netdata_promql_allocation_acquire(account, invalid[i], 1024, &rejected) == NETDATA_PROMQL_INVALID_ARGUMENT);
        assert(!rejected);
    }
    after = engine_observations(engine);
    for (unsigned i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++) assert(after.memory[i].current == before.memory[i].current);
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
        static const uint64_t mixed[] = {UINT64_C(0xbff0000000000000), UINT64_C(0x8000000000000000), 0, UINT64_C(0x3ff0000000000000)};
        sample->value.payload.float_bits = f->mixed_values ? mixed[c->index % 4] : netdata_promql_double_bits((double)(c->index + 1));
        c->index++;
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
static void temporary_probe(fixture *f, const netdata_promql_accounting_context_t *account, unsigned phase) {
    f->temporary_phases |= 1u << phase;
    void *buffer = foreign_buffer(257);
    netdata_promql_allocation_handle_t *allocation = foreign_allocation(account, NETDATA_PROMQL_MEMORY_WORKSPACE, 257);
    free(buffer);
    assert(netdata_promql_allocation_release(allocation) == NETDATA_PROMQL_OK);
}
static uint32_t temporary_fault(fixture *f, unsigned phase, netdata_promql_bytes_t *error) {
    if (f->temporary_cancel == phase) assert(netdata_promql_control_cancel(f->control) == NETDATA_PROMQL_OK);
    if (f->temporary_failure != phase) return NETDATA_PROMQL_OK;
    static const uint8_t message[] = {'t','e','m','p','o','r','a','r','y',0,255,'f','a','i','l','u','r','e'};
    memcpy(f->error, message, sizeof(message));
    *error = bytes(f->error, sizeof(message));
    return NETDATA_PROMQL_PROVIDER_ERROR;
}
static uint32_t temporary_os_error(fixture *f, netdata_promql_bytes_t *error) {
    assert(snprintf((char *)f->error, sizeof(f->error), "temporary I/O: %d", errno) > 0);
    *error = bytes(f->error, strlen((const char *)f->error));
    return NETDATA_PROMQL_PROVIDER_ERROR;
}
static temporary_store *temporary_token(fixture *f, void *handle) {
    assert(handle == (f->temporary_null ? NULL : f->temporary_token));
    assert(f->temporary_token);
    return f->temporary_token;
}
static uint32_t temporary_create(void *context, void *query, uint64_t count,
                                 const netdata_promql_callback_control_t *control, void **out,
                                 netdata_promql_bytes_t *error) {
    fixture *f = (fixture *)context;
    (void)query;
    const netdata_promql_accounting_context_t *account = callback_account(control);
    temporary_probe(f, account, 0);
    uint32_t status = temporary_fault(f, 1, error);
    if (status != NETDATA_PROMQL_OK) return status;
    const char *directory = getenv("NETDATA_PROMQL_FIXTURE_SCRATCH");
    struct stat info;
    assert(directory && directory[0] == '/' && strlen(directory) >= 10);
    assert(stat(directory, &info) == 0 && S_ISDIR(info.st_mode));
    assert(count <= (uint64_t)INT64_MAX / 8);
    int fd = open(directory, O_TMPFILE | O_RDWR | O_CLOEXEC, 0600);
    if (fd < 0) return temporary_os_error(f, error);
    if (ftruncate(fd, (off_t)(count * 8)) != 0) {
        int cause = errno;
        assert(close(fd) == 0);
        errno = cause;
        return temporary_os_error(f, error);
    }
    temporary_store *store = (temporary_store *)malloc(sizeof(*store));
    assert(store);
    store->fd = fd;
    store->count = count;
    store->allocation = foreign_allocation(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, sizeof(*store));
    assert(!f->temporary_token);
    f->temporary_token = store;
    f->temporary_opened++;
    *out = f->temporary_null ? NULL : store;
    return NETDATA_PROMQL_OK;
}
static uint32_t temporary_transfer(void *context, void *handle, uint64_t offset, void *buffer, uint64_t count,
                                   const netdata_promql_callback_control_t *control, uint64_t *transferred,
                                   netdata_promql_bytes_t *error, unsigned write_mode) {
    fixture *f = (fixture *)context;
    temporary_probe(f, callback_account(control), write_mode ? 2 : 1);
    uint32_t status = temporary_fault(f, write_mode ? 3 : 2, error);
    if (status != NETDATA_PROMQL_OK) return status;
    temporary_store *store = temporary_token(f, handle);
    assert(offset <= store->count && count <= store->count - offset && count <= 1024);
    size_t bytes_left = (size_t)(count * 8), done = 0;
    while (done < bytes_left) {
        ssize_t part = write_mode ? pwrite(store->fd, (uint8_t *)buffer + done, bytes_left - done, (off_t)(offset * 8 + done))
                                  : pread(store->fd, (uint8_t *)buffer + done, bytes_left - done, (off_t)(offset * 8 + done));
        if (part < 0 && errno == EINTR) continue;
        if (part <= 0) return temporary_os_error(f, error);
        done += (size_t)part;
    }
    *transferred = f->temporary_wrong_phase != (write_mode ? 3u : 2u) ? count :
                   f->temporary_wrong_length < 0 ? count - 1 : count + (unsigned)f->temporary_wrong_length;
    return NETDATA_PROMQL_OK;
}
static uint32_t temporary_read(void *context, void *handle, uint64_t offset, uint64_t *buffer, uint64_t count,
                               const netdata_promql_callback_control_t *control, uint64_t *transferred,
                               netdata_promql_bytes_t *error) {
    return temporary_transfer(context, handle, offset, buffer, count, control, transferred, error, 0);
}
static uint32_t temporary_write(void *context, void *handle, uint64_t offset, const uint64_t *buffer, uint64_t count,
                                const netdata_promql_callback_control_t *control, uint64_t *transferred,
                                netdata_promql_bytes_t *error) {
    return temporary_transfer(context, handle, offset, (void *)buffer, count, control, transferred, error, 1);
}
static netdata_promql_resources_t temporary_resources(void *context, void *handle,
                                                      const netdata_promql_accounting_context_t *account) {
    fixture *f = (fixture *)context;
    (void)temporary_token(f, handle);
    temporary_probe(f, account, 3);
    return no_resources(context, handle, account);
}
static void temporary_close(void *context, void *handle, const netdata_promql_accounting_context_t *account) {
    fixture *f = (fixture *)context;
    temporary_probe(f, account, 4);
    temporary_store *store = temporary_token(f, handle);
    netdata_promql_allocation_handle_t *allocation = store->allocation;
    assert(close(store->fd) == 0);
    free(store);
    f->temporary_token = NULL;
    assert(netdata_promql_allocation_release(allocation) == NETDATA_PROMQL_OK);
    f->temporary_closed++;
    memset(f->error, 'X', sizeof(f->error));
}
static netdata_promql_provider_t temporary_provider(fixture *f) {
    netdata_promql_provider_t p = provider(f);
    p.temporary_create = temporary_create; p.temporary_read = temporary_read; p.temporary_write = temporary_write;
    p.temporary_resources = temporary_resources; p.temporary_close = temporary_close;
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
    netdata_promql_query_observations_t compilation, reported;
    assert(netdata_promql_query_compilation_observations(query, &compilation) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_observations(report, &reported) == NETDATA_PROMQL_OK);
    netdata_promql_statistics_t empty;
    memset(&empty, 0, sizeof(empty));
    statistics_equal(&compilation.statistics, &empty);
    statistics_equal(&reported.statistics, &empty);
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
    assert(observed.temporary_storage.extent.current == 0 && observed.temporary_storage.stores.current == 0);
    assert(observed.temporary_storage.extent.acquired == observed.temporary_storage.extent.released);
    assert(observed.temporary_storage.stores.acquired == observed.temporary_storage.stores.released);
}
static uint64_t temporary_window(netdata_promql_engine_t *engine, uint64_t rows, uint64_t maximum, uint64_t *sample_peak) {
    netdata_promql_query_t *query = compile(engine, "quantile_over_time(0.5,m[100000s])");
    fixture f;
    init_fixture(&f, rows, maximum);
    f.mixed_values = 1;
    netdata_promql_provider_t p = temporary_provider(&f);
    netdata_promql_request_t request = instant((int64_t)rows * 1000);
    netdata_promql_result_t *result = NULL;
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_collect(query, &p, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
    released(&f);
    assert(f.temporary_opened > 0 && f.temporary_closed == f.temporary_opened && !f.temporary_token);
    assert(f.temporary_phases == 31);
    netdata_promql_query_observations_t observed;
    assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
    statistics_valid(&observed.statistics);
    assert(observed.statistics.samples.total_queryable_samples == rows);
    assert(observed.statistics.samples.samples_read >= rows);
    *sample_peak = observed.statistics.samples.peak_samples;
    assert(observed.samples_fetched == f.samples_returned && observed.pages == f.pages_returned);
    assert(observed.temporary_storage.extent.peak == rows * 8);
    assert(observed.temporary_storage.stores.peak == 1);
    assert(observed.temporary_storage.stores.acquired == f.temporary_opened);
    assert(observed.temporary_storage.stores.released == f.temporary_closed);
    assert(observed.temporary_storage.extent.acquired == rows * 8 * f.temporary_opened);
    assert(observed.temporary_storage.extent.current == 0 && observed.temporary_storage.stores.current == 0);
    assert(observed.temporary_storage.read_calls && observed.temporary_storage.written_bytes >= rows * 8);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current == 0);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].peak == maximum * sizeof(netdata_promql_sample_t));
    netdata_promql_point_info_t point;
    assert(netdata_promql_result_point(result, 0, 0, &point) == NETDATA_PROMQL_OK);
    assert(point.value_kind == NETDATA_PROMQL_VALUE_FLOAT && point.value_bits == 0);
    assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    printf("{\"temporary_rows\":%" PRIu64 ",\"page_size\":%" PRIu64 ",\"workspace_peak\":%" PRIu64
           ",\"borrowed_peak\":%" PRIu64 ",\"extent_peak\":%" PRIu64 ",\"read_calls\":%" PRIu64 ",\"write_calls\":%" PRIu64 "}\n",
           rows, maximum, observed.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak,
           observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].peak, observed.temporary_storage.extent.peak,
           observed.temporary_storage.read_calls, observed.temporary_storage.write_calls);
    return observed.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak;
}
static void temporary_failure_cases(netdata_promql_engine_t *engine) {
    const unsigned faults[][4] = {{1,1,0,0}, {2,2,0,0}, {3,3,0,0}, {0,2,0,0}, {0,3,0,0},
                                 {0,0,2,0}, {0,0,2,1}, {0,0,3,0}, {0,0,3,1}, {0,0,0,0}, {0,0,0,0}};
    for (unsigned i = 0; i < 11; i++) {
        netdata_promql_query_t *query = compile(engine, "quantile_over_time(0.5,m[2048s])");
        fixture f;
        init_fixture(&f, 2048, 17);
        f.mixed_values = 1;
        f.temporary_failure = faults[i][0]; f.temporary_cancel = faults[i][1];
        f.temporary_wrong_phase = faults[i][2]; f.temporary_wrong_length = faults[i][3] ? 1 : -1;
        f.temporary_null = i == 10;
        assert(netdata_promql_control_create(engine, NULL, &f.control) == NETDATA_PROMQL_OK);
        netdata_promql_provider_t p = i == 9 ? provider(&f) : temporary_provider(&f);
        netdata_promql_request_t request = instant(2048000);
        netdata_promql_result_t *result = NULL;
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_collect(query, &p, &request, f.control, &result, &report);
        released(&f);
        assert(f.temporary_opened == f.temporary_closed && !f.temporary_token);
        assert(i == 10 ? f.temporary_opened > 0 : f.temporary_opened == (i != 0 && i != 9));
        netdata_promql_query_observations_t observations;
        assert(netdata_promql_report_observations(report, &observations) == NETDATA_PROMQL_OK);
        assert(observations.temporary_storage.extent.current == 0 && observations.temporary_storage.stores.current == 0);
        if (i == 10) {
            assert(status == NETDATA_PROMQL_OK && result);
            assert(f.temporary_phases == 31);
            assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
        } else {
            assert(!result && status != NETDATA_PROMQL_OK);
            uint32_t expected_status = i < 3 || i == 9 ? NETDATA_PROMQL_PROVIDER_ERROR :
                i == 3 || i == 4 ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_EVALUATION_ERROR;
            assert(status == expected_status);
            netdata_promql_error_t error;
            assert(netdata_promql_report_error(report, 0, &error) == NETDATA_PROMQL_OK);
            if (i < 3) {
                static const uint8_t expected[] = {'t','e','m','p','o','r','a','r','y',0,255,'f','a','i','l','u','r','e'};
                assert(error.message.len >= sizeof(expected));
                assert(memcmp(error.message.data + error.message.len - sizeof(expected), expected, sizeof(expected)) == 0);
            }
        }
        assert(netdata_promql_control_release(f.control) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    }
    puts("{\"temporary_failures\":\"pass\",\"cases\":11,\"callback_phases\":5}");
}
static uint64_t long_window(netdata_promql_engine_t *engine, uint64_t rows, uint64_t maximum, uint64_t *sample_peak) {
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
    statistics_valid(&observed.statistics);
    assert(observed.statistics.samples.total_queryable_samples == rows);
    assert(observed.statistics.samples.samples_read >= rows);
    *sample_peak = observed.statistics.samples.peak_samples;
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
static void basic_statistics_cases(netdata_promql_engine_t *engine) {
    const char *sources[] = {"42", "\"text\"", "m", "m", "m[3s]", "sum_over_time(m[3s])", "m @ 3"};
    const uint64_t totals[] = {0, 0, 1, 3, 3, 3, 3};
    const uint64_t sizes[] = {1, 17, 1024};
    for (unsigned page = 0; page < 3; page++) {
        for (unsigned i = 0; i < 7; i++) {
            netdata_promql_query_t *query = compile(engine, sources[i]);
            fixture f;
            init_fixture(&f, 2048, sizes[page]);
            netdata_promql_provider_t p = provider(&f);
            netdata_promql_request_t request = instant(3000);
            if (i == 3 || i == 6) {
                request.kind = NETDATA_PROMQL_REQUEST_RANGE;
                request.start_ms = 2000; request.end_ms = 4000; request.step_ms = 1000;
            }
            netdata_promql_result_t *result = NULL;
            netdata_promql_report_t *report = NULL;
            assert(netdata_promql_collect(query, &p, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
            released(&f);
            netdata_promql_query_observations_t fixed, live;
            assert(netdata_promql_report_observations(report, &fixed) == NETDATA_PROMQL_OK);
            assert(netdata_promql_result_live_observations(result, &live) == NETDATA_PROMQL_OK);
            statistics_equal(&fixed.statistics, &live.statistics);
            assert(fixed.statistics.samples.total_queryable_samples == totals[i]);
            if (totals[i]) {
                assert(fixed.statistics.samples.samples_read > 0 && fixed.statistics.samples.peak_samples > 0);
            } else assert(fixed.statistics.samples.samples_read == 0);
            if (i == 1) assert(fixed.statistics.samples.peak_samples == 0);
            if (i == 3 || i == 6) assert(fixed.statistics.samples.peak_samples >= 3);
            if (i == 4) assert(fixed.statistics.samples.peak_samples == 4);
            if (i == 2 && page == 2) assert(fixed.statistics.samples.samples_read < fixed.samples_fetched);
            assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
            assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
            assert(netdata_promql_result_live_observations(result, &live) == NETDATA_PROMQL_OK);
            statistics_equal(&fixed.statistics, &live.statistics);
            assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
        }
    }
    puts("{\"basic_statistics\":\"pass\",\"cases\":21,\"page_sizes\":3}");
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
    netdata_promql_statistics_t committed_statistics;
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
    assert(observations->temporary_storage.extent.current == 0 && observations->temporary_storage.stores.current == 0);
    statistics_valid(&observations->statistics);
    assert(observations->statistics.samples.total_queryable_samples == 1);
    assert(observations->statistics.samples.samples_read >= 1);
    assert(observations->statistics.samples.peak_samples >= 1);
    ((delivery *)context)->committed_statistics = observations->statistics;
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
        if (status == NETDATA_PROMQL_OK) {
            netdata_promql_query_observations_t fixed, live;
            assert(netdata_promql_report_observations(report, &fixed) == NETDATA_PROMQL_OK);
            assert(netdata_promql_report_live_observations(report, &live) == NETDATA_PROMQL_OK);
            statistics_equal(&d.committed_statistics, &fixed.statistics);
            statistics_equal(&d.committed_statistics, &live.statistics);
        }
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

typedef struct {
    delivery base;
    unsigned kind, buckets;
    netdata_promql_report_t *child_report;
} histogram_delivery;

static uint32_t histogram_bucket(void *context, const netdata_promql_histogram_bucket_t *bucket,
                                 const netdata_promql_callback_control_t *control,
                                 netdata_promql_bytes_t *error) {
    histogram_delivery *d = (histogram_delivery *)context;
    assert(checked_control(control) == NETDATA_PROMQL_OK);
    assert(bucket->lower_inclusive <= 1 && bucket->upper_inclusive <= 1);
    unsigned index = d->buckets++;
    if (d->kind == 0) {
        const double lower[] = {-0.25, 0.5, 1}, upper[] = {0.25, 1, 2}, count[] = {2, 0, 4};
        assert(index < 3);
        assert(bucket->lower_bits == netdata_promql_double_bits(lower[index]));
        assert(bucket->upper_bits == netdata_promql_double_bits(upper[index]));
        assert(bucket->count_bits == netdata_promql_double_bits(count[index]));
        assert(bucket->lower_inclusive == (index == 0) && bucket->upper_inclusive == 1);
    }
    if (d->kind == 1 && index == 1) assert(bucket->count_bits == netdata_promql_double_bits(0));
    if (d->kind == 3) assert(bucket->count_bits == (index == 0 ? UINT64_C(0x7ff8000000000042) : UINT64_C(0x7ff0000000000000)));
    if (d->kind >= 5) {
        assert(netdata_promql_control_cancel(d->base.control) == NETDATA_PROMQL_OK);
        if (d->kind == 5) {
            *error = bytes("bucket\0error", 12);
            return NETDATA_PROMQL_OUTPUT_ERROR;
        }
    }
    return NETDATA_PROMQL_OK;
}

static uint32_t histogram_begin(void *context, const netdata_promql_output_header_t *header,
                                const netdata_promql_callback_control_t *control,
                                netdata_promql_bytes_t *error) {
    histogram_delivery *d = (histogram_delivery *)context;
    assert(output_begin(&d->base, header, control, error) == NETDATA_PROMQL_OK);
    netdata_promql_span_t span = {0, 2};
    double buckets[] = {0, 4, 3}, bounds[] = {-1, 1};
    netdata_promql_float_histogram_t histogram;
    memset(&histogram, 0, sizeof(histogram));
    histogram.zero_count_bits = netdata_promql_double_bits(2);
    histogram.zero_threshold_bits = netdata_promql_double_bits(0.25);
    histogram.positive_spans.data = &span; histogram.positive_spans.len = 1; histogram.positive_spans.present = 1;
    histogram.positive_buckets.data = buckets; histogram.positive_buckets.len = 2; histogram.positive_buckets.present = 1;
    if (d->kind == 1) {
        histogram.schema = -53;
        histogram.zero_count_bits = 0;
        span.length = 3; buckets[0] = 2; buckets[1] = 0;
        histogram.positive_buckets.len = 3;
        histogram.custom_values.data = bounds; histogram.custom_values.len = 2; histogram.custom_values.present = 1;
    }
    if (d->kind == 2) { memset(&histogram, 0, sizeof(histogram)); histogram.schema = 7; }
    if (d->kind == 3) {
        histogram.zero_count_bits = 0;
        buckets[0] = netdata_promql_double_from_bits(UINT64_C(0x7ff8000000000042));
        buckets[1] = netdata_promql_double_from_bits(UINT64_C(0x7ff0000000000000));
    }
    if (d->kind == 4) histogram.positive_spans.present = 2;
    uint32_t status = netdata_promql_callback_histogram_buckets(&histogram, control, histogram_bucket, d, &d->child_report);
    if (status == NETDATA_PROMQL_OK) {
        assert(netdata_promql_report_release(d->child_report) == NETDATA_PROMQL_OK);
        d->child_report = NULL;
        return status;
    }
    if (status == NETDATA_PROMQL_CANCELLED || status == NETDATA_PROMQL_DEADLINE) return status;
    if (d->child_report) {
        netdata_promql_error_t child;
        assert(netdata_promql_report_error(d->child_report, 0, &child) == NETDATA_PROMQL_OK);
        *error = child.message;
    } else *error = text("invalid histogram view");
    return NETDATA_PROMQL_OUTPUT_ERROR;
}

static void histogram_abort(void *context, const netdata_promql_accounting_context_t *account) {
    histogram_delivery *d = (histogram_delivery *)context;
    if (d->child_report) {
        assert(netdata_promql_report_release(d->child_report) == NETDATA_PROMQL_OK);
        d->child_report = NULL;
    }
    output_abort(&d->base, account);
}

static void histogram_delivery_cases(netdata_promql_engine_t *engine) {
    for (unsigned kind = 0; kind < 7; kind++) {
        histogram_delivery d;
        memset(&d, 0, sizeof(d)); d.kind = kind;
        assert(netdata_promql_control_create(engine, NULL, &d.base.control) == NETDATA_PROMQL_OK);
        fixture f; init_fixture(&f, 5, 7);
        netdata_promql_provider_t p = provider(&f);
        netdata_promql_query_t *query = compile(engine, "m");
        netdata_promql_request_t request = instant(3000);
        netdata_promql_output_t output = delivery_table(&d.base);
        output.context = &d; output.begin = histogram_begin; output.abort = histogram_abort;
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_execute(query, &p, &request, d.base.control, &output, &report);
        assert(status == (kind < 4 ? NETDATA_PROMQL_OK : kind < 6 ? NETDATA_PROMQL_OUTPUT_ERROR : NETDATA_PROMQL_CANCELLED));
        assert(d.buckets == (kind == 2 || kind == 4 ? 0 : kind >= 5 ? 1 : kind == 3 ? 2 : 3));
        assert(!d.child_report && d.base.calls[3] == (kind >= 4));
        if (kind == 5) {
            netdata_promql_error_t error;
            assert(netdata_promql_report_error(report, 0, &error) == NETDATA_PROMQL_OK);
            assert(error.message.len == 12 && memcmp(error.message.data, "bucket\0error", 12) == 0);
        }
        released(&f);
        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
        assert(netdata_promql_control_release(d.base.control) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    }
    puts("{\"histogram_buckets\":\"pass\",\"cases\":7}");
}
typedef struct {
    delivery base;
    unsigned kind, comparisons;
    netdata_promql_report_t *child_report;
} sort_delivery;

static uint32_t sort_compare(void *context, uint64_t left, uint64_t right,
                              const netdata_promql_callback_control_t *control,
                              int32_t *ordering, netdata_promql_bytes_t *error) {
    sort_delivery *d = (sort_delivery *)context;
    static const unsigned keys[] = {2, 1, 1};
    assert(checked_control(control) == NETDATA_PROMQL_OK && left < 3 && right < 3);
    d->comparisons++;
    *ordering = keys[left] < keys[right] ? -17 : keys[left] > keys[right] ? 23 : 0;
    if(d->kind == 1 || d->kind == 2) {
        assert(netdata_promql_control_cancel(d->base.control) == NETDATA_PROMQL_OK);
        if(d->kind == 1) { *error = bytes("compare\0error", 13); return NETDATA_PROMQL_OUTPUT_ERROR; }
    }
    return NETDATA_PROMQL_OK;
}

static uint32_t sort_begin(void *context, const netdata_promql_output_header_t *header,
                            const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    sort_delivery *d = (sort_delivery *)context;
    assert(output_begin(&d->base, header, control, error) == NETDATA_PROMQL_OK);
    uint64_t indices[] = {0, 1, 2};
    if(d->kind == 5) assert(netdata_promql_control_cancel(d->base.control) == NETDATA_PROMQL_OK);
    uint32_t status = netdata_promql_callback_sort_indices(d->kind == 3 || d->kind == 4 ? NULL : indices,
        d->kind == 3 ? 1 : d->kind == 4 ? 0 : 3, control, sort_compare, d, &d->child_report);
    if(status == NETDATA_PROMQL_OK) {
        if(d->kind == 0) assert(indices[0] == 1 && indices[1] == 2 && indices[2] == 0);
        assert(netdata_promql_report_release(d->child_report) == NETDATA_PROMQL_OK);
        d->child_report = NULL;
        return NETDATA_PROMQL_OK;
    }
    if(d->child_report) {
        netdata_promql_error_t failure;
        assert(netdata_promql_report_error(d->child_report, 0, &failure) == NETDATA_PROMQL_OK);
        *error = failure.message;
    }
    else *error = text("invalid sort array");
    return status == NETDATA_PROMQL_INVALID_ARGUMENT ? NETDATA_PROMQL_OUTPUT_ERROR : status;
}

static void sort_abort(void *context, const netdata_promql_accounting_context_t *account) {
    sort_delivery *d = (sort_delivery *)context;
    if(d->child_report) {
        assert(netdata_promql_report_release(d->child_report) == NETDATA_PROMQL_OK);
        d->child_report = NULL;
    }
    output_abort(&d->base, account);
}

static void sort_delivery_cases(netdata_promql_engine_t *engine) {
    for(unsigned kind = 0; kind < 6; kind++) {
        sort_delivery d;
        memset(&d, 0, sizeof(d)); d.kind = kind;
        assert(netdata_promql_control_create(engine, NULL, &d.base.control) == NETDATA_PROMQL_OK);
        fixture f; init_fixture(&f, 5, 7);
        netdata_promql_provider_t p = provider(&f);
        netdata_promql_query_t *query = compile(engine, "m");
        netdata_promql_request_t request = instant(3000);
        netdata_promql_output_t output = delivery_table(&d.base);
        output.context = &d; output.begin = sort_begin; output.abort = sort_abort;
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_execute(query, &p, &request, d.base.control, &output, &report);
        uint32_t expected = kind == 0 || kind == 4 ? NETDATA_PROMQL_OK :
            kind == 1 || kind == 3 ? NETDATA_PROMQL_OUTPUT_ERROR : NETDATA_PROMQL_CANCELLED;
        assert(status == expected && !d.child_report && d.base.calls[3] == (expected != NETDATA_PROMQL_OK));
        assert(d.comparisons == (kind == 0 ? 3 : kind == 1 || kind == 2 ? 1 : 0));
        if(kind == 1) {
            netdata_promql_error_t failure;
            assert(netdata_promql_report_error(report, 0, &failure) == NETDATA_PROMQL_OK);
            assert(failure.message.len == 13 && memcmp(failure.message.data, "compare\0error", 13) == 0);
        }
        released(&f);
        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
        assert(netdata_promql_control_release(d.base.control) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    }
    puts("{\"index_sort\":\"pass\",\"cases\":6}");
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
    for (unsigned test = 0; test < 11; test++) {
        netdata_promql_query_t *query = compile(engine, test == 5 ? "m[step()]" : "m");
        fixture f;
        init_fixture(&f, 3, 1);
        netdata_promql_provider_t p = provider(&f);
        unsigned old_provider = test == 3 || test == 7 || test == 8;
        unsigned old_output = test == 6 || test == 9 || test == 10;
        if (old_provider) p.header.abi_version = test == 3 ? 1 : test - 5;
        netdata_promql_request_t request = instant(3000);
        if (test == 2) request.kind = UINT32_MAX;
        delivery d;
        memset(&d, 0, sizeof(d));
        d.accounting_engine = engine;
        netdata_promql_output_t output = delivery_table(&d);
        if (old_output) output.header.abi_version = test == 6 ? 1 : test - 7;
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
        assert(d.calls[3] == !old_output);
        assert(d.null_accounting == (test == 0));
        assert(d.accounting_callbacks == (test > 0 && !old_output));
        assert(!f.opened[0]);
        released(&f);
        if (report) assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        if (d.control) assert(netdata_promql_control_release(d.control) == NETDATA_PROMQL_OK);
        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    }
    puts("{\"foreign_early_abort\":\"pass\",\"cases\":11}");
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
static void precise_requests(netdata_promql_engine_t *engine) {
    netdata_promql_precise_request_t request;
    memset(&request, 0, sizeof(request));
    request.header.abi_version = NETDATA_PROMQL_ABI_VERSION;
    request.header.struct_size = sizeof(request);
    request.kind = NETDATA_PROMQL_REQUEST_INSTANT;
    request.start.unix_seconds = -1;
    request.start.nanoseconds = 999999999;
    request.lookback_ns = INT64_C(300000000000);
    fixture f;
    init_fixture(&f, 17, 1);
    netdata_promql_provider_t p = provider(&f);
    netdata_promql_query_t *query = compile(engine, "vector(time())");
    netdata_promql_result_t *result = NULL;
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_collect_precise(query, &p, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
    netdata_promql_point_info_t point;
    assert(netdata_promql_result_point(result, 0, 0, &point) == NETDATA_PROMQL_OK);
    assert(point.time_ms == 0 && point.value_bits == 0);
    released(&f);
    assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);

    query = compile(engine, "m");
    memset(&request.start, 0, sizeof(request.start));
    request.kind = NETDATA_PROMQL_REQUEST_RANGE;
    request.end.nanoseconds = 2000000;
    request.interval_ns = 500000;
    result = NULL; report = NULL;
    assert(netdata_promql_collect_precise(query, &p, &request, NULL, &result, &report) == NETDATA_PROMQL_EVALUATION_ERROR);
    assert(!result && report);
    netdata_promql_error_t error;
    const char message[] = "unexpected error: runtime error: integer divide by zero";
    assert(netdata_promql_report_error(report, 0, &error) == NETDATA_PROMQL_OK);
    assert(error.message.len == sizeof(message) - 1 && !memcmp(error.message.data, message, sizeof(message) - 1));
    released(&f);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    puts("{\"precise_requests\":\"pass\",\"cases\":2}");
}
static void protocol_report_account(void *context, void *handle, const netdata_promql_accounting_context_t *account) {
    fixture *source = (fixture *)context;
    assert(handle == source);
    foreign_accounting_probe(source->accounting_engine, account);
    source->query_buffer = foreign_buffer(4096);
    source->query_allocation = foreign_allocation(account, NETDATA_PROMQL_MEMORY_RESULT, 4096);
}

static void protocol_primitives(netdata_promql_engine_t *engine) {
    netdata_promql_timestamp_t stamp;
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_api_parse_time(engine, text("1969-12-31T23:59:59.999999999Z"), NULL, &stamp, &report) == NETDATA_PROMQL_OK);
    assert(stamp.unix_seconds == -1 && stamp.nanoseconds == 999999999 && stamp.reserved == 0);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    int64_t duration = 0;
    assert(netdata_promql_api_parse_duration(engine, text("0.0015"), NULL, &duration, &report) == NETDATA_PROMQL_OK);
    assert(duration == 1500000);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_api_parse_time(engine, text("bad"), NULL, &stamp, &report) == NETDATA_PROMQL_PARSE_ERROR);
    netdata_promql_error_t error;
    assert(netdata_promql_report_error(report, 0, &error) == NETDATA_PROMQL_OK);
    const char *expected_error = "cannot parse \"bad\" to a valid timestamp";
    assert(error.position_present == 0 && error.message.len == strlen(expected_error));
    assert(memcmp(error.message.data, expected_error, (size_t)error.message.len) == 0);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);

    netdata_promql_numeric_text_t number;
    assert(netdata_promql_api_scalar_float_text(UINT64_C(0x8000000000000001), &number) == NETDATA_PROMQL_OK);
    assert(number.len == 327 && memcmp(number.bytes, "-0.", 3) == 0 && number.bytes[326] == '5');
    assert(netdata_promql_api_timestamp_text(100, &number) == NETDATA_PROMQL_OK);
    assert(number.len == 5 && memcmp(number.bytes, "0.100", 5) == 0);
    assert(netdata_promql_api_scalar_timestamp_text(100, &number) == NETDATA_PROMQL_OK);
    assert(number.len == 3 && memcmp(number.bytes, "0.1", 3) == 0);

    netdata_promql_selector_t *selector = NULL;
    assert(netdata_promql_selector_compile(engine, text("{}"), NULL, &selector, &report) == NETDATA_PROMQL_OK);
    uint32_t nonempty = 1;
    assert(netdata_promql_selector_nonempty(selector, NULL, &nonempty) == NETDATA_PROMQL_OK && nonempty == 0);
    assert(netdata_promql_selector_release(selector) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_selector_compile(engine, text("{zone=~\"x|y\",missing!=\"present\"}"), NULL, &selector, &report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    netdata_promql_label_t label;
    label.name = text("zone"); label.value = text("x");
    netdata_promql_labels_t labels; labels.data = &label; labels.len = 1;
    uint32_t matched = 0;
    assert(netdata_promql_selector_matches(selector, labels, NULL, &matched) == NETDATA_PROMQL_OK && matched == 1);
    label.value = text("z");
    assert(netdata_promql_selector_matches(selector, labels, NULL, &matched) == NETDATA_PROMQL_OK && matched == 0);
    assert(netdata_promql_selector_release(selector) == NETDATA_PROMQL_OK);
    const char *limit_inputs[] = {"", "-0", "+0007", "2147483648", "9223372036854775807", "-1", "9223372036854775808"};
    const int64_t limit_values[] = {0, 0, 7, INT64_C(2147483648), INT64_MAX, 0, 0};
    for(unsigned i = 0; i < sizeof(limit_values) / sizeof(limit_values[0]); i++) {
        int64_t value = 99;
        assert(netdata_promql_api_parse_limit(engine, text(limit_inputs[i]), NULL, &value, &report) ==
               (i < 5 ? NETDATA_PROMQL_OK : NETDATA_PROMQL_PARSE_ERROR));
        assert(value == limit_values[i]);
        fixture owner;
        init_fixture(&owner, 0, 1);
        owner.accounting_engine = engine;
        assert(netdata_promql_report_accounted(report, protocol_report_account, &owner, &owner) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        assert(engine_observations(engine).memory[NETDATA_PROMQL_MEMORY_RESULT].current >= 4096);
        free(owner.query_buffer);
        assert(netdata_promql_allocation_release(owner.query_allocation) == NETDATA_PROMQL_OK);
    }
    puts("{\"protocol_primitives\":\"pass\"}");
}

typedef struct {
    delivery output;
    fixture *source;
    uint64_t rows;
    unsigned stop;
} metadata_output;

static uint32_t metadata_begin(void *context, const netdata_promql_output_header_t *header,
                               const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    metadata_output *output = (metadata_output *)context;
    assert(header->value_type == NETDATA_PROMQL_TYPE_VECTOR && header->start_ms == -123 && header->end_ms == 4567);
    return output_status(&output->output, 1, control, error);
}

static uint32_t metadata_chunk(void *context, const netdata_promql_chunk_t *chunk,
                               const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    metadata_output *output = (metadata_output *)context;
    assert(chunk->kind == NETDATA_PROMQL_CHUNK_SERIES);
    assert(chunk->payload.series.points.len == 0 && chunk->payload.series.labels.len == 2);
    assert(chunk->payload.series.series_id == output->rows++);
    uint32_t status = output_status(&output->output, 2, control, error);
    return status == NETDATA_PROMQL_OK && output->stop ? NETDATA_PROMQL_END : status;
}

static uint32_t metadata_commit(void *context, const netdata_promql_query_observations_t *observations,
                                const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    metadata_output *output = (metadata_output *)context;
    assert(observations->pages == 0 && observations->samples_fetched == 0 && observations->output_samples == 0);
    assert(observations->statistics.samples.total_queryable_samples == 0 && observations->statistics.samples.samples_read == 0);
    assert(observations->statistics.samples.peak_samples == 0 && observations->memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    released(output->source);
    return output_status(&output->output, 3, control, error);
}

static void metadata_abort(void *context, const netdata_promql_accounting_context_t *account) {
    output_abort(&((metadata_output *)context)->output, account);
}

static void metadata_discovery(netdata_promql_engine_t *engine) {
    for(unsigned test = 0; test < 6; test++) {
        fixture source;
        init_fixture(&source, 8192, 1);
        netdata_promql_provider_t input = provider(&source);
        netdata_promql_selector_t *selector = NULL;
        netdata_promql_report_t *report = NULL;
        if(test) {
            assert(netdata_promql_selector_compile(engine, text(test == 3 ? "n" : "m"), NULL, &selector, &report) == NETDATA_PROMQL_OK);
            assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        }
        metadata_output output;
        memset(&output, 0, sizeof(output));
        output.source = &source;
        output.stop = test == 2;
        // Cleanup allocation probes would hide the cancelled roster's workspace peak.
        output.output.accounting_engine = test == 5 ? NULL : engine;
        output.output.retain_output = 1;
        if(test == 4) {
            output.output.failure = output.output.cancel_at = 2;
            assert(netdata_promql_control_create(engine, NULL, &output.output.control) == NETDATA_PROMQL_OK);
        }
        if(test == 5) {
            assert(netdata_promql_control_create(engine, NULL, &output.output.control) == NETDATA_PROMQL_OK);
            assert(netdata_promql_control_cancel(output.output.control) == NETDATA_PROMQL_OK);
        }
        netdata_promql_output_t sink;
        memset(&sink, 0, sizeof(sink));
        sink.header.abi_version = NETDATA_PROMQL_ABI_VERSION;
        sink.header.struct_size = sizeof(sink);
        sink.context = &output;
        sink.begin = metadata_begin; sink.chunk = metadata_chunk; sink.commit = metadata_commit; sink.abort = metadata_abort;
        const netdata_promql_selector_t *selectors[4096];
        for(size_t i = 0; i < sizeof(selectors) / sizeof(selectors[0]); i++) selectors[i] = selector;
        netdata_promql_bounds_t bounds = {-123, 4567};
        uint32_t status = netdata_promql_discover(engine, &input, bounds, selectors, test == 5 ? 4096 : test ? 1 : 0,
                                                 output.output.control, &sink, &report);
        assert(status == (test == 4 ? NETDATA_PROMQL_OUTPUT_ERROR : test == 5 ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_OK));
        assert(output.rows == (test == 3 || test == 5 ? 0u : 1u));
        assert(source.opened[3] == 0 && source.samples_returned == 0 && source.temporary_opened == 0);
        assert(output.output.calls[3] == (test == 4 || test == 5));
        if(test == 5) {
            netdata_promql_query_observations_t observed;
            assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
            assert(observed.memory[NETDATA_PROMQL_MEMORY_WORKSPACE].peak == 0 && observed.work == 0);
            assert(!source.opened[0] && !output.output.calls[0]);
        }
        released(&source);
        if(selector) assert(netdata_promql_selector_release(selector) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        if(output.output.control) assert(netdata_promql_control_release(output.output.control) == NETDATA_PROMQL_OK);
        if(test != 4 && test != 5) {
            assert(output.output.allocation && engine_observations(engine).memory[NETDATA_PROMQL_MEMORY_RESULT].current >= 4096);
            free(output.output.buffer);
            assert(netdata_promql_allocation_release(output.output.allocation) == NETDATA_PROMQL_OK);
        }
    }
    puts("{\"metadata_discovery\":\"pass\",\"cases\":6}");
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
        uint64_t first_samples, second_samples;
        uint64_t first = long_window(engine, 8192, sizes[i], &first_samples);
        balanced(engine, &idle);
        uint64_t second = long_window(engine, 65536, sizes[i], &second_samples);
        assert(first == second);
        assert(first_samples == second_samples);
        balanced(engine, &idle);
    }
    const uint64_t temporary_sizes[] = {1, 17, 1024};
    for (unsigned i = 0; i < 3; i++) {
        uint64_t first_samples, second_samples;
        uint64_t first = temporary_window(engine, 2048, temporary_sizes[i], &first_samples);
        balanced(engine, &idle);
        uint64_t second = temporary_window(engine, 100000, temporary_sizes[i], &second_samples);
        assert(first == second);
        assert(first_samples == second_samples);
        balanced(engine, &idle);
    }
    temporary_failure_cases(engine);
    balanced(engine, &idle);
    basic_statistics_cases(engine);
    balanced(engine, &idle);
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
    histogram_delivery_cases(engine);
    balanced(engine, &idle);
    sort_delivery_cases(engine);
    balanced(engine, &idle);
    precise_requests(engine);
    balanced(engine, &idle);
    protocol_primitives(engine);
    balanced(engine, &idle);
    metadata_discovery(engine);
    balanced(engine, &idle);
    label_field_boundary(engine);
    balanced(engine, &idle);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    foreign_allocation_lifetime();
    puts("{\"native_embedding\":\"pass\"}");
    return 0;
}
