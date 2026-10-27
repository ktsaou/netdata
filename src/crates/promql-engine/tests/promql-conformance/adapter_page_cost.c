/* Exercise actual C page conversion without retaining a complete input history. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main
#include <time.h>

static uint64_t adapter_clock(void) {
    struct timespec now;
    assert(clock_gettime(CLOCK_MONOTONIC, &now) == 0);
    return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

int main(int argc, char **argv) {
    assert(argc == 6);
    uint64_t rows = strtoull(argv[1], NULL, 10);
    uint64_t maximum = strtoull(argv[2], NULL, 10);
    unsigned deadline = (unsigned)strtoul(argv[4], NULL, 10);
    unsigned repeats = (unsigned)strtoul(argv[5], NULL, 10);
    assert((rows == 1025 || rows == 65537) && maximum && maximum <= 1024);
    assert(deadline <= 1 && repeats && repeats <= 100);
    double expected;
    if (strcmp(argv[3], "sum_over_time") == 0) expected = (double)(rows * (rows + 1) / 2);
    else if (strcmp(argv[3], "count_over_time") == 0) expected = (double)rows;
    else { assert(strcmp(argv[3], "rate") == 0); expected = 1.0; }
    char expression[100];
    int length = snprintf(expression, sizeof(expression), "%s(m[%" PRIu64 "s])", argv[3], rows);
    assert(length > 0 && (size_t)length < sizeof(expression));
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(NULL, &engine) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    netdata_promql_query_t *query = compile(engine, expression);
    netdata_promql_engine_observations_t compiled = engine_observations(engine);
    for (unsigned repeat = 0; repeat < repeats; repeat++) {
        fixture f;
        init_fixture(&f, rows, maximum);
        netdata_promql_provider_t input = provider(&f);
        netdata_promql_control_options_t options;
        assert(netdata_promql_control_options_default(&options) == NETDATA_PROMQL_OK);
        options.deadline_present = deadline;
        options.seconds = 3600;
        assert(netdata_promql_control_create(engine, &options, &f.control) == NETDATA_PROMQL_OK);
        netdata_promql_request_t request = instant((int64_t)rows * 1000);
        netdata_promql_result_t *result = NULL;
        netdata_promql_report_t *report = NULL;
        uint64_t start = adapter_clock();
        uint32_t status = netdata_promql_collect(query, &input, &request, f.control, &result, &report);
        uint64_t elapsed = adapter_clock() - start;
        assert(status == NETDATA_PROMQL_OK && result && report);
        released(&f);
        netdata_promql_query_observations_t observed;
        assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
        statistics_valid(&observed.statistics);
        assert(observed.statistics.samples.total_queryable_samples == rows);
        assert(observed.samples_fetched == f.samples_returned && observed.pages == f.pages_returned);
        assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
        assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].peak == maximum * sizeof(netdata_promql_sample_t));
        assert(observed.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current == 0);
        netdata_promql_result_info_t info;
        netdata_promql_series_info_t series;
        netdata_promql_point_info_t point;
        netdata_promql_label_t label;
        assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
        assert(info.header.value_type == NETDATA_PROMQL_TYPE_VECTOR && info.series_count == 1);
        assert(!info.scalar_present && !info.string_present);
        assert(netdata_promql_result_series_info(result, 0, &series) == NETDATA_PROMQL_OK);
        assert(series.point_count == 1 && series.label_count == 1);
        assert(netdata_promql_result_label(result, 0, 0, &label) == NETDATA_PROMQL_OK);
        label_copy_equal(label, f.labels[1]);
        assert(netdata_promql_result_point(result, 0, 0, &point) == NETDATA_PROMQL_OK);
        assert(point.time_ms == request.start_ms && point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
        assert(point.value_bits == netdata_promql_double_bits(expected));
        printf("{\"function\":\"%s\",\"rows\":%" PRIu64 ",\"page\":%" PRIu64
               ",\"deadline\":%u,\"repeat\":%u,\"ns\":%" PRIu64 ",\"bits\":\"%016" PRIx64
               "\",\"annotations\":%" PRIu64 ",\"calls\":%" PRIu64 ",\"pages\":%" PRIu64
               ",\"fetched\":%" PRIu64 ",\"work\":%" PRIu64 ",\"output\":%" PRIu64
               ",\"statistics\":[%" PRIu64 ",%" PRIu64 ",%" PRIu64 "],\"memory\":[",
               argv[3], rows, maximum, deadline, repeat, elapsed, point.value_bits, info.annotation_count,
               observed.provider_calls, observed.pages, observed.samples_fetched, observed.work, observed.output_samples,
               observed.statistics.samples.total_queryable_samples, observed.statistics.samples.samples_read,
               observed.statistics.samples.peak_samples);
        for (unsigned kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++) {
            netdata_promql_memory_observation_t value = observed.memory[kind];
            assert(!value.overflowed && value.acquired - value.released == value.current);
            printf("%s[%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]", kind ? "," : "",
                   value.current, value.peak, value.acquired, value.released);
        }
        puts("]}");
        assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        assert(netdata_promql_control_release(f.control) == NETDATA_PROMQL_OK);
        balanced(engine, &compiled);
    }
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    error_phases(engine);
    balanced(engine, &idle);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    return 0;
}
