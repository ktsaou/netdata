/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Generate adjacent values through the established one-page C provider. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

static uint32_t adjacent_next(void *context, void *handle,
                              const netdata_promql_callback_control_t *control,
                              netdata_promql_page_t *out, netdata_promql_resources_t *resources,
                              netdata_promql_bytes_t *error) {
    fixture *f = (fixture *)context;
    assert(f->opened[3] == f->closed[3] + 1);
    uint32_t status = cursor_next(context, handle, control, out, resources, error);
    if (status != NETDATA_PROMQL_OK) return status;
    cursor *c = (cursor *)handle;
    assert(out->samples.len <= f->maximum);
    for (uint64_t index = 0; index < out->samples.len; index++) {
        netdata_promql_sample_t *sample = &c->page[index];
        uint64_t ordinal = (uint64_t)(sample->time_ms / 1000 - 1);
        sample->value.payload.float_bits = UINT64_C(0x3ff0000000000000) + ordinal % 2;
    }
    return NETDATA_PROMQL_OK;
}

static netdata_promql_query_observations_t adjacent_window(netdata_promql_engine_t *engine,
                                                          uint64_t rows, uint64_t maximum,
                                                          unsigned computed) {
    char expression[128];
    int length = snprintf(expression, sizeof(expression), computed ?
                          "quantile_over_time(.5,(m*1)[%" PRIu64 "s:1s])" :
                          "quantile_over_time(.5,m[%" PRIu64 "s])", rows);
    assert(length > 0 && (size_t)length < sizeof(expression));
    netdata_promql_query_t *query = compile(engine, expression);
    fixture f;
    init_fixture(&f, rows, maximum);
    netdata_promql_provider_t p = provider(&f);
    p.cursor_next = adjacent_next;
    netdata_promql_request_t request = instant((int64_t)rows * 1000);
    netdata_promql_result_t *result = NULL;
    netdata_promql_report_t *report = NULL;
    assert(netdata_promql_collect(query, &p, &request, NULL, &result, &report) == NETDATA_PROMQL_OK);
    released(&f);
    netdata_promql_result_info_t info;
    assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
    assert(info.header.value_type == NETDATA_PROMQL_TYPE_VECTOR);
    assert(info.series_count == 1 && info.annotation_count == 0);
    netdata_promql_series_info_t series;
    assert(netdata_promql_result_series_info(result, 0, &series) == NETDATA_PROMQL_OK);
    assert(series.label_count == 1 && series.point_count == 1);
    netdata_promql_label_t label;
    assert(netdata_promql_result_label(result, 0, 0, &label) == NETDATA_PROMQL_OK);
    assert(label.name.len == f.labels[1].name.len && label.value.len == f.labels[1].value.len);
    assert(memcmp(label.name.data, f.labels[1].name.data, label.name.len) == 0);
    assert(memcmp(label.value.data, f.labels[1].value.data, label.value.len) == 0);
    netdata_promql_point_info_t point;
    assert(netdata_promql_result_point(result, 0, 0, &point) == NETDATA_PROMQL_OK);
    assert(point.time_ms == (int64_t)rows * 1000 && point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
    assert(point.value_bits == UINT64_C(0x3ff0000000000000));
    netdata_promql_query_observations_t observed;
    assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
    statistics_valid(&observed.statistics);
    assert(observed.statistics.samples.total_queryable_samples == rows && observed.output_samples == 1);
    assert(observed.statistics.samples.samples_read <= observed.samples_fetched);
    assert(observed.samples_fetched == f.samples_returned && observed.pages == f.pages_returned);
    assert(observed.temporary_storage.extent.peak == 0 && observed.temporary_storage.stores.peak == 0);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current == 0);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].peak == maximum * sizeof(netdata_promql_sample_t));
    assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    printf("{\"adjacent_points\":%" PRIu64 ",\"computed\":%u,\"page_size\":%" PRIu64
           ",\"fetched\":%" PRIu64 ",\"pages\":%" PRIu64 ",\"cursors\":%" PRIu64
           ",\"work\":%" PRIu64 ",\"logical_samples\":%" PRIu64 ",\"sample_peak\":%" PRIu64 ",\"memory\":[",
           rows, computed, maximum, observed.samples_fetched, observed.pages, f.opened[3], observed.work,
           observed.statistics.samples.total_queryable_samples, observed.statistics.samples.peak_samples);
    for (unsigned kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++) {
        netdata_promql_memory_observation_t memory = observed.memory[kind];
        printf("%s{\"current\":%" PRIu64 ",\"peak\":%" PRIu64 ",\"acquired\":%" PRIu64
               ",\"released\":%" PRIu64 ",\"overflowed\":%u}", kind ? "," : "", memory.current,
               memory.peak, memory.acquired, memory.released, (unsigned)memory.overflowed);
    }
    puts("]}");
    return observed;
}

int main(void) {
    netdata_promql_engine_options_t options;
    assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
    options.lookback_ms = 1;
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    const uint64_t pages[] = {1, 17, 1024};
    for (unsigned computed = 0; computed < 2; computed++) {
        for (unsigned page = 0; page < 3; page++) {
            netdata_promql_query_observations_t first = adjacent_window(engine, 2048, pages[page], computed);
            balanced(engine, &idle);
            netdata_promql_query_observations_t second = adjacent_window(engine, 100000, pages[page], computed);
            balanced(engine, &idle);
            for (unsigned kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++) {
                assert(first.memory[kind].peak == second.memory[kind].peak);
                assert(!first.memory[kind].overflowed && !second.memory[kind].overflowed);
            }
            assert(first.statistics.samples.peak_samples == second.statistics.samples.peak_samples);
        }
    }
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    puts("{\"rank_prefix_embedding\":\"pass\",\"generated_queries\":12}");
    return 0;
}
