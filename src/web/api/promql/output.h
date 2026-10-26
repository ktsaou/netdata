// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_OUTPUT_H
#define NETDATA_PROMQL_OUTPUT_H

#include "json.h"

typedef struct promql_output PROMQL_OUTPUT;

// Borrowed counters remain valid through synchronous commit or error serialization.
typedef struct {
    const uint64_t *pages, *points, *reconstruction_points;
    const bool *overflowed;
    size_t tiers;
} PROMQL_OUTPUT_STORAGE;

void promql_output_storage(PROMQL_OUTPUT *output, PROMQL_OUTPUT_STORAGE storage);
bool promql_output_resources(PROMQL_BODY *body, const netdata_promql_query_observations_t *observations,
                             PROMQL_OUTPUT_STORAGE storage, const char *snapshot,
                             PROMQL_READ_FRAME *frame, int *error);

PROMQL_OUTPUT *promql_output_create(const netdata_promql_accounting_context_t *account,
                                    const char *directory, uint64_t limit, bool stats, bool gzip, int *error);
void promql_output_stats_parameter(PROMQL_OUTPUT *output, netdata_promql_bytes_t value);
netdata_promql_output_t promql_output_callbacks(PROMQL_OUTPUT *output);
PROMQL_BODY *promql_output_take_body(PROMQL_OUTPUT *output);
void promql_output_destroy(PROMQL_OUTPUT *output);

#endif
