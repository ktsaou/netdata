// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_DBENGINE_DEMAND_H
#define NETDATA_DBENGINE_DEMAND_H

#include "rrdengine.h"

#ifdef __cplusplus
extern "C" {
#endif

#define RRDENG_READ_MAX_SPANS (2 * RRD_STORAGE_TIERS - 1)

typedef struct { time_t first, last; } RRDENG_READ_INTERVAL;
typedef struct { time_t first, last; uint32_t tier; } RRDENG_READ_TIER_SPAN;

typedef struct rrdeng_read_promise {
    struct rrdeng_read_promise *prev, *next;
    METRIC *metric;
    nd_uuid_t uuid;
    time_t first, last;
    RRDENG_READ_INTERVAL required[RRDENG_READ_MAX_SPANS];
    uint32_t required_count;
    uint32_t cadence;
    bool expired;
} RRDENG_READ_PROMISE;

typedef enum { RRDENG_DEMAND_READY, RRDENG_DEMAND_GAP, RRDENG_DEMAND_END, RRDENG_DEMAND_ERROR } RRDENG_DEMAND_STATUS;

typedef struct {
    PGD *data;
    time_t first, last;
    uint32_t cadence, entries;
} RRDENG_DEMAND_PAGE;

typedef struct rrdeng_reader RRDENG_READER;

bool rrdeng_read_freeze_metric(METRIC *metric, RRDENG_READ_PROMISE *promise, time_t first, time_t last);
bool rrdeng_read_freeze_tiers(METRIC *const *metrics, uint32_t tiers, RRDENG_READ_PROMISE *promises,
                            time_t first, time_t last, RRDENG_READ_TIER_SPAN *spans, uint32_t *count);
bool rrdeng_read_plan_tiers(RRDENG_READ_PROMISE *promises, uint32_t tiers, time_t first, time_t last,
                          RRDENG_READ_TIER_SPAN *spans, uint32_t *count);
void rrdeng_read_unfreeze_metric(RRDENG_READ_PROMISE *promise);
bool rrdeng_read_promise_valid(const RRDENG_READ_PROMISE *promise);
void rrdeng_read_expire_metric_unsafe(METRIC *metric, time_t first, time_t last);
void rrdeng_read_expire_file(struct rrdengine_datafile *datafile);

static inline bool rrdeng_read_promise_overlaps(const RRDENG_READ_PROMISE *promise, time_t first, time_t last) {
    for(uint32_t i = 0; i < promise->required_count; i++)
        if(promise->required[i].first <= last && promise->required[i].last >= first)
            return true;
    return false;
}

bool rrdeng_reader_open(METRIC *metric, const RRDENG_READ_PROMISE *promise,
                       time_t first, time_t last, const RRDENG_READ_OPERATION *operation,
                       RRDENG_READER **reader, RRDENG_READ_ERROR *error);
RRDENG_DEMAND_STATUS rrdeng_reader_acquire(RRDENG_READER *reader, time_t minimum,
                       const RRDENG_READ_OPERATION *operation, RRDENG_DEMAND_PAGE *page,
                       RRDENG_READ_ERROR *error);
bool rrdeng_reader_point(RRDENG_READER *reader, uint32_t position, STORAGE_POINT *point,
                       const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error);
bool rrdeng_reader_counter_before(RRDENG_READER *reader, uint32_t position, double *counter,
                       const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error);
void rrdeng_reader_close(RRDENG_READER *reader, const RRDENG_READ_OPERATION *operation);

#ifdef __cplusplus
}
#endif

#endif
