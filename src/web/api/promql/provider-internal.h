// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_PROVIDER_INTERNAL_H
#define NETDATA_PROMQL_PROVIDER_INTERNAL_H

#include "provider.h"
#include "backing.h"
#include "checkpoint.h"
#include "database/contexts/rrdcontext-internal.h"
#include "database/engine/demand.h"
#include "database/engine/rrdengineapi.h"

typedef struct promql_host {
    struct promql_host *next;
    RRDHOST *host;
    size_t references;
} PROMQL_HOST;

typedef struct {
    STRING *name, *value;
    char *generated_name;
} PROMQL_LABEL_REFERENCE;

typedef struct promql_physical {
    struct promql_provider_query *query;
    PROMQL_HOST *host;
    nd_uuid_t uuid;
    size_t references;
    netdata_promql_label_t *labels;
    PROMQL_LABEL_REFERENCE *label_references;
    netdata_promql_allocation_handle_t *metadata_view;
    uint64_t metadata_view_bytes;
    size_t label_count, label_capacity;
    const char *name;
    char *alias;
    bool incremental, basis_selected, anchored;
    METRIC *metrics[RRD_STORAGE_TIERS];
    RRDENG_READ_PROMISE promises[RRD_STORAGE_TIERS];
    RRDENG_READ_TIER_SPAN spans[RRDENG_READ_MAX_SPANS];
    uint32_t span_count;
    PROMQL_CHECKPOINTS *checkpoints;
} PROMQL_PHYSICAL;

typedef struct promql_provider_query {
    PROMQL_PROVIDER_CONTEXT *context;
    netdata_promql_bounds_t bounds;
    time_t first, last;
    bool empty;
    PROMQL_HOST *hosts;
    PROMQL_PHYSICAL **physical;
    size_t capacity, used, tombstones;
    PROMQL_CHECKPOINT_BACKING checkpoints;
} PROMQL_PROVIDER_QUERY;

static inline int64_t promql_ceil_seconds(int64_t milliseconds) {
    return milliseconds / 1000 + (milliseconds % 1000 > 0);
}

static inline int64_t promql_floor_seconds(int64_t milliseconds) {
    return milliseconds / 1000 - (milliseconds % 1000 < 0);
}

static inline bool promql_storage_interval(netdata_promql_bounds_t bounds, time_t *first, time_t *last) {
    int64_t begin = MAX(promql_ceil_seconds(bounds.start_ms), INT64_C(1));
    int64_t end = MIN(promql_floor_seconds(bounds.end_ms), (int64_t)nd_time_t_max());
    if(bounds.start_ms > bounds.end_ms || begin > end) return false;
    *first = (time_t)begin;
    *last = (time_t)end;
    return true;
}

typedef struct {
    PROMQL_PROVIDER_QUERY *query;
    struct dictionary_foreach hosts, contexts, instances, metrics;
    bool hosts_started, contexts_started, instances_started, metrics_started, scoped_done;
    PROMQL_HOST *host;
    PROMQL_PHYSICAL *pending_alias;
    netdata_promql_matcher_t *prefilters;
    size_t prefilter_count;
} PROMQL_SELECTION;

typedef struct {
    PROMQL_PHYSICAL *physical;
    netdata_promql_label_t labels[];
} PROMQL_SERIES;

int promql_label_compare(netdata_promql_bytes_t a, netdata_promql_bytes_t b);
void promql_labels_sort(netdata_promql_label_t *labels, size_t count);
PROMQL_HOST *promql_host_acquire(PROMQL_PROVIDER_QUERY *query, RRDHOST *host,
                                 const netdata_promql_accounting_context_t *account, bool *busy, bool *unavailable);
void promql_host_release(PROMQL_PROVIDER_QUERY *query, PROMQL_HOST *host);
void promql_physical_release(PROMQL_PHYSICAL *physical);
uint32_t promql_metadata_next(PROMQL_SELECTION *selection, PROMQL_READ_FRAME *frame,
                              netdata_promql_series_open_t *out, netdata_promql_bytes_t *error);
void promql_selection_destroy(PROMQL_SELECTION *selection);
bool promql_selection_prefilters(PROMQL_SELECTION *selection, netdata_promql_matchers_t matchers,
                                 const netdata_promql_accounting_context_t *account);
uint32_t promql_provider_error(PROMQL_PROVIDER_CONTEXT *context, const char *message,
                               netdata_promql_bytes_t *error);

#endif
