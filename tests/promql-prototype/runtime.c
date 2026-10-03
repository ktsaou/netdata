/* SPDX-License-Identifier: GPL-3.0-or-later */
#define _POSIX_C_SOURCE 200809L
#include "abi.h"
#include <time.h>

uint64_t pp_monotonic_ns(void)
{
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now))
        return 0;
    return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}
