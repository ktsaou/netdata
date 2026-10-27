#include "netdata-promql.h"
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#define CHECK(x) do { if (!(x)) { fprintf(stderr, "check failed at line %d\n", __LINE__); return 1; } } while (0)
int main(int argc, char **argv) {
    CHECK(argc == 3);
    unsigned mode = (unsigned)strtoul(argv[1], NULL, 10);
    unsigned long iterations = strtoul(argv[2], NULL, 10);
    CHECK(mode < 4 && iterations > 0);
    netdata_promql_engine_t *engine = NULL;
    netdata_promql_control_t *control = NULL;
    netdata_promql_control_options_t options;
    CHECK(netdata_promql_engine_create(NULL, &engine) == NETDATA_PROMQL_OK);
    CHECK(netdata_promql_control_options_default(&options) == NETDATA_PROMQL_OK);
    options.deadline_present = mode != 0;
    options.seconds = mode == 2 ? 0 : 3600;
    CHECK(netdata_promql_control_create(engine, &options, &control) == NETDATA_PROMQL_OK);
    if (mode == 3) CHECK(netdata_promql_control_cancel(control) == NETDATA_PROMQL_OK);
    uint32_t expected = mode == 2 ? NETDATA_PROMQL_DEADLINE : mode == 3 ? NETDATA_PROMQL_CANCELLED : NETDATA_PROMQL_OK;
    struct timespec before, after;
    netdata_promql_control_state_t state = {0};
    unsigned failures = 0;
    CHECK(clock_gettime(CLOCK_MONOTONIC, &before) == 0);
    for (unsigned long i = 0; i < iterations; ++i) {
        failures |= netdata_promql_control_state(control, &state) != NETDATA_PROMQL_OK;
        failures |= state.status != expected;
        failures |= state.deadline_present != (mode != 0);
        failures |= state.remaining.nanoseconds >= 1000000000 || state.remaining.reserved != 0;
    }
    CHECK(clock_gettime(CLOCK_MONOTONIC, &after) == 0);
    CHECK(!failures);
    CHECK(netdata_promql_control_release(control) == NETDATA_PROMQL_OK);
    CHECK(netdata_promql_engine_release(engine) == NETDATA_PROMQL_OK);
    double seconds = (after.tv_sec-before.tv_sec) + (after.tv_nsec-before.tv_nsec)*1e-9;
    printf("{\"mode\":%u,\"iterations\":%lu,\"seconds\":%.9f}\n", mode, iterations, seconds);
    return 0;
}
