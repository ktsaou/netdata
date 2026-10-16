/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Reuse the paged foreign provider and complete ownership checks. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

int main(void) {
    const uint64_t pages[] = {1, 17, 1024};
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        options.flags |= NETDATA_PROMQL_EXPERIMENTAL_FUNCTIONS;
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned page = 0; page < 3; page++) {
            for (unsigned window = 0; window < 2; window++) {
                for (unsigned function = 0; function < 3; function++) {
                    join_fixture source;
                    memset(&source, 0, sizeof(source));
                    init_fixture(&source.source, 3, pages[page]);
                    source.aggregate_members = 1;
                    for (unsigned row = 0; row < 4; row++) {
                        source.labels[row][0].name = text("__name__");
                        source.labels[row][0].value = text("m_total");
                        source.labels[row][1].name = text("job");
                        source.labels[row][1].value = text(row < 2 ? "a" : "b");
                        source.labels[row][2].name = text("le");
                        source.labels[row][2].value = text(row % 2 ? "+Inf" : "1");
                    }
                    netdata_promql_provider_t input = provider(&source.source);
                    input.context = &source;
                    input.selection_next = join_selection_next;
                    input.cursor_open = join_cursor_open;
                    input.cursor_next = join_cursor_next;
                    const char *templates[] = {
                        "histogram_quantile(.5,sum by(job,le)(%s))",
                        "histogram_fraction(0,1,sum by(job,le)(%s))",
                        "histogram_quantiles(sum by(job,le)(%s),\"q\",.25,.75)"
                    };
                    char expression[160];
                    int length = snprintf(expression, sizeof(expression), templates[function],
                                          window ? "rate(m_total[3s])" : "m_total");
                    assert(length > 0 && (size_t)length < sizeof(expression));
                    netdata_promql_query_t *query = compile(engine, expression);
                    netdata_promql_request_t request = instant(3000);
                    netdata_promql_result_t *result = NULL;
                    netdata_promql_report_t *report = NULL;
                    assert(netdata_promql_collect(query, &input, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
                    released(&source.source);
                    netdata_promql_result_info_t info;
                    assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
                    assert(info.annotation_count == 0 && info.series_count == (function == 2 ? 4u : 2u));
                    unsigned seen = 0;
                    for (uint64_t row = 0; row < info.series_count; row++) {
                        netdata_promql_series_info_t series;
                        assert(netdata_promql_result_series_info(result, row, &series) == NETDATA_PROMQL_OK);
                        assert(series.point_count == 1 && series.label_count == (function == 2 ? 2u : 1u));
                        netdata_promql_label_t job;
                        assert(netdata_promql_result_label(result, row, 0, &job) == NETDATA_PROMQL_OK);
                        assert(job.name.len == 3 && !memcmp(job.name.data, "job", 3));
                        assert(job.value.len == 1 && (job.value.data[0] == 'a' || job.value.data[0] == 'b'));
                        unsigned ordinal = (unsigned)(job.value.data[0] - 'a');
                        double value = function == 0 ? .5 : 1;
                        if (function == 2) {
                            netdata_promql_label_t quantile;
                            assert(netdata_promql_result_label(result, row, 1, &quantile) == NETDATA_PROMQL_OK);
                            assert(quantile.name.len == 1 && quantile.name.data[0] == 'q');
                            assert(quantile.value.len == 4);
                            unsigned upper = !memcmp(quantile.value.data, "0.75", 4);
                            assert(upper || !memcmp(quantile.value.data, "0.25", 4));
                            value = upper ? .75 : .25;
                            ordinal = ordinal * 2 + upper;
                        }
                        assert(!(seen & (1u << ordinal)));
                        seen |= 1u << ordinal;
                        netdata_promql_point_info_t point;
                        assert(netdata_promql_result_point(result, row, 0, &point) == NETDATA_PROMQL_OK);
                        assert(point.time_ms == 3000 && point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
                        assert(point.value_bits == netdata_promql_double_bits(value));
                    }
                    assert(seen == (function == 2 ? 15u : 3u));
                    netdata_promql_query_observations_t observed;
                    assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
                    statistics_valid(&observed.statistics);
                    assert(observed.statistics.samples.total_queryable_samples == (window ? 12u : 4u));
                    printf("{\"function\":%u,\"window\":%u,\"delayed\":%u,\"page\":%" PRIu64
                           ",\"work\":%" PRIu64 ",\"fetched\":%" PRIu64 ",\"statistics\":[%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]}\n",
                           function, window, delayed, pages[page], observed.work, observed.samples_fetched,
                           observed.statistics.samples.total_queryable_samples, observed.statistics.samples.samples_read,
                           observed.statistics.samples.peak_samples);
                    assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
                    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
                    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
                    balanced(engine, &idle);
                }
            }
        }
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    return 0;
}
