// SPDX-License-Identifier: GPL-3.0-or-later

#include "demand.h"
#include "rrdengineapi.h"
#include "database/rrd.h"
#include "database/rrddim-collection.h"
#include "database/rrddim-backfill.h"

#define COLLECTION_BASE ((time_t)1700002800)

#ifdef ENABLE_PROMQL
int promql_live_provider_fixture(void);
#endif

static RRDSET *collection_chart(const char *id) {
    return rrdset_create(localhost, "promql_fixture", id, NULL, "fixture", "fixture.collection",
                         "Collection fixture", "KiB/s", "fixture", "fixture", 1, 1, RRDSET_TYPE_LINE);
}

static int collection_point(RRDDIM *rd, unsigned tier, const char *name, time_t when,
                             double sum, double minimum, double maximum, uint32_t count,
                             bool counter, double endpoint, SN_FLAGS required, unsigned duration) {
    RRDENG_READER *reader = NULL;
    RRDENG_READ_ERROR error;
    RRDENG_DEMAND_PAGE page;
    STORAGE_POINT point = STORAGE_POINT_UNSET;
    int errors = 0;
    METRIC *metric = (METRIC *)rd->tiers[tier].smh;
    if(!rrdeng_reader_open(metric, NULL, when, when, NULL, &reader, &error) ||
       rrdeng_reader_acquire(reader, when, NULL, &page, &error) != RRDENG_DEMAND_READY)
        errors++;
    else {
        uint32_t position = (uint32_t)((when - page.first) / page.cadence);
        if(!rrdeng_reader_point(reader, position, &point, NULL, &error)) errors++;
        else {
            errors += point.end_time_s != when || point.end_time_s - point.start_time_s != duration ||
                point.count != count || rrdeng_page_type_is_counter(pgd_type(page.data)) != counter ||
                (point.flags & required) != required;
            if(isnan(sum)) errors += !isnan(point.sum) || (point.flags & SP_FLAG_COUNTER_VALUE_VALID);
            else {
                errors += point.sum != sum;
                if(tier) errors += point.min != minimum || point.max != maximum;
                if(counter) errors += !(point.flags & SP_FLAG_COUNTER_VALUE_VALID) || point.counter != endpoint;
                else errors += !!(point.flags & SP_FLAG_COUNTER_VALUE_VALID);
            }
        }
    }
    if(errors)
        fprintf(stderr, "collection mismatch: %s tier%u time%jd code%u sum%.17g count%u min%.17g max%.17g counter%.17g flags%x\n",
                name, tier, (intmax_t)when, error.code, (double)point.sum, point.count,
                (double)point.min, (double)point.max, point.counter, point.flags);
    printf("{\"collection_case\":\"%s\",\"tier\":%u,\"errors\":%d}\n", name, tier, errors);
    rrdeng_reader_close(reader, NULL);
    return errors;
}

static int collection_buckets(void) {
    RRDSET *chart = collection_chart("buckets");
    if(!chart) return 1;
    chart->collection_modulo = 1;
    RRDDIM *mixed = rrddim_add(chart, "mixed", NULL, 1, 1, RRD_ALGORITHM_ABSOLUTE);
    RRDDIM *gap = rrddim_add(chart, "gap", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *negative = rrddim_add(chart, "negative", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    if(!mixed || !gap || !negative) { rrdset_free(chart); return 1; }
    for(unsigned i = 1; i <= 7201; i++) {
        rrddim_set_algorithm(chart, mixed, i <= 4 ? RRD_ALGORITHM_ABSOLUTE : RRD_ALGORITHM_INCREMENTAL);
        usec_t when = (usec_t)(COLLECTION_BASE + i) * USEC_PER_SEC;
        rrddim_store_metric(mixed, when, i <= 4 ? 100 : 1, SN_FLAG_NOT_ANOMALOUS);
        bool missing = i == 5 || (i >= 11 && i <= 20);
        rrddim_store_metric(gap, when, missing ? NAN : 2,
                            SN_FLAG_NOT_ANOMALOUS | (i == 15 ? SN_FLAG_RESET : SN_FLAG_NONE));
        rrddim_store_metric(negative, when, (i - 1) % 10 < 5 ? -2 : -4, SN_FLAG_NOT_ANOMALOUS);
    }
    // Publication must use the saved bucket provenance, including delayed pure counter buckets.
    rrddim_set_algorithm(chart, mixed, RRD_ALGORITHM_ABSOLUTE);
    rrddim_finalize_collection_and_check_retention(mixed);
    rrddim_finalize_collection_and_check_retention(gap);
    rrddim_finalize_collection_and_check_retention(negative);
    int errors = 0;
    const unsigned widths[] = {1, 10, 60, 600, 3600};
    for(unsigned tier = 1; tier < RRD_STORAGE_TIERS; tier++) {
        unsigned width = widths[tier];
        errors += collection_point(mixed, tier, "mixed-ordinary", COLLECTION_BASE + width,
                                    width + 396, 1, 100, width, false, 0, SP_FLAG_ORDINARY_SOURCE, width);
        errors += collection_point(mixed, tier, "mixed-counter", COLLECTION_BASE + 2 * width,
                                    width, 1, 1, width, true, width, SP_FLAG_COUNTER_SOURCE, width);
        errors += collection_point(negative, tier, "negative-extrema", COLLECTION_BASE + width,
                                    -3.0 * width, -4, -2, width, true, 3.0 * width,
                                    SP_FLAG_COUNTER_SOURCE | SP_FLAG_NEGATIVE_PRESENTATION, width);
        uint32_t count = tier == 1 ? 9 : width - 11;
        errors += collection_point(gap, tier, "partial-gap", COLLECTION_BASE + width,
                                    2.0 * count, 2, 2, count, true, 2.0 * width,
                                    SP_FLAG_HAS_GAP | (tier == 1 ? 0 : SN_FLAG_RESET), width);
    }
    errors += collection_point(gap, 1, "all-gap-reset", COLLECTION_BASE + 20,
                                NAN, NAN, NAN, 0, true, 0, SP_FLAG_HAS_GAP | SN_FLAG_RESET, 10);
    errors += collection_point(gap, 1, "after-gap-reset", COLLECTION_BASE + 30,
                                20, 2, 2, 10, true, 20, SP_FLAG_COUNTER_SOURCE, 10);
    // Ordinary tier0 retains its existing decimal-codec rounding.
    errors += collection_point(mixed, 0, "mixed-gauge-start", COLLECTION_BASE + 1,
                                100.00000000000001, 100, 100, 1, false, 0, SP_FLAG_ORDINARY_SOURCE, 1);
    errors += collection_point(mixed, 0, "mixed-counter-start", COLLECTION_BASE + 5,
                                1, 1, 1, 1, true, 1, SP_FLAG_COUNTER_SOURCE, 1);
    rrdset_free(chart);
    return errors;
}

static void collection_raw(RRDSET *chart, RRDDIM *rd, collected_number value, time_t when, unsigned elapsed) {
    rd->collector.last_collected_time = (struct timeval){.tv_sec = when};
    rrddim_set_collected_int(rd, value);
    rrddim_set_updated(rd);
    rd->collector.counter++;
    rrddim_update_collected_max_from_int(rd, value);
    chart->usec_since_last_update = (usec_t)elapsed * USEC_PER_SEC;
    rrdset_timed_done(chart, (struct timeval){.tv_sec = when}, false);
}

static int collection_raw_counter(void) {
    RRDSET *chart = collection_chart("raw");
    if(!chart) return 1;
    RRDDIM *rd = rrddim_add(chart, "counter", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    if(!rd) { rrdset_free(chart); return 1; }
    chart->last_collected_time = chart->last_updated = rd->collector.last_collected_time =
        (struct timeval){.tv_sec = COLLECTION_BASE - 1};
    collection_raw(chart, rd, 100, COLLECTION_BASE, 1);
    for(unsigned i = 1; i <= 8; i++)
        collection_raw(chart, rd, i < 6 ? 100 + 2 * i : 10 + 2 * (i - 6), COLLECTION_BASE + i, 1);
    rrdset_set_update_every_s(chart, 2);
    collection_raw(chart, rd, 18, COLLECTION_BASE + 10, 2);
    rrddim_finalize_collection_and_check_retention(rd);
    int errors = collection_point(rd, 0, "raw-counter", COLLECTION_BASE + 3,
                                  2, 2, 2, 1, true, 6, SP_FLAG_COUNTER_SOURCE, 1);
    errors += collection_point(rd, 0, "raw-reset", COLLECTION_BASE + 6,
                                0, 0, 0, 1, true, 0, SN_FLAG_RESET, 1);
    errors += collection_point(rd, 0, "raw-new-cadence", COLLECTION_BASE + 10,
                                2, 2, 2, 1, true, 8, SP_FLAG_COUNTER_SOURCE, 2);
    rrdset_free(chart);
    return errors;
}

static int collection_backfill(void) {
    RRDSET *chart = collection_chart("backfill");
    if(!chart) return 1;
    RRDDIM *fine = rrddim_add(chart, "fine", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    RRDDIM *coarse = rrddim_add(chart, "coarse", NULL, 1, 1, RRD_ALGORITHM_INCREMENTAL);
    if(!fine || !coarse) { rrdset_free(chart); return 1; }
    for(unsigned i = 1; i <= 21; i++)
        storage_engine_store_metric(fine->tiers[0].sch, (usec_t)(COLLECTION_BASE + i) * USEC_PER_SEC,
                                    5, 5, 5, 1, 0, SN_FLAG_NOT_ANOMALOUS | SP_FLAG_ORDINARY_SOURCE);
    for(unsigned i = 1; i <= 7; i++)
        storage_engine_store_metric(coarse->tiers[1].sch, (usec_t)(COLLECTION_BASE + 10 * i) * USEC_PER_SEC,
                                    50, 5, 5, 10, 0, SN_FLAG_NOT_ANOMALOUS | SP_FLAG_ORDINARY_SOURCE);
    RRD_BACKFILL saved = default_backfill;
    default_backfill = RRD_BACKFILL_FULL;
    int errors = !backfill_tier_from_smaller_tiers(fine, 1, COLLECTION_BASE + 21);
    errors += !backfill_tier_from_smaller_tiers(coarse, 2, COLLECTION_BASE + 70);
    default_backfill = saved;
    errors += (coarse->tiers[2].last_completed_point.flags & SP_LEGACY_TIER_UNKNOWN) != SP_LEGACY_TIER_UNKNOWN;
    rrddim_finalize_collection_and_check_retention(fine);
    rrddim_finalize_collection_and_check_retention(coarse);
    errors += collection_point(fine, 1, "ordinary-backfill", COLLECTION_BASE + 10,
                                50, 5, 5, 10, false, 0, SP_FLAG_ORDINARY_SOURCE, 10);
    errors += collection_point(coarse, 2, "coarse-backfill", COLLECTION_BASE + 60,
                                300, 5, 5, 60, false, 0, SP_LEGACY_TIER_UNKNOWN, 60);
    rrdset_free(chart);
    return errors;
}

int promql_dbengine_collection_fixture(void) {
    unsigned char saved_db[sizeof(localhost->db)];
    memcpy(saved_db, localhost->db, sizeof(saved_db));
    RRD_DB_MODE saved_mode = localhost->rrd_memory_mode;
    localhost->rrd_memory_mode = RRD_DB_MODE_DBENGINE;
    const unsigned widths[] = {1, 10, 60, 600, 3600};
    for(unsigned tier = 0; tier < RRD_STORAGE_TIERS; tier++) {
        localhost->db[tier].mode = RRD_DB_MODE_DBENGINE;
        localhost->db[tier].eng = storage_engine_get(RRD_DB_MODE_DBENGINE);
        localhost->db[tier].si = (STORAGE_INSTANCE *)multidb_ctx[tier];
        localhost->db[tier].tier_grouping = widths[tier];
    }
    int errors = collection_buckets();
    errors += collection_raw_counter();
    errors += collection_backfill();
#ifdef ENABLE_PROMQL
    errors += promql_live_provider_fixture();
#endif
    memcpy(localhost->db, saved_db, sizeof(saved_db));
    localhost->rrd_memory_mode = saved_mode;
    return errors;
}
