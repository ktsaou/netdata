// SPDX-License-Identifier: GPL-3.0-or-later

// Backing offsets must remain wide on 32-bit hosts.
#ifndef _FILE_OFFSET_BITS
#define _FILE_OFFSET_BITS 64
#endif

#include "backing.h"

_Static_assert(sizeof(off_t) >= sizeof(int64_t), "PromQL backing requires 64-bit file offsets");

static bool file_offset(uint64_t value, off_t *out, int *error) {
    *out = (off_t)value;
    if(*out < 0 || (uint64_t)*out != value) {
        *error = EOVERFLOW;
        return false;
    }
    return true;
}

PROMQL_FILE *promql_file_open(const char *directory, const netdata_promql_accounting_context_t *account,
                             uint32_t kind, int *error) {
    *error = 0;
    if(!directory || !*directory) { *error = ENOENT; return NULL; }
    PROMQL_FILE *file = promql_malloc(account, kind, sizeof(*file));
    if(!file) { *error = ENOMEM; return NULL; }
    file->fd = -1;
#ifdef O_TMPFILE
    file->fd = open(directory, O_TMPFILE | O_RDWR | O_CLOEXEC, 0600);
#endif
    if(file->fd == -1) {
        size_t bytes = strlen(directory);
        if(bytes > SIZE_MAX - sizeof("/promql-XXXXXX")) {
            *error = EOVERFLOW;
            promql_free(file);
            return NULL;
        }
        char *name = promql_malloc(account, kind, bytes + sizeof("/promql-XXXXXX"));
        if(!name) { *error = ENOMEM; promql_free(file); return NULL; }
        memcpy(name, directory, bytes);
        memcpy(name + bytes, "/promql-XXXXXX", sizeof("/promql-XXXXXX"));
        file->fd = mkstemp(name);
        if(file->fd == -1) *error = errno;
        else if(unlink(name) || fcntl(file->fd, F_SETFD, FD_CLOEXEC) == -1) {
            *error = errno;
            close(file->fd);
            file->fd = -1;
        }
        promql_free(name);
    }
    if(file->fd == -1) { promql_free(file); return NULL; }
    return file;
}

bool promql_file_size(PROMQL_FILE *file, uint64_t length, int *error) {
    off_t position;
    if(!file_offset(length, &position, error)) return false;
    if(ftruncate(file->fd, position)) { *error = errno; return false; }
    file->length = length;
    return true;
}

static bool file_transfer(PROMQL_FILE *file, uint64_t offset, void *buffer, size_t length,
                          bool write, PROMQL_READ_FRAME *frame, int *error) {
    *error = 0;
    if(length > UINT64_MAX - offset) { *error = EOVERFLOW; return false; }
    if(!write && (offset > file->length || length > file->length - offset)) {
        *error = EINVAL;
        return false;
    }
    while(length) {
        if(frame && !promql_read_poll(frame)) { *error = ECANCELED; return false; }
        off_t position;
        if(!file_offset(offset, &position, error)) return false;
        size_t count = MIN(length, (size_t)65536);
#ifdef NETDATA_INTERNAL_CHECKS
        if(write) file->observations.write_calls++;
        else file->observations.read_calls++;
#endif
        ssize_t transferred = write ? pwrite(file->fd, buffer, count, position) : pread(file->fd, buffer, count, position);
        if(transferred == -1 && errno == EINTR) continue;
        if(transferred <= 0) { *error = transferred ? errno : EIO; return false; }
#ifdef NETDATA_INTERNAL_CHECKS
        if(write) file->observations.write_bytes += (uint64_t)transferred;
        else file->observations.read_bytes += (uint64_t)transferred;
#endif
        offset += (size_t)transferred;
        buffer = (uint8_t *)buffer + transferred;
        length -= (size_t)transferred;
    }
    if(write) file->length = MAX(file->length, offset);
    return true;
}

bool promql_file_read(PROMQL_FILE *file, uint64_t offset, void *buffer, size_t length,
                      PROMQL_READ_FRAME *frame, int *error) {
    return file_transfer(file, offset, buffer, length, false, frame, error);
}

bool promql_file_write(PROMQL_FILE *file, uint64_t offset, const void *buffer, size_t length,
                       PROMQL_READ_FRAME *frame, int *error) {
    return file_transfer(file, offset, (void *)buffer, length, true, frame, error);
}

void promql_file_close(PROMQL_FILE *file) {
    if(!file) return;
    if(file->fd != -1) close(file->fd);
    promql_free(file);
}
