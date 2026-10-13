// SPDX-License-Identifier: GPL-3.0-or-later

#include "provider-internal.h"
#include "database/rrd.h"
#include "database/rrddim-collection.h"
#include "daemon/config/netdata-conf-directories.h"
#include "aclk/mqtt_websockets/mqtt_ng.h"
#include "web/rtc/webrtc.h"

#define PROVIDER_BASE ((time_t)1700002800)

int promql_output_fixture(netdata_promql_engine_t *engine);
int promql_request_fixture(netdata_promql_engine_t *engine);
int promql_api_fixture(void);
int promql_aclk_source_fixture(netdata_promql_engine_t *engine);
int promql_rtc_frame_fixture(void);
int promql_operations_fixture(netdata_promql_engine_t *engine);
int promql_metadata_reference_fixture(netdata_promql_engine_t *engine, RRDSET *chart, RRDDIM *dimension);

typedef struct {
    const char *name, *expression;
    unsigned when, lookback;
    bool absolute;
    double value;
    uint64_t series;
    uint32_t status;
} PROVIDER_CASE;

typedef struct { uint64_t rows; } DISCOVERY_RESULT;

static uint32_t discovery_begin(void *context __maybe_unused, const netdata_promql_output_header_t *header,
                                 const netdata_promql_callback_control_t *control __maybe_unused,
                                 netdata_promql_bytes_t *error __maybe_unused) {
    return header->value_type == NETDATA_PROMQL_TYPE_VECTOR ? NETDATA_PROMQL_OK : NETDATA_PROMQL_OUTPUT_ERROR;
}

static uint32_t discovery_chunk(void *context, const netdata_promql_chunk_t *chunk,
                                 const netdata_promql_callback_control_t *control __maybe_unused,
                                 netdata_promql_bytes_t *error __maybe_unused) {
    if(chunk->kind == NETDATA_PROMQL_CHUNK_ANNOTATIONS) return NETDATA_PROMQL_OK;
    if(chunk->kind != NETDATA_PROMQL_CHUNK_SERIES || chunk->payload.series.points.len) return NETDATA_PROMQL_OUTPUT_ERROR;
    ((DISCOVERY_RESULT *)context)->rows++;
    return NETDATA_PROMQL_OK;
}

static uint32_t discovery_commit(void *context __maybe_unused,
                                  const netdata_promql_query_observations_t *observations __maybe_unused,
                                  const netdata_promql_callback_control_t *control __maybe_unused,
                                  netdata_promql_bytes_t *error __maybe_unused) {
    return NETDATA_PROMQL_OK;
}

static void discovery_abort(void *context __maybe_unused,
                              const netdata_promql_accounting_context_t *account __maybe_unused) {}

static int discovery_case(netdata_promql_engine_t *engine, netdata_promql_selector_t *selector,
                            const char *name, netdata_promql_bounds_t bounds, uint64_t expected) {
    PROMQL_PROVIDER_CONTEXT context = {.scope = PROMQL_HOST_GUID, .host = localhost->machine_guid,
        .backing_directory = netdata_configured_cache_dir};
    netdata_promql_provider_t provider = promql_provider(&context);
    DISCOVERY_RESULT result = {0};
    netdata_promql_output_t output = {.header = {NETDATA_PROMQL_ABI_VERSION, sizeof(output)}, .context = &result,
        .begin = discovery_begin, .chunk = discovery_chunk, .commit = discovery_commit, .abort = discovery_abort};
    netdata_promql_report_t *report = NULL;
    const netdata_promql_selector_t *selectors[] = {selector};
    uint32_t status = netdata_promql_discover(engine, &provider, bounds, selectors, 1, NULL, &output, &report);
    netdata_promql_query_observations_t observations = {0};
    if(report) netdata_promql_report_observations(report, &observations);
    uint64_t reads = 0;
    for(size_t i = 0; i < RRD_STORAGE_TIERS; i++) reads += context.storage_points[i] + context.storage_pages[i];
    int errors = status != NETDATA_PROMQL_OK || result.rows != expected || reads ||
        context.reconstruction_points || observations.samples_fetched;
    if(errors) fprintf(stderr, "discovery mismatch: %s status%u rows%"PRIu64" expected%"PRIu64" reads%"PRIu64"\n",
                        name, status, result.rows, expected, reads);
    if(report) netdata_promql_report_release(report);
    printf("{\"discovery_case\":\"%s\",\"status\":%u,\"rows\":%"PRIu64",\"errors\":%d}\n",
           name, status, result.rows, errors);
    return errors;
}

static int provider_case(netdata_promql_engine_t *engine, const PROVIDER_CASE *test, const char *directory,
                         netdata_promql_query_observations_t *observed) {
    PROMQL_PROVIDER_CONTEXT context = {.scope = PROMQL_HOST_GUID, .host = localhost->machine_guid,
        .backing_directory = directory, .absolute = test->absolute};
    netdata_promql_provider_t provider = promql_provider(&context);
    netdata_promql_query_t *query = NULL;
    netdata_promql_report_t *report = NULL;
    netdata_promql_result_t *result = NULL;
    uint32_t status = netdata_promql_compile(engine, promql_bytes(test->expression), NULL, &query, &report);
    if(report) { netdata_promql_report_release(report); report = NULL; }
    netdata_promql_precise_request_t request = {
        .header = {NETDATA_PROMQL_ABI_VERSION, sizeof(request)}, .kind = NETDATA_PROMQL_REQUEST_INSTANT,
        .start = {.unix_seconds = PROVIDER_BASE + test->when}, .lookback_ns = (int64_t)test->lookback * INT64_C(1000000000),
    };
    if(status == NETDATA_PROMQL_OK)
        status = netdata_promql_collect_precise(query, &provider, &request, NULL, &result, &report);
    int errors = status != test->status;
    netdata_promql_query_observations_t observations = {0};
    if(report) netdata_promql_report_observations(report, &observations);
    if(observed) *observed = observations;
    if(status == NETDATA_PROMQL_OK) {
        netdata_promql_result_info_t info = {0};
        errors += netdata_promql_result_info(result, &info) != NETDATA_PROMQL_OK;
        errors += info.series_count != test->series;
        if(test->series && info.series_count) {
            netdata_promql_point_info_t point = {0};
            errors += netdata_promql_result_point(result, 0, 0, &point) != NETDATA_PROMQL_OK;
            errors += point.value_kind != NETDATA_PROMQL_VALUE_FLOAT ||
                netdata_promql_double_from_bits(point.value_bits) != test->value ||
                point.time_ms != (int64_t)(PROVIDER_BASE + test->when) * 1000;
            if(errors) fprintf(stderr, "provider value: %s got %.17g expected %.17g\n", test->name,
                                netdata_promql_double_from_bits(point.value_bits), test->value);
        }
    }
    if(errors && report) {
        netdata_promql_error_t failure;
        if(netdata_promql_report_error(report, 0, &failure) == NETDATA_PROMQL_OK)
            fprintf(stderr, "provider failure: %s status%u %.*s\n", test->name, status,
                    (int)failure.message.len, (const char *)failure.message.data);
    }
    if(result) netdata_promql_result_release(result);
    if(query) netdata_promql_query_release(query);
    if(report) netdata_promql_report_release(report);
    netdata_promql_engine_observations_t remaining = {0};
    errors += netdata_promql_engine_observations(engine, &remaining) != NETDATA_PROMQL_OK;
    errors += remaining.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current != 0;
    printf("{\"provider_case\":\"%s\",\"status\":%u,\"errors\":%d,\"logical_pages\":%"PRIu64
           ",\"logical_samples\":%"PRIu64",\"cursor_points\":%"PRIu64",\"provider_folds\":%"PRIu64
           ",\"provider_peak\":%"PRIu64",\"remaining_provider\":%"PRIu64"}\n",
           test->name, status, errors, observations.pages, observations.samples_fetched,
           context.storage_points[0], context.reconstruction_points,
           observations.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].peak,
           remaining.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current);
    return errors;
}

typedef struct {
    netdata_promql_engine_t *engine;
    unsigned mode, points;
    int errors;
    bool alias_shared, file_shared;
    uint32_t open_error, seek_error, next_error;
} CHECKPOINT_SCENARIO;

static bool checkpoint_series(netdata_promql_provider_t *provider, void *query, const char *dimension,
                               bool alias, netdata_promql_bounds_t bounds,
                               const netdata_promql_callback_control_t *control, netdata_promql_series_open_t *out) {
    netdata_promql_matcher_t matchers[] = {
        {.kind = NETDATA_PROMQL_MATCH_EQUAL, .name = promql_bytes("netdata_context"),
         .value = promql_bytes("fixture.checkpoints")},
        {.kind = NETDATA_PROMQL_MATCH_EQUAL, .name = promql_bytes("netdata_dimension_id"),
         .value = promql_bytes(dimension)},
    };
    netdata_promql_selection_request_t request = {.bounds = bounds, .matchers = {matchers, 2}};
    void *selection = NULL;
    netdata_promql_bytes_t error = {0};
    const netdata_promql_accounting_context_t *account = NULL;
    if(netdata_promql_callback_accounting(control, &account) != NETDATA_PROMQL_OK ||
       provider->selection_open(provider->context, query, &request, control, &selection, &error) != NETDATA_PROMQL_OK)
        return false;
    bool found = false;
    for(;;) {
        netdata_promql_series_open_t series = {0};
        netdata_promql_provider_diagnostics_t diagnostics;
        uint32_t status = provider->selection_next(provider->context, selection, control, &series, &diagnostics, &error);
        if(status != NETDATA_PROMQL_OK) break;
        bool name_matches = false, dimension_matches = false;
        for(uint64_t i = 0; i < series.labels.len; i++) {
            netdata_promql_label_t label = series.labels.data[i];
            if(!promql_label_compare(label.name, promql_bytes("__name__")))
                name_matches = !promql_label_compare(label.value, promql_bytes(alias ? "fixture.checkpoints" : "fixture_checkpoints"));
            if(!promql_label_compare(label.name, promql_bytes("netdata_dimension_id")))
                dimension_matches = !promql_label_compare(label.value, promql_bytes(dimension));
        }
        if(name_matches && dimension_matches) { *out = series; found = true; break; }
        provider->series_close(provider->context, series.handle, account);
    }
    provider->selection_close(provider->context, selection, account);
    return found;
}

static int checkpoint_read_all(netdata_promql_provider_t *provider, void *cursor, unsigned points,
                               const netdata_promql_callback_control_t *control) {
    unsigned read = 0;
    netdata_promql_bytes_t error = {0};
    for(;;) {
        netdata_promql_page_t page;
        netdata_promql_resources_t observations;
        uint32_t status = provider->cursor_next(provider->context, cursor, control, &page, &observations, &error);
        if(status == NETDATA_PROMQL_END) return read != points;
        if(status != NETDATA_PROMQL_OK || !page.samples.len || page.samples.len > 1024) return 1;
        for(uint64_t i = 0; i < page.samples.len; i++) {
            const netdata_promql_sample_t *sample = &page.samples.data[i];
            read++;
            if(sample->time_ms != (int64_t)(PROVIDER_BASE + read) * 1000 ||
               sample->value.kind != NETDATA_PROMQL_VALUE_FLOAT ||
               sample->value.payload.float_bits != netdata_promql_double_bits(2.0 * read)) return 1;
        }
    }
}

static uint32_t checkpoint_begin(void *handle, const netdata_promql_output_header_t *header __maybe_unused,
                                  const netdata_promql_callback_control_t *control, netdata_promql_bytes_t *error) {
    CHECKPOINT_SCENARIO *test = handle;
    PROMQL_PROVIDER_CONTEXT context = {.scope = PROMQL_HOST_GUID, .host = localhost->machine_guid,
                                      .backing_directory = netdata_configured_cache_dir};
    netdata_promql_provider_t provider = promql_provider(&context);
    netdata_promql_bounds_t bounds = {(int64_t)(PROVIDER_BASE + 1) * 1000,
                                     (int64_t)(PROVIDER_BASE + test->points) * 1000};
    const netdata_promql_accounting_context_t *account = NULL;
    void *query = NULL;
    netdata_promql_series_open_t series = {0}, other = {0};
    netdata_promql_cursor_open_t cursor = {0}, second = {0};
    if(netdata_promql_callback_accounting(control, &account) != NETDATA_PROMQL_OK ||
       provider.query_open(provider.context, bounds, control, &query, error) != NETDATA_PROMQL_OK ||
       !checkpoint_series(&provider, query, test->mode == 1 ? "legacy" : "native", false, bounds, control, &series) ||
       provider.cursor_open(provider.context, series.handle, bounds, control, &cursor, error) != NETDATA_PROMQL_OK) {
        test->errors++; goto finished;
    }
    test->errors += checkpoint_read_all(&provider, cursor.handle, test->points, control);
    PROMQL_PROVIDER_QUERY *owned = query;
    if(!owned->checkpoints.file) { test->errors++; goto finished; }
    if(test->mode == 2 || test->mode == 5) {
        test->errors += !checkpoint_series(&provider, query, test->mode == 5 ? "legacy" : "native",
                                           test->mode == 2, bounds, control, &other);
        if(!other.handle) goto finished;
        test->alias_shared = ((PROMQL_SERIES *)series.handle)->physical == ((PROMQL_SERIES *)other.handle)->physical;
        test->errors += test->alias_shared != (test->mode == 2);
        PROMQL_FILE *file = owned->checkpoints.file;
        test->errors += provider.cursor_open(provider.context, other.handle, bounds, control, &second, error) != NETDATA_PROMQL_OK;
        if(second.handle) test->errors += checkpoint_read_all(&provider, second.handle, test->points, control);
        test->file_shared = file == owned->checkpoints.file;
        test->errors += !test->file_shared;
    }
    if(test->mode == 3 || test->mode == 4) {
        // Only this query's anonymous checkpoint file is truncated; DBengine files remain intact.
        if(ftruncate(owned->checkpoints.file->fd, 0)) { test->errors++; goto finished; }
        int64_t target = (int64_t)(PROVIDER_BASE + test->points / 2) * 1000;
        if(test->mode == 3) {
            PROMQL_PHYSICAL *physical = ((PROMQL_SERIES *)series.handle)->physical;
            size_t references = physical->references;
            netdata_promql_engine_observations_t before = {0}, after = {0};
            test->errors += netdata_promql_engine_observations(test->engine, &before) != NETDATA_PROMQL_OK;
            netdata_promql_bounds_t later = {target, bounds.end_ms};
            test->open_error = provider.cursor_open(provider.context, series.handle, later, control, &second, error);
            test->errors += netdata_promql_engine_observations(test->engine, &after) != NETDATA_PROMQL_OK;
            test->errors += test->open_error != NETDATA_PROMQL_PROVIDER_ERROR || second.handle != NULL ||
                physical->references != references ||
                before.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current != after.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current;
        }
        else {
            test->seek_error = provider.cursor_seek(provider.context, cursor.handle, target, control, error);
            netdata_promql_page_t page;
            netdata_promql_resources_t observations;
            test->next_error = provider.cursor_next(provider.context, cursor.handle, control, &page, &observations, error);
            test->errors += test->seek_error != NETDATA_PROMQL_PROVIDER_ERROR || test->next_error != test->seek_error ||
                page.samples.data != NULL || page.samples.len != 0;
        }
    }
    else {
        const unsigned targets[] = {1, test->points / 2 + 1, 33, test->points - 3, 17};
        for(size_t i = 0; i < sizeof(targets) / sizeof(targets[0]); i++) {
            int64_t when = (int64_t)(PROVIDER_BASE + targets[i]) * 1000;
            netdata_promql_page_t page;
            netdata_promql_resources_t observations;
            uint32_t status = provider.cursor_seek(provider.context, cursor.handle, when, control, error);
            if(status == NETDATA_PROMQL_OK)
                status = provider.cursor_next(provider.context, cursor.handle, control, &page, &observations, error);
            test->errors += status != NETDATA_PROMQL_OK;
            if(status == NETDATA_PROMQL_OK)
                test->errors += !page.samples.len || page.samples.data[0].time_ms != when ||
                    page.samples.data[0].value.payload.float_bits != netdata_promql_double_bits(2.0 * targets[i]);
        }
    }
finished:
    if(second.handle) provider.cursor_close(provider.context, second.handle, account);
    if(cursor.handle) provider.cursor_close(provider.context, cursor.handle, account);
    if(other.handle) provider.series_close(provider.context, other.handle, account);
    if(series.handle) provider.series_close(provider.context, series.handle, account);
    if(query) provider.query_close(provider.context, query, account);
    if(test->errors) { *error = promql_bytes("counter checkpoint fixture failed"); return NETDATA_PROMQL_OUTPUT_ERROR; }
    return NETDATA_PROMQL_OK;
}

static uint32_t checkpoint_chunk(void *context __maybe_unused, const netdata_promql_chunk_t *chunk __maybe_unused,
                                  const netdata_promql_callback_control_t *control __maybe_unused,
                                  netdata_promql_bytes_t *error __maybe_unused) {
    return NETDATA_PROMQL_OK;
}

static int checkpoint_callbacks(netdata_promql_engine_t *engine, unsigned points) {
    const char *names[] = {"native-seeks", "legacy-seeks", "alias-basis", "failed-open-cleanup",
                          "failed-seek-sticky", "two-metrics-one-file"};
    int errors = 0;
    for(unsigned mode = 0; mode < sizeof(names) / sizeof(names[0]); mode++) {
        CHECKPOINT_SCENARIO test = {.engine = engine, .mode = mode, .points = points};
        PROMQL_PROVIDER_CONTEXT context = {.scope = PROMQL_HOST_GUID, .host = localhost->machine_guid,
                                          .backing_directory = netdata_configured_cache_dir};
        netdata_promql_provider_t provider = promql_provider(&context);
        netdata_promql_query_t *query = NULL;
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_compile(engine, promql_bytes("vector(0)"), NULL, &query, &report);
        if(report) { netdata_promql_report_release(report); report = NULL; }
        netdata_promql_precise_request_t request = {
            .header = {NETDATA_PROMQL_ABI_VERSION, sizeof(request)}, .kind = NETDATA_PROMQL_REQUEST_INSTANT,
            .start = {.unix_seconds = PROVIDER_BASE + points}, .lookback_ns = INT64_C(1000000000),
        };
        netdata_promql_output_t output = {.header = {NETDATA_PROMQL_ABI_VERSION, sizeof(output)}, .context = &test,
            .begin = checkpoint_begin, .chunk = checkpoint_chunk, .commit = discovery_commit, .abort = discovery_abort};
        if(status == NETDATA_PROMQL_OK)
            status = netdata_promql_execute_precise(query, &provider, &request, NULL, &output, &report);
        netdata_promql_query_observations_t observations = {0};
        if(report) netdata_promql_report_observations(report, &observations);
        if(report) netdata_promql_report_release(report);
        if(query) netdata_promql_query_release(query);
        netdata_promql_engine_observations_t remaining = {0};
        test.errors += netdata_promql_engine_observations(engine, &remaining) != NETDATA_PROMQL_OK;
        test.errors += status != NETDATA_PROMQL_OK || remaining.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current != 0;
        printf("{\"checkpoint_callback\":\"%s\",\"errors\":%d,\"status\":%u,\"open_error\":%u,\"seek_error\":%u,"
               "\"next_error\":%u,\"alias_shared\":%s,\"file_shared\":%s,\"provider_peak\":%"PRIu64
               ",\"remaining_provider\":%"PRIu64"}\n", names[mode], test.errors, status, test.open_error,
               test.seek_error, test.next_error, test.alias_shared ? "true" : "false", test.file_shared ? "true" : "false",
               observations.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].peak,
               remaining.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].current);
        errors += test.errors;
    }
    return errors;
}

static int checkpoint_pages_fixture(netdata_promql_engine_t *engine) {
    const unsigned pages[] = {16, 17, 33, 129, 513, 2049};
    const unsigned points_per_page = 4;
    RRDSET *chart = rrdset_create(localhost, "promql_fixture", "checkpoints", NULL, "fixture", "fixture.checkpoints",
                                 "Checkpoint fixture", "events", "fixture", "fixture", 1, 1, RRDSET_TYPE_LINE);
    if(!chart) return 1;
    RRDDIM *native = rrddim_add(chart, "native", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *legacy = rrddim_add(chart, "legacy", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *reset_gap = rrddim_add(chart, "reset_gap", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *legacy_negative = rrddim_add(chart, "legacy_negative", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    if(!native || !legacy || !reset_gap || !legacy_negative) { rrdset_free(chart); return 1; }
    for(unsigned i = 1; i <= pages[5] * points_per_page; i++) {
        usec_t when = (usec_t)(PROVIDER_BASE + i) * USEC_PER_SEC;
        storage_engine_store_metric(native->tiers[0].sch, when, 2, 2, 2, 1, 0,
                                    SN_FLAG_NOT_ANOMALOUS | SP_FLAG_COUNTER_SOURCE);
        storage_engine_store_metric(legacy->tiers[0].sch, when, 2, 2, 2, 1, 0,
                                    SN_FLAG_NOT_ANOMALOUS | SP_FLAG_ORDINARY_SOURCE);
        double gap_value = i == 73 ? NAN : 2;
        storage_engine_store_metric(reset_gap->tiers[0].sch, when, gap_value, gap_value, gap_value,
                                    i == 73 ? 0 : 1, 0, SN_FLAG_NOT_ANOMALOUS | SP_FLAG_COUNTER_SOURCE |
                                    (i == 69 || i == 73 ? SN_FLAG_RESET : SN_FLAG_NONE));
        storage_engine_store_metric(legacy_negative->tiers[0].sch, when, -2, -2, -2, 1, 0,
                                    SN_FLAG_NOT_ANOMALOUS | SP_FLAG_ORDINARY_SOURCE);
        if(i % points_per_page == 0) {
            storage_engine_store_flush(native->tiers[0].sch);
            storage_engine_store_flush(legacy->tiers[0].sch);
            storage_engine_store_flush(reset_gap->tiers[0].sch);
            storage_engine_store_flush(legacy_negative->tiers[0].sch);
        }
    }
    rrddim_finalize_collection_and_check_retention(native);
    rrddim_finalize_collection_and_check_retention(legacy);
    rrddim_finalize_collection_and_check_retention(reset_gap);
    rrddim_finalize_collection_and_check_retention(legacy_negative);
    int errors = 0;
    for(unsigned mode = 0; mode < 2; mode++) {
        const char *dimension = mode ? "legacy" : "native";
        uint64_t reference_peak = 0, longest_peak = 0;
        for(size_t i = 0; i < sizeof(pages) / sizeof(pages[0]); i++) {
            char name[96], expression[256];
            unsigned points = pages[i] * points_per_page;
            snprintfz(name, sizeof(name) - 1, "checkpoint-%s-%u", dimension, pages[i]);
            snprintfz(expression, sizeof(expression) - 1,
                      "sum_over_time(fixture_checkpoints{netdata_dimension_id=\"%s\"}[%us])", dimension, points);
            PROVIDER_CASE test = {name, expression, points, points, false, (double)points * (points + 1), 1, 0};
            netdata_promql_query_observations_t observations = {0};
            errors += provider_case(engine, &test, netdata_configured_cache_dir, &observations);
            uint64_t peak = observations.memory[NETDATA_PROMQL_MEMORY_PROVIDER_HELD].peak;
            if(pages[i] == 129) reference_peak = peak;
            if(pages[i] == 2049) longest_peak = peak;
        }
        // Longer history may grow only the bounded copy window, not a checkpoint timeline in RAM.
        int failures = !reference_peak || !longest_peak ||
            (longest_peak > reference_peak && longest_peak - reference_peak > 12 * 1024);
        errors += failures;
        printf("{\"checkpoint_memory\":\"%s\",\"short_peak\":%"PRIu64",\"long_peak\":%"PRIu64
               ",\"errors\":%d}\n", dimension, reference_peak, longest_peak, failures);
    }
    errors += checkpoint_callbacks(engine, pages[5] * points_per_page);
    const PROVIDER_CASE no_backing[] = {
        {"checkpoint-short-no-backing", "fixture_checkpoints{netdata_dimension_id=\"native\"}", 64, 64, false, 128, 1, 0},
        {"checkpoint-long-no-backing", "fixture_checkpoints{netdata_dimension_id=\"native\"}", 68, 68, false, 0, 0,
         NETDATA_PROMQL_PROVIDER_ERROR},
    };
    for(size_t i = 0; i < sizeof(no_backing) / sizeof(no_backing[0]); i++)
        errors += provider_case(engine, &no_backing[i], NULL, NULL);
    double gap_sum = 0;
    unsigned gap_counter = 0;
    for(unsigned i = 1; i <= pages[5] * points_per_page; i++) {
        if(i == 69 || i == 73) gap_counter = 0;
        if(i != 73) { gap_counter += 2; gap_sum += gap_counter; }
    }
    unsigned points = pages[5] * points_per_page;
    char gap_expression[256], alias_expression[256], negative_expression[256];
    snprintfz(gap_expression, sizeof(gap_expression) - 1,
              "sum_over_time(fixture_checkpoints{netdata_dimension_id=\"reset_gap\"}[%us])", points);
    snprintfz(alias_expression, sizeof(alias_expression) - 1,
              "{__name__=\"fixture.checkpoints\",netdata_dimension_id=\"reset_gap\"}+"
              "fixture_checkpoints{netdata_dimension_id=\"reset_gap\"}");
    snprintfz(negative_expression, sizeof(negative_expression) - 1,
              "sum_over_time(fixture_checkpoints{netdata_dimension_id=\"legacy_negative\"}[%us])", points);
    const PROVIDER_CASE spill_values[] = {
        {"checkpoint-reset-gap", gap_expression, points, points, false, gap_sum, 1, 0},
        {"checkpoint-reset-gap-alias", alias_expression, points, points, false, 2.0 * gap_counter, 1, 0},
        {"checkpoint-negative", negative_expression, points, points, false, -(double)points * (points + 1), 1, 0},
        {"checkpoint-absolute", negative_expression, points, points, true, (double)points * (points + 1), 1, 0},
    };
    for(size_t i = 0; i < sizeof(spill_values) / sizeof(spill_values[0]); i++)
        errors += provider_case(engine, &spill_values[i], netdata_configured_cache_dir, NULL);
    rrdset_free(chart);
    return errors;
}

int promql_live_provider_fixture(void) {
    if(netdata_promql_abi_version() != NETDATA_PROMQL_ABI_VERSION) return 1;
    RRDSET *chart = rrdset_create(localhost, "promql_fixture", "provider", NULL, "fixture", "fixture.provider",
                                 "Provider fixture", "KiB", "fixture", "fixture", 1, 1, RRDSET_TYPE_LINE);
    if(!chart) return 1;
    RRDDIM *mixed = rrddim_add(chart, "mixed", NULL, 1, 1, RRD_ALGORITHM_ABSOLUTE);
    RRDDIM *positive = rrddim_add(chart, "positive", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *negative = rrddim_add(chart, "negative", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *legacy = rrddim_add(chart, "legacy", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *gauge = rrddim_add(chart, "gauge", NULL, 1, 1, RRD_ALGORITHM_ABSOLUTE);
    RRDDIM *reset_gap = rrddim_add(chart, "reset_gap", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *seam = rrddim_add(chart, "seam", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *seam_negative = rrddim_add(chart, "seam_negative", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *seam_native = rrddim_add(chart, "seam_native", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    if(!mixed || !positive || !negative || !legacy || !gauge || !reset_gap || !seam || !seam_negative ||
       !seam_native) { rrdset_free(chart); return 1; }
    rrdlabels_add(chart->rrdlabels, "__name__", "chart-value", RRDLABEL_SRC_AUTO);
    rrdlabels_add(chart->rrdlabels, "netdata_units", "chart-units", RRDLABEL_SRC_AUTO);
    for(unsigned i = 1; i <= 4097; i++) {
        usec_t when = (usec_t)(PROVIDER_BASE + i) * USEC_PER_SEC;
        rrddim_set_algorithm(chart, mixed, i <= 4 ? RRD_ALGORITHM_ABSOLUTE : RRD_ALGORITHM_INCREMENTAL);
        rrddim_store_metric(mixed, when, i <= 4 ? 100 : i == 10 ? NAN : 1,
                            SN_FLAG_NOT_ANOMALOUS | (i == 12 ? SN_FLAG_RESET : SN_FLAG_NONE));
        rrddim_store_metric(positive, when, 2, SN_FLAG_NOT_ANOMALOUS | (i == 20 ? SN_FLAG_RESET : SN_FLAG_NONE));
        rrddim_store_metric(negative, when, -2, SN_FLAG_NOT_ANOMALOUS);
        storage_engine_store_metric(legacy->tiers[0].sch, when, -2, -2, -2, 1, 0,
                                    SN_FLAG_NOT_ANOMALOUS | SP_FLAG_ORDINARY_SOURCE);
        rrddim_store_metric(gauge, when, 5, SN_FLAG_NOT_ANOMALOUS);
        rrddim_store_metric(reset_gap, when, i == 1 ? 50 : i == 2 ? NAN : 2,
                            SN_FLAG_NOT_ANOMALOUS | (i == 2 ? SN_FLAG_RESET : SN_FLAG_NONE));
    }
    // Direct tier writers make the retained fine/coarse boundary independent of collection aggregation.
    RRDDIM *seams[] = {seam, seam_negative, seam_native};
    for(size_t d = 0; d < sizeof(seams) / sizeof(seams[0]); d++) {
        storage_engine_store_change_collection_frequency(seams[d]->tiers[1].sch, 10);
        for(unsigned i = 1; i <= 20; i++)
            storage_engine_store_metric(seams[d]->tiers[0].sch, (usec_t)(PROVIDER_BASE + i) * USEC_PER_SEC,
                2, 2, 2, 1, 0, SN_FLAG_NOT_ANOMALOUS | SP_FLAG_COUNTER_SOURCE);
        for(unsigned i = 10; i <= 40; i += 10) {
            double sum = d == 1 ? -10 : 10;
            storage_engine_store_metric(seams[d]->tiers[1].sch, (usec_t)(PROVIDER_BASE + i) * USEC_PER_SEC,
                sum, sum / 10, sum / 10, 10, 0,
                SN_FLAG_NOT_ANOMALOUS | (d == 2 ? SP_FLAG_COUNTER_SOURCE : SP_FLAG_ORDINARY_SOURCE));
        }
        rrddim_finalize_collection_and_check_retention(seams[d]);
    }
    rrddim_finalize_collection_and_check_retention(mixed);
    rrddim_finalize_collection_and_check_retention(positive);
    rrddim_finalize_collection_and_check_retention(negative);
    rrddim_finalize_collection_and_check_retention(legacy);
    rrddim_finalize_collection_and_check_retention(gauge);
    rrddim_finalize_collection_and_check_retention(reset_gap);
    netdata_promql_engine_t *engine = NULL;
    int errors = netdata_promql_engine_create(NULL, &engine) != NETDATA_PROMQL_OK;
    const PROVIDER_CASE cases[] = {
        {"legacy-start", "{__name__=\"fixture.provider\",netdata_dimension_id=\"mixed\"}", 7, 30, false, 403.00000000000006, 1, 0},
        {"anchored-start", "fixture_provider{netdata_dimension_id=\"mixed\"}", 7, 1, false, 3, 1, 0},
        {"gap-prefix", "fixture_provider{netdata_dimension_id=\"mixed\"}", 11, 1, false, 6, 1, 0},
        {"reset-gap-prefix", "fixture_provider{netdata_dimension_id=\"reset_gap\"}", 3, 1, false, 2, 1, 0},
        {"alias-common-basis", "{__name__=\"fixture.provider\",netdata_dimension_id=\"mixed\"}+fixture_provider{netdata_dimension_id=\"mixed\"}", 7, 30, false, 806.00000000000011, 1, 0},
        {"aliases-logical", "sum({__name__=~\"fixture[._]provider\",netdata_dimension_id=\"positive\"})", 4, 30, false, 16, 1, 0},
        {"empty-grid", "fixture_provider{netdata_dimension_id=\"mixed\"}", 10, 30, false, 0, 0, 0},
        {"reset", "fixture_provider{netdata_dimension_id=\"mixed\"}", 12, 30, false, 1, 1, 0},
        {"positive-counter", "fixture_provider{netdata_dimension_id=\"negative\"}", 4, 30, false, 8, 1, 0},
        {"legacy-negative", "fixture_provider{netdata_dimension_id=\"legacy\"}", 4, 30, false, -8, 1, 0},
        {"legacy-absolute", "fixture_provider{netdata_dimension_id=\"legacy\"}", 4, 30, true, 8, 1, 0},
        {"gauge", "fixture_provider{netdata_dimension_id=\"gauge\"}", 4, 30, false, 5, 1, 0},
        {"fine-coarse-fine", "fixture_provider{netdata_dimension_id=\"seam\"}", 20, 30, false, 40, 1, 0},
        {"fine-coarse-first", "fixture_provider{netdata_dimension_id=\"seam\"}", 30, 40, false, 50, 1, 0},
        {"fine-coarse-last", "fixture_provider{netdata_dimension_id=\"seam\"}", 40, 50, false, 60, 1, 0},
        {"fine-coarse-negative", "fixture_provider{netdata_dimension_id=\"seam_negative\"}", 40, 50, false, 20, 1, 0},
        {"fine-coarse-absolute", "fixture_provider{netdata_dimension_id=\"seam_negative\"}", 40, 50, true, 60, 1, 0},
        {"fine-coarse-native", "fixture_provider{netdata_dimension_id=\"seam_native\"}", 40, 50, false, 60, 1, 0},
        {"fine-coarse-alias", "{__name__=\"fixture.provider\",netdata_dimension_id=\"seam\"}+fixture_provider{netdata_dimension_id=\"seam\"}", 40, 50, false, 120, 1, 0},
        {"fine-coarse-replay", "sum_over_time(fixture_provider{netdata_dimension_id=\"seam\"}[40s])", 40, 50, false, 530, 1, 0},
        {"reserved-chart-labels", "fixture_provider{netdata_dimension_id=\"gauge\",netdata_chart_label___name__=\"chart-value\",netdata_units=\"KiB\",netdata_chart_label_netdata_units=\"chart-units\"}", 4, 30, false, 5, 1, 0},
        {"backed-quantile", "quantile_over_time(.5, fixture_provider{netdata_dimension_id=\"positive\"}[4096s])", 4097, 30, false, 4061, 1, 0},
    };
    if(engine) {
        errors += promql_metadata_reference_fixture(engine, chart, gauge);
        for(size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++)
            errors += provider_case(engine, &cases[i], netdata_configured_cache_dir, NULL);
        errors += checkpoint_pages_fixture(engine);
        errors += promql_output_fixture(engine);
        errors += promql_request_fixture(engine);
        errors += promql_api_fixture();
        errors += mqtt_ng_source_unittest();
        errors += promql_aclk_source_fixture(engine);
        errors += promql_rtc_frame_fixture();
#ifdef NETDATA_INTERNAL_CHECKS
        const char *peer_fixture = getenv("PROMQL_RTC_PEER_FIXTURE");
        if(peer_fixture && !strcmp(peer_fixture, "1")) errors += webrtc_promql_peer_fixture();
#endif
        netdata_promql_selector_t *selector = NULL;
        netdata_promql_report_t *report = NULL;
        uint32_t status = netdata_promql_selector_compile(engine, promql_bytes("fixture_provider"), NULL, &selector, &report);
        errors += status != NETDATA_PROMQL_OK;
        if(report) netdata_promql_report_release(report);
        if(selector) {
            int64_t base = (int64_t)PROVIDER_BASE * 1000;
            errors += discovery_case(engine, selector, "negative", (netdata_promql_bounds_t){-2000, -1000}, 0);
            errors += discovery_case(engine, selector, "reversed", (netdata_promql_bounds_t){base + 2000, base + 1000}, 0);
            errors += discovery_case(engine, selector, "between-seconds", (netdata_promql_bounds_t){base + 1001, base + 1999}, 0);
            errors += discovery_case(engine, selector, "zero", (netdata_promql_bounds_t){0, 0}, 0);
            errors += discovery_case(engine, selector, "positive-wide", (netdata_promql_bounds_t){1, INT64_MAX}, 9);
            errors += discovery_case(engine, selector, "full-clock", (netdata_promql_bounds_t){INT64_MIN, INT64_MAX}, 9);
            errors += discovery_case(engine, selector, "future-wide", (netdata_promql_bounds_t){INT64_MAX - 999, INT64_MAX}, 0);
            netdata_promql_selector_release(selector);
        }
        errors += promql_operations_fixture(engine);
        netdata_promql_engine_release(engine);
    }
    rrdset_free(chart);
    return errors;
}
