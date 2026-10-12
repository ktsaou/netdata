// SPDX-License-Identifier: GPL-3.0-or-later

#ifndef NETDATA_PROMQL_OPERATIONS_H
#define NETDATA_PROMQL_OPERATIONS_H

#include "memory.h"

typedef struct promql_operation {
    struct promql_operation *next;
    uint64_t id;
    int fd;
    netdata_promql_control_t *control, *pending;
} PROMQL_OPERATION;

uint32_t promql_operation_begin(PROMQL_OPERATION *operation, netdata_promql_engine_t *engine, int fd,
                                 const netdata_promql_control_options_t *options);
uint32_t promql_operation_timeout(PROMQL_OPERATION *operation, netdata_promql_engine_t *engine, int64_t duration_ns);
void promql_operation_activate(PROMQL_OPERATION *operation);
void promql_operation_end(PROMQL_OPERATION *operation);
void promql_operations_cancel_id(uint64_t id);
void promql_operations_tick(void);
void promql_operations_shutdown(void);
bool promql_operations_active(void);

#endif
