/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Reuse the paged C provider; each cursor borrows its own immutable histogram. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

typedef struct {
    join_fixture source;
    netdata_promql_float_histogram_t histograms[4];
} native_fixture;

typedef struct {
    cursor *raw;
    const netdata_promql_float_histogram_t *histogram;
} native_cursor;

static uint32_t native_open(void *context, void *series, netdata_promql_bounds_t bounds,
                           const netdata_promql_callback_control_t *control,
                           netdata_promql_cursor_open_t *out, netdata_promql_bytes_t *error) {
    native_fixture *f = (native_fixture *)context;
    assert((uintptr_t)series >= 1 && (uintptr_t)series <= 4);
    uint32_t status = cursor_open(context, series, bounds, control, out, error);
    if (status != NETDATA_PROMQL_OK) return status;
    native_cursor *c = (native_cursor *)calloc(1, sizeof(*c));
    if (!c) {
        cursor_close(context, out->handle, callback_account(control));
        out->handle = NULL;
        return NETDATA_PROMQL_PROVIDER_ERROR;
    }
    c->raw = (cursor *)out->handle;
    c->histogram = &f->histograms[(uintptr_t)series - 1];
    out->handle = c;
    return NETDATA_PROMQL_OK;
}

static void native_close(void *context, void *handle,
                         const netdata_promql_accounting_context_t *account) {
    native_cursor *c = (native_cursor *)handle;
    cursor_close(context, c->raw, account);
    free(c);
}

static netdata_promql_resources_t native_resources(
        void *context, void *handle, const netdata_promql_accounting_context_t *account) {
    native_cursor *c = (native_cursor *)handle;
    netdata_promql_resources_t result = cursor_resources(context, c->raw, account);
    result.held_bytes += sizeof(*c);
    return result;
}

static uint32_t native_seek(void *context, void *handle, int64_t time_ms,
                           const netdata_promql_callback_control_t *control,
                           netdata_promql_bytes_t *error) {
    return cursor_seek(context, ((native_cursor *)handle)->raw, time_ms, control, error);
}

static uint32_t native_next(void *context, void *handle,
                           const netdata_promql_callback_control_t *control,
                           netdata_promql_page_t *out, netdata_promql_resources_t *resources,
                           netdata_promql_bytes_t *error) {
    native_cursor *c = (native_cursor *)handle;
    uint32_t status = cursor_next(context, c->raw, control, out, resources, error);
    resources->held_bytes += sizeof(*c);
    if (status == NETDATA_PROMQL_OK) {
        for (uint64_t i = 0; i < out->samples.len; i++) {
            c->raw->page[i].value.kind = NETDATA_PROMQL_VALUE_FLOAT_HISTOGRAM;
            c->raw->page[i].value.payload.float_histogram = c->histogram;
        }
        out->resources = *resources;
    }
    return status;
}

int main(void) {
    const uint64_t pages[] = {1, 17, 1024};
    const char *functions[] = {"histogram_sum", "histogram_count", "histogram_avg"};
    unsigned cases = 0;
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned page = 0; page < 3; page++)
            for (unsigned mode = 0; mode < 3; mode++)
                for (unsigned average = 0; average < 2; average++)
                    for (unsigned function = 0; function < 3; function++) {
                        native_fixture source;
                        memset(&source, 0, sizeof(source));
                        init_fixture(&source.source.source, 3, pages[page]);
                        source.source.aggregate_members = 1;
                        for (unsigned row = 0; row < 4; row++) {
                            netdata_promql_label_t *labels = source.source.labels[row];
                            labels[0].name = text("__name__"); labels[0].value = text("m_total");
                            labels[1].name = text("job"); labels[1].value = text(row < 2 ? "a" : "b");
                            labels[2].name = text("member"); labels[2].value = text(row % 2 ? "1" : "0");
                            netdata_promql_float_histogram_t *h = &source.histograms[row];
                            h->zero_threshold_bits = netdata_promql_double_bits(128);
                            h->count_bits = netdata_promql_double_bits(row % 2 ? 3 : 1);
                            h->zero_count_bits = h->count_bits;
                            h->sum_bits = netdata_promql_double_bits((row % 2 ? 12 : 2) * (row < 2 ? 1 : 2));
                        }
                        netdata_promql_provider_t input = provider(&source.source.source);
                        input.context = &source;
                        input.selection_next = join_selection_next;
                        input.cursor_open = native_open;
                        input.cursor_close = native_close;
                        input.cursor_resources = native_resources;
                        input.cursor_seek = native_seek;
                        input.cursor_next = native_next;
                        char expression[100];
                        int length = snprintf(expression, sizeof(expression), "%s(%s by(job)(m_total))",
                                              functions[function], average ? "avg" : "sum");
                        assert(length > 0 && (size_t)length < sizeof(expression));
                        netdata_promql_query_t *query = compile(engine, expression);
                        netdata_promql_request_t request = instant(mode == 1 ? 1000 : 3000);
                        if (mode) {
                            request.kind = NETDATA_PROMQL_REQUEST_RANGE;
                            request.end_ms = 3000;
                            request.step_ms = 1000;
                        }
                        netdata_promql_result_t *result = NULL;
                        netdata_promql_report_t *report = NULL;
                        assert(netdata_promql_collect(query, &input, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
                        released(&source.source.source);
                        netdata_promql_result_info_t info;
                        assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
                        assert(info.header.value_type == (mode ? NETDATA_PROMQL_TYPE_MATRIX : NETDATA_PROMQL_TYPE_VECTOR));
                        assert(info.annotation_count == 0 && info.series_count == 2);
                        unsigned seen = 0;
                        unsigned points = mode == 1 ? 3 : 1;
                        for (uint64_t row = 0; row < info.series_count; row++) {
                            netdata_promql_series_info_t series;
                            assert(netdata_promql_result_series_info(result, row, &series) == NETDATA_PROMQL_OK);
                            assert(series.point_count == points && series.label_count == 1);
                            netdata_promql_label_t job;
                            assert(netdata_promql_result_label(result, row, 0, &job) == NETDATA_PROMQL_OK);
                            assert(job.name.len == 3 && !memcmp(job.name.data, "job", 3));
                            assert(job.value.len == 1 && (job.value.data[0] == 'a' || job.value.data[0] == 'b'));
                            unsigned ordinal = (unsigned)(job.value.data[0] - 'a');
                            assert(!(seen & (1u << ordinal)));
                            seen |= 1u << ordinal;
                            double factor = ordinal ? 2 : 1;
                            double expected = function == 0 ? 14 * factor : function == 1 ? 4 : 3.5 * factor;
                            if (average && function != 2) expected /= 2;
                            for (unsigned index = 0; index < points; index++) {
                                netdata_promql_point_info_t point;
                                assert(netdata_promql_result_point(result, row, index, &point) == NETDATA_PROMQL_OK);
                                assert(point.time_ms == (mode == 1 ? (int64_t)(index + 1) * 1000 : 3000));
                                assert(point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
                                assert(point.value_bits == netdata_promql_double_bits(expected));
                            }
                        }
                        assert(seen == 3);
                        netdata_promql_query_observations_t observed;
                        assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
                        statistics_valid(&observed.statistics);
                        assert(observed.statistics.samples.total_queryable_samples == 44u * points);
                        printf("{\"function\":%u,\"average\":%u,\"mode\":%u,\"delayed\":%u,\"page\":%" PRIu64
                               ",\"work\":%" PRIu64 ",\"fetched\":%" PRIu64 ",\"logical\":%" PRIu64 "}\n",
                               function, average, mode, delayed, pages[page], observed.work,
                               observed.samples_fetched, observed.statistics.samples.total_queryable_samples);
                        assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
                        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
                        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
                        balanced(engine, &idle);
                        cases++;
                    }
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    assert(cases == 108);
    printf("{\"native_group_maps\":\"pass\",\"cases\":%u}\n", cases);
    return 0;
}
