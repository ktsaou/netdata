/* Reuse actual paged C callbacks and calibrated quantile failures. */
#define main retained_embedding_main
#include "c_embedding.c"
#undef main

int main(void) {
    assert(netdata_promql_abi_version() == NETDATA_PROMQL_ABI_VERSION);
    netdata_promql_engine_options_t options;
    assert(netdata_promql_engine_options_default(&options) == NETDATA_PROMQL_OK);
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_OK);
    netdata_promql_engine_observations_t idle = engine_observations(engine);
    const uint64_t pages[] = {1, 17, 1024};
    for (unsigned i = 0; i < 3; i++) {
        uint64_t first_samples, second_samples;
        uint64_t first = temporary_window(engine, 2048, pages[i], &first_samples);
        balanced(engine, &idle);
        uint64_t second = temporary_window(engine, 4096, pages[i], &second_samples);
        balanced(engine, &idle);
        assert(first == second && first_samples == second_samples);
    }
    temporary_failure_cases(engine);
    balanced(engine, &idle);
    four_series_float_query(engine, FOUR_SERIES_NUMERIC, NULL);
    balanced(engine, &idle);
    four_series_float_query(engine, FOUR_SERIES_NUMERIC, "quantile_over_time(.5,(m*1)[2s:1s])");
    balanced(engine, &idle);
    assert(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    puts("{\"rank_completion_embedding\":\"pass\",\"generated_queries\":6,\"failure_cases\":11,\"four_series_cases\":12}");
    return 0;
}
