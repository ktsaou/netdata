// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_CHECKPOINT_H
#define NETDATA_PROMQL_CHECKPOINT_H

#include "backing.h"

typedef struct {
    int64_t next;
    uint64_t counter_bits;
} PROMQL_CHECKPOINT;

typedef struct promql_checkpoints PROMQL_CHECKPOINTS;

typedef struct {
    PROMQL_FILE *file;
    void *window;
    size_t window_bytes;
} PROMQL_CHECKPOINT_BACKING;

bool promql_checkpoints_append(PROMQL_CHECKPOINTS **owner, PROMQL_CHECKPOINT_BACKING *backing,
                               const char *directory, PROMQL_CHECKPOINT value, PROMQL_READ_FRAME *frame, int *error);
bool promql_checkpoints_find(const PROMQL_CHECKPOINTS *owner, PROMQL_CHECKPOINT_BACKING *backing,
                             int64_t wanted, PROMQL_CHECKPOINT *value, bool *found,
                             PROMQL_READ_FRAME *frame, int *error);
const PROMQL_CHECKPOINT *promql_checkpoints_last(const PROMQL_CHECKPOINTS *owner);
const PROMQL_CHECKPOINT *promql_checkpoints_recent(const PROMQL_CHECKPOINTS *owner);
void promql_checkpoints_remember(PROMQL_CHECKPOINTS *owner, PROMQL_CHECKPOINT value);
void promql_checkpoints_free(PROMQL_CHECKPOINTS *owner);
void promql_checkpoint_backing_close(PROMQL_CHECKPOINT_BACKING *backing);

#endif
