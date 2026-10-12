// SPDX-License-Identifier: GPL-3.0-or-later

#include "operations.h"

static pthread_mutex_t operations_lock = PTHREAD_MUTEX_INITIALIZER;
static PROMQL_OPERATION *operations;
static uint64_t next_id;
static bool stopping;

static void operation_cancel(PROMQL_OPERATION *operation) {
    netdata_promql_control_cancel(operation->control);
    if(operation->pending) netdata_promql_control_cancel(operation->pending);
}

uint32_t promql_operation_begin(PROMQL_OPERATION *operation, netdata_promql_engine_t *engine, int fd,
                                 const netdata_promql_control_options_t *options) {
    *operation = (PROMQL_OPERATION){.fd = fd};
    pthread_mutex_lock(&operations_lock);
    uint32_t status = stopping ? NETDATA_PROMQL_CANCELLED : next_id == UINT64_MAX ? NETDATA_PROMQL_INTERNAL_ERROR :
        netdata_promql_control_create(engine, options, &operation->control);
    if(status == NETDATA_PROMQL_OK) {
        operation->id = ++next_id;
        operation->next = operations;
        operations = operation;
    }
    pthread_mutex_unlock(&operations_lock);
    return status;
}

uint32_t promql_operation_timeout(PROMQL_OPERATION *operation, netdata_promql_engine_t *engine, int64_t duration_ns) {
    netdata_promql_control_options_t options;
    uint32_t status = netdata_promql_control_options_default(&options);
    if(status != NETDATA_PROMQL_OK) return status;
    options.deadline_present = 1;
    if(duration_ns > 0) {
        options.seconds = (uint64_t)duration_ns / UINT64_C(1000000000);
        options.nanoseconds = (uint32_t)((uint64_t)duration_ns % UINT64_C(1000000000));
    }
    netdata_promql_control_t *pending = NULL;
    status = netdata_promql_control_create(engine, &options, &pending);
    if(status != NETDATA_PROMQL_OK) return status;
    pthread_mutex_lock(&operations_lock);
    netdata_promql_control_state_t state;
    if(stopping || netdata_promql_control_state(operation->control, &state) != NETDATA_PROMQL_OK ||
       state.status == NETDATA_PROMQL_CANCELLED) netdata_promql_control_cancel(pending);
    operation->pending = pending;
    pthread_mutex_unlock(&operations_lock);
    return NETDATA_PROMQL_OK;
}

void promql_operation_activate(PROMQL_OPERATION *operation) {
    pthread_mutex_lock(&operations_lock);
    netdata_promql_control_t *old = NULL;
    if(operation->pending) {
        netdata_promql_control_state_t current, pending;
        bool keep_current = netdata_promql_control_state(operation->control, &current) == NETDATA_PROMQL_OK &&
            netdata_promql_control_state(operation->pending, &pending) == NETDATA_PROMQL_OK &&
            (current.status == NETDATA_PROMQL_CANCELLED || (current.deadline_present &&
             (current.status == NETDATA_PROMQL_DEADLINE || current.remaining.seconds < pending.remaining.seconds ||
              (current.remaining.seconds == pending.remaining.seconds &&
               current.remaining.nanoseconds <= pending.remaining.nanoseconds))));
        old = keep_current ? operation->pending : operation->control;
        if(!keep_current) operation->control = operation->pending;
        operation->pending = NULL;
    }
    pthread_mutex_unlock(&operations_lock);
    if(old) netdata_promql_control_release(old);
}

void promql_operation_end(PROMQL_OPERATION *operation) {
    pthread_mutex_lock(&operations_lock);
    PROMQL_OPERATION **link = &operations;
    while(*link && *link != operation) link = &(*link)->next;
    if(*link) *link = operation->next;
    operation->id = 0;
    pthread_mutex_unlock(&operations_lock);
    if(operation->pending) netdata_promql_control_release(operation->pending);
    if(operation->control) netdata_promql_control_release(operation->control);
    operation->pending = operation->control = NULL;
}

void promql_operations_cancel_id(uint64_t id) {
    if(!id) return;
    pthread_mutex_lock(&operations_lock);
    for(PROMQL_OPERATION *operation = operations; operation; operation = operation->next)
        if(operation->id == id) { operation_cancel(operation); break; }
    pthread_mutex_unlock(&operations_lock);
}

void promql_operations_tick(void) {
    pthread_mutex_lock(&operations_lock);
    for(PROMQL_OPERATION *operation = operations; operation; operation = operation->next)
        if(operation->fd >= 0 && is_socket_closed(operation->fd)) operation_cancel(operation);
    pthread_mutex_unlock(&operations_lock);
}

void promql_operations_shutdown(void) {
    pthread_mutex_lock(&operations_lock);
    stopping = true;
    for(PROMQL_OPERATION *operation = operations; operation; operation = operation->next) operation_cancel(operation);
    pthread_mutex_unlock(&operations_lock);
}

bool promql_operations_active(void) {
    pthread_mutex_lock(&operations_lock);
    bool active = operations != NULL;
    pthread_mutex_unlock(&operations_lock);
    return active;
}
