/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Exercise literal range arithmetic and retained results through actual C callbacks. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

static uint32_t predecessor_next(void *context, void *handle,
                                  const netdata_promql_callback_control_t *control,
                                  netdata_promql_page_t *out, netdata_promql_resources_t *resources,
                                  netdata_promql_bytes_t *error) {
    uint32_t status = cursor_next(context, handle, control, out, resources, error);
    if (status != NETDATA_PROMQL_OK) return status;
    cursor *c = (cursor *)handle;
    for (uint64_t i = 0; i < out->samples.len; i++) {
        int64_t tick = c->page[i].time_ms / 1000;
        assert(tick >= 1 && tick <= 4);
        c->page[i].value.payload.float_bits = netdata_promql_double_bits((double)(5 - tick));
    }
    return status;
}

int main(void) {
    const struct { const char *text; double expected; } expressions[] = {
        {"rate(m_total[4s]) + 2", 3.0},
        {"2 + rate(m_total[4s])", 3.0},
        {"rate(m_total[4s]) - 2", -1.0},
        {"2 - rate(m_total[4s])", 1.0},
        {"rate(m_total[4s]) * 2", 2.0},
        {"2 * rate(m_total[4s])", 2.0},
        {"rate(m_total[4s]) / 2", 0.5},
        {"2 / rate(m_total[4s])", 2.0},
        {"rate(m_total[4s]) ^ 2", 1.0},
        {"2 ^ rate(m_total[4s])", 2.0},
        {"rate(m_total[4s]) % 2", 1.0},
        {"2 % rate(m_total[4s])", 0.0},
        {"rate(m_total[4s]) atan2 1", 0.7853981633974483},
        {"1 atan2 rate(m_total[4s])", 0.7853981633974483},
        {"increase(m_total[4s]) + 2", 6.0},
        {"2 + increase(m_total[4s])", 6.0},
        {"increase(m_total[4s]) - 2", 2.0},
        {"2 - increase(m_total[4s])", -2.0},
        {"increase(m_total[4s]) * 2", 8.0},
        {"2 * increase(m_total[4s])", 8.0},
        {"increase(m_total[4s]) / 2", 2.0},
        {"2 / increase(m_total[4s])", 0.5},
        {"increase(m_total[4s]) ^ 2", 16.0},
        {"2 ^ increase(m_total[4s])", 16.0},
        {"increase(m_total[4s]) % 2", 0.0},
        {"2 % increase(m_total[4s])", 2.0},
        {"increase(m_total[4s]) atan2 4", 0.7853981633974483},
        {"4 atan2 increase(m_total[4s])", 0.7853981633974483},
        {"sum_over_time(m_total[4s]) + 2", 12.0},
        {"2 + sum_over_time(m_total[4s])", 12.0},
        {"sum_over_time(m_total[4s]) - 2", 8.0},
        {"2 - sum_over_time(m_total[4s])", -8.0},
        {"sum_over_time(m_total[4s]) * 2", 20.0},
        {"2 * sum_over_time(m_total[4s])", 20.0},
        {"sum_over_time(m_total[4s]) / 2", 5.0},
        {"2 / sum_over_time(m_total[4s])", 0.2},
        {"sum_over_time(m_total[4s]) ^ 2", 100.0},
        {"2 ^ sum_over_time(m_total[4s])", 1024.0},
        {"sum_over_time(m_total[4s]) % 2", 0.0},
        {"2 % sum_over_time(m_total[4s])", 2.0},
        {"sum_over_time(m_total[4s]) atan2 10.0", 0.7853981633974483},
        {"10.0 atan2 sum_over_time(m_total[4s])", 0.7853981633974483},
        {"avg_over_time(m_total[4s]) + 2", 4.5},
        {"2 + avg_over_time(m_total[4s])", 4.5},
        {"avg_over_time(m_total[4s]) - 2", 0.5},
        {"2 - avg_over_time(m_total[4s])", -0.5},
        {"avg_over_time(m_total[4s]) * 2", 5.0},
        {"2 * avg_over_time(m_total[4s])", 5.0},
        {"avg_over_time(m_total[4s]) / 2", 1.25},
        {"2 / avg_over_time(m_total[4s])", 0.8},
        {"avg_over_time(m_total[4s]) ^ 2", 6.25},
        {"4 ^ avg_over_time(m_total[4s])", 32.0},
        {"avg_over_time(m_total[4s]) % 2", 0.5},
        {"2 % avg_over_time(m_total[4s])", 2.0},
        {"avg_over_time(m_total[4s]) atan2 2.5", 0.7853981633974483},
        {"2.5 atan2 avg_over_time(m_total[4s])", 0.7853981633974483},
        {"count_over_time(m_total[4s]) + 2", 6.0},
        {"2 + count_over_time(m_total[4s])", 6.0},
        {"count_over_time(m_total[4s]) - 2", 2.0},
        {"2 - count_over_time(m_total[4s])", -2.0},
        {"count_over_time(m_total[4s]) * 2", 8.0},
        {"2 * count_over_time(m_total[4s])", 8.0},
        {"count_over_time(m_total[4s]) / 2", 2.0},
        {"2 / count_over_time(m_total[4s])", 0.5},
        {"count_over_time(m_total[4s]) ^ 2", 16.0},
        {"2 ^ count_over_time(m_total[4s])", 16.0},
        {"count_over_time(m_total[4s]) % 2", 0.0},
        {"2 % count_over_time(m_total[4s])", 2.0},
        {"count_over_time(m_total[4s]) atan2 4", 0.7853981633974483},
        {"4 atan2 count_over_time(m_total[4s])", 0.7853981633974483},
        {"min_over_time(m_total[4s]) + 2", 3.0},
        {"2 + min_over_time(m_total[4s])", 3.0},
        {"min_over_time(m_total[4s]) - 2", -1.0},
        {"2 - min_over_time(m_total[4s])", 1.0},
        {"min_over_time(m_total[4s]) * 2", 2.0},
        {"2 * min_over_time(m_total[4s])", 2.0},
        {"min_over_time(m_total[4s]) / 2", 0.5},
        {"2 / min_over_time(m_total[4s])", 2.0},
        {"min_over_time(m_total[4s]) ^ 2", 1.0},
        {"2 ^ min_over_time(m_total[4s])", 2.0},
        {"min_over_time(m_total[4s]) % 2", 1.0},
        {"2 % min_over_time(m_total[4s])", 0.0},
        {"min_over_time(m_total[4s]) atan2 1", 0.7853981633974483},
        {"1 atan2 min_over_time(m_total[4s])", 0.7853981633974483},
        {"max_over_time(m_total[4s]) + 2", 6.0},
        {"2 + max_over_time(m_total[4s])", 6.0},
        {"max_over_time(m_total[4s]) - 2", 2.0},
        {"2 - max_over_time(m_total[4s])", -2.0},
        {"max_over_time(m_total[4s]) * 2", 8.0},
        {"2 * max_over_time(m_total[4s])", 8.0},
        {"max_over_time(m_total[4s]) / 2", 2.0},
        {"2 / max_over_time(m_total[4s])", 0.5},
        {"max_over_time(m_total[4s]) ^ 2", 16.0},
        {"2 ^ max_over_time(m_total[4s])", 16.0},
        {"max_over_time(m_total[4s]) % 2", 0.0},
        {"2 % max_over_time(m_total[4s])", 2.0},
        {"max_over_time(m_total[4s]) atan2 4", 0.7853981633974483},
        {"4 atan2 max_over_time(m_total[4s])", 0.7853981633974483},
        {"changes(m_total[4s]) + 2", 0x1.4000000000000p+2},
        {"2 + changes(m_total[4s])", 0x1.4000000000000p+2},
        {"changes(m_total[4s]) - 2", 0x1.0000000000000p+0},
        {"2 - changes(m_total[4s])", -0x1.0000000000000p+0},
        {"changes(m_total[4s]) * 2", 0x1.8000000000000p+2},
        {"2 * changes(m_total[4s])", 0x1.8000000000000p+2},
        {"changes(m_total[4s]) / 2", 0x1.8000000000000p+0},
        {"2 / changes(m_total[4s])", 0x1.5555555555555p-1},
        {"changes(m_total[4s]) ^ 2", 0x1.2000000000000p+3},
        {"2 ^ changes(m_total[4s])", 0x1.0000000000000p+3},
        {"changes(m_total[4s]) % 2", 0x1.0000000000000p+0},
        {"2 % changes(m_total[4s])", 0x1.0000000000000p+1},
        {"changes(m_total[4s]) atan2 2", 0x1.f730bd281f69bp-1},
        {"2 atan2 changes(m_total[4s])", 0x1.2d0ead6066395p-1},
        {"resets(m_total[4s]) + 2", 0x1.4000000000000p+2},
        {"2 + resets(m_total[4s])", 0x1.4000000000000p+2},
        {"resets(m_total[4s]) - 2", 0x1.0000000000000p+0},
        {"2 - resets(m_total[4s])", -0x1.0000000000000p+0},
        {"resets(m_total[4s]) * 2", 0x1.8000000000000p+2},
        {"2 * resets(m_total[4s])", 0x1.8000000000000p+2},
        {"resets(m_total[4s]) / 2", 0x1.8000000000000p+0},
        {"2 / resets(m_total[4s])", 0x1.5555555555555p-1},
        {"resets(m_total[4s]) ^ 2", 0x1.2000000000000p+3},
        {"2 ^ resets(m_total[4s])", 0x1.0000000000000p+3},
        {"resets(m_total[4s]) % 2", 0x1.0000000000000p+0},
        {"2 % resets(m_total[4s])", 0x1.0000000000000p+1},
        {"resets(m_total[4s]) atan2 2", 0x1.f730bd281f69bp-1},
        {"2 atan2 resets(m_total[4s])", 0x1.2d0ead6066395p-1}
    };
    const uint64_t pages[] = {1, 17, 1024};
    unsigned cases = 0;
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned expression = 0; expression < sizeof(expressions) / sizeof(expressions[0]); expression++) {
            {
                for (unsigned page = 0; page < 3; page++) {
                    for (unsigned arm = 0; arm < 3; arm++) {
                        fixture source;
                        memset(&source, 0, sizeof(source));
                        init_fixture(&source, 4, pages[page]);
                        source.labels[0].value = text("m_total");
                        source.failure = arm == 1 ? 6 : 0;
                        if (arm == 2) {
                            source.cancel_at = 6;
                            assert(netdata_promql_control_create(engine, NULL, &source.control) == NETDATA_PROMQL_OK);
                        }
                        netdata_promql_provider_t input = provider(&source);
                        if (strstr(expressions[expression].text, "changes(") ||
                            strstr(expressions[expression].text, "resets(")) input.cursor_next = predecessor_next;
                        netdata_promql_query_t *query = compile(engine, expressions[expression].text);
                        netdata_promql_request_t request = instant(4000);
                        netdata_promql_result_t *result = NULL;
                        netdata_promql_report_t *report = NULL;
                        uint32_t status = netdata_promql_collect(query, &input, &request, source.control, &result, &report);
                        assert(status == (arm == 1 ? NETDATA_PROMQL_PROVIDER_ERROR : arm == 2 ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_OK));
                        assert(report && (arm ? !result : result != NULL));
                        released(&source);
                        netdata_promql_query_observations_t observed;
                        assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
                        statistics_valid(&observed.statistics);
                        assert(observed.samples_fetched == source.samples_returned);
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
                            assert(label.name.len == source.labels[1].name.len && label.value.len == source.labels[1].value.len);
                            assert(!memcmp(label.name.data, source.labels[1].name.data, (size_t)label.name.len));
                            assert(!memcmp(label.value.data, source.labels[1].value.data, (size_t)label.value.len));
                            assert(netdata_promql_result_point(result, 0, 0, &point) == NETDATA_PROMQL_OK);
                            assert(point.time_ms == 4000 && point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
                            assert(point.value_bits == netdata_promql_double_bits(expressions[expression].expected));
                            assert(observed.statistics.samples.total_queryable_samples == 4);
                            assert(observed.statistics.samples.samples_read == 8u);
                            assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
                        }
                        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
                        if (source.control) assert(netdata_promql_control_release(source.control) == NETDATA_PROMQL_OK);
                        balanced(engine, &idle);
                        cases++;
                    }
                }
            }
        }
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    assert(cases == 2268);
    printf("{\"literal_rate\":\"pass\",\"cases\":%u}\n", cases);
    return 0;
}
