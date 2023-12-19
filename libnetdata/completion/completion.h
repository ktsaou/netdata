// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_COMPLETION_H
#define NETDATA_COMPLETION_H

#include "../libnetdata.h"

enum COMPLETION_SOURCE {
    COMPLETION_SOURCE_COMMANDS_INIT = 0,
    COMPLETION_SOURCE_GENERATE_DBENGINE_DATASET = 1,
    COMPLETION_SOURCE_DBENGINE_STRESS_TEST = 2,
    COMPLETION_SOURCE_RRDENG_POPULATE_MRG = 3,
    COMPLETION_SOURCE_RRDENG_EXIT = 4,
    COMPLETION_SOURCE_RRDENG_PREPARE_EXIT = 5,
    COMPLETION_SOURCE_METADATA_SYNC_SHUTDOWN = 6,
    COMPLETION_SOURCE_SPAWN_INIT = 7,
    COMPLETION_SOURCE_METADATA_SYNC_SHUTDOWN_PREPARE = 8,
    COMPLETION_SOURCE_METADATA_SYNC_INIT = 9,
    COMPLETION_SOURCE_MAIN_CACHE_FLUSH_DIRTY_PAGE_CALLBACK = 10,
    COMPLETION_SOURCE_PG_CACHE_PRELOAD_1 = 11,
    COMPLETION_SOURCE_PG_CACHE_PRELOAD_2 = 12,
};

struct completion {
    enum COMPLETION_SOURCE source;
    uv_mutex_t mutex;
    uv_cond_t cond;
    volatile unsigned completed;
    volatile unsigned completed_jobs;
};

void completion_init(struct completion *p, enum COMPLETION_SOURCE source);

void completion_destroy(struct completion *p);

void completion_wait_for(struct completion *p);

void completion_mark_complete(struct completion *p);

unsigned completion_wait_for_a_job(struct completion *p, unsigned completed_jobs);
void completion_mark_complete_a_job(struct completion *p);
bool completion_is_done(struct completion *p);

#endif /* NETDATA_COMPLETION_H */
