// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_STORAGE_POINT_H
#define NETDATA_STORAGE_POINT_H

#include "storage_number/storage_number.h"

typedef struct storage_point {
    NETDATA_DOUBLE min;     // when count > 1, this is the minimum among them
    NETDATA_DOUBLE max;     // when count > 1, this is the maximum among them
    NETDATA_DOUBLE sum;     // the point sum - divided by count gives the average
    double counter;        // reconstructed counter endpoint when COUNTER_VALUE_VALID is set

    // end_time - start_time = point duration
    time_t start_time_s;    // the time the point starts
    time_t end_time_s;      // the time the point ends

    uint32_t count;         // the number of original points aggregated
    uint32_t anomaly_count; // the number of original points found anomalous

    SN_FLAGS flags;         // flags stored with the point
} STORAGE_POINT;

#define storage_point_unset(x)                     do { \
    (x).min = (x).max = (x).sum = NAN;                  \
    (x).counter = NAN;                                  \
    (x).count = 0;                                      \
    (x).anomaly_count = 0;                              \
    (x).flags = SN_FLAG_NONE;                           \
    (x).start_time_s = 0;                               \
    (x).end_time_s = 0;                                 \
    } while(0)

#define storage_point_empty(x, start_s, end_s)     do { \
    (x).min = (x).max = (x).sum = NAN;                  \
    (x).counter = NAN;                                  \
    (x).count = 1;                                      \
    (x).anomaly_count = 0;                              \
    (x).flags = SN_FLAG_NONE;                           \
    (x).start_time_s = start_s;                         \
    (x).end_time_s = end_s;                             \
    } while(0)

#define STORAGE_POINT_UNSET (STORAGE_POINT){ .min = NAN, .max = NAN, .sum = NAN, .counter = NAN, .start_time_s = 0, .end_time_s = 0, .count = 0, .anomaly_count = 0, .flags = SN_FLAG_NONE }

static inline bool storage_counter_fold(double *counter, const STORAGE_POINT *sp, bool positive) {
    if(sp->flags & SN_FLAG_RESET)
        *counter = 0.0;

    if(isnan(sp->sum))
        return !(sp->flags & SP_FLAG_COUNTER_INVALID);

    if((sp->flags & (SP_FLAG_COUNTER_INVALID | SP_FLAG_COUNT_UNKNOWN)) || !sp->count ||
       sp->end_time_s <= sp->start_time_s || !isfinite(sp->sum))
        return false;

    // Each operation rounds to binary64, including on targets with excess precision or FMA.
    volatile double value = positive ? fabs((double)sp->sum) : (double)sp->sum;
    volatile double mean = value / (double)sp->count;
    volatile double contribution = mean * (double)(sp->end_time_s - sp->start_time_s);
    volatile double result = *counter + contribution;
    if(!isfinite(mean) || !isfinite(contribution) || !isfinite(result))
        return false;
    *counter = result;
    return true;
}

static inline bool storage_counter_fold_legacy(double *counter, const STORAGE_POINT *sp, bool positive) {
    // Legacy counts are the available retained approximation, not a new-format completeness promise.
    STORAGE_POINT retained = *sp;
    retained.flags &= ~SP_FLAG_COUNT_UNKNOWN;
    return storage_counter_fold(counter, &retained, positive);
}

static inline bool storage_point_has_counter_source(const STORAGE_POINT *sp) {
    return (sp->flags & SP_SOURCE_FLAGS) == SP_FLAG_COUNTER_SOURCE;
}

static inline void storage_point_merge_collection(STORAGE_POINT *dst, const STORAGE_POINT *src) {
    if(!dst->end_time_s) {
        dst->start_time_s = src->start_time_s;
        dst->end_time_s = src->end_time_s;
    }
    else {
        dst->start_time_s = MIN(dst->start_time_s, src->start_time_s);
        dst->end_time_s = MAX(dst->end_time_s, src->end_time_s);
    }
    dst->flags |= src->flags & ~SP_FLAG_COUNTER_VALUE_VALID;
    dst->flags |= (src->flags & SP_SOURCE_FLAGS) ? SN_FLAG_NONE : SP_FLAG_ORDINARY_SOURCE;
    dst->flags &= ~SP_FLAG_COUNTER_VALUE_VALID;
    dst->counter = NAN;
    if(!netdata_double_isnumber(src->sum)) {
        dst->flags |= SP_FLAG_HAS_GAP;
        return;
    }
    if(!dst->count) {
        dst->sum = src->sum;
        dst->min = src->min;
        dst->max = src->max;
        dst->count = src->count;
        dst->anomaly_count = src->anomaly_count;
        return;
    }
    dst->sum += src->sum;
    dst->min = MIN(dst->min, src->min);
    dst->max = MAX(dst->max, src->max);
    uint64_t count = (uint64_t)dst->count + src->count;
    uint64_t anomalous = (uint64_t)dst->anomaly_count + src->anomaly_count;
    if(dst->flags & SP_FLAG_COUNTER_SOURCE) {
        if(count > UINT32_MAX) dst->flags |= SP_FLAG_COUNT_UNKNOWN;
        if(anomalous > UINT32_MAX) dst->flags |= SP_FLAG_ANOMALY_COUNT_UNKNOWN;
        dst->count = (uint32_t)MIN(count, UINT32_MAX);
        dst->anomaly_count = (uint32_t)MIN(anomalous, UINT32_MAX);
    }
    else {
        dst->count = (uint32_t)count;
        dst->anomaly_count = (uint32_t)anomalous;
    }
}

#define storage_point_is_unset(x) (!(x).count)
#define storage_point_is_gap(x) (!netdata_double_isnumber((x).sum))
#define storage_point_is_zero(x) (!(x).count || (netdata_double_is_zero((x).min) && netdata_double_is_zero((x).max) && netdata_double_is_zero((x).sum) && (x).anomaly_count == 0))

#define storage_point_merge_to(dst, src) do {           \
        if(storage_point_is_unset(dst))                 \
            (dst) = (src);                              \
                                                        \
        else if(!storage_point_is_unset(src) &&         \
                !storage_point_is_gap(src)) {           \
                                                        \
            if((src).start_time_s < (dst).start_time_s) \
                (dst).start_time_s = (src).start_time_s;\
                                                        \
            if((src).end_time_s > (dst).end_time_s)     \
                (dst).end_time_s = (src).end_time_s;    \
                                                        \
            if((src).min < (dst).min)                   \
                (dst).min = (src).min;                  \
                                                        \
            if((src).max > (dst).max)                   \
                (dst).max = (src).max;                  \
                                                        \
            (dst).sum += (src).sum;                     \
                                                        \
            (dst).count += (src).count;                 \
            (dst).anomaly_count += (src).anomaly_count; \
                                                        \
            (dst).flags |= (src).flags & SN_FLAG_RESET; \
        }                                               \
        (dst).counter = NAN;                            \
        (dst).flags &= ~SP_FLAG_COUNTER_VALUE_VALID;    \
} while(0)

#define storage_point_add_to(dst, src) do {             \
        if(storage_point_is_unset(dst))                 \
            (dst) = (src);                              \
                                                        \
        else if(!storage_point_is_unset(src) &&         \
                !storage_point_is_gap(src)) {           \
                                                        \
            if((src).start_time_s < (dst).start_time_s) \
                (dst).start_time_s = (src).start_time_s;\
                                                        \
            if((src).end_time_s > (dst).end_time_s)     \
                (dst).end_time_s = (src).end_time_s;    \
                                                        \
            (dst).min += (src).min;                     \
            (dst).max += (src).max;                     \
            (dst).sum += (src).sum;                     \
                                                        \
            (dst).count += (src).count;                 \
            (dst).anomaly_count += (src).anomaly_count; \
                                                        \
            (dst).flags |= (src).flags & SN_FLAG_RESET; \
        }                                               \
        (dst).counter = NAN;                            \
        (dst).flags &= ~SP_FLAG_COUNTER_VALUE_VALID;    \
} while(0)

#define storage_point_make_positive(sp) do {            \
        if(!storage_point_is_unset(sp) &&               \
           !storage_point_is_gap(sp)) {                 \
                                                        \
            if(unlikely(signbit((sp).sum)))             \
                (sp).sum = -(sp).sum;                   \
                                                        \
            if(unlikely(signbit((sp).min)))             \
                (sp).min = -(sp).min;                   \
                                                        \
            if(unlikely(signbit((sp).max)))             \
                (sp).max = -(sp).max;                   \
                                                        \
            if(unlikely((sp).min > (sp).max)) {         \
                NETDATA_DOUBLE t = (sp).min;            \
                (sp).min = (sp).max;                    \
                (sp).max = t;                           \
            }                                           \
        }                                               \
} while(0)

#define storage_point_anomaly_rate(sp) \
    (NETDATA_DOUBLE)(storage_point_is_unset(sp) ? 0.0 : (NETDATA_DOUBLE)((sp).anomaly_count) * 100.0 / (NETDATA_DOUBLE)((sp).count))

#define storage_point_average_value(sp) \
    ((sp).count ? (sp).sum / (NETDATA_DOUBLE)((sp).count) : 0.0)


#endif //NETDATA_STORAGE_POINT_H
