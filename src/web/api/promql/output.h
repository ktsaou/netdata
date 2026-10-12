// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_OUTPUT_H
#define NETDATA_PROMQL_OUTPUT_H

#include "json.h"

typedef struct promql_output PROMQL_OUTPUT;

PROMQL_OUTPUT *promql_output_create(const netdata_promql_accounting_context_t *account,
                                    const char *directory, uint64_t limit, bool stats, bool gzip, int *error);
void promql_output_stats_parameter(PROMQL_OUTPUT *output, netdata_promql_bytes_t value);
netdata_promql_output_t promql_output_callbacks(PROMQL_OUTPUT *output);
PROMQL_BODY *promql_output_take_body(PROMQL_OUTPUT *output);
void promql_output_destroy(PROMQL_OUTPUT *output);

#endif
