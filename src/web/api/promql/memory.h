// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_PROVIDER_MEMORY_H
#define NETDATA_PROMQL_PROVIDER_MEMORY_H

#include "libnetdata/libnetdata.h"
#include "netdata-promql.h"
#include "database/engine/read-types.h"

typedef union {
    max_align_t alignment;
    struct {
        netdata_promql_allocation_handle_t *lease;
        size_t bytes;
        uint32_t kind;
    } owner;
} PROMQL_MEMORY;

static inline void *promql_malloc(const netdata_promql_accounting_context_t *account, uint32_t kind, size_t bytes) {
    if(bytes > SIZE_MAX - sizeof(PROMQL_MEMORY)) return NULL;
    netdata_promql_allocation_handle_t *lease = NULL;
    if(netdata_promql_allocation_acquire(account, kind, sizeof(PROMQL_MEMORY) + bytes, &lease) != NETDATA_PROMQL_OK)
        return NULL;
    PROMQL_MEMORY *block = callocz(1, sizeof(*block) + bytes);
    block->owner.lease = lease;
    block->owner.bytes = bytes;
    block->owner.kind = kind;
    return block + 1;
}

static inline void promql_free(void *data) {
    if(!data) return;
    PROMQL_MEMORY *block = (PROMQL_MEMORY *)data - 1;
    netdata_promql_allocation_handle_t *lease = block->owner.lease;
    freez(block);
    netdata_promql_allocation_release(lease);
}

static inline void *promql_resize(const netdata_promql_accounting_context_t *account, void *data, size_t bytes) {
    PROMQL_MEMORY *block = (PROMQL_MEMORY *)data - 1;
    void *replacement = promql_malloc(account, block->owner.kind, bytes);
    if(!replacement) return NULL;
    memcpy(replacement, data, MIN(bytes, block->owner.bytes));
    // Retain both allocation credits until the old storage has actually been freed.
    promql_free(data);
    return replacement;
}

static inline char *promql_strdup(const netdata_promql_accounting_context_t *account, const char *text) {
    if(!text) text = "";
    size_t bytes = strlen(text) + 1;
    char *copy = promql_malloc(account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, bytes);
    if(copy) memcpy(copy, text, bytes);
    return copy;
}

static inline netdata_promql_bytes_t promql_bytes(const char *text) {
    return (netdata_promql_bytes_t){.data = (const uint8_t *)text, .len = text ? strlen(text) : 0};
}

typedef struct {
    const netdata_promql_callback_control_t *control;
    const netdata_promql_control_t *call_control;
    const netdata_promql_accounting_context_t *account;
    uint32_t interrupted;
} PROMQL_READ_FRAME;

static inline bool promql_read_resize(void *allocation, uint64_t bytes) {
    return netdata_promql_allocation_resize(allocation, bytes) == NETDATA_PROMQL_OK;
}

static inline void promql_read_release(void *allocation) {
    netdata_promql_allocation_release(allocation);
}

static inline bool promql_read_acquire(void *context, RRDENG_READ_MEMORY_KIND kind, uint64_t bytes,
                                      RRDENG_READ_LEASE *out) {
    PROMQL_READ_FRAME *frame = context;
    *out = (RRDENG_READ_LEASE){0};
    const uint32_t categories[] = {NETDATA_PROMQL_MEMORY_WORKSPACE, NETDATA_PROMQL_MEMORY_PROVIDER_HELD,
                                   NETDATA_PROMQL_MEMORY_BORROWED_VIEW};
    if((unsigned)kind >= sizeof(categories) / sizeof(categories[0]))
        return false;
    netdata_promql_allocation_handle_t *lease = NULL;
    if(netdata_promql_allocation_acquire(frame->account, categories[kind], bytes, &lease) != NETDATA_PROMQL_OK)
        return false;
    *out = (RRDENG_READ_LEASE){.allocation = lease, .resize = promql_read_resize, .release = promql_read_release};
    return true;
}

static inline bool promql_read_poll(void *context) {
    PROMQL_READ_FRAME *frame = context;
    if(frame->interrupted != NETDATA_PROMQL_OK) return false;
    if(!frame->control && !frame->call_control) {
        if(frame->account) return true;
        frame->interrupted = NETDATA_PROMQL_INTERNAL_ERROR;
        return false;
    }
    netdata_promql_control_state_t state;
    uint32_t status = frame->control ? netdata_promql_callback_control_state(frame->control, &state) :
        netdata_promql_control_state(frame->call_control, &state);
    if(status == NETDATA_PROMQL_OK) status = state.status;
    if(status != NETDATA_PROMQL_OK) frame->interrupted = status;
    return status == NETDATA_PROMQL_OK;
}

static inline RRDENG_READ_OPERATION promql_read_operation(PROMQL_READ_FRAME *frame,
                                                         const netdata_promql_callback_control_t *control) {
    *frame = (PROMQL_READ_FRAME){.control = control};
    if(netdata_promql_callback_accounting(control, &frame->account) != NETDATA_PROMQL_OK)
        frame->interrupted = NETDATA_PROMQL_INTERNAL_ERROR;
    return (RRDENG_READ_OPERATION){.context = frame, .poll = promql_read_poll, .acquire = promql_read_acquire};
}

#endif
