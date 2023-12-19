// SPDX-License-Identifier: GPL-3.0-or-later

#include "../libnetdata.h"
#include "libnetdata/locks/mutex.h"

#ifdef NETDATA_REPLACE_SPINLOCK_WITH_MUTEX

void spinlock_init(spinlock_t *spinlock)
{
    netdata_mutex_init(&spinlock->mutex);
}

void spinlock_lock(spinlock_t *spinlock)
{
    netdata_mutex_lock(&spinlock->mutex);
}

void spinlock_unlock(spinlock_t *spinlock)
{
    netdata_mutex_unlock(&spinlock->mutex);
}

bool spinlock_trylock(spinlock_t *spinlock)
{
    return netdata_mutex_trylock(&spinlock->mutex) == 0;
}

#else /* NETDATA_REPLACE_SPINLOCK_WITH_MUTEX */

void spinlock_init(spinlock_t *spinlock)
{
    memset(spinlock, 0, sizeof(spinlock_t));
}

void spinlock_lock(spinlock_t *spinlock)
{
    static const struct timespec ns = { .tv_sec = 0, .tv_nsec = 1 };

#ifdef NETDATA_INTERNAL_CHECKS
    size_t spins = 0;
#endif

    netdata_thread_disable_cancelability();

    for (int i = 1;
         __atomic_load_n(&spinlock->locked, __ATOMIC_RELAXED) ||
         __atomic_test_and_set(&spinlock->locked, __ATOMIC_ACQUIRE);
         i++)
    {
#ifdef NETDATA_INTERNAL_CHECKS
        spins++;
#endif

        if (unlikely(i == 8))
        {
            i = 0;
            nanosleep(&ns, NULL);
        }
    }

    // we have the lock

#ifdef NETDATA_INTERNAL_CHECKS
    spinlock->spins += spins;
    spinlock->locker_pid = gettid();
#endif
}

void spinlock_unlock(spinlock_t *spinlock)
{
#ifdef NETDATA_INTERNAL_CHECKS
    spinlock->locker_pid = 0;
#endif
    __atomic_clear(&spinlock->locked, __ATOMIC_RELEASE);
    netdata_thread_enable_cancelability();
}

bool spinlock_trylock(spinlock_t *spinlock)
{
    netdata_thread_disable_cancelability();

    if (!__atomic_load_n(&spinlock->locked, __ATOMIC_RELAXED) && !__atomic_test_and_set(&spinlock->locked, __ATOMIC_ACQUIRE))
    {
        // we got the lock
        return true;
    }
    else
    {
        // we didn't get the lock
        netdata_thread_enable_cancelability();
        return false;
    }
}

#endif /* NETDATA_REPLACE_SPINLOCK_WITH_MUTEX */

// ----------------------------------------------------------------------------
// rw_spinlock implementation

void rw_spinlock_init(rw_spinlock_t *rw_spinlock)
{
    rw_spinlock->readers = 0;
    spinlock_init(&rw_spinlock->spinlock);
}

void rw_spinlock_read_lock(rw_spinlock_t *rw_spinlock)
{
    netdata_thread_disable_cancelability();

    spinlock_lock(&rw_spinlock->spinlock);
    __atomic_add_fetch(&rw_spinlock->readers, 1, __ATOMIC_RELAXED);
    spinlock_unlock(&rw_spinlock->spinlock);
}

void rw_spinlock_read_unlock(rw_spinlock_t *rw_spinlock)
{
    int32_t x = __atomic_sub_fetch(&rw_spinlock->readers, 1, __ATOMIC_RELAXED);

#ifdef NETDATA_INTERNAL_CHECKS
    if (x < 0)
        fatal("rw_spinlock_t: readers is negative %d", x);
#else
    (void) x;
#endif

    netdata_thread_enable_cancelability();
}

void rw_spinlock_write_lock(rw_spinlock_t *rw_spinlock)
{
    static const struct timespec ns = { .tv_sec = 0, .tv_nsec = 1 };

    while (1)
    {
        spinlock_lock(&rw_spinlock->spinlock);

        if (__atomic_load_n(&rw_spinlock->readers, __ATOMIC_RELAXED) == 0)
        {
            break;
        }

        // Busy wait until all readers have released their locks.
        spinlock_unlock(&rw_spinlock->spinlock);
        nanosleep(&ns, NULL);
    }
}

void rw_spinlock_write_unlock(rw_spinlock_t *rw_spinlock)
{
    spinlock_unlock(&rw_spinlock->spinlock);
}

bool rw_spinlock_tryread_lock(rw_spinlock_t *rw_spinlock)
{
    bool ok = false;

    if (spinlock_trylock(&rw_spinlock->spinlock))
    {
        __atomic_add_fetch(&rw_spinlock->readers, 1, __ATOMIC_RELAXED);
        spinlock_unlock(&rw_spinlock->spinlock);
        netdata_thread_disable_cancelability();
        ok = true;
    }

    return ok;
}

bool rw_spinlock_trywrite_lock(rw_spinlock_t *rw_spinlock)
{
    if (!spinlock_trylock(&rw_spinlock->spinlock))
    {
        return false;
    }

    if (__atomic_load_n(&rw_spinlock->readers, __ATOMIC_RELAXED) == 0)
    {
        // No readers, we've successfully acquired the write lock
        return true;
    }

    // There are readers, unlock the spinlock and return false
    spinlock_unlock(&rw_spinlock->spinlock);
    return false;
}
