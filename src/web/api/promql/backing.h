// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_BACKING_H
#define NETDATA_PROMQL_BACKING_H

#include "memory.h"

typedef struct {
    int fd;
    uint64_t length;
} PROMQL_FILE;

PROMQL_FILE *promql_file_open(const char *directory, const netdata_promql_accounting_context_t *account,
                             uint32_t kind, int *error);
bool promql_file_size(PROMQL_FILE *file, uint64_t length, int *error);
bool promql_file_read(PROMQL_FILE *file, uint64_t offset, void *buffer, size_t length,
                      PROMQL_READ_FRAME *frame, int *error);
bool promql_file_write(PROMQL_FILE *file, uint64_t offset, const void *buffer, size_t length,
                       PROMQL_READ_FRAME *frame, int *error);
void promql_file_close(PROMQL_FILE *file);

#endif
