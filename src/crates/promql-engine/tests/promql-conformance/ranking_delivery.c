/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Direct-selector ranking delivery through the default C library. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

static uint32_t ranking_chunk(void *context, const netdata_promql_chunk_t *chunk,
                              const netdata_promql_callback_control_t *control,
                              netdata_promql_bytes_t *error) {
    join_delivery *d = (join_delivery *)context;
    uint32_t position = d->output.calls[1];
    assert(chunk->kind == NETDATA_PROMQL_CHUNK_SERIES && chunk->reserved == 0);
    const netdata_promql_series_chunk_t *series = &chunk->payload.series;
    assert(series->series_id == position % 4 && series->labels.len == 1 && series->points.len == 1);
    assert(series->labels.data[0].name.len == 4);
    assert(memcmp(series->labels.data[0].name.data, "zone", 4) == 0);
    assert(series->labels.data[0].value.len == 1);
    assert(series->labels.data[0].value.data[0] == '0' + position % 4);
    const netdata_promql_sample_t *point = &series->points.data[0];
    assert(point->time_ms == (position < 4 ? 1000 : 2000));
    assert(point->value.kind == NETDATA_PROMQL_VALUE_FLOAT);
    assert(point->value.payload.float_bits == netdata_promql_double_bits(position < 4 ? 1 : 2));
    uint32_t status = output_status(&d->output, 2, control, error);
    if (d->arm_after && d->output.calls[1] == d->arm_after)
        d->input->source.failure = 6;
    return status;
}

static void ranking_case(netdata_promql_engine_t *engine, const char *expression,
                         uint64_t page, unsigned delayed, unsigned arm) {
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    join_fixture source;
    memset(&source, 0, sizeof(source));
    init_fixture(&source.source, 2, page);
    static const uint8_t zones[] = {'0', '1', '2', '3'};
    for (unsigned row = 0; row < 4; row++) {
        source.labels[row][0] = source.source.labels[0];
        source.labels[row][1].name = text("zone");
        source.labels[row][1].value = bytes(&zones[row], 1);
    }
    netdata_promql_provider_t input = provider(&source.source);
    input.context = &source;
    input.selection_next = join_selection_next;
    input.cursor_open = join_cursor_open;
    input.cursor_next = join_cursor_next;
    netdata_promql_query_t *query = compile(engine, expression);
    netdata_promql_request_t request = instant(1000);
    request.kind = NETDATA_PROMQL_REQUEST_RANGE;
    request.end_ms = 2000;
    request.step_ms = 1000;
    join_delivery output;
    memset(&output, 0, sizeof(output));
    output.input = &source;
    output.query_kind = FOUR_SERIES_FLOAT_MAP;
    output.arm_after = arm == 1 ? 1 : arm == 2 ? 2 : arm == 3 ? 4 : 0;
    output.output.failure = arm == 4 ? 2 : 0;
    if (arm == 5) {
        output.output.cancel_at = 2;
        assert(netdata_promql_control_create(engine, NULL, &output.output.control) == NETDATA_PROMQL_OK);
    }
    netdata_promql_output_t table = delivery_table(&output.output);
    table.context = &output;
    table.begin = join_begin;
    table.chunk = ranking_chunk;
    table.commit = join_commit;
    netdata_promql_result_t *result = NULL;
    netdata_promql_report_t *report = NULL;
    uint32_t status = arm == 6 ?
        netdata_promql_collect(query, &input, &request, NULL, &result, &report) :
        netdata_promql_execute(query, &input, &request, output.output.control, &table, &report);
    uint32_t expected = arm == 0 || arm == 6 ? NETDATA_PROMQL_OK :
                        arm < 4 ? NETDATA_PROMQL_PROVIDER_ERROR :
                        arm == 4 ? NETDATA_PROMQL_OUTPUT_ERROR : NETDATA_PROMQL_CANCELLED;
    assert(status == expected && report);
    released(&source.source);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    if (arm == 6) {
        assert(result);
        memset(source.labels, 0, sizeof(source.labels));
        netdata_promql_result_info_t info;
        assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
        assert(info.header.value_type == NETDATA_PROMQL_TYPE_MATRIX);
        assert(info.series_count == 4 && info.annotation_count == 0);
        for (uint64_t row = 0; row < 4; row++) {
            netdata_promql_series_info_t series;
            assert(netdata_promql_result_series_info(result, row, &series) == NETDATA_PROMQL_OK);
            assert(series.label_count == 1 && series.point_count == 2);
            netdata_promql_label_t label;
            assert(netdata_promql_result_label(result, row, 0, &label) == NETDATA_PROMQL_OK);
            assert(label.name.len == 4 && memcmp(label.name.data, "zone", 4) == 0);
            assert(label.value.len == 1 && label.value.data[0] == '0' + row);
            for (uint64_t position = 0; position < 2; position++) {
                netdata_promql_point_info_t point;
                assert(netdata_promql_result_point(result, row, position, &point) == NETDATA_PROMQL_OK);
                assert(point.time_ms == (int64_t)(position + 1) * 1000);
                assert(point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
                assert(point.value_bits == netdata_promql_double_bits((double)position + 1));
            }
        }
        assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
    } else {
        assert(!result && output.output.calls[0] == 1);
        unsigned chunks = arm == 0 ? 8 : arm == 2 ? 2 : arm == 3 ? 4 : 1;
        assert(output.output.calls[1] == chunks);
        assert(output.output.calls[2] == (arm == 0) && output.output.calls[3] == (arm != 0));
    }
    netdata_promql_query_observations_t observed;
    assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    statistics_valid(&observed.statistics);
    if (expected == NETDATA_PROMQL_OK) {
        assert(observed.output_samples == 8);
        assert(observed.statistics.samples.total_queryable_samples == 8);
    }
    printf("{\"ranking_delivery\":\"pass\",\"expression\":");
    print_json_string(expression);
    printf(",\"page\":%" PRIu64 ",\"delayed\":%u,\"arm\":%u,\"status\":%u,\"output_samples\":%" PRIu64
           ",\"work\":%" PRIu64 ",\"fetched\":%" PRIu64 ",\"memory\":[",
           page, delayed, arm, status, observed.output_samples, observed.work, observed.samples_fetched);
    for (unsigned kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++) {
        netdata_promql_memory_observation_t memory = observed.memory[kind];
        printf("%s[%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]", kind ? "," : "",
               memory.current, memory.peak, memory.acquired, memory.released);
    }
    puts("]}");
    if (output.output.control)
        assert(netdata_promql_control_release(output.output.control) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    balanced(engine, &idle);
}

typedef struct {
    join_fixture input;
    unsigned negative;
    uint64_t second_output_cursor;
} subset_fixture;

static uint32_t subset_cursor_next(void *context, void *handle,
                                   const netdata_promql_callback_control_t *control,
                                   netdata_promql_page_t *out, netdata_promql_resources_t *resources,
                                   netdata_promql_bytes_t *error) {
    subset_fixture *f = (subset_fixture *)context;
    assert(f->input.latest_cursor == handle);
    assert(f->input.source.opened[3] == f->input.source.closed[3] + 1);
    uint32_t status = join_cursor_next(&f->input, handle, control, out, resources, error);
    if (status != NETDATA_PROMQL_OK) return status;
    cursor *c = (cursor *)handle;
    assert(out->samples.data == c->page);
    for (uint64_t i = 0; i < out->samples.len; i++) {
        double value = 10.0 * f->input.latest_series + (double)(c->page[i].time_ms / 1000);
        c->page[i].value.payload.float_bits = netdata_promql_double_bits(f->negative ? -value : value);
    }
    return status;
}

static uint32_t subset_chunk(void *context, const netdata_promql_chunk_t *chunk,
                              const netdata_promql_callback_control_t *control,
                              netdata_promql_bytes_t *error) {
    join_delivery *d = (join_delivery *)context;
    subset_fixture *f = (subset_fixture *)d->input;
    unsigned position = d->output.calls[1];
    assert(chunk->kind == NETDATA_PROMQL_CHUNK_SERIES && chunk->reserved == 0);
    const netdata_promql_series_chunk_t *series = &chunk->payload.series;
    assert(series->series_id == position % 2 && series->labels.len == 1 && series->points.len == 1);
    assert(series->labels.data[0].name.len == 4);
    assert(memcmp(series->labels.data[0].name.data, "zone", 4) == 0);
    assert(series->labels.data[0].value.len == 1);
    assert(series->labels.data[0].value.data[0] == '2' + position % 2);
    const netdata_promql_sample_t *point = &series->points.data[0];
    unsigned tick = position / 2 + 1;
    assert(point->time_ms == (int64_t)tick * 1000);
    assert(point->value.kind == NETDATA_PROMQL_VALUE_FLOAT);
    assert(point->value.payload.float_bits == netdata_promql_double_bits(10 * (position % 2 + 2) + tick));
    if (position == 1) f->second_output_cursor = d->input->source.opened[3];
    uint32_t status = output_status(&d->output, 2, control, error);
    if (d->arm_after && d->output.calls[1] == d->arm_after) d->input->source.failure = 6;
    return status;
}

static uint32_t subset_commit(void *context, const netdata_promql_query_observations_t *observations,
                               const netdata_promql_callback_control_t *control,
                               netdata_promql_bytes_t *error) {
    join_delivery *d = (join_delivery *)context;
    assert(!d->arm_after && d->output.calls[1] == 4 && observations->output_samples == 4);
    assert(observations->statistics.samples.total_queryable_samples == 8);
    assert(observations->memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    statistics_valid(&observations->statistics);
    released(&d->input->source);
    return output_status(&d->output, 3, control, error);
}

static uint64_t subset_case(netdata_promql_engine_t *engine, const char *expression,
                            uint64_t page, unsigned delayed, unsigned negative,
                            unsigned arm, uint64_t warm_cursor) {
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    subset_fixture source;
    memset(&source, 0, sizeof(source));
    init_fixture(&source.input.source, 2, page);
    source.negative = negative;
    static const uint8_t zones[] = {'0', '1', '2', '3'};
    for (unsigned row = 0; row < 4; row++) {
        source.input.labels[row][0] = source.input.source.labels[0];
        source.input.labels[row][1].name = text("zone");
        source.input.labels[row][1].value = bytes(&zones[row], 1);
    }
    if (arm == 2) { assert(warm_cursor); source.input.fail_on_cursor = warm_cursor; }
    netdata_promql_provider_t input = provider(&source.input.source);
    input.context = &source;
    input.selection_next = join_selection_next;
    input.cursor_open = join_cursor_open;
    input.cursor_next = subset_cursor_next;
    netdata_promql_query_t *query = compile(engine, expression);
    netdata_promql_request_t request = instant(1000);
    request.kind = NETDATA_PROMQL_REQUEST_RANGE;
    request.end_ms = 2000;
    request.step_ms = 1000;
    join_delivery output;
    memset(&output, 0, sizeof(output));
    output.input = &source.input;
    output.query_kind = FOUR_SERIES_FLOAT_MAP;
    output.arm_after = arm == 1 ? 1 : arm == 3 ? 2 : 0;
    output.output.failure = arm == 4 ? 2 : 0;
    if (arm == 5) {
        output.output.cancel_at = 2;
        assert(netdata_promql_control_create(engine, NULL, &output.output.control) == NETDATA_PROMQL_OK);
    }
    netdata_promql_output_t table = delivery_table(&output.output);
    table.context = &output;
    table.begin = join_begin;
    table.chunk = subset_chunk;
    table.commit = subset_commit;
    netdata_promql_result_t *result = NULL;
    netdata_promql_report_t *report = NULL;
    uint32_t status = arm == 6 ?
        netdata_promql_collect(query, &input, &request, NULL, &result, &report) :
        netdata_promql_execute(query, &input, &request, output.output.control, &table, &report);
    uint32_t expected = arm == 0 || arm == 6 ? NETDATA_PROMQL_OK :
                        arm < 4 ? NETDATA_PROMQL_PROVIDER_ERROR :
                        arm == 4 ? NETDATA_PROMQL_OUTPUT_ERROR : NETDATA_PROMQL_CANCELLED;
    assert(status == expected && report);
    if (arm == 1) assert(source.input.latest_series == 3);
    if (arm == 2) assert(source.input.failed_cursor == warm_cursor && source.input.latest_series == 3);
    if (arm == 3) assert(source.input.latest_series == 0);
    released(&source.input.source);
    assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
    if (arm == 6) {
        assert(result);
        memset(source.input.labels, 0, sizeof(source.input.labels));
        netdata_promql_result_info_t info;
        assert(netdata_promql_result_info(result, &info) == NETDATA_PROMQL_OK);
        assert(info.header.value_type == NETDATA_PROMQL_TYPE_MATRIX);
        assert(info.series_count == 2 && info.annotation_count == 0);
        for (uint64_t row = 0; row < 2; row++) {
            netdata_promql_series_info_t series;
            assert(netdata_promql_result_series_info(result, row, &series) == NETDATA_PROMQL_OK);
            assert(series.label_count == 1 && series.point_count == 2);
            netdata_promql_label_t label;
            assert(netdata_promql_result_label(result, row, 0, &label) == NETDATA_PROMQL_OK);
            assert(label.name.len == 4 && memcmp(label.name.data, "zone", 4) == 0);
            assert(label.value.len == 1 && label.value.data[0] == '2' + row);
            for (uint64_t position = 0; position < 2; position++) {
                netdata_promql_point_info_t point;
                assert(netdata_promql_result_point(result, row, position, &point) == NETDATA_PROMQL_OK);
                assert(point.time_ms == (int64_t)(position + 1) * 1000);
                assert(point.value_kind == NETDATA_PROMQL_VALUE_FLOAT);
                assert(point.value_bits == netdata_promql_double_bits(10 * (row + 2) + position + 1));
            }
        }
        assert(netdata_promql_result_release(result) == NETDATA_PROMQL_OK);
    } else {
        assert(!result && output.output.calls[0] == 1);
        unsigned chunks = arm == 0 ? 4 : arm == 3 ? 2 : 1;
        assert(output.output.calls[1] == chunks);
        assert(output.output.calls[2] == (arm == 0) && output.output.calls[3] == (arm != 0));
    }
    netdata_promql_query_observations_t observed;
    assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    assert(observed.samples_fetched == source.input.source.samples_returned);
    assert(observed.pages == source.input.source.pages_returned);
    statistics_valid(&observed.statistics);
    if (expected == NETDATA_PROMQL_OK) {
        assert(observed.output_samples == 4);
        assert(observed.statistics.samples.total_queryable_samples == 8);
    }
    printf("{\"ranking_delivery\":\"pass\",\"expression\":");
    print_json_string(expression);
    printf(",\"page\":%" PRIu64 ",\"delayed\":%u,\"arm\":%u,\"status\":%u,\"output_samples\":%" PRIu64
           ",\"work\":%" PRIu64 ",\"fetched\":%" PRIu64 ",\"memory\":[",
           page, delayed, arm, status, observed.output_samples, observed.work, observed.samples_fetched);
    for (unsigned kind = 0; kind < NETDATA_PROMQL_MEMORY_KINDS; kind++) {
        netdata_promql_memory_observation_t memory = observed.memory[kind];
        printf("%s[%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 "]", kind ? "," : "",
               memory.current, memory.peak, memory.acquired, memory.released);
    }
    puts("]}");
    if (output.output.control) assert(netdata_promql_control_release(output.output.control) == NETDATA_PROMQL_OK);
    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
    balanced(engine, &idle);
    return source.second_output_cursor;
}

int main(void) {
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        const char *expressions[] = {"abs(topk by(zone)(1,m))", "abs(bottomk by(zone)(1,m))"};
        const uint64_t pages[] = {1, 17, 1024};
        for (unsigned shape = 0; shape < 2; shape++)
            for (unsigned page = 0; page < 3; page++)
                for (unsigned arm = 0; arm < 7; arm++)
                    ranking_case(engine, expressions[shape], pages[page], delayed, arm);
        const char *subsets[] = {"abs(topk(2,m))", "abs(bottomk(2,m))"};
        for (unsigned shape = 0; shape < 2; shape++)
            for (unsigned page = 0; page < 3; page++) {
                uint64_t warm_cursor = subset_case(engine, subsets[shape], pages[page], delayed, shape, 0, 0);
                assert(warm_cursor);
                for (unsigned arm = 1; arm < 7; arm++)
                    subset_case(engine, subsets[shape], pages[page], delayed, shape, arm, warm_cursor);
            }
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    return 0;
}
