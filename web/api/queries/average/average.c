// SPDX-License-Identifier: GPL-3.0-or-later

#include "average.h"

// ----------------------------------------------------------------------------
// average

static __thread struct grouping_average {
    int used;
    long resampling_group;
    calculated_number resampling_divisor;
    calculated_number sum;
    size_t count;
} g = {
    .used = 0,
    .resampling_divisor = 1,
    .resampling_group = 1,
    .sum = 0,
    .count = 0
};

void *grouping_create_average(RRDR *r __maybe_unused) {
    if(unlikely(g.used))
        fatal("Grouping functions cannot be used in parallel for multiple queries per thread");

    g.resampling_divisor = r->internal.resampling_divisor;
    g.resampling_group = r->internal.resampling_group;
    g.sum = 0;
    g.count = 0;
    g.used = 1;
    return &g;
}

// resets when switches dimensions
// so, clear everything to restart
void grouping_reset_average(RRDR *r __maybe_unused) {
    g.sum = 0;
    g.count = 0;
}

void grouping_free_average(RRDR *r __maybe_unused) {
    g.used = 0;
    g.sum = 0;
    g.count = 0;
    g.resampling_divisor = 1;
    g.resampling_group = 1;
}

void grouping_add_average(RRDR *r __maybe_unused, calculated_number value) {
    g.sum += value;
    g.count++;
}

calculated_number grouping_flush_average(RRDR *r __maybe_unused,  RRDR_VALUE_FLAGS *rrdr_value_options_ptr __maybe_unused) {
    calculated_number value;

    if(unlikely(!g.count)) {
        value = 0.0;
        *rrdr_value_options_ptr |= RRDR_VALUE_EMPTY;
    }
    else {
        if(unlikely(g.resampling_group != 1)) {
            if (unlikely(r->result_options & RRDR_RESULT_OPTION_VARIABLE_STEP))
                value = g.sum / g.count / g.resampling_divisor;
            else
                value = g.sum / g.resampling_divisor;
        } else
            value = g.sum / g.count;
    }

    g.sum = 0.0;
    g.count = 0;

    return value;
}
