/* Reuse actual C callbacks and calibrated provider/output failure owners. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

int main(void) {
    const char *expressions[] = {
        "-((sum by(zone)(rate(m_total[3s])) > bool 0) * -1)",
        "(-((sum by(zone)(rate(m_total[3s])) > bool 0) * -1))",
        "(sum by(zone)(rate(m_total[3s])) > 0) > bool 0",
        "-((abs(sum by(zone)(rate(m_total[3s]))) > bool 0) * -1)",
        "abs(label_join(sum by(zone)(m_total),\"zone\",\"\",\"zone\")) > bool 0",
        "abs(label_replace(sum by(zone)(m_total),\"zone\",\"$1\",\"zone\",\"(.*)\")) > bool 0",
        "abs(label_join(sum by(zone)(rate(m_total[3s])),\"zone\",\"\",\"zone\")) > bool 0",
        "abs(label_replace(sum by(zone)(rate(m_total[3s])),\"zone\",\"$1\",\"zone\",\"(.*)\")) > bool 0",
        "(sum by(zone)(m_total) + on(zone) sum by(zone)(m_total)) > bool 0",
        "(sum by(zone)(m_total) or sum by(zone)(m_total)) > bool 0",
        "(sum by(zone)(rate(m_total[3s])) + on(zone) sum by(zone)(rate(m_total[3s]))) > bool 0",
        "(sum by(zone)(rate(m_total[3s])) or sum by(zone)(rate(m_total[3s]))) > bool 0",
        "sum by(zone)(m_total+m_total) > bool 0",
        "sum by(zone)(rate(m_total[3s])+rate(m_total[3s])) > bool 0",
        "clamp_min(sum by(zone)(m_total+m_total),scalar(vector(time()))) > bool 0",
        "sum by(zone)(sum by(zone)(m_total)+on(zone)sum by(zone)(m_total)) > bool 0",
        "sum by(zone)(sum by(zone)(rate(m_total[3s]))+on(zone)sum by(zone)(rate(m_total[3s]))) > bool 0",
        "clamp_min(sum by(zone)(sum by(zone)(m_total)+on(zone)sum by(zone)(rate(m_total[3s]))),scalar(vector(time()))) > bool 0"
    };
    for (unsigned delayed = 0; delayed < 2; delayed++) {
        netdata_promql_engine_options_t options;
        assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
        if (delayed) options.flags |= NETDATA_PROMQL_DELAYED_NAME_REMOVAL;
        netdata_promql_engine_t *engine = NULL;
        assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
        netdata_promql_engine_observations_t idle = engine_observations(engine);
        for (unsigned expression = 0; expression < sizeof(expressions) / sizeof(expressions[0]); expression++) {
            four_series_float_query_members(engine, FOUR_SERIES_RATE, expressions[expression], 1);
            balanced(engine, &idle);
        }
        assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    }
    return 0;
}
