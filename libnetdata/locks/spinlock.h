// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_SPINLOCK_H
#define NETDATA_SPINLOCK_H 

#include <stdbool.h>
#include <stdint.h>
#include "mutex.h"

#ifdef NETDATA_REPLACE_SPINLOCK_WITH_MUTEX
    typedef struct
    {
        netdata_mutex_t mutex;
    } spinlock_t;

    #define SPINLOCK_INITIALIZER { .mutex = NETDATA_MUTEX_INITIALIZER }
#else /* NETDATA_REPLACE_SPINLOCK_WITH_MUTEX */
    typedef struct
    {
        bool locked;
    #ifdef NETDATA_INTERNAL_CHECKS
        pid_t locker_pid;
        size_t spins;
    #endif
    } spinlock_t;

    #define SPINLOCK_INITIALIZER { .locked = false }
#endif /* NETDATA_REPLACE_SPINLOCK_WITH_MUTEX */

void spinlock_init(spinlock_t *spinlock);
void spinlock_lock(spinlock_t *spinlock);
void spinlock_unlock(spinlock_t *spinlock);
bool spinlock_trylock(spinlock_t *spinlock);

typedef struct {
    int32_t readers;
    spinlock_t spinlock;
} rw_spinlock_t;

#define RW_SPINLOCK_INITIALIZER { .readers = 0, .spinlock = SPINLOCK_INITIALIZER }

void rw_spinlock_init(rw_spinlock_t *rw_spinlock);
void rw_spinlock_read_lock(rw_spinlock_t *rw_spinlock);
void rw_spinlock_read_unlock(rw_spinlock_t *rw_spinlock);
void rw_spinlock_write_lock(rw_spinlock_t *rw_spinlock);
void rw_spinlock_write_unlock(rw_spinlock_t *rw_spinlock);
bool rw_spinlock_tryread_lock(rw_spinlock_t *rw_spinlock);
bool rw_spinlock_trywrite_lock(rw_spinlock_t *rw_spinlock);

#endif /* NETDATA_SPINLOCK_H */
