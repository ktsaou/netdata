/* SPDX-License-Identifier: GPL-3.0-or-later */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main
int main(void) {
    const char *operators[] = {"sum", "avg", "count", "min", "max", "group", "stdvar", "stddev"};
    const char *children[] = {"m_total / 3", "3 / m_total", "m_total * 3", "ceil(m_total)"};
    for (unsigned child = 0; child < 4; child++) {
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned op = 0; op < 8; op++) {
            char expression[256];
            int length = snprintf(expression, sizeof(expression),
                "(%s by(zone)(%s)+1) / on(zone) (%s by(zone)(%s)+1)",
                operators[op], children[child], operators[op], children[child]);
            assert(length > 0 && (size_t)length < sizeof(expression));
            four_series_float_query_members(engine, FOUR_SERIES_RATE, expression, 1, 0);
            balanced(engine, &idle);
        }
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    }
    puts("{\"scalar_map_label_borrow\":\"pass\",\"cases\":768}");
    return 0;
}
