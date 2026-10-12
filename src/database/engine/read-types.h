// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_DBENGINE_READ_TYPES_H
#define NETDATA_DBENGINE_READ_TYPES_H

#include "libnetdata/libnetdata.h"

typedef enum {
    RRDENG_READ_OK = 0,
    RRDENG_READ_CANCELLED,
    RRDENG_READ_IO,
    RRDENG_READ_EXTENT,
    RRDENG_READ_CHECKSUM,
    RRDENG_READ_DECOMPRESSION,
    RRDENG_READ_DESCRIPTOR,
    RRDENG_READ_PAYLOAD,
    RRDENG_READ_EXPIRED,
    RRDENG_READ_ACCOUNTING,
} RRDENG_READ_ERROR_CODE;

typedef struct {
    RRDENG_READ_ERROR_CODE code;
    int system_errno;
    uint32_t file, block, slot;
} RRDENG_READ_ERROR;

typedef struct {
    void *allocation;
    bool (*resize)(void *allocation, uint64_t bytes);
    void (*release)(void *allocation);
} RRDENG_READ_LEASE;

typedef enum {
    RRDENG_READ_MEMORY_WORKSPACE,
    RRDENG_READ_MEMORY_PAGE,
    RRDENG_READ_MEMORY_BORROWED,
} RRDENG_READ_MEMORY_KIND;

typedef struct {
    void *context;
    bool (*poll)(void *context);
    bool (*acquire)(void *context, RRDENG_READ_MEMORY_KIND kind, uint64_t bytes, RRDENG_READ_LEASE *lease);
} RRDENG_READ_OPERATION;

static inline void rrdeng_read_lease_release(RRDENG_READ_LEASE *lease) {
    if(lease->allocation)
        lease->release(lease->allocation);
    *lease = (RRDENG_READ_LEASE){0};
}

static inline bool rrdeng_read_lease_resize(RRDENG_READ_LEASE *lease, uint64_t bytes) {
    return !lease->allocation || lease->resize(lease->allocation, bytes);
}

static inline bool rrdeng_read_operation_poll(const RRDENG_READ_OPERATION *operation) {
    return !operation || !operation->poll || operation->poll(operation->context);
}

static inline bool rrdeng_read_lease_acquire(const RRDENG_READ_OPERATION *operation,
                                           RRDENG_READ_MEMORY_KIND kind, uint64_t bytes,
                                           RRDENG_READ_LEASE *lease) {
    *lease = (RRDENG_READ_LEASE){0};
    return !operation || !operation->acquire || operation->acquire(operation->context, kind, bytes, lease);
}

typedef enum {
    RRDENG_PAGE_UNVERIFIED = 0,
    RRDENG_PAGE_COLLECTED,
    RRDENG_PAGE_LOADED,
    RRDENG_PAGE_FAILED,
} RRDENG_PAGE_ORIGIN;

typedef struct {
    uint32_t lock_initialized;
    SPINLOCK lock;
    RRDENG_PAGE_ORIGIN origin;
    RRDENG_READ_ERROR error;
    time_t first, last;
    uint32_t cadence, entries;
} RRDENG_PAGE_PROVENANCE;

static inline void rrdeng_page_provenance_lock(RRDENG_PAGE_PROVENANCE *provenance) {
    uint32_t expected = 0;
    if(__atomic_compare_exchange_n(&provenance->lock_initialized, &expected, 1, false,
                                  __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
        spinlock_init(&provenance->lock);
        __atomic_store_n(&provenance->lock_initialized, 2, __ATOMIC_RELEASE);
    }
    else while(__atomic_load_n(&provenance->lock_initialized, __ATOMIC_ACQUIRE) != 2)
        sleep_usec(1);
    spinlock_lock(&provenance->lock);
}

static inline void rrdeng_page_provenance_unlock(RRDENG_PAGE_PROVENANCE *provenance) {
    spinlock_unlock(&provenance->lock);
}

#endif
