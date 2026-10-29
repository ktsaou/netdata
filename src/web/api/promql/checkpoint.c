// SPDX-License-Identifier: GPL-3.0-or-later

#include "checkpoint.h"

#define PROMQL_CHECKPOINT_INLINE 16U
#define PROMQL_CHECKPOINT_WINDOW 8192U

_Static_assert(sizeof(PROMQL_CHECKPOINT) == 16, "PromQL checkpoints require fixed-width records");

struct promql_checkpoints {
    uint64_t count;
    PROMQL_CHECKPOINT first, last, recent;
    uint64_t base, capacity;
    PROMQL_CHECKPOINT points[];
};

static bool checkpoint_poll(PROMQL_READ_FRAME *frame, int *error) {
    if(promql_read_poll(frame)) return true;
    *error = ECANCELED;
    return false;
}

static bool checkpoint_offset(uint64_t base, uint64_t index, uint64_t *offset, int *error) {
    if(index > UINT64_MAX / sizeof(PROMQL_CHECKPOINT) ||
       index * sizeof(PROMQL_CHECKPOINT) > UINT64_MAX - base) {
        *error = EOVERFLOW;
        return false;
    }
    *offset = base + index * sizeof(PROMQL_CHECKPOINT);
    return true;
}

static bool checkpoint_window(PROMQL_CHECKPOINT_BACKING *backing, uint64_t bytes,
                               PROMQL_READ_FRAME *frame, int *error) {
    size_t needed = (size_t)MIN(bytes, (uint64_t)PROMQL_CHECKPOINT_WINDOW);
    if(backing->window_bytes >= needed) return true;
    if(!checkpoint_poll(frame, error)) return false;
    // No checkpoint records live in this disposable copy window between relocations.
    promql_free(backing->window);
    backing->window = NULL;
    backing->window_bytes = 0;
    void *window = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, needed);
    if(!window) { *error = ENOMEM; return false; }
    backing->window = window;
    backing->window_bytes = needed;
    return true;
}

static bool checkpoint_copy(const PROMQL_CHECKPOINTS *owner, PROMQL_CHECKPOINT_BACKING *backing,
                              uint64_t destination, PROMQL_READ_FRAME *frame, int *error) {
    uint64_t bytes;
    if(!checkpoint_offset(0, owner->count, &bytes, error)) return false;
    if(owner->count <= PROMQL_CHECKPOINT_INLINE)
        return promql_file_write(backing->file, destination, owner->points, (size_t)bytes, frame, error);
    if(!checkpoint_window(backing, bytes, frame, error)) return false;
    uint64_t copied = 0;
    while(copied < bytes) {
        size_t length = (size_t)MIN(bytes - copied, (uint64_t)backing->window_bytes);
        if(!promql_file_read(backing->file, owner->base + copied, backing->window, length, frame, error) ||
           !promql_file_write(backing->file, destination + copied, backing->window, length, frame, error))
            return false;
        copied += length;
    }
    return true;
}

bool promql_checkpoints_append(PROMQL_CHECKPOINTS **handle, PROMQL_CHECKPOINT_BACKING *backing,
                               const char *directory, PROMQL_CHECKPOINT value, PROMQL_READ_FRAME *frame, int *error) {
    *error = 0;
    if(!checkpoint_poll(frame, error)) return false;
    PROMQL_CHECKPOINTS *owner = *handle;
    if(owner && value.next <= owner->last.next) return true;
    if(!owner) {
        owner = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD,
                               sizeof(*owner) + PROMQL_CHECKPOINT_INLINE * sizeof(value));
        if(!owner) { *error = ENOMEM; return false; }
        owner->count = 1;
        owner->first = owner->last = owner->recent = owner->points[0] = value;
        if(!checkpoint_poll(frame, error)) { promql_free(owner); return false; }
        *handle = owner;
        return true;
    }
    if(owner->count < PROMQL_CHECKPOINT_INLINE) {
        owner->points[owner->count] = value;
        if(!checkpoint_poll(frame, error)) return false;
        owner->last = value;
        owner->count++;
        return true;
    }
    if(owner->count == UINT64_MAX) { *error = EOVERFLOW; return false; }

    PROMQL_CHECKPOINTS *replacement = NULL;
    if(owner->count == PROMQL_CHECKPOINT_INLINE) {
        replacement = promql_malloc(frame->account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, sizeof(*replacement));
        if(!replacement) { *error = ENOMEM; return false; }
        *replacement = *owner;
    }
    uint64_t base = owner->base, capacity = owner->capacity;
    if(replacement || owner->count == capacity) {
        uint64_t previous = capacity ? capacity : PROMQL_CHECKPOINT_INLINE;
        if(previous > UINT64_MAX / 2) { *error = EOVERFLOW; goto failed; }
        capacity = previous * 2;
        if(!backing->file) {
            backing->file = promql_file_open(directory, frame->account, NETDATA_PROMQL_MEMORY_PROVIDER_HELD, error);
            if(!backing->file) goto failed;
        }
        base = backing->file->length;
        uint64_t end;
        if(!checkpoint_offset(base, capacity, &end, error) ||
           !promql_file_size(backing->file, end, error) ||
           !checkpoint_copy(owner, backing, base, frame, error))
            goto failed;
    }
    uint64_t offset;
    if(!checkpoint_offset(base, owner->count, &offset, error) ||
       !promql_file_write(backing->file, offset, &value, sizeof(value), frame, error) ||
       !checkpoint_poll(frame, error))
        goto failed;

    PROMQL_CHECKPOINTS *published = replacement ? replacement : owner;
    published->base = base;
    published->capacity = capacity;
    published->last = value;
    published->count++;
    if(replacement) {
        // The original prefix remains valid until every write and final control check succeeds.
        *handle = replacement;
        promql_free(owner);
    }
    return true;

failed:
    promql_free(replacement);
    return false;
}

bool promql_checkpoints_find(const PROMQL_CHECKPOINTS *owner, PROMQL_CHECKPOINT_BACKING *backing,
                             int64_t wanted, PROMQL_CHECKPOINT *value, bool *found,
                             PROMQL_READ_FRAME *frame, int *error) {
    *error = 0;
    *found = false;
    *value = (PROMQL_CHECKPOINT){0};
    if(!checkpoint_poll(frame, error)) return false;
    if(!owner) return true;
    *value = owner->first;
    if(wanted < owner->first.next) return true;
    *found = true;
    if(wanted == owner->first.next) return true;
    if(wanted >= owner->last.next) { *value = owner->last; return true; }
    uint64_t low = 0, high = owner->count;
    while(low < high) {
        if(!checkpoint_poll(frame, error)) return false;
        uint64_t middle = low + (high - low) / 2;
        PROMQL_CHECKPOINT point;
        if(owner->count <= PROMQL_CHECKPOINT_INLINE) point = owner->points[(size_t)middle];
        else {
            uint64_t offset;
            if(!checkpoint_offset(owner->base, middle, &offset, error) ||
               !promql_file_read(backing->file, offset, &point, sizeof(point), frame, error))
                return false;
        }
        if(point.next <= wanted) { *value = point; low = middle + 1; }
        else high = middle;
    }
    return true;
}

const PROMQL_CHECKPOINT *promql_checkpoints_last(const PROMQL_CHECKPOINTS *owner) {
    return owner ? &owner->last : NULL;
}

const PROMQL_CHECKPOINT *promql_checkpoints_recent(const PROMQL_CHECKPOINTS *owner) {
    return owner ? &owner->recent : NULL;
}

void promql_checkpoints_remember(PROMQL_CHECKPOINTS *owner, PROMQL_CHECKPOINT value) {
    owner->recent = value;
}

void promql_checkpoints_free(PROMQL_CHECKPOINTS *owner) {
    promql_free(owner);
}

void promql_checkpoint_backing_close(PROMQL_CHECKPOINT_BACKING *backing) {
    promql_free(backing->window);
    promql_file_close(backing->file);
    *backing = (PROMQL_CHECKPOINT_BACKING){0};
}
