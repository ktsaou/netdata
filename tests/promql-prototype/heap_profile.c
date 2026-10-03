/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _GNU_SOURCE
#include "heap_profile.h"
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* Interposition also sees C++'s dynamic operator new and Rust's System allocator.
 * Fixed bookkeeping avoids counting the profiler's own allocations. */
extern void *__libc_malloc(size_t);
extern void *__libc_calloc(size_t, size_t);
extern void *__libc_realloc(void *, size_t);
extern void __libc_free(void *);
extern void *__libc_memalign(size_t, size_t);

#define SLOTS (1U << 18)
typedef struct {
    uintptr_t address;
    size_t bytes;
    unsigned state;
} Entry;
static Entry entries[SLOTS];
static PPHeapStats stats;
static int active;

static size_t hash(uintptr_t address)
{
    address >>= 4;
    address ^= address >> 23;
    return (address * UINT64_C(0x9e3779b97f4a7c15)) & (SLOTS - 1);
}
static Entry *find(uintptr_t address)
{
    size_t i = hash(address);
    for (size_t n = 0; n < SLOTS; n++, i = (i + 1) & (SLOTS - 1)) {
        if (!entries[i].state)
            return NULL;
        if (entries[i].state == 1 && entries[i].address == address)
            return entries + i;
    }
    return NULL;
}
static void record(void *p, size_t bytes)
{
    if (!active || !p)
        return;
    stats.allocation_calls++;
    stats.requested_bytes += bytes;
    size_t i = hash((uintptr_t)p), spare = SLOTS;
    for (size_t n = 0; n < SLOTS; n++, i = (i + 1) & (SLOTS - 1)) {
        if (entries[i].state == 2 && spare == SLOTS)
            spare = i;
        if (!entries[i].state) {
            if (spare != SLOTS)
                i = spare;
            entries[i] = (Entry){(uintptr_t)p, bytes, 1};
            stats.live_bytes += bytes;
            if (stats.live_bytes > stats.peak_live_bytes)
                stats.peak_live_bytes = stats.live_bytes;
            return;
        }
    }
    if (spare != SLOTS) {
        entries[spare] = (Entry){(uintptr_t)p, bytes, 1};
        stats.live_bytes += bytes;
        if (stats.live_bytes > stats.peak_live_bytes)
            stats.peak_live_bytes = stats.live_bytes;
        return;
    }
    stats.tracking_overflows++;
}
static void forget(uintptr_t address)
{
    if (!active || !address)
        return;
    Entry *e = find(address);
    if (e) {
        stats.live_bytes -= e->bytes;
        e->state = 2;
    }
    else
        stats.untracked_frees++;
}
void pp_heap_begin(void)
{
    active = 0;
    memset(entries, 0, sizeof entries);
    memset(&stats, 0, sizeof stats);
    active = 1;
}
PPHeapStats pp_heap_end(void)
{
    active = 0;
    return stats;
}
void *malloc(size_t bytes)
{
    void *p = __libc_malloc(bytes);
    record(p, bytes);
    return p;
}
void *calloc(size_t count, size_t bytes)
{
    void *p = __libc_calloc(count, bytes);
    if (p)
        record(p, count * bytes);
    return p;
}
void free(void *p)
{
    int saved_errno = errno;
    forget((uintptr_t)p);
    __libc_free(p);
    errno = saved_errno;
}
void *realloc(void *old, size_t bytes)
{
    uintptr_t address = (uintptr_t)old;
    void *p = __libc_realloc(old, bytes);
    if (active)
        stats.reallocation_calls++;
    if (p || !bytes) {
        forget(address);
        record(p, bytes);
    }
    return p;
}
int posix_memalign(void **out, size_t alignment, size_t bytes)
{
    if (alignment < sizeof(void *) || alignment & (alignment - 1))
        return EINVAL;
    int saved_errno = errno;
    void *p = __libc_memalign(alignment, bytes);
    int result = p ? 0 : ENOMEM;
    errno = saved_errno;
    if (p) {
        record(p, bytes);
        *out = p;
    }
    return result;
}
void *aligned_alloc(size_t alignment, size_t bytes)
{
    void *p = __libc_memalign(alignment, bytes);
    record(p, bytes);
    return p;
}
