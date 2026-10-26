// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_PROVIDER_H
#define NETDATA_PROMQL_PROVIDER_H

#include "memory.h"

typedef enum { PROMQL_FLEET, PROMQL_HOST_GUID, PROMQL_HOST_NODE, PROMQL_HOST_NAME } PROMQL_HOST_SCOPE;

typedef struct {
    PROMQL_HOST_SCOPE scope;
    const char *host;
    const char *backing_directory;
    bool absolute;
    // Only metric-name discovery with exclusively name predicates may project metadata.
    bool names_only;
    uint64_t storage_pages[RRD_STORAGE_TIERS];
    // Cursor reads and provider folds exclude native page-prefix decoding work.
    uint64_t storage_points[RRD_STORAGE_TIERS];
    uint64_t reconstruction_points;
    bool counters_overflowed;
    char error[192];
} PROMQL_PROVIDER_CONTEXT;

netdata_promql_provider_t promql_provider(PROMQL_PROVIDER_CONTEXT *context);

#endif
