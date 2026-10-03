/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _POSIX_C_SOURCE 200809L
#include "heap_profile.h"
#include <assert.h>
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>

int main(void)
{
    void *outside = malloc(7);
    assert(outside);
    pp_heap_begin();
    void *a = malloc(64), *b = calloc(4, 8), *c = NULL;
    assert(a && b && !posix_memalign(&c, 64, 16));
    a = realloc(a, 128);
    assert(a);
    volatile size_t impossible = SIZE_MAX;
    errno = 0;
    void *failed = realloc(a, impossible);
    assert(!failed && errno == ENOMEM);
    free(a);
    free(b);
    free(c);
    free(outside);
    PPHeapStats got = pp_heap_end();
    assert(got.allocation_calls == 4 && got.reallocation_calls == 2);
    assert(got.requested_bytes == 240 && got.peak_live_bytes == 176 && !got.live_bytes);
    assert(!got.tracking_overflows && got.untracked_frees == 1);
    pp_heap_begin();
    got = pp_heap_end();
    assert(!got.allocation_calls && !got.live_bytes && !got.peak_live_bytes);
    return 0;
}
