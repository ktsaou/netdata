// SPDX-License-Identifier: GPL-3.0-or-later

#include "provider-internal.h"
#include "api-internal.h"
#include "database/sqlite/sqlite_functions.h"
#include "database/sqlite/sqlite_metadata.h"

extern __thread sqlite3 *db_meta_thread, *db_context_thread;

typedef struct {
    const char *table, *fragment, *guid;
    bool prepare, owner;
    unsigned statements, rows;
    int errors;
} LOAD_CHECK;

static int refuse_reader(void *data, int action, const char *table, const char *column __maybe_unused,
                          const char *database __maybe_unused, const char *trigger __maybe_unused) {
    LOAD_CHECK *check = data;
    return check->prepare && action == SQLITE_READ && table && !strcmp(table, check->table) ? SQLITE_DENY : SQLITE_OK;
}

static int load_trace(unsigned event, void *data, void *statement, void *unused __maybe_unused) {
    LOAD_CHECK *check = data;
    const char *sql = sqlite3_sql(statement);
    if(!sql || !strstr(sql, check->fragment)) return 0;
    if(event == SQLITE_TRACE_STMT) {
        check->statements++;
        if(check->owner) {
            rrd_rdlock();
            RRDHOST *host = rrdhost_find_by_guid(check->guid);
            check->errors += !host || __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != RRD_CONTEXT_LOAD_LOADING;
            if(host) {
                bool writable = rw_spinlock_trywrite_lock(&host->metadata_lifetime_lock);
                check->errors += writable;
                if(writable) rw_spinlock_write_unlock(&host->metadata_lifetime_lock);
                unsigned before = check->statements;
                rrdhost_load_rrdcontext_data(host, false);
                check->errors += check->statements != before ||
                    __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != RRD_CONTEXT_LOAD_LOADING;
            }
            rrd_rdunlock();
        }
    }
    if(event == SQLITE_TRACE_ROW) {
        check->rows++;
        if(!check->owner && !check->prepare && check->rows == 1) sqlite3_interrupt(sqlite3_db_handle(statement));
    }
    return 0;
}

static bool insert_host(sqlite3 *db, const char *sql, nd_uuid_t uuid) {
    sqlite3_stmt *statement = NULL;
    bool ok = sqlite3_prepare_v2(db, sql, -1, &statement, NULL) == SQLITE_OK &&
        sqlite3_bind_blob(statement, 1, uuid, sizeof(nd_uuid_t), SQLITE_STATIC) == SQLITE_OK && sqlite3_step(statement) == SQLITE_DONE;
    if(statement) sqlite3_finalize(statement);
    return ok;
}

static bool seed_readers(nd_uuid_t uuid) {
    const char *contexts = "INSERT INTO context VALUES('fixture.a',?1,1,'load','line','KiB',1,0,0,0,'fixture'),"
        "('fixture.b',?1,1,'load','line','KiB',1,0,0,0,'fixture')";
    const char *charts = "INSERT INTO chart VALUES(X'10000000000040008000000000000001',?1,'load','a','a','fixture.a','load','KiB',1,1,0,'fixture'),"
        "(X'10000000000040008000000000000002',?1,'load','b','b','fixture.b','load','KiB',1,1,0,'fixture')";
    const char *dimensions = "INSERT INTO dimension VALUES(X'20000000000040008000000000000001',X'10000000000040008000000000000001','a','a','',0),"
        "(X'20000000000040008000000000000002',X'10000000000040008000000000000002','b','b','',0)";
    return sqlite3_exec(db_context_thread, "DELETE FROM context", NULL, NULL, NULL) == SQLITE_OK &&
        sqlite3_exec(db_meta_thread, "DELETE FROM chart; DELETE FROM dimension", NULL, NULL, NULL) == SQLITE_OK &&
        insert_host(db_context_thread, contexts, uuid) && insert_host(db_meta_thread, charts, uuid) &&
        sqlite3_exec(db_meta_thread, dimensions, NULL, NULL, NULL) == SQLITE_OK;
}

static int unavailable_response(RRDHOST *host, bool metadata, bool zero_limit) {
    size_t memory = 0;
    struct web_client *client = web_client_create(&memory);
    client->mode = HTTP_REQUEST_MODE_GET;
    client->promql_scoped = true;
    memcpy(client->promql_host_guid, host->machine_guid, sizeof(client->promql_host_guid));
    buffer_strcat(client->url_as_received, metadata ? (zero_limit ? "/api/v1/metadata?limit=0" : "/api/v1/metadata") : "/api/v1/labels?match[]=fixture_a");
    int errors = (metadata ? api_v1_promql_metadata(host, client, NULL) : api_v1_promql_labels(host, client, NULL)) != 500;
    PROMQL_BODY *body = client->response.promql_body;
    char text[1024];
    uint64_t length = promql_body_length(body);
    int io = 0;
    if(body && length < sizeof(text) && promql_body_read(body, 0, text, (size_t)length, NULL, &io)) {
        text[length] = 0;
        errors += !strstr(text, "initial host metadata load is unavailable");
    }
    else errors++;
    web_client_free(client);
    errors += memory != 0 || promql_operations_active();
    return errors;
}

static int unavailable_discovery(RRDHOST *host) {
    return unavailable_response(host, false, false) + unavailable_response(host, true, false) +
        unavailable_response(host, true, true);
}

typedef struct {
    PROMQL_PROVIDER_QUERY query;
    RRDHOST *host;
    PROMQL_HOST *first, *alias;
    bool duplicate;
    int errors;
} HOST_REFERENCES;

static void acquire_references(void *data, void *unused __maybe_unused,
                                const netdata_promql_accounting_context_t *account) {
    HOST_REFERENCES *references = data;
    bool busy, unavailable;
    references->first = promql_host_acquire(&references->query, references->host, account, &busy, &unavailable);
    references->errors += !references->first || busy || unavailable;
    if(references->duplicate) {
        references->alias = promql_host_acquire(&references->query, references->host, account, &busy, &unavailable);
        references->errors += references->alias != references->first || busy || unavailable;
    }
}

static int write_available(RRDHOST *host, bool expected) {
    bool writable = rw_spinlock_trywrite_lock(&host->metadata_lifetime_lock);
    if(writable) rw_spinlock_write_unlock(&host->metadata_lifetime_lock);
    return writable != expected;
}

static int query_references_case(RRDHOST *host) {
    netdata_promql_engine_t *engine = NULL;
    int errors = netdata_promql_engine_create(NULL, &engine) != NETDATA_PROMQL_OK;
    netdata_promql_report_t *reports[2] = {0};
    HOST_REFERENCES references[2] = {{.host = host, .duplicate = true}, {.host = host}};
    int64_t limit;
    if(engine) {
        for(size_t i = 0; i < 2; i++) {
            errors += netdata_promql_api_parse_limit(engine, (netdata_promql_bytes_t){0}, NULL, &limit, &reports[i]) != NETDATA_PROMQL_OK;
            if(reports[i]) errors += netdata_promql_report_accounted(reports[i], acquire_references, &references[i], NULL) != NETDATA_PROMQL_OK;
            errors += references[i].errors;
        }
        errors += write_available(host, false);
        promql_host_release(&references[0].query, references[0].first);
        errors += write_available(host, false);
        promql_host_release(&references[0].query, references[0].alias);
        errors += write_available(host, false);
        promql_host_release(&references[1].query, references[1].first);
        errors += write_available(host, true);
        for(size_t i = 0; i < 2; i++) if(reports[i]) netdata_promql_report_release(reports[i]);
        netdata_promql_engine_observations_t observed = {0};
        errors += netdata_promql_engine_observations(engine, &observed) != NETDATA_PROMQL_OK;
        for(size_t i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++)
            if(i != NETDATA_PROMQL_MEMORY_ACCOUNTING) errors += observed.memory[i].current != 0;
        netdata_promql_engine_release(engine);
    }
    printf("{\"host_readiness_case\":\"query-references\",\"errors\":%d}\n", errors);
    fflush(stdout);
    return errors;
}

static int host_case(unsigned number, const char *table, const char *fragment, bool prepare, bool owner) {
    char guid[UUID_STR_LEN];
    snprintf(guid, sizeof(guid), "7030f515-956f-49d2-9c86-%012u", number);
    nd_uuid_t uuid;
    int errors = uuid_parse(guid, uuid) != 0 || !seed_readers(uuid);
    LOAD_CHECK check = {.table = table, .fragment = fragment, .guid = guid, .prepare = prepare, .owner = owner};
    sqlite3 *selected = !strcmp(table, "context") ? db_context_thread : db_meta_thread;
    errors += sqlite3_set_authorizer(selected, refuse_reader, &check) != SQLITE_OK;
    errors += sqlite3_trace_v2(selected, SQLITE_TRACE_STMT | SQLITE_TRACE_ROW, load_trace, &check) != SQLITE_OK;
    RRDHOST *host = errors ? NULL : rrdhost_find_or_create("load-fixture", "load-fixture", guid,
        "linux", "UTC", "UTC", 0, "fixture", "fixture", 1, default_rrd_history_entries, RRD_DB_MODE_DBENGINE,
        false, false, NULL, NULL, NULL, false, 0, 0, NULL, false);
    if(host) {
        unsigned statements = check.statements;
        DICTIONARY *contexts = host->rrdctx.contexts;
        RRD_CONTEXT_LOAD_STATE expected = owner ? RRD_CONTEXT_LOAD_COMPLETE : RRD_CONTEXT_LOAD_UNAVAILABLE;
        errors += __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != expected;
        rrdhost_load_rrdcontext_data(host, false);
        errors += check.statements != statements || host->rrdctx.contexts != contexts ||
            __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != expected;
        if(!owner) errors += unavailable_discovery(host);
        else errors += query_references_case(host);
        errors += owner ? check.rows != 2 : prepare ? check.rows != 0 : check.rows != 1;
    }
    else errors++;
    sqlite3_trace_v2(selected, 0, NULL, NULL);
    sqlite3_set_authorizer(selected, NULL, NULL);
    errors += check.errors;
    if(host) {
        bool writable = rw_spinlock_trywrite_lock(&host->metadata_lifetime_lock);
        if(writable) rrdhost_free___consume_metadata_lifetime_writelock(host);
        else errors++;
        rrd_rdlock(); errors += rrdhost_find_by_guid(guid) != NULL; rrd_rdunlock();
    }
    printf("{\"host_readiness_case\":\"%s-%s\",\"errors\":%d,\"statements\":%u,\"rows\":%u}\n",
        table, owner ? "owner" : prepare ? "prepare" : "step", errors, check.statements, check.rows);
    fflush(stdout);
    return errors;
}

typedef struct {
    pthread_mutex_t lock;
    uint64_t id;
    bool done;
    int expected, errors;
} METADATA_WAITER;

typedef struct {
    pthread_mutex_t lock;
    pthread_cond_t changed;
    RRDHOST *host;
    sqlite3 *meta, *context;
    bool entered, resume;
    unsigned statements;
    int errors;
} METADATA_LOADER;

static void waiter_operation(void *data, uint64_t id) {
    METADATA_WAITER *waiter = data;
    pthread_mutex_lock(&waiter->lock);
    if(id) waiter->id = id;
    pthread_mutex_unlock(&waiter->lock);
}

static void metadata_waiter(void *data) {
    METADATA_WAITER *waiter = data;
    size_t memory = 0;
    struct web_client *client = web_client_create(&memory);
    client->mode = HTTP_REQUEST_MODE_GET;
    now_monotonic_high_precision_timeval(&client->timings.tv_in);
    client->timings.timeout_ut = 30 * USEC_PER_SEC;
    client->promql_operation_hook = waiter_operation;
    client->promql_operation_hook_data = waiter;
    buffer_strcat(client->url_as_received, "/api/v1/labels?match[]=fixture_a");
    int errors = api_v1_promql_labels(localhost, client, NULL) != waiter->expected;
    PROMQL_BODY *body = client->response.promql_body;
    char text[1024];
    uint64_t length = promql_body_length(body);
    int io = 0;
    if(body && length < sizeof(text) && promql_body_read(body, 0, text, (size_t)length, NULL, &io)) {
        text[length] = 0;
        errors += waiter->expected == 499 ? !strstr(text, "\"errorType\":\"canceled\"") :
            waiter->expected == 200 ? strcmp(text, "{\"status\":\"success\",\"data\":[]}") != 0 :
            !strstr(text, "initial host metadata load is unavailable");
    }
    else errors++;
    web_client_free(client);
    errors += memory != 0;
    pthread_mutex_lock(&waiter->lock);
    waiter->errors = errors;
    waiter->done = true;
    pthread_mutex_unlock(&waiter->lock);
}

static int loader_gate(unsigned event, void *data, void *statement, void *unused __maybe_unused) {
    METADATA_LOADER *loader = data;
    const char *sql = sqlite3_sql(statement);
    if(event != SQLITE_TRACE_STMT || !sql || !strstr(sql, "FROM context c")) return 0;
    pthread_mutex_lock(&loader->lock);
    loader->statements++;
    loader->entered = true;
    struct timespec deadline;
    clock_gettime(CLOCK_REALTIME, &deadline); deadline.tv_sec += 30;
    while(!loader->resume) {
        if(pthread_cond_timedwait(&loader->changed, &loader->lock, &deadline)) { loader->errors++; break; }
    }
    pthread_mutex_unlock(&loader->lock);
    return 0;
}

static void metadata_loader(void *data) {
    METADATA_LOADER *loader = data;
    sqlite3 *saved_meta = db_meta_thread, *saved_context = db_context_thread;
    db_meta_thread = loader->meta; db_context_thread = loader->context;
    loader->errors += sqlite3_trace_v2(loader->context, SQLITE_TRACE_STMT, loader_gate, loader) != SQLITE_OK;
    rrdhost_load_rrdcontext_data(loader->host, false);
    sqlite3_trace_v2(loader->context, 0, NULL, NULL);
    db_meta_thread = saved_meta; db_context_thread = saved_context;
}

static void loader_resume(METADATA_LOADER *loader) {
    pthread_mutex_lock(&loader->lock);
    loader->resume = true;
    pthread_cond_broadcast(&loader->changed);
    pthread_mutex_unlock(&loader->lock);
}

static int only_fixture_pending(RRDHOST *fixture) {
    int errors = 0;
    RRDHOST *host;
    rrd_rdlock();
    dfe_start_reentrant(rrdhost_root_index, host) {
        errors += host != fixture && rrdhost_flag_check(host, RRDHOST_FLAG_PENDING_CONTEXT_LOAD);
    }
    dfe_done(host);
    rrd_rdunlock();
    return errors;
}

static int waiter_case(const char *name, RRDHOST *host, RRDHOST_ACQUIRED *item,
                        size_t baseline, int expected, bool reset, METADATA_LOADER *loader) {
    METADATA_WAITER waiter = {.lock = PTHREAD_MUTEX_INITIALIZER, .expected = expected};
    ND_THREAD *thread = nd_thread_create("PROMQL_META_WAIT", NETDATA_THREAD_OPTION_DONT_LOG, metadata_waiter, &waiter);
    if(thread) nd_thread_set_manual_join(thread);
    int errors = !thread;
    uint64_t id = 0;
    bool acquired = false;
    usec_t until = now_monotonic_usec() + 5 * USEC_PER_SEC;
    while(thread && now_monotonic_usec() < until) {
        pthread_mutex_lock(&waiter.lock);
        id = waiter.id;
        bool done = waiter.done;
        pthread_mutex_unlock(&waiter.lock);
        acquired = dictionary_acquired_item_references((const DICTIONARY_ITEM *)item) > baseline;
        if((id && acquired) || done) break;
        sleep_usec(1000);
    }
    errors += !id || !acquired;
    if(expected == 499 || errors) promql_operations_cancel_id(id);
    if(reset) {
        errors += only_fixture_pending(host);
        reset_host_context_load_flag();
        errors += rrdhost_flag_check(host, RRDHOST_FLAG_PENDING_CONTEXT_LOAD);
        errors += __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) !=
            (loader ? RRD_CONTEXT_LOAD_LOADING : RRD_CONTEXT_LOAD_UNAVAILABLE);
    }
    if(loader) loader_resume(loader);
    if(thread) errors += nd_thread_join(thread) != 0;
    errors += waiter.errors || !waiter.done || promql_operations_active() ||
        dictionary_acquired_item_references((const DICTIONARY_ITEM *)item) != baseline;
    netdata_promql_engine_observations_t observed = {0};
    errors += promql_api_observations(&observed) != NETDATA_PROMQL_OK || observed.active_executions != 0;
    for(size_t i = 0; i < NETDATA_PROMQL_MEMORY_KINDS; i++)
        if(i != NETDATA_PROMQL_MEMORY_ACCOUNTING) errors += observed.memory[i].current != 0;
    pthread_mutex_destroy(&waiter.lock);
    printf("{\"host_readiness_case\":\"%s\",\"errors\":%d,\"acquired\":%s}\n", name, errors, acquired ? "true" : "false");
    fflush(stdout);
    return errors;
}

static int pending_metadata_case(bool reset_only) {
    const char *guid = reset_only ? "7030f515-956f-49d2-9c86-000000000009" : "7030f515-956f-49d2-9c86-000000000008";
    nd_uuid_t uuid;
    int errors = uuid_parse(guid, uuid) != 0 || !seed_readers(uuid);
    RRDHOST *host = errors ? NULL : rrdhost_find_or_create("wait-fixture", "wait-fixture", guid,
        "linux", "UTC", "UTC", 0, "fixture", "fixture", 1, default_rrd_history_entries, RRD_DB_MODE_DBENGINE,
        false, false, NULL, NULL, NULL, false, 0, 0, NULL, true);
    if(!host) return errors + 1;
    errors += __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != RRD_CONTEXT_LOAD_UNLOADED || host->rrdctx.contexts != NULL;
    rrd_rdlock();
    RRDHOST_ACQUIRED *item = rrdhost_find_and_acquire(guid);
    rrd_rdunlock();
    if(item) {
        size_t baseline = dictionary_acquired_item_references((const DICTIONARY_ITEM *)item);
        if(reset_only) errors += waiter_case("unloaded-reset", host, item, baseline, 500, true, NULL);
        else {
            errors += waiter_case("unloaded-cancel", host, item, baseline, 499, false, NULL);
            errors += __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != RRD_CONTEXT_LOAD_UNLOADED;
            METADATA_LOADER loader = {.lock = PTHREAD_MUTEX_INITIALIZER, .changed = PTHREAD_COND_INITIALIZER,
                .host = host, .meta = db_meta_thread, .context = db_context_thread};
            ND_THREAD *thread = nd_thread_create("PROMQL_META_LOAD", NETDATA_THREAD_OPTION_DONT_LOG, metadata_loader, &loader);
            if(thread) nd_thread_set_manual_join(thread);
            usec_t until = now_monotonic_usec() + 5 * USEC_PER_SEC;
            bool entered = false;
            while(thread && now_monotonic_usec() < until) {
                pthread_mutex_lock(&loader.lock); entered = loader.entered; pthread_mutex_unlock(&loader.lock);
                if(entered) break;
                sleep_usec(1000);
            }
            errors += !thread || !entered;
            if(thread && entered) {
                errors += __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != RRD_CONTEXT_LOAD_LOADING;
                errors += waiter_case("loading-cancel", host, item, baseline, 499, false, NULL);
                errors += __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != RRD_CONTEXT_LOAD_LOADING;
                errors += waiter_case("loading-complete", host, item, baseline, 200, true, &loader);
            }
            loader_resume(&loader);
            if(thread) errors += nd_thread_join(thread) != 0;
            errors += loader.errors || loader.statements != 1 ||
                __atomic_load_n(&host->rrdctx.initial_load, __ATOMIC_ACQUIRE) != RRD_CONTEXT_LOAD_COMPLETE;
            pthread_cond_destroy(&loader.changed); pthread_mutex_destroy(&loader.lock);
        }
        rrdhost_acquired_release(item);
    }
    else errors++;
    bool writable = rw_spinlock_trywrite_lock(&host->metadata_lifetime_lock);
    if(writable) rrdhost_free___consume_metadata_lifetime_writelock(host);
    else errors++;
    rrd_rdlock(); errors += rrdhost_find_by_guid(guid) != NULL; rrd_rdunlock();
    return errors;
}

int promql_host_readiness_fixture(void) {
    sqlite3 *saved_meta = db_meta_thread, *saved_context = db_context_thread;
    db_meta_thread = db_context_thread = NULL;
    int errors = sqlite3_open_v2(":memory:", &db_meta_thread, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_NOMUTEX, NULL) != SQLITE_OK ||
        sqlite3_open_v2(":memory:", &db_context_thread, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_NOMUTEX, NULL) != SQLITE_OK;
    const char *context_schema = "CREATE TABLE context(id TEXT,host_id BLOB,version INT,title TEXT,chart_type TEXT,unit TEXT,priority INT,"
        "first_time_t INT,last_time_t INT,deleted INT,family TEXT)";
    const char *meta_schema = "CREATE TABLE chart(chart_id BLOB,host_id BLOB,type TEXT,id TEXT,name TEXT,context TEXT,title TEXT,unit TEXT,"
        "priority INT,update_every INT,chart_type INT,family TEXT);"
        "CREATE TABLE dimension(dim_id BLOB,chart_id BLOB,id TEXT,name TEXT,options TEXT,algorithm INT)";
    if(!errors) errors += sqlite3_exec(db_context_thread, context_schema, NULL, NULL, NULL) != SQLITE_OK ||
        sqlite3_exec(db_meta_thread, meta_schema, NULL, NULL, NULL) != SQLITE_OK;
    if(!errors) {
        const char *tables[] = {"context", "chart", "dimension"};
        const char *fragments[] = {"FROM context c", "FROM chart c", "FROM dimension d,"};
        for(unsigned i = 0; i < 3; i++) {
            errors += host_case(i * 2 + 1, tables[i], fragments[i], true, false);
            errors += host_case(i * 2 + 2, tables[i], fragments[i], false, false);
        }
        errors += host_case(7, tables[0], fragments[0], false, true);
        errors += pending_metadata_case(false);
        errors += pending_metadata_case(true);
    }
    sql_close_thread_db_safe(&db_meta_thread);
    sql_close_thread_db_safe(&db_context_thread);
    db_meta_thread = saved_meta; db_context_thread = saved_context;
    return errors;
}
