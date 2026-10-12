// SPDX-License-Identifier: GPL-3.0-or-later

#include "provider.h"
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

static int provider_case(netdata_promql_engine_t *engine, const PROVIDER_CASE *test) {
    PROMQL_PROVIDER_CONTEXT context = {.scope = PROMQL_HOST_GUID, .host = localhost->machine_guid,
        .backing_directory = netdata_configured_cache_dir, .absolute = test->absolute};
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
    printf("{\"provider_case\":\"%s\",\"status\":%u,\"errors\":%d,\"logical_pages\":%"PRIu64
           ",\"logical_samples\":%"PRIu64",\"cursor_points\":%"PRIu64",\"provider_folds\":%"PRIu64"}\n",
           test->name, status, errors, observations.pages, observations.samples_fetched,
           context.storage_points[0], context.reconstruction_points);
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
        for(size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) errors += provider_case(engine, &cases[i]);
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
