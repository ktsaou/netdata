/* Generated paged inputs; deadline-enabled label comparison workload. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main
#include <time.h>

typedef struct {
    fixture input;
    netdata_promql_label_t labels[128][17];
    unsigned series;
} label_fixture;

static uint32_t label_next(void *context, void *handle,
                           const netdata_promql_callback_control_t *control,
                           netdata_promql_series_open_t *out,
                           netdata_promql_provider_diagnostics_t *diagnostics,
                           netdata_promql_bytes_t *error) {
    label_fixture *f = (label_fixture *)context;
    (void)diagnostics;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    status = returned(&f->input, 3, error);
    if (status != NETDATA_PROMQL_OK) return status;
    uint8_t *position = (uint8_t *)handle;
    if (*position == f->series) return NETDATA_PROMQL_END;
    f->input.opened[2]++;
    out->handle = (void *)(uintptr_t)(*position + 1);
    out->labels.data = f->labels[(*position)++];
    out->labels.len = 17;
    return NETDATA_PROMQL_OK;
}

static uint64_t clock_ns(void) {
    struct timespec now;
    assert(clock_gettime(CLOCK_MONOTONIC, &now) == 0);
    return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

int main(int argc, char **argv) {
    assert(argc == 7);
    unsigned series = (unsigned)strtoul(argv[1], NULL, 10);
    unsigned width = (unsigned)strtoul(argv[2], NULL, 10);
    unsigned deadline = (unsigned)strtoul(argv[3], NULL, 10);
    unsigned operation = (unsigned)strtoul(argv[4], NULL, 10);
    unsigned repeats = (unsigned)strtoul(argv[5], NULL, 10);
    unsigned page = (unsigned)strtoul(argv[6], NULL, 10);
    assert(series > 0 && series <= 128 && width >= 16 && width <= 4097);
    assert(deadline <= 1 && operation < 4 && repeats > 0 && repeats <= 20);
    assert(page == 1 || page == 17 || page == 1024);
    const char *expressions[] = {
        "m", "m and on(zone) m", "m + on(zone) m",
        "count by(zone)(count_over_time(m[3s])) and on(zone) count by(zone)(m)"
    };
    uint8_t *values = (uint8_t *)malloc((size_t)series * width);
    assert(values);
    memset(values, 'x', (size_t)series * width);
    for (unsigned i = 0; i < series; i++) {
        values[(size_t)(i + 1) * width - 3] = '0' + i / 100;
        values[(size_t)(i + 1) * width - 2] = '0' + i / 10 % 10;
        values[(size_t)(i + 1) * width - 1] = '0' + i % 10;
    }
    char names[15][4];
    for (unsigned i = 0; i < 15; i++) assert(snprintf(names[i], sizeof(names[i]), "d%02u", i) == 3);
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(NULL, &engine) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    netdata_promql_query_t *query = compile(engine, expressions[operation]);
    netdata_promql_engine_observations_t compiled = engine_observations(engine);
    for (unsigned repeat = 0; repeat < repeats; repeat++) {
        label_fixture f;
        memset(&f, 0, sizeof(f));
        init_fixture(&f.input, 32, page);
        f.series = series;
        for (unsigned i = 0; i < series; i++) {
            f.labels[i][0] = f.input.labels[0];
            for (unsigned j = 0; j < 15; j++) {
                f.labels[i][j + 1].name = text(names[j]);
                f.labels[i][j + 1].value = bytes(values, width);
            }
            f.labels[i][16].name = text("zone");
            f.labels[i][16].value = bytes(values + (size_t)i * width, width);
        }
        netdata_promql_provider_t input = provider(&f.input);
        input.context = &f;
        input.selection_next = label_next;
        netdata_promql_control_options_t options;
        assert(netdata_promql_control_options_default(&options) == NETDATA_PROMQL_OK);
        options.deadline_present = deadline;
        options.seconds = 3600;
        assert(netdata_promql_control_create(engine, &options, &f.input.control) == NETDATA_PROMQL_OK);
        netdata_promql_request_t request = instant(2000);
        request.kind = NETDATA_PROMQL_REQUEST_RANGE;
        request.end_ms = 32000;
        request.step_ms = 1000;
        netdata_promql_result_t *result = NULL;
        netdata_promql_report_t *report = NULL;
        uint64_t start = clock_ns();
        uint32_t status = netdata_promql_collect(query, &input, &request, f.input.control, &result, &report);
        uint64_t elapsed = clock_ns() - start;
        assert(status == NETDATA_PROMQL_OK && result && report);
        released(&f.input);
        netdata_promql_result_info_t info;
        assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
        assert(info.header.value_type == NETDATA_PROMQL_TYPE_MATRIX && info.series_count == series);
        assert(!info.annotation_count && !info.scalar_present && !info.string_present);
        for (unsigned i = 0; i < series; i++) {
            netdata_promql_series_info_t row;
            assert(netdata_promql_result_series_info(result, i, &row) == NETDATA_PROMQL_OK);
            unsigned label_count = operation >= 2 ? 1 : 17;
            assert(row.point_count == 31 && row.label_count == label_count);
            for (unsigned j = 0; j < label_count; j++) {
                netdata_promql_label_t label;
                assert(netdata_promql_result_label(result, i, j, &label) == NETDATA_PROMQL_OK);
                label_copy_equal(label, f.labels[i][operation >= 2 ? 16 : j]);
            }
            for (unsigned j = 0; j < 31; j++) {
                netdata_promql_point_info_t point;
                assert(netdata_promql_result_point(result, i, j, &point) == NETDATA_PROMQL_OK);
                assert(point.time_ms == ((int64_t)j + 2) * 1000 && point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
                double value = operation == 3 ? 1.0 : (double)(j + 2) * (operation == 2 ? 2.0 : 1.0);
                assert(point.value_bits == netdata_promql_double_bits(value));
            }
        }
        netdata_promql_query_observations_t observed;
        assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
        statistics_valid(&observed.statistics);
        assert(!observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current);
        assert(observed.samples_fetched == f.input.samples_returned);
        printf("{\"series\":%u,\"width\":%u,\"deadline\":%u,\"op\":%u,\"repeat\":%u,\"page\":%u,\"ns\":%" PRIu64
               ",\"work\":%" PRIu64 ",\"fetched\":%" PRIu64 ",\"calls\":%" PRIu64 ",\"memory\":[",
               series, width, deadline, operation, repeat, page, elapsed, observed.work, observed.samples_fetched,
               observed.provider_calls);
        for (unsigned i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++) {
            netdata_promql_memory_observation_t m = observed.memory[i];
            printf("%s[%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]", i ? "," : "", m.current, m.peak, m.acquired, m.released);
        }
        puts("]}");
        assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
        assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
        assert(netdata_promql_control_release(f.input.control) == NETDATA_PROMQL_OK);
        balanced(engine, &compiled);
    }
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    balanced(engine, &idle);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    free(values);
    return 0;
}
