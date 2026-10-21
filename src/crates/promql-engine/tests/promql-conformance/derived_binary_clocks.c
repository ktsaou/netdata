/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Reuse complete labels, values, callback ordering and provider-failure checks. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

int main(void) {
    const char *expressions[] = {
        "max_over_time((m / on(zone) m)[2s:1s])",
        "max_over_time((((m / on(zone) m)))[2s:1s])",
        "abs(max_over_time((m / on(zone) m)[2s:1s]))",
        "max_over_time((m == bool on(zone) m)[2s:1s])",
    };
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned i = 0; i < sizeof(expressions) / sizeof(expressions[0]); i++)
            four_series_float_query(engine, FOUR_SERIES_JOIN, expressions[i]);
        balanced(engine, &idle);
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    puts("{\"derived_binary_clocks\":\"pass\",\"cases\":72}");
    return 0;
}
