/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Reuse complete C outputs, allocation ownership and actual required callback failures. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

int main(void) {
    const char *expressions[] = {
        "max_over_time((sum by(zone)(rate(m_total[3s])))[2s:1s]) > bool 0",
        "max_over_time((((sum by(zone)(rate(m_total[3s])))))[2s:1s]) > bool 0",
        "sum_over_time((sum by(zone)(rate(m_total[3s])))[2s:1s]) > bool 0",
    };
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned i = 0; i < sizeof(expressions) / sizeof(expressions[0]); i++)
            four_series_float_query_members(engine, FOUR_SERIES_RATE, expressions[i], 1, 0);
        balanced(engine, &idle);
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    puts("{\"grouped_subquery_clocks\":\"pass\",\"cases\":72}");
    return 0;
}
