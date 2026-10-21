/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Exercise consumed histogram discovery through the paged C provider. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

typedef struct {
    fixture source;
    netdata_promql_label_t labels[8][4];
    uint64_t fail_cursor, failed_cursor;
} discovery_source;

static uint32_t discovery_selection(void *context, void *handle,
                                   const netdata_promql_callback_control_t *control,
                                   netdata_promql_series_open_t *out,
                                   netdata_promql_provider_diagnostics_t *diagnostics,
                                   netdata_promql_bytes_t *error) {
    discovery_source *f = (discovery_source *)context;
    (void)diagnostics;
    uint32_t status = checked_control(control);
    if (status != NETDATA_PROMQL_OK) return status;
    status = returned(&f->source, 3, error);
    if (status != NETDATA_PROMQL_OK) return status;
    uint8_t *position = (uint8_t *)handle;
    if (*position == 8) return NETDATA_PROMQL_END;
    out->handle = (void *)(uintptr_t)(*position + 1);
    out->labels.data = f->labels[(*position)++];
    out->labels.len = 4;
    f->source.opened[2]++;
    return NETDATA_PROMQL_OK;
}

static uint32_t discovery_next(void *context, void *handle,
                              const netdata_promql_callback_control_t *control,
                              netdata_promql_page_t *out, netdata_promql_resources_t *resources,
                              netdata_promql_bytes_t *error) {
    discovery_source *f = (discovery_source *)context;
    if (f->fail_cursor && f->source.opened[3] == f->fail_cursor) {
        f->failed_cursor = f->fail_cursor;
        f->source.failure = 6;
    }
    return cursor_next(&f->source, handle, control, out, resources, error);
}

typedef struct {
    delivery output;
    discovery_source *source;
    uint64_t first_cursors;
} discovery_delivery;

static uint32_t discovery_begin(void *context, const netdata_promql_output_header_t *header,
                               const netdata_promql_callback_control_t *control,
                               netdata_promql_bytes_t *error) {
    discovery_delivery *d = (discovery_delivery *)context;
    assert(header->value_type == NETDATA_PROMQL_TYPE_MATRIX);
    assert(header->start_ms == 4000 && header->end_ms == 5000 && header->step_ms == 1000);
    return output_status(&d->output, 1, control, error);
}

static uint32_t discovery_chunk(void *context, const netdata_promql_chunk_t *chunk,
                               const netdata_promql_callback_control_t *control,
                               netdata_promql_bytes_t *error) {
    discovery_delivery *d = (discovery_delivery *)context;
    unsigned index = d->output.calls[1];
    if (!index) d->first_cursors = d->source->source.opened[3];
    assert(chunk->kind == NETDATA_PROMQL_CHUNK_SERIES);
    const netdata_promql_series_chunk_t *series = &chunk->payload.series;
    assert(series->series_id == index % 2 && series->labels.len == 1 && series->points.len == 1);
    assert(series->labels.data[0].name.len == 3 && !memcmp(series->labels.data[0].name.data, "job", 3));
    assert(series->labels.data[0].value.len == 1 && series->labels.data[0].value.data[0] == 'a' + index % 2);
    assert(series->points.data[0].time_ms == (index < 2 ? 4000 : 5000));
    assert(series->points.data[0].value.kind == NETDATA_PROMQL_VALUE_FLOAT);
    assert(series->points.data[0].value.payload.float_bits == netdata_promql_double_bits(.5));
    return output_status(&d->output, 2, control, error);
}

static uint32_t discovery_commit(void *context, const netdata_promql_query_observations_t *observed,
                                const netdata_promql_callback_control_t *control,
                                netdata_promql_bytes_t *error) {
    discovery_delivery *d = (discovery_delivery *)context;
    assert(d->output.calls[1] == 4 && observed->output_samples == 4);
    assert(observed->memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
    statistics_valid(&observed->statistics);
    released(&d->source->source);
    return output_status(&d->output, 3, control, error);
}

int main(void) {
    const uint64_t pages[] = {1, 17, 1024};
    const char *functions[] = {"rate", "increase", "delta"};
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        const char *aggregates[] = {"sum", "avg"};
        for (unsigned aggregate = 0; aggregate < 2; aggregate++)
        for (unsigned function = 0; function < 3; function++) {
            char expression[192];
            int length = snprintf(expression, sizeof(expression),
                "max_over_time((histogram_quantile(.5,%s by(job,le)(%s(m_total[3s]))))[2s:1s])", aggregates[aggregate], functions[function]);
            assert(length > 0 && (size_t)length < sizeof(expression));
            netdata_promql_query_t *query = compile(engine, expression);
            for (unsigned page = 0; page < 3; page++) {
                uint64_t first_cursors = 0;
                for (unsigned arm = 0; arm < 4; arm++) {
                    discovery_source source;
                    memset(&source, 0, sizeof(source));
                    init_fixture(&source.source, 5, pages[page]);
                    source.fail_cursor = arm == 1 ? first_cursors + 1 : 0;
                    for (unsigned row = 0; row < 8; row++) {
                        const char *names[] = {"__name__", "job", "le", "member"};
                        const char *values[] = {"m_total", row < 4 ? "a" : "b", row % 4 < 2 ? "1" : "+Inf", row % 2 ? "1" : "0"};
                        for (unsigned label = 0; label < 4; label++) {
                            source.labels[row][label].name = text(names[label]);
                            source.labels[row][label].value = text(values[label]);
                        }
                    }
                    netdata_promql_provider_t input = provider(&source.source);
                    input.context = &source;
                    input.selection_next = discovery_selection;
                    input.cursor_next = discovery_next;
                    discovery_delivery output;
                    memset(&output, 0, sizeof(output));
                    output.source = &source;
                    output.output.failure = arm == 3 ? 2 : 0;
                    if (arm == 2) {
                        output.output.cancel_at = 2;
                        assert(netdata_promql_control_create(engine, NULL, &output.output.control) == NETDATA_PROMQL_OK);
                    }
                    netdata_promql_output_t table = delivery_table(&output.output);
                    table.context = &output;
                    table.begin = discovery_begin;
                    table.chunk = discovery_chunk;
                    table.commit = discovery_commit;
                    netdata_promql_request_t request = instant(4000);
                    request.kind = NETDATA_PROMQL_REQUEST_RANGE;
                    request.end_ms = 5000;
                    request.step_ms = 1000;
                    netdata_promql_report_t *report = NULL;
                    uint32_t status = netdata_promql_execute(query, &input, &request, output.output.control, &table, &report);
                    const uint32_t expected[] = {NETDATA_PROMQL_OK, NETDATA_PROMQL_PROVIDER_ERROR, NETDATA_PROMQL_CANCELLED, NETDATA_PROMQL_OUTPUT_ERROR};
                    assert(status == expected[arm]);
                    if (!arm) first_cursors = output.first_cursors;
                    if (arm == 1) assert(source.failed_cursor == first_cursors + 1 && output.output.calls[1] > 0);
                    assert(report && output.output.calls[0] == 1);
                    assert(output.output.calls[2] == (arm ? 0u : 1u) && output.output.calls[3] == (arm ? 1u : 0u));
                    if (arm >= 2) assert(output.output.calls[1] == 1);
                    netdata_promql_query_observations_t observed;
                    assert(netdata_promql_report_observations(report, &observed) == NETDATA_PROMQL_OK);
                    assert(observed.samples_fetched == source.source.samples_returned && observed.pages == source.source.pages_returned);
                    assert(observed.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
                    released(&source.source);
                    printf("{\"histogram_discovery\":\"pass\",\"aggregate\":\"%s\",\"function\":\"%s\",\"page\":%" PRIu64 ",\"delayed\":%u,\"arm\":%u,\"fetched\":%" PRIu64 "}\n",
                           aggregates[aggregate], functions[function], pages[page], delayed, arm, observed.samples_fetched);
                    if (output.output.control) assert(netdata_promql_control_release(output.output.control) == NETDATA_PROMQL_OK);
                    assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
                }
            }
            assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
            balanced(engine, &idle);
        }
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    return 0;
}
