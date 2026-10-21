/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Exercise comparison-set discovery with the existing paged callback owner. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

typedef struct {
    join_delivery output;
    uint32_t arm, first_series;
} comparison_set_delivery;

static uint32_t comparison_set_chunk(void *context, const netdata_promql_chunk_t *chunk,
                                     const netdata_promql_callback_control_t *control,
                                     netdata_promql_bytes_t *error) {
    comparison_set_delivery *d = (comparison_set_delivery *)context;
    if (!d->output.output.calls[1]) d->first_series = d->output.input->latest_series;
    uint32_t status = join_chunk(&d->output, chunk, control, error);
    if (d->arm == 3 && d->output.output.calls[1] == 1) d->output.input->source.failure = 6;
    return status;
}

static void comparison_set_cases(netdata_promql_engine_t *engine, const char *expression) {
    const uint64_t pages[] = {1, 17, 1024};
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    for (unsigned page = 0; page < 3; page++) {
        uint64_t first_output = 0;
        uint32_t first_series = 0;
        for (unsigned arm = 0; arm < 6; arm++) {
            join_fixture source;
            memset(&source, 0, sizeof(source));
            init_fixture(&source.source, 2, pages[page]);
            source.source.labels[0].value = text("m_total");
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
            input.cursor_next = instant_set_cursor_next;
            if (arm == 1) source.fail_on_cursor = 1;
            if (arm == 2) {
                assert(first_output > 4);
                source.fail_on_cursor = first_output;
            }
            netdata_promql_query_t *query = compile(engine, expression);
            netdata_promql_request_t request = instant(2000);
            request.kind = NETDATA_PROMQL_REQUEST_RANGE;
            request.end_ms = 3000;
            request.step_ms = 1000;
            comparison_set_delivery output;
            memset(&output, 0, sizeof(output));
            output.output.input = &source;
            output.output.query_kind = FOUR_SERIES_RATE;
            output.arm = arm;
            if (arm == 4) {
                output.output.output.cancel_at = 2;
                assert(netdata_promql_control_create(engine, NULL, &output.output.output.control) == NETDATA_PROMQL_OK);
            }
            if (arm == 5) output.output.output.failure = 2;
            netdata_promql_output_t table = delivery_table(&output.output.output);
            table.context = &output;
            table.begin = join_begin;
            table.chunk = comparison_set_chunk;
            table.commit = join_commit;
            netdata_promql_report_t *report = NULL;
            uint32_t status = netdata_promql_execute(query, &input, &request, output.output.output.control, &table, &report);
            uint32_t expected = !arm ? NETDATA_PROMQL_OK : arm == 4 ? NETDATA_PROMQL_CANCELLED :
                                arm == 5 ? NETDATA_PROMQL_OUTPUT_ERROR : NETDATA_PROMQL_PROVIDER_ERROR;
            assert(status == expected && report);
            delivery *d = &output.output.output;
            assert(d->calls[0] == 1 && d->calls[1] == (!arm ? 8u : arm < 3 ? 0u : 1u));
            assert(d->calls[2] == (!arm ? 1u : 0u) && d->calls[3] == (arm ? 1u : 0u));
            if (!arm) { first_output = output.output.first_output_cursors; first_series = output.first_series; }
            if (arm == 2) assert(source.failed_cursor == first_output && source.latest_series == first_series);
            released(&source.source);
            netdata_promql_query_observations_t observations;
            assert(netdata_promql_report_observations(report, &observations) == NETDATA_PROMQL_OK);
            assert(observations.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current == 0);
            statistics_valid(&observations.statistics);
            fputs("{\"comparison_set_case\":\"pass\",\"expression\":", stdout);
            print_json_string(expression);
            printf(",\"page\":%" PRIu64 ",\"arm\":%u,\"status\":%u,\"first_output_cursor\":%" PRIu64
                   ",\"first_series\":%u,\"fetched\":%" PRIu64 ",\"work\":%" PRIu64 "}\n",
                   pages[page], arm, status, first_output, first_series, observations.samples_fetched, observations.work);
            assert(netdata_promql_query_release(query) == NETDATA_PROMQL_OK);
            if (d->control) assert(netdata_promql_control_release(d->control) == NETDATA_PROMQL_OK);
            assert(netdata_promql_report_release(report) == NETDATA_PROMQL_OK);
            balanced(engine, &idle);
        }
    }
}

int main(void) {
    const char *expressions[] = {
        "(m_total > bool 0) and on(zone) m_total",
        "(0 < bool m_total) or on(zone) (m_total < 0)",
        "(m_total > bool 0) unless on(zone) (m_total < 0)",
        "(m_total > bool 0) and on(zone) (m_total == bool 1)",
        "((m_total > bool 0) and on(zone) m_total) or on(zone) (m_total < 0)",
    };
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        for (unsigned i = 0; i < sizeof(expressions) / sizeof(expressions[0]); i++)
            comparison_set_cases(engine, expressions[i]);
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    printf("{\"comparison_sets\":\"pass\",\"cases\":%zu}\n", sizeof(expressions) / sizeof(expressions[0]) * 2 * 3 * 6);
    return 0;
}
