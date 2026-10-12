/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "abi3-header.h"
#include <assert.h>
#include <stdio.h>

int main(void) {
    /* Keep the previous declaration independent of the current header. */
    assert(NETDATA_PROMQL_ABI_VERSION == 3);
    assert(netdata_promql_abi_version() == 4);
    netdata_promql_engine_options_t options;
    memset(&options, 0, sizeof(options));
    options.header.abi_version = NETDATA_PROMQL_ABI_VERSION;
    options.header.struct_size = sizeof(options);
    options.lookback_ms = 300000;
    options.no_step_subquery_interval_ms = 60000;
    netdata_promql_engine_t *engine = NULL;
    assert(netdata_promql_engine_create(&options, &engine) == NETDATA_PROMQL_INVALID_ARGUMENT);
    assert(engine == NULL);
    puts("{\"old_header_rejected\":\"pass\",\"caller_abi\":3,\"library_abi\":4,\"engine_options\":\"rejected\"}");
    return 0;
}
