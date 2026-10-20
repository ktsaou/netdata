/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Keep the provider, complete outputs and callback phases shared with grouped joins. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

int main(void) {
    const char *expressions[] = {"m/scalar(avg(m))", "scalar(avg(m))/m"};
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        completed_group_value_expressions(engine, expressions);
        balanced(engine, &idle);
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    puts("{\"scalar_group_values\":\"pass\",\"cases\":72}");
    return 0;
}
