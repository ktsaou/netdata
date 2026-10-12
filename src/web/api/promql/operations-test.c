// SPDX-License-Identifier: GPL-3.0-or-later

#include "operations.h"

static int control_status(netdata_promql_control_t *control, uint32_t expected) {
    netdata_promql_control_state_t state = {0};
    return netdata_promql_control_state(control, &state) != NETDATA_PROMQL_OK || state.status != expected;
}

static int operation_result(const char *name, int errors) {
    printf("{\"operation_case\":\"%s\",\"errors\":%d}\n", name, errors);
    return errors;
}

static int closed_socket_case(netdata_promql_engine_t *engine) {
    int sockets[2];
    if(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets)) return operation_result("socket-close", 1);
    PROMQL_OPERATION operation;
    int errors = promql_operation_begin(&operation, engine, sockets[0], NULL) != NETDATA_PROMQL_OK;
    if(!errors) {
        errors += promql_operation_timeout(&operation, engine, INT64_C(20000000000)) != NETDATA_PROMQL_OK;
        close(sockets[1]); sockets[1] = -1;
        promql_operations_tick();
        errors += control_status(operation.control, NETDATA_PROMQL_CANCELLED);
        errors += control_status(operation.pending, NETDATA_PROMQL_CANCELLED);
        promql_operation_activate(&operation);
        errors += control_status(operation.control, NETDATA_PROMQL_CANCELLED);
    }
    promql_operation_end(&operation);
    close(sockets[0]);
    if(sockets[1] >= 0) close(sockets[1]);
    errors += promql_operations_active();
    return operation_result("socket-close", errors);
}

static int retired_id_case(netdata_promql_engine_t *engine) {
    int sockets[2];
    if(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets)) return operation_result("retired-id-same-fd", 1);
    PROMQL_OPERATION first, second;
    int errors = promql_operation_begin(&first, engine, sockets[0], NULL) != NETDATA_PROMQL_OK;
    uint64_t retired = first.id;
    promql_operation_end(&first);
    errors += promql_operation_begin(&second, engine, sockets[0], NULL) != NETDATA_PROMQL_OK;
    errors += !retired || second.id <= retired;
    promql_operations_cancel_id(retired);
    promql_operations_tick();
    errors += control_status(second.control, NETDATA_PROMQL_OK);
    promql_operation_end(&second);
    close(sockets[0]); close(sockets[1]);
    errors += promql_operations_active();
    return operation_result("retired-id-same-fd", errors);
}

static bool duration_after(netdata_promql_duration_t a, netdata_promql_duration_t b) {
    return a.seconds > b.seconds || (a.seconds == b.seconds && a.nanoseconds > b.nanoseconds);
}

static int deadline_case(netdata_promql_engine_t *engine, bool caller_first) {
    netdata_promql_control_options_t options;
    int errors = netdata_promql_control_options_default(&options) != NETDATA_PROMQL_OK;
    options.deadline_present = 1;
    options.seconds = caller_first ? 10 : 20;
    PROMQL_OPERATION operation;
    errors += promql_operation_begin(&operation, engine, -1, &options) != NETDATA_PROMQL_OK;
    errors += promql_operation_timeout(&operation, engine,
        caller_first ? INT64_C(20000000000) : INT64_C(10000000000)) != NETDATA_PROMQL_OK;
    netdata_promql_control_state_t before = {0}, after = {0};
    errors += netdata_promql_control_state(caller_first ? operation.control : operation.pending, &before) != NETDATA_PROMQL_OK;
    promql_operation_activate(&operation);
    errors += netdata_promql_control_state(operation.control, &after) != NETDATA_PROMQL_OK;
    errors += before.status != NETDATA_PROMQL_OK || after.status != NETDATA_PROMQL_OK || !after.deadline_present ||
        duration_after(after.remaining, before.remaining) || operation.pending != NULL;
    promql_operation_end(&operation);
    return operation_result(caller_first ? "caller-deadline-first" : "uri-deadline-first", errors);
}

static int cancel_pending_case(netdata_promql_engine_t *engine) {
    PROMQL_OPERATION operation;
    int errors = promql_operation_begin(&operation, engine, -1, NULL) != NETDATA_PROMQL_OK;
    promql_operations_cancel_id(operation.id);
    errors += promql_operation_timeout(&operation, engine, INT64_C(10000000000)) != NETDATA_PROMQL_OK;
    errors += control_status(operation.pending, NETDATA_PROMQL_CANCELLED);
    promql_operation_activate(&operation);
    errors += control_status(operation.control, NETDATA_PROMQL_CANCELLED);
    promql_operation_end(&operation);
    return operation_result("cancel-before-activation", errors);
}

typedef struct { bool stop; uint64_t ticks; } WATCHER;

static void *watcher_main(void *data) {
    WATCHER *watcher = data;
    do {
        promql_operations_tick();
        __atomic_add_fetch(&watcher->ticks, 1, __ATOMIC_RELEASE);
    } while(!__atomic_load_n(&watcher->stop, __ATOMIC_ACQUIRE));
    return NULL;
}

static int concurrent_retire_case(netdata_promql_engine_t *engine) {
    int sockets[2];
    if(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets)) return operation_result("watcher-retirement", 1);
    WATCHER watcher = {0};
    pthread_t thread;
    int errors = pthread_create(&thread, NULL, watcher_main, &watcher) != 0;
    if(!errors) {
        uint64_t retired = 0;
        for(size_t i = 0; i < 256; i++) {
            PROMQL_OPERATION operation;
            errors += promql_operation_begin(&operation, engine, sockets[0], NULL) != NETDATA_PROMQL_OK;
            promql_operations_cancel_id(retired);
            errors += control_status(operation.control, NETDATA_PROMQL_OK);
            retired = operation.id;
            promql_operation_end(&operation);
        }
        __atomic_store_n(&watcher.stop, true, __ATOMIC_RELEASE);
        errors += pthread_join(thread, NULL) != 0;
        errors += !__atomic_load_n(&watcher.ticks, __ATOMIC_ACQUIRE);
    }
    close(sockets[0]); close(sockets[1]);
    errors += promql_operations_active();
    return operation_result("watcher-retirement", errors);
}

static int shutdown_case(netdata_promql_engine_t *engine) {
    PROMQL_OPERATION first, second, refused;
    int errors = promql_operation_begin(&first, engine, -1, NULL) != NETDATA_PROMQL_OK;
    errors += promql_operation_begin(&second, engine, -1, NULL) != NETDATA_PROMQL_OK;
    errors += promql_operation_timeout(&second, engine, INT64_C(10000000000)) != NETDATA_PROMQL_OK;
    promql_operations_shutdown();
    errors += control_status(first.control, NETDATA_PROMQL_CANCELLED);
    errors += control_status(second.control, NETDATA_PROMQL_CANCELLED);
    errors += control_status(second.pending, NETDATA_PROMQL_CANCELLED);
    errors += promql_operation_begin(&refused, engine, -1, NULL) != NETDATA_PROMQL_CANCELLED;
    errors += !!refused.control || !!refused.id;
    promql_operation_end(&refused);
    promql_operation_end(&first); promql_operation_end(&second);
    errors += promql_operations_active();
    netdata_promql_engine_observations_t observed = {0};
    errors += netdata_promql_engine_observations(engine, &observed) != NETDATA_PROMQL_OK;
    for(size_t i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++)
        if(i != NETDATA_PROMQL_MEMORY_ACCOUNTING) errors += observed.memory[i].current != 0;
    return operation_result("shutdown-admission", errors);
}

int promql_operations_fixture(netdata_promql_engine_t *engine) {
    int errors = closed_socket_case(engine);
    errors += retired_id_case(engine);
    errors += deadline_case(engine, true);
    errors += deadline_case(engine, false);
    errors += cancel_pending_case(engine);
    errors += concurrent_retire_case(engine);
    errors += shutdown_case(engine);
    return errors;
}
