/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Share complete output and callback checks with ordinary scalar reductions. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

int main(void) {
    const char *expressions[][2] = {
        {"m/(scalar(avg(m))+0)", "(scalar(avg(m))+0)/m"},
        {"m/(0+scalar(avg(m)))", "(0+scalar(avg(m)))/m"},
    };
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned side = 0; side < 2; side++)
            completed_group_value_expressions(engine, expressions[side]);
        balanced(engine, &idle);
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    puts("{\"scalar_arithmetic_values\":\"pass\",\"cases\":144}");
    return 0;
}
