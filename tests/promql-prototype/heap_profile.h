/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef PROMQL_PROTOTYPE_HEAP_PROFILE_H
#define PROMQL_PROTOTYPE_HEAP_PROFILE_H
#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint64_t allocation_calls, reallocation_calls, requested_bytes;
    uint64_t peak_live_bytes, live_bytes, tracking_overflows, untracked_frees;
} PPHeapStats;

/* Profiling-only, single-threaded glibc caller. Never use its timings as release timings. */
void pp_heap_begin(void);
PPHeapStats pp_heap_end(void);
#endif
