/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Keep one-reset subtraction order observable through actual C callbacks. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

typedef struct {
    fixture source;
    unsigned two_resets;
} reset_fixture;

static uint32_t reset_next(void *context, void *handle,
                           const netdata_promql_callback_control_t *control,
                           netdata_promql_page_t *out, netdata_promql_resources_t *resources,
                           netdata_promql_bytes_t *error) {
    reset_fixture *source = (reset_fixture *)context;
    uint32_t status = cursor_next(&source->source, handle, control, out, resources, error);
    if (status != NETDATA_PROMQL_OK) return status;
    cursor *c = (cursor *)handle;
    const double values[] = {18014398509481984.0, 0.0, 1.0, source->two_resets ? 0.0 : 1.0};
    for (uint64_t i = 0; i < out->samples.len; i++) {
        uint64_t index = (uint64_t)c->page[i].time_ms / 1000 - 1;
        assert(index < 4);
        c->page[i].value.payload.float_bits = netdata_promql_double_bits(values[index]);
    }
    return status;
}

int main(void) {
    const char *expressions[] = {"rate(m_total[4s]) > bool 0", "increase(m_total[4s]) > bool 0"};
    const uint64_t pages[] = {1, 17, 1024};
    unsigned cases = 0;
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned expression = 0; expression < 2; expression++) {
            for (unsigned two = 0; two < 2; two++) {
                for (unsigned page = 0; page < 3; page++) {
                    for (unsigned arm = 0; arm < 3; arm++) {
                        reset_fixture source;
                        memset(&source, 0, sizeof(source));
                        init_fixture(&source.source, 4, pages[page]);
                        source.source.labels[0].value = text("m_total");
                        source.two_resets = two;
                        source.source.failure = arm == 1 ? 6 : 0;
                        if (arm == 2) {
                            source.source.cancel_at = 6;
                            assert(netdata_promql_control_create(engine, NULL, &source.source.control) == NETDATA_PROMQL_OK);
                        }
                        netdata_promql_provider_t input = provider(&source.source);
                        input.context = &source;
                        input.cursor_next = reset_next;
                        netdata_promql_query_t *query = compile(engine, expressions[expression]);
                        netdata_promql_request_t request = instant(4000);
                        netdata_promql_result_t *result = NULL;
                        netdata_promql_report_t *report = NULL;
                        uint32_t status = netdata_promql_collect(query, &input, &request, source.source.control, &result, &report);
                        assert(status == (arm == 1 ? NETDATA_PROMQL_PROVIDER_ERROR : arm == 2 ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_OK));
                        assert(report && (arm ? !result : result != NULL));
                        released(&source.source);
                        netdata_promql_query_observations_t observed;
                        assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
                        statistics_valid(&observed.statistics);
                        assert(observed.samples_fetched == source.source.samples_returned);
                        assert(observed.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current == 0);
                        assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
                        assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
                        if (!arm) {
                            netdata_promql_result_info_t info;
                            netdata_promql_series_info_t series;
                            netdata_promql_point_info_t point;
                            netdata_promql_label_t label;
                            assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
                            assert(info.header.value_type == NETDATA_PROMQL_TYPE_VECTOR && info.series_count == 1 && info.annotation_count == 0);
                            assert(netdata_promql_result_series_info(result, 0, &series) == NETDATA_PROMQL_OK);
                            assert(series.label_count == 1 && series.point_count == 1);
                            assert(netdata_promql_result_label(result, 0, 0, &label) == NETDATA_PROMQL_OK);
                            assert(label.name.len == source.source.labels[1].name.len && label.value.len == source.source.labels[1].value.len);
                            assert(!memcmp(label.name.data, source.source.labels[1].name.data, (size_t)label.name.len));
                            assert(!memcmp(label.value.data, source.source.labels[1].value.data, (size_t)label.value.len));
                            assert(netdata_promql_result_point(result, 0, 0, &point) == NETDATA_PROMQL_OK);
                            assert(point.time_ms == 4000 && point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
                            assert(point.value_bits == netdata_promql_double_bits((double)two));
                            assert(observed.statistics.samples.total_queryable_samples == 4);
                            assert(observed.statistics.samples.samples_read == (two ? 16u : 8u));
                            assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
                        }
                        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
                        if (source.source.control) assert(netdata_promql_control_release(source.source.control) == NETDATA_PROMQL_OK);
                        balanced(engine, &idle);
                        cases++;
                    }
                }
            }
        }
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    assert(cases == 72);
    printf("{\"single_reset\":\"pass\",\"cases\":%u}\n", cases);
    return 0;
}
