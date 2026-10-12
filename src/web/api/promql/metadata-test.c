// SPDX-License-Identifier: GPL-3.0-or-later

#include "provider-internal.h"
#include "database/rrddim-collection.h"
#include "daemon/config/netdata-conf-directories.h"

#define REFERENCE_BASE INT64_C(1700002800)
extern sqlite3 *db_context_meta;

typedef struct {
    RRDSET *chart;
    RRDDIM *dimension;
    RRDINSTANCE *instance;
    STRING *chart_name, *label_name, *label_value;
    uint64_t rows, pointer_checks, sql_reads;
    int errors;
    bool frozen;
} REFERENCE_RESULT;

static const netdata_promql_label_t *find_label(netdata_promql_labels_t labels, const char *name) {
    size_t length = strlen(name);
    for(uint64_t i = 0; i < labels.len; i++)
        if(labels.data[i].name.len == length && !memcmp(labels.data[i].name.data, name, length)) return &labels.data[i];
    return NULL;
}

static int retain_reference_label(STRING *name, STRING *value, RRDLABEL_SRC source __maybe_unused, void *context) {
    REFERENCE_RESULT *result = context;
    const char *wanted = result->frozen ? "shared_reference" : "cache_probe";
    if(!strcmp(string2str(name), wanted)) {
        result->label_name = string_dup(name);
        result->label_value = string_dup(value);
    }
    return 0;
}

static uint32_t reference_begin(void *context __maybe_unused, const netdata_promql_output_header_t *header,
                                 const netdata_promql_callback_control_t *control __maybe_unused,
                                 netdata_promql_bytes_t *error __maybe_unused) {
    return header->value_type == NETDATA_PROMQL_TYPE_VECTOR ? NETDATA_PROMQL_OK : NETDATA_PROMQL_OUTPUT_ERROR;
}

static void check_pointer(REFERENCE_RESULT *result, netdata_promql_labels_t labels, const char *name, const char *bytes) {
    const netdata_promql_label_t *label = find_label(labels, name);
    result->errors += !label || label->value.data != (const uint8_t *)bytes ||
        (label && (label->value.len != strlen(bytes) || memcmp(label->value.data, bytes, (size_t)label->value.len)));
    result->pointer_checks++;
}

static uint32_t reference_chunk(void *context, const netdata_promql_chunk_t *chunk,
                                 const netdata_promql_callback_control_t *control __maybe_unused,
                                 netdata_promql_bytes_t *error __maybe_unused) {
    REFERENCE_RESULT *result = context;
    if(chunk->kind == NETDATA_PROMQL_CHUNK_ANNOTATIONS) return NETDATA_PROMQL_OK;
    if(chunk->kind != NETDATA_PROMQL_CHUNK_SERIES || chunk->payload.series.points.len) return NETDATA_PROMQL_OUTPUT_ERROR;
    netdata_promql_labels_t labels = chunk->payload.series.labels;
    if(result->frozen) {
        check_pointer(result, labels, "netdata_context", string2str(result->chart->context));
        check_pointer(result, labels, "netdata_chart_name", string2str(result->chart_name));
        check_pointer(result, labels, "netdata_dimension_name", string2str(result->dimension->name));
        check_pointer(result, labels, "netdata_units", string2str(result->chart->units));
        check_pointer(result, labels, "shared_reference", string2str(result->label_value));
        const netdata_promql_label_t *label = find_label(labels, "shared_reference");
        result->errors += !label || label->name.data != (const uint8_t *)string2str(result->label_name);
        result->pointer_checks++;
        if(!result->rows) {
            rrdset_reset_name(result->chart, "reference_changed");
            rrdlabels_add(result->chart->rrdlabels, "shared_reference", "changed-value", RRDLABEL_SRC_AUTO);
            result->errors += result->chart->name == result->chart_name;
        }
    }
    else {
        if(!result->label_value)
            rrdlabels_walkthrough_read_string(result->instance->rrdlabels, retain_reference_label, result);
        check_pointer(result, labels, "cache_probe", string2str(result->label_value));
    }
    result->rows++;
    return NETDATA_PROMQL_OK;
}

static uint32_t reference_commit(void *context __maybe_unused,
                                  const netdata_promql_query_observations_t *observations __maybe_unused,
                                  const netdata_promql_callback_control_t *control __maybe_unused,
                                  netdata_promql_bytes_t *error __maybe_unused) { return NETDATA_PROMQL_OK; }

static void reference_abort(void *context __maybe_unused,
                              const netdata_promql_accounting_context_t *account __maybe_unused) {}

static int reference_discover(netdata_promql_engine_t *engine, netdata_promql_selector_t *selector,
                               REFERENCE_RESULT *result, const char *name, uint64_t rows, uint64_t reads) {
    result->rows = result->pointer_checks = 0;
    PROMQL_PROVIDER_CONTEXT context = {.scope = PROMQL_HOST_GUID, .host = localhost->machine_guid,
        .backing_directory = netdata_configured_cache_dir};
    netdata_promql_provider_t provider = promql_provider(&context);
    netdata_promql_output_t output = {.header = {NETDATA_PROMQL_ABI_VERSION, sizeof(output)}, .context = result,
        .begin = reference_begin, .chunk = reference_chunk, .commit = reference_commit, .abort = reference_abort};
    netdata_promql_report_t *report = NULL;
    const netdata_promql_selector_t *selectors[] = {selector};
    uint32_t status = netdata_promql_discover(engine, &provider,
        (netdata_promql_bounds_t){REFERENCE_BASE * 1000 + 1, (REFERENCE_BASE + 4097) * 1000},
        selectors, 1, NULL, &output, &report);
    result->errors += status != NETDATA_PROMQL_OK || result->rows != rows || result->sql_reads != reads;
    if(report) netdata_promql_report_release(report);
    netdata_promql_engine_observations_t observations = {0};
    result->errors += netdata_promql_engine_observations(engine, &observations) != NETDATA_PROMQL_OK ||
        observations.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current ||
        observations.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current;
    printf("{\"metadata_ref_case\":\"%s\",\"errors\":%d,\"rows\":%"PRIu64",\"pointer_checks\":%"PRIu64
           ",\"sql_reads\":%"PRIu64",\"remaining_provider\":%"PRIu64",\"remaining_views\":%"PRIu64"}\n",
           name, result->errors, result->rows, result->pointer_checks, result->sql_reads,
           observations.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current,
           observations.memory[NETDATA_PROMQL_MEMORY_BORROWED_VIEW].current);
    return result->errors;
}

static netdata_promql_selector_t *reference_selector(netdata_promql_engine_t *engine, const char *text) {
    netdata_promql_selector_t *selector = NULL;
    netdata_promql_report_t *report = NULL;
    netdata_promql_selector_compile(engine, promql_bytes(text), NULL, &selector, &report);
    if(report) netdata_promql_report_release(report);
    return selector;
}

static int label_trace(unsigned event, void *context, void *statement, void *unused __maybe_unused) {
    if(event == SQLITE_TRACE_STMT) {
        const char *sql = sqlite3_sql(statement);
        if(sql && strstr(sql, "SELECT l.label_key, l.label_value, l.source_type FROM meta.chart_label"))
            ((REFERENCE_RESULT *)context)->sql_reads++;
    }
    return 0;
}

int promql_metadata_reference_fixture(netdata_promql_engine_t *engine, RRDSET *chart, RRDDIM *dimension) {
    REFERENCE_RESULT result = {.chart = chart, .dimension = dimension, .frozen = true,
        .chart_name = string_dup(chart->name)};
    rrdlabels_add(chart->rrdlabels, "shared_reference", "original-value", RRDLABEL_SRC_AUTO);
    rrdlabels_walkthrough_read_string(chart->rrdlabels, retain_reference_label, &result);
    netdata_promql_selector_t *selector = reference_selector(engine,
        "{__name__=~\"fixture[._]provider\",netdata_dimension_id=\"gauge\"}");
    int errors = !selector;
    if(selector) {
        errors += reference_discover(engine, selector, &result, "immutable-source-views", 2, 0);
        netdata_promql_selector_release(selector);
    }
    rrdset_reset_name(chart, string2str(result.chart_name));
    rrdlabels_add(chart->rrdlabels, "shared_reference", "original-value", RRDLABEL_SRC_AUTO);
    string_freez(result.chart_name);
    string_freez(result.label_name);
    string_freez(result.label_value);

    RRDSET *cached = rrdset_create(localhost, "promql_fixture", "cached", NULL, "fixture", "fixture.cached",
        "Cached labels fixture", "KiB", "fixture", "fixture", 1, 1, RRDSET_TYPE_LINE);
    if(!cached) return errors + 1;
    RRDDIM *cached_dimension = rrddim_add(cached, "value", NULL, 1, 1, RRD_ALGORITHM_ABSOLUTE);
    if(!cached_dimension) { rrdset_free(cached); return errors + 1; }
    rrddim_store_metric(cached_dimension, (usec_t)(REFERENCE_BASE + 2) * USEC_PER_SEC, 5, SN_FLAG_NOT_ANOMALOUS);
    rrddim_finalize_collection_and_check_retention(cached_dimension);
    RRDCONTEXT_ACQUIRED *context_owner = rrdcontext_acquired_dup(cached->rrdcontexts.rrdcontext);
    RRDINSTANCE_ACQUIRED *instance_owner = rrdinstance_acquired_dup(cached->rrdcontexts.rrdinstance);
    RRDINSTANCE *instance = rrdinstance_acquired_value(instance_owner);
    sqlite3_stmt *statement = NULL;
    const char *schema = "CREATE TABLE IF NOT EXISTS meta.chart_label(chart_id BLOB, source_type INT, label_key TEXT,"
        "label_value TEXT, date_created INT, PRIMARY KEY(chart_id,label_key))";
    bool seeded = db_context_meta && sqlite3_exec(db_context_meta, schema, NULL, NULL, NULL) == SQLITE_OK &&
        sqlite3_prepare_v2(db_context_meta, "INSERT OR REPLACE INTO meta.chart_label(chart_id,source_type,label_key,label_value)"
        " VALUES(?1,?2,'cache_probe','cache-value')", -1, &statement, NULL) == SQLITE_OK &&
        sqlite3_bind_blob(statement, 1, cached->chart_uuid, sizeof(cached->chart_uuid), SQLITE_STATIC) == SQLITE_OK &&
        sqlite3_bind_int(statement, 2, RRDLABEL_SRC_AUTO) == SQLITE_OK && sqlite3_step(statement) == SQLITE_DONE;
    if(statement) sqlite3_finalize(statement);
    rrdset_free(cached);
    rrdinstance_metadata_lock(instance);
    rrdlabels_flush(instance->rrdlabels);
    rrd_flag_set(instance, RRD_FLAG_DEMAND_LABELS | RRD_FLAG_OWN_LABELS);
    rrdinstance_metadata_changed(instance);
    rrdinstance_metadata_unlock(instance);
    result = (REFERENCE_RESULT){.instance = instance};
    selector = reference_selector(engine, "fixture_cached");
    if(seeded && selector && sqlite3_trace_v2(db_context_meta, SQLITE_TRACE_STMT, label_trace, &result) == SQLITE_OK) {
        errors += reference_discover(engine, selector, &result, "archived-cold", 1, 1);
        errors += rrd_flag_check(instance, RRD_FLAG_DEMAND_LABELS);
        errors += reference_discover(engine, selector, &result, "archived-hit", 1, 1);
        sqlite3_trace_v2(db_context_meta, 0, NULL, NULL);
    }
    else { fprintf(stderr, "metadata cache fixture could not seed or observe private SQL rows\n"); errors++; }
    if(selector) netdata_promql_selector_release(selector);
    string_freez(result.label_name);
    string_freez(result.label_value);
    rrdinstance_release(instance_owner);
    rrdcontext_release(context_owner);
    return errors;
}
