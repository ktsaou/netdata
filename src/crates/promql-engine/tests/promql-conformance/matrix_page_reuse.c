/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Generated counters exercise sparse and overlapping range windows. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main
#include <time.h>

static uint64_t clock_ns(void) {
    struct timespec now;
    assert(clock_gettime(CLOCK_MONOTONIC, &now) == 0);
    return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

int main(int argc, char **argv) {
    assert(argc == 7);
    unsigned count = (unsigned)strtoul(argv[1], NULL, 10);
    unsigned page = (unsigned)strtoul(argv[2], NULL, 10);
    unsigned step = (unsigned)strtoul(argv[3], NULL, 10);
    unsigned operation = (unsigned)strtoul(argv[4], NULL, 10);
    unsigned repeats = (unsigned)strtoul(argv[5], NULL, 10);
    unsigned deadline = (unsigned)strtoul(argv[6], NULL, 10);
    assert(count >= 32 && count <= 65537 && (page == 1 || page == 17 || page == 1024));
    assert((step == 1 || step == 10 || step == 60) && operation < 7 && repeats && repeats <= 20 && deadline <= 1);
    const char *expressions[] = {"m_total", "rate(m_total[2s])", "sum_over_time(m_total[2s])",
                                  "avg_over_time(m_total[2s])", "count_over_time(m_total[2s])",
                                  "rate(m_total[2s]) / rate(m_total[10s])",
                                  "max_over_time(rate(m_total[2s])[20s:10s])"};
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(NULL, &engine) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    netdata_promql_query_t *query = compile(engine, expressions[operation]);
    netdata_promql_engine_observations_t compiled = engine_observations(engine);
    for (unsigned repeat = 0; repeat < repeats; repeat++) {
        fixture f;
        init_fixture(&f, count, page);
        f.labels[0].value = text("m_total");
        netdata_promql_provider_t input = provider(&f);
        netdata_promql_control_options_t options;
        assert(netdata_promql_control_options_default(&options) == NETDATA_PROMQL_OK);
        options.deadline_present = deadline;
        options.seconds = 3600;
        assert(netdata_promql_control_create(engine, &options, &f.control) == NETDATA_PROMQL_OK);
        unsigned first = operation == 6 ? 22 : operation == 5 ? 10 : 2;
        netdata_promql_request_t request = instant((int64_t)first * 1000);
        request.kind = NETDATA_PROMQL_REQUEST_RANGE;
        request.end_ms = (int64_t)count * 1000;
        request.step_ms = (int64_t)step * 1000;
        netdata_promql_result_t *result = NULL;
        netdata_promql_report_t *report = NULL;
        uint64_t start = clock_ns();
        uint32_t status = netdata_promql_collect(query, &input, &request, f.control, &result, &report);
        uint64_t elapsed = clock_ns() - start;
        assert(status == NETDATA_PROMQL_OK && result && report);
        released(&f);
        netdata_promql_result_info_t info;
        assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
        assert(info.header.value_type == NETDATA_PROMQL_TYPE_MATRIX && info.series_count == 1);
        assert(!info.annotation_count && !info.scalar_present && !info.string_present);
        netdata_promql_series_info_t row;
        assert(netdata_promql_result_series_info(result, 0, &row) == NETDATA_PROMQL_OK);
        assert(row.point_count == (count - first) / step + 1 && row.label_count == (operation ? 1u : 2u));
        for (unsigned j = 0; j < row.label_count; j++) {
            netdata_promql_label_t label;
            assert(netdata_promql_result_label(result, 0, j, &label) == NETDATA_PROMQL_OK);
            label_copy_equal(label, f.labels[operation ? 1 : j]);
        }
        for (unsigned j = 0; j < row.point_count; j++) {
            unsigned tick = first + j * step;
            netdata_promql_point_info_t point;
            assert(netdata_promql_result_point(result, 0, j, &point) == NETDATA_PROMQL_OK);
            assert(point.time_ms == (int64_t)tick * 1000 && point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
            double expected[] = {(double)tick, 1.0, 2.0 * tick - 1.0, (double)tick - 0.5, 2.0, 1.0, 1.0};
            assert(point.value_bits == netdata_promql_double_bits(expected[operation]));
        }
        netdata_promql_query_observations_t observed;
        assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
        statistics_valid(&observed.statistics);
        assert(observed.samples_fetched == f.samples_returned);
        printf("{\"count\":%u,\"page\":%u,\"step\":%u,\"op\":%u,\"repeat\":%u,\"deadline\":%u,\"ns\":%" PRIu64
               ",\"work\":%" PRIu64 ",\"fetched\":%" PRIu64 ",\"calls\":%" PRIu64 ",\"read\":%" PRIu64 ",\"memory\":[",
               count, page, step, operation, repeat, deadline, elapsed, observed.work,
               observed.samples_fetched, observed.provider_calls, observed.statistics.samples.samples_read);
        for (unsigned i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++) {
            netdata_promql_memory_observation_t m = observed.memory[i];
            printf("%s[%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]", i ? "," : "", m.current, m.peak, m.acquired, m.released);
        }
        puts("]}");
        assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        assert(netdata_promql_control_release(f.control) == NETDATA_PROMQL_OK);
        balanced(engine, &compiled);
    }
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    balanced(engine, &idle);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    return 0;
}
