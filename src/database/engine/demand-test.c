// SPDX-License-Identifier: GPL-3.0-or-later

#include "demand.h"
#include "rrdengineapi.h"
#include "pagecache.h"
#include "database/rrd.h"
#include "daemon/config/netdata-conf-directories.h"

#define FIXTURE_POINTS 257U
#define FIXTURE_METRICS 7U
#define FIXTURE_START ((time_t)1700000000)

int promql_dbengine_collection_fixture(void);

typedef enum {
    READ_TEST_ACCEPT,
    READ_TEST_REFUSE_ACQUIRE,
    READ_TEST_REFUSE_GROWTH,
    READ_TEST_CANCEL_GROWTH,
    READ_TEST_REFUSE_AND_CANCEL_GROWTH,
} READ_TEST_MODE;

typedef struct {
    netdata_mutex_t lock;
    netdata_cond_t drained;
    uint64_t current[3], peak[3], case_peak[3], handles, acquisitions;
    READ_TEST_MODE mode;
    RRDENG_READ_MEMORY_KIND fault_kind;
    uint64_t fault_hits;
    bool cancelled, armed, positive_acquisition;
} READ_TEST_ACCOUNT;

typedef struct {
    READ_TEST_ACCOUNT *account;
    uint64_t bytes;
    RRDENG_READ_MEMORY_KIND kind;
} READ_TEST_LEASE;

static bool read_test_resize(void *allocation, uint64_t bytes) {
    READ_TEST_LEASE *lease = allocation;
    READ_TEST_ACCOUNT *account = lease->account;
    netdata_mutex_lock(&account->lock);
    if(bytes > lease->bytes && account->armed && lease->kind == account->fault_kind &&
       account->mode >= READ_TEST_REFUSE_GROWTH) {
        account->fault_hits++;
        if(account->mode == READ_TEST_CANCEL_GROWTH || account->mode == READ_TEST_REFUSE_AND_CANCEL_GROWTH)
            __atomic_store_n(&account->cancelled, true, __ATOMIC_RELEASE);
        if(account->mode != READ_TEST_CANCEL_GROWTH) {
            netdata_mutex_unlock(&account->lock);
            return false;
        }
    }
    account->current[lease->kind] -= lease->bytes;
    account->current[lease->kind] += bytes;
    account->peak[lease->kind] = MAX(account->peak[lease->kind], account->current[lease->kind]);
    account->case_peak[lease->kind] = MAX(account->case_peak[lease->kind], account->current[lease->kind]);
    lease->bytes = bytes;
    netdata_mutex_unlock(&account->lock);
    return true;
}

static void read_test_release(void *allocation) {
    READ_TEST_LEASE *lease = allocation;
    READ_TEST_ACCOUNT *account = lease->account;
    read_test_resize(allocation, 0);
    freez(lease);
    netdata_mutex_lock(&account->lock);
    account->handles--;
    if(!account->handles)
        netdata_cond_broadcast(&account->drained);
    netdata_mutex_unlock(&account->lock);
}

static bool read_test_acquire(void *context, RRDENG_READ_MEMORY_KIND kind, uint64_t bytes,
                              RRDENG_READ_LEASE *out) {
    READ_TEST_ACCOUNT *account = context;
    *out = (RRDENG_READ_LEASE){0};
    netdata_mutex_lock(&account->lock);
    if(account->armed && account->mode == READ_TEST_REFUSE_ACQUIRE && kind == account->fault_kind &&
       (!account->positive_acquisition || bytes)) {
        account->fault_hits++;
        netdata_mutex_unlock(&account->lock);
        return false;
    }
    netdata_mutex_unlock(&account->lock);
    READ_TEST_LEASE *lease = callocz(1, sizeof(*lease));
    lease->account = account;
    lease->kind = kind;
    lease->bytes = bytes;
    netdata_mutex_lock(&account->lock);
    account->handles++;
    account->acquisitions++;
    account->current[kind] += bytes;
    account->peak[kind] = MAX(account->peak[kind], account->current[kind]);
    account->case_peak[kind] = MAX(account->case_peak[kind], account->current[kind]);
    netdata_mutex_unlock(&account->lock);
    *out = (RRDENG_READ_LEASE){.allocation = lease, .resize = read_test_resize, .release = read_test_release};
    return true;
}

static bool read_test_poll(void *context) {
    return !__atomic_load_n(&((READ_TEST_ACCOUNT *)context)->cancelled, __ATOMIC_ACQUIRE);
}

static void read_test_drain(READ_TEST_ACCOUNT *account) {
    netdata_mutex_lock(&account->lock);
    // Work leases can survive page completion until libuv retires the callback.
    while(account->handles)
        netdata_cond_wait(&account->drained, &account->lock);
    fatal_assert(!account->current[0] && !account->current[1] && !account->current[2]);
    netdata_mutex_unlock(&account->lock);
}

static unsigned fixture_cadence(unsigned tier, unsigned metric, unsigned index) {
    const unsigned cadence[] = {1, 10, 60, 600, 3600};
    return cadence[tier] * (metric == 6 && index > 128 ? 2 : 1);
}

static time_t fixture_time(unsigned tier, unsigned metric, unsigned index) {
    unsigned base = fixture_cadence(tier, metric, 1);
    uint64_t elapsed = metric == 6 && index > 128 ? 128 + 2 * (uint64_t)(index - 128) : index;
    return FIXTURE_START + (time_t)(elapsed * base);
}

static bool fixture_counter(unsigned metric, unsigned index) {
    return metric != 1 || (index > 64 && index <= 128) || index > 192;
}

static bool fixture_gap(unsigned metric, unsigned index) {
    return metric == 3 || (metric == 2 && (index == 100 || index == 101));
}

static bool fixture_reset(unsigned metric, unsigned index) {
    return (metric == 2 && index == 100) || (metric == 3 && index == 70);
}

static double fixture_mean(unsigned metric, unsigned index) {
    return fixture_counter(metric, index) ? (metric == 4 ? -2.0 : 2.0) : 5.0;
}

static uint32_t fixture_count(unsigned tier, unsigned metric, unsigned index) {
    if(fixture_gap(metric, index)) return 0;
    return !tier ? 1 : metric == 1 ? 10 : 100000;
}

static void fixture_uuid(unsigned metric, nd_uuid_t *uuid) {
    char value[UUID_STR_LEN + 1];
    snprintfz(value, sizeof(value) - 1, "92d6da9f-4a6c-43c5-9dd5-%012u", metric + 1);
    fatal_assert(uuid_parse(value, *uuid) == 0);
}

// Legacy singletons have no stored interval; reopening must use the ordinary reader fallback.
static int fixture_singletons_write(struct rrdengine_instance *ctx) {
    int errors = 0;
    for(unsigned metric = 100; metric < 102; metric++) {
        nd_uuid_t uuid;
        fixture_uuid(metric, &uuid);
        bool added;
        METRIC *handle = mrg_metric_add_and_acquire(main_mrg, (MRG_ENTRY){
            .uuid = &uuid, .section = (Word_t)ctx,
        }, &added);
        if(!handle) { errors++; continue; }
        STORAGE_COLLECT_HANDLE *collector = rrdeng_store_metric_init((STORAGE_METRIC_HANDLE *)handle, 7, NULL);
        if(collector) {
            rrdeng_store_metric_next(collector, (usec_t)FIXTURE_START * USEC_PER_SEC,
                                    42, 42, 42, 1, 0, SN_FLAG_NOT_ANOMALOUS | SP_FLAG_ORDINARY_SOURCE);
            rrdeng_store_metric_finalize(collector);
        }
        else errors++;
        mrg_metric_release(main_mrg, handle);
    }
    return errors;
}

static int fixture_evict_clean(PGC *cache, Word_t section, Word_t metric, time_t first);

static int fixture_singleton_unknown_cache(struct rrdengine_instance *ctx, METRIC *metric) {
    Word_t id = mrg_metric_id(main_mrg, metric);
    PGC_PAGE *page = pgc_page_get_and_acquire(main_cache, (Word_t)ctx, id, FIXTURE_START, PGC_SEARCH_EXACT);
    if(!page) return 1;
    PGD *data = pgc_page_data(page);
    uint32_t type = pgd_type(data);
    bool singleton = pgd_slots_used(data) == 1;
    pgc_page_release(main_cache, page);
    if(!singleton || fixture_evict_clean(main_cache, (Word_t)ctx, id, FIXTURE_START)) return 1;
    // Recreate the same fixture value with unknown cached cadence, as an older reader can publish it.
    PGD *writer = pgd_create(type, 2);
    pgd_append_point(writer, (usec_t)FIXTURE_START * USEC_PER_SEC, 42, 42, 42, 1, 0,
                     SN_FLAG_NOT_ANOMALOUS, 0);
    uint8_t buffer[RRDENG_BLOCK_SIZE];
    uint32_t size = pgd_disk_footprint(writer);
    fatal_assert(size <= sizeof(buffer));
    pgd_copy_to_extent(writer, buffer, size);
    PGD *copy = pgd_create_from_disk_data(type, buffer, size);
    pgd_free(writer);
    RRDENG_PAGE_PROVENANCE provenance = {
        .origin = RRDENG_PAGE_LOADED, .first = FIXTURE_START, .last = FIXTURE_START, .entries = 1,
    };
    bool added;
    page = pgc_page_add_and_acquire(main_cache, (PGC_ENTRY){
        .section = (Word_t)ctx, .metric_id = id, .start_time_s = FIXTURE_START, .end_time_s = FIXTURE_START,
        .data = copy, .size = pgd_memory_footprint(copy), .custom_data = (uint8_t *)&provenance,
    }, &added);
    if(!added) pgd_free(copy);
    pgc_page_release(main_cache, page);
    return !added;
}

static int fixture_singleton_geometry(void) {
    int errors = 0;
    unsigned checks = 0;
    for(uint8_t type = 0; type <= RRDENG_PAGE_TYPE_MAX; type++) {
        bool counter = rrdeng_page_type_is_counter(type);
        struct rrdeng_extent_page_descr descriptor = {
            .type = type, .start_time_ut = (usec_t)FIXTURE_START * USEC_PER_SEC,
            .page_length = rrdeng_page_header_size(type) +
                (rrdeng_page_type_base(type) == RRDENG_PAGE_TYPE_GORILLA_32BIT ?
                 RRDENG_GORILLA_32BIT_BUFFER_SIZE : page_type_size[type]),
        };
        if(counter) {
            descriptor.counter.entries = 1;
            descriptor.counter.update_every_s = 7;
        }
        else if(type == RRDENG_PAGE_TYPE_GORILLA_32BIT) descriptor.gorilla.entries = 1;
        else descriptor.end_time_ut = descriptor.start_time_ut;
        VALIDATED_PAGE_DESCRIPTOR vd = validate_extent_page_descr(&descriptor, 0, 7, false);
        struct page_details readers[2] = {
            {.first_time_s = FIXTURE_START, .last_time_s = FIXTURE_START, .update_every_s = 7},
            {.first_time_s = FIXTURE_START, .last_time_s = FIXTURE_START, .update_every_s = 5},
        };
        errors += !vd.is_valid || vd.entries != 1 || !pdc_page_matches_descriptor(&readers[0], &vd);
        errors += pdc_page_matches_descriptor(&readers[1], &vd) != !counter;
        readers[1].update_every_s = 0;
        errors += pdc_page_matches_descriptor(&readers[1], &vd);
        readers[1].update_every_s = 5;
        readers[1].last_time_s++;
        errors += pdc_page_matches_descriptor(&readers[1], &vd);
        readers[1].last_time_s--;
        readers[1].first_time_s--;
        errors += pdc_page_matches_descriptor(&readers[1], &vd);
        readers[1].first_time_s++;
        vd.entries = 2;
        vd.end_time_s += 7;
        readers[1].last_time_s += 7;
        errors += pdc_page_matches_descriptor(&readers[1], &vd);
        if(counter) descriptor.counter.update_every_s = 0;
        vd = validate_extent_page_descr(&descriptor, 0, 0, false);
        errors += vd.is_valid == counter;
        checks += 7;
    }
    printf("{\"storage_geometry\":\"singleton-intervals\",\"checks\":%u,\"errors\":%d}\n", checks, errors);
    return errors;
}

static int fixture_singletons_read(struct rrdengine_instance *ctx, unsigned tier, READ_TEST_ACCOUNT *account) {
    int errors = 0;
    RRDENG_READ_OPERATION operation = {.context = account, .poll = read_test_poll, .acquire = read_test_acquire};
    for(unsigned metric = 100; metric < 102; metric++) {
        nd_uuid_t uuid;
        fixture_uuid(metric, &uuid);
        METRIC *handle = mrg_metric_get_and_acquire_by_uuid(main_mrg, &uuid, (Word_t)ctx);
        if(!handle) { errors++; continue; }
        int setup_errors = mrg_metric_get_update_every_s(main_mrg, handle) != 0;
        PGC_PAGE *descriptor = pgc_page_get_and_acquire(open_cache, (Word_t)ctx,
                mrg_metric_id(main_mrg, handle), FIXTURE_START, PGC_SEARCH_EXACT);
        bool open = descriptor != NULL;
        if(descriptor) {
            setup_errors += pgc_page_update_every_s(descriptor) != 0;
            pgc_page_release(open_cache, descriptor);
        }
        setup_errors += !open;
        uint32_t cadence = metric == 100 ? 5 : nd_profile.update_every;
        if(metric == 100) mrg_metric_set_update_every(main_mrg, handle, cadence);
        for(unsigned warm = 0; warm < (tier ? 2U : 4U); warm++) {
            if(warm == 1) setup_errors += fixture_singleton_unknown_cache(ctx, handle);
            // Tier0 has a migrated v2 index; evict its catalogue entry to exercise that route.
            if(warm == 2) {
                setup_errors += fixture_evict_clean(main_cache, (Word_t)ctx, mrg_metric_id(main_mrg, handle), FIXTURE_START);
                setup_errors += fixture_evict_clean(open_cache, (Word_t)ctx, mrg_metric_id(main_mrg, handle), FIXTURE_START);
            }
            RRDENG_READER *reader = NULL;
            RRDENG_READ_ERROR error = {0};
            RRDENG_DEMAND_PAGE page = {0};
            STORAGE_POINT point = STORAGE_POINT_UNSET;
            int failed = setup_errors;
            if(!rrdeng_reader_open(handle, NULL, FIXTURE_START, FIXTURE_START, &operation, &reader, &error))
                failed++;
            else if(rrdeng_reader_acquire(reader, FIXTURE_START, &operation, &page, &error) != RRDENG_DEMAND_READY)
                failed++;
            else {
                failed += page.entries != 1 || page.first != FIXTURE_START || page.last != FIXTURE_START ||
                          page.cadence != cadence;
                if(!rrdeng_reader_point(reader, 0, &point, &operation, &error)) failed++;
                else failed += point.end_time_s != FIXTURE_START || point.start_time_s != FIXTURE_START - cadence ||
                               point.sum != 42 || point.count != 1;
            }
            rrdeng_reader_close(reader, &operation);
            read_test_drain(account);
            printf("{\"storage_singleton\":\"%s-%s\",\"tier\":%u,\"source\":\"%s\","
                   "\"error_code\":%u,\"cadence\":%u,\"errors\":%d}\n",
                   metric == 100 ? "known" : "default",
                   warm == 3 ? "journal-warm" : warm == 2 ? "journal" : warm ? "warm" : "cold", tier,
                   warm == 2 ? "journal" : warm ? "main" : "open", error.code, page.cadence, failed);
            errors += failed;
        }
        mrg_metric_release(main_mrg, handle);
    }
    return errors;
}

static int fixture_write(struct rrdengine_instance *ctx, unsigned tier, unsigned first, unsigned last) {
    int errors = 0;
    for(unsigned metric = 0; metric < FIXTURE_METRICS; metric++) {
        nd_uuid_t uuid;
        fixture_uuid(metric, &uuid);
        bool added;
        METRIC *handle = mrg_metric_add_and_acquire(main_mrg, (MRG_ENTRY){
            .uuid = &uuid, .section = (Word_t)ctx,
        }, &added);
        if(!handle) { errors++; continue; }
        STORAGE_COLLECT_HANDLE *collector = rrdeng_store_metric_init((STORAGE_METRIC_HANDLE *)handle,
                                           fixture_cadence(tier, metric, first), NULL);
        if(!collector) {
            mrg_metric_release(main_mrg, handle);
            errors++;
            continue;
        }
        for(unsigned index = first; index <= last; index++) {
            if(metric == 6 && index == 129)
                rrdeng_store_metric_change_collection_frequency(collector, fixture_cadence(tier, metric, index));
            SN_FLAGS flags = SN_FLAG_NOT_ANOMALOUS |
                (fixture_counter(metric, index) ? SP_FLAG_COUNTER_SOURCE : SP_FLAG_ORDINARY_SOURCE);
            if(fixture_reset(metric, index)) flags |= SN_FLAG_RESET;
            if(fixture_gap(metric, index)) flags |= SP_FLAG_HAS_GAP;
            if(metric == 4) flags |= SP_FLAG_NEGATIVE_PRESENTATION;
            uint32_t count = fixture_count(tier, metric, index);
            double mean = fixture_mean(metric, index);
            double sum = fixture_gap(metric, index) ? NAN : mean * (count ? count : 1);
            rrdeng_store_metric_next(collector, (usec_t)fixture_time(tier, metric, index) * USEC_PER_SEC,
                                    sum, mean, mean, count, 0, flags);
            if(index == 64 || index == 170)
                rrdeng_store_metric_flush_current_page(collector);
        }
        // Rejected chronology must not select a different format or alter the next counter anchor.
        rrdeng_store_metric_next(collector, (usec_t)fixture_time(tier, metric, last) * USEC_PER_SEC,
                                999, 999, 999, 1, 0, SP_FLAG_ORDINARY_SOURCE);
        rrdeng_store_metric_finalize(collector);
        mrg_metric_release(main_mrg, handle);
    }
    return errors;
}

static int fixture_read_metric(struct rrdengine_instance *ctx, unsigned tier, unsigned metric,
                               unsigned last, READ_TEST_ACCOUNT *account) {
    nd_uuid_t uuid;
    fixture_uuid(metric, &uuid);
    METRIC *handle = mrg_metric_get_and_acquire_by_uuid(main_mrg, &uuid, (Word_t)ctx);
    if(!handle) return 1;
    RRDENG_READ_OPERATION operation = {.context = account, .poll = read_test_poll, .acquire = read_test_acquire};
    RRDENG_READER *reader = NULL;
    RRDENG_READ_ERROR error;
    time_t minimum = fixture_time(tier, metric, 1), end = fixture_time(tier, metric, last);
    int errors = 0;
    if(!rrdeng_reader_open(handle, NULL, minimum, end, &operation, &reader, &error)) {
        fprintf(stderr, "fixture open error: tier%u metric%u code%u\n", tier, metric, error.code);
        errors++;
        goto finished;
    }
    unsigned index = 1, pages = 0, gaps = 0;
    double expected_counter = 0;
    bool initialized = false;
    while(minimum <= end) {
        RRDENG_DEMAND_PAGE page;
        RRDENG_DEMAND_STATUS status = rrdeng_reader_acquire(reader, minimum, &operation, &page, &error);
        if(status == RRDENG_DEMAND_GAP && page.first == minimum && page.last >= minimum &&
           page.last < fixture_time(tier, metric, index)) {
            minimum = page.last + 1;
            gaps++;
            continue;
        }
        if(status != RRDENG_DEMAND_READY) {
            fprintf(stderr, "fixture read error: tier%u metric%u index%u status%u code%u file%u block%u slot%u\n",
                    tier, metric, index, status, error.code, error.file, error.block, error.slot);
            errors++;
            break;
        }
        pages++;
        for(uint32_t position = 0; position < page.entries && index <= last; position++, index++) {
            STORAGE_POINT point = STORAGE_POINT_UNSET;
            if(!rrdeng_reader_point(reader, position, &point, &operation, &error)) {
                fprintf(stderr, "fixture point error: tier%u metric%u index%u code%u\n", tier, metric, index, error.code);
                errors++;
                goto finished;
            }
            bool counter = fixture_counter(metric, index);
            if(counter && !initialized) {
                initialized = true;
                expected_counter = 0;
            }
            if(initialized) {
                if(fixture_reset(metric, index)) expected_counter = 0;
                if(!fixture_gap(metric, index))
                    expected_counter += fabs(fixture_mean(metric, index)) * fixture_cadence(tier, metric, index);
            }
            bool valid = point.end_time_s == fixture_time(tier, metric, index) &&
                point.end_time_s - point.start_time_s == fixture_cadence(tier, metric, index) &&
                rrdeng_page_type_is_counter(pgd_type(page.data)) == counter &&
                !!(point.flags & SN_FLAG_RESET) == fixture_reset(metric, index);
            if(fixture_gap(metric, index)) valid &= isnan(point.sum);
            else {
                uint32_t expected_count = fixture_count(tier, metric, index);
                valid &= point.count == expected_count && point.sum == fixture_mean(metric, index) * expected_count;
                if(tier) valid &= point.min == fixture_mean(metric, index) && point.max == fixture_mean(metric, index);
            }
            if(counter && !fixture_gap(metric, index))
                valid &= (point.flags & SP_FLAG_COUNTER_VALUE_VALID) && point.counter == expected_counter;
            else valid &= !(point.flags & SP_FLAG_COUNTER_VALUE_VALID) && isnan(point.counter);
            if(!valid) {
                fprintf(stderr, "fixture mismatch: tier%u metric%u index%u type%u sum%.17g count%u counter%.17g expected%.17g flags%x\n",
                        tier, metric, index, pgd_type(page.data), (double)point.sum, point.count,
                        point.counter, expected_counter, point.flags);
                errors++;
                goto finished;
            }
        }
        minimum = page.last + 1;
    }
    if(index != last + 1) errors++;
    printf("{\"storage_metric\":%u,\"tier\":%u,\"points\":%u,\"pages\":%u,\"gaps\":%u,\"errors\":%d}\n",
           metric, tier, index - 1, pages, gaps, errors);
finished:
    rrdeng_reader_close(reader, &operation);
    mrg_metric_release(main_mrg, handle);
    return errors;
}

static int fixture_evict_clean(PGC *cache, Word_t section, Word_t metric, time_t first) {
    PGC_PAGE *page = pgc_page_get_and_acquire(cache, section, metric, first, PGC_SEARCH_EXACT);
    if(!page) return 0;
    if(!pgc_is_page_clean(page)) {
        pgc_page_release(cache, page);
        return 1;
    }
    // This API consumes the reference even when it cannot evict the page.
    bool evicted = pgc_page_to_clean_evict_or_release(cache, page);
    page = pgc_page_get_and_acquire(cache, section, metric, first, PGC_SEARCH_EXACT);
    if(page) pgc_page_release(cache, page);
    return !evicted || page != NULL;
}

typedef struct {
    const char *name;
    READ_TEST_MODE mode;
    RRDENG_READ_MEMORY_KIND kind;
    RRDENG_READ_ERROR_CODE expected;
    bool cold, collision, before_open, positive_acquisition, cancel_point;
} READ_TEST_CASE;

static int fixture_failure_case(struct rrdengine_instance *ctx, METRIC *metric,
                                RRDENG_DEMAND_PAGE geometry, struct extent_io_data extent,
                                const READ_TEST_CASE *test, READ_TEST_ACCOUNT *account) {
    Word_t section = (Word_t)ctx, id = mrg_metric_id(main_mrg, metric);
    read_test_drain(account);
    int errors = 0;
    if(test->cold || test->collision) {
        errors += fixture_evict_clean(main_cache, section, id, geometry.first);
        errors += fixture_evict_clean(extent_cache, section, extent.fileno, extent.block);
    }
    PGC_PAGE *collision = NULL;
    if(test->collision && !errors) {
        RRDENG_PAGE_PROVENANCE provenance = {
            .origin = RRDENG_PAGE_FAILED, .error = {.code = RRDENG_READ_PAYLOAD},
            .first = geometry.first, .last = geometry.last,
            .cadence = geometry.cadence, .entries = geometry.entries,
        };
        bool added = false;
        collision = pgc_page_add_and_acquire(main_cache, (PGC_ENTRY){
            .section = section, .metric_id = id, .start_time_s = geometry.first,
            .end_time_s = geometry.last, .update_every_s = geometry.cadence,
            .data = PGD_EMPTY, .size = 0, .custom_data = (uint8_t *)&provenance,
        }, &added);
        errors += !collision || !added;
    }
    netdata_mutex_lock(&account->lock);
    account->mode = test->mode;
    account->fault_kind = test->kind;
    account->fault_hits = 0;
    account->armed = test->before_open;
    account->positive_acquisition = test->positive_acquisition;
    account->case_peak[0] = account->case_peak[1] = account->case_peak[2] = 0;
    __atomic_store_n(&account->cancelled, test->before_open && test->expected == RRDENG_READ_CANCELLED,
                     __ATOMIC_RELEASE);
    netdata_mutex_unlock(&account->lock);
    RRDENG_READ_OPERATION operation = {.context = account, .poll = read_test_poll, .acquire = read_test_acquire};
    RRDENG_READER *reader = NULL;
    RRDENG_READ_ERROR error = {0};
    RRDENG_DEMAND_PAGE page;
    if(!errors && rrdeng_reader_open(metric, NULL, geometry.first, geometry.last, &operation, &reader, &error)) {
        netdata_mutex_lock(&account->lock);
        account->armed = true;
        netdata_mutex_unlock(&account->lock);
        RRDENG_DEMAND_STATUS status = rrdeng_reader_acquire(reader, geometry.first, &operation, &page, &error);
        if(status == RRDENG_DEMAND_READY) {
            if(test->cancel_point)
                __atomic_store_n(&account->cancelled, true, __ATOMIC_RELEASE);
            STORAGE_POINT point = STORAGE_POINT_UNSET;
            bool valid = rrdeng_reader_point(reader, 0, &point, &operation, &error);
            if(test->expected == RRDENG_READ_OK)
                errors += !valid || point.end_time_s != geometry.first || point.sum != 2 || point.counter != 2;
            else errors += valid;
        }
        else errors += status != RRDENG_DEMAND_ERROR;
        if(test->expected != RRDENG_READ_OK) {
            RRDENG_READ_ERROR first = error, repeated;
            __atomic_store_n(&account->cancelled, false, __ATOMIC_RELEASE);
            status = rrdeng_reader_acquire(reader, geometry.first, &operation, &page, &repeated);
            errors += status != RRDENG_DEMAND_ERROR || repeated.code != first.code ||
                repeated.system_errno != first.system_errno || repeated.file != first.file ||
                repeated.block != first.block || repeated.slot != first.slot;
        }
    }
    errors += error.code != test->expected;
    rrdeng_reader_close(reader, &operation);
    read_test_drain(account);
    if(collision) {
        bool clean = pgc_is_page_clean(collision);
        if(clean) errors += !pgc_page_to_clean_evict_or_release(main_cache, collision);
        else { pgc_page_release(main_cache, collision); errors++; }
    }
    netdata_mutex_lock(&account->lock);
    errors += test->mode != READ_TEST_ACCEPT && !account->fault_hits;
    errors += test->collision && test->expected == RRDENG_READ_OK && !account->case_peak[RRDENG_READ_MEMORY_PAGE];
    printf("{\"storage_fault\":\"%s\",\"errors\":%d,\"code\":%u,\"file\":%u,\"block\":%u,\"slot\":%u,"
           "\"fault_hits\":%" PRIu64 ",\"workspace_peak\":%" PRIu64 ",\"private_peak\":%" PRIu64
           ",\"borrowed_peak\":%" PRIu64 ",\"remaining_handles\":%" PRIu64 "}\n",
           test->name, errors, error.code, error.file, error.block, error.slot, account->fault_hits,
           account->case_peak[0], account->case_peak[1], account->case_peak[2], account->handles);
    account->mode = READ_TEST_ACCEPT;
    account->armed = false;
    __atomic_store_n(&account->cancelled, false, __ATOMIC_RELEASE);
    netdata_mutex_unlock(&account->lock);
    return errors;
}

static int fixture_reader_failures(struct rrdengine_instance *ctx, READ_TEST_ACCOUNT *account) {
    nd_uuid_t uuid;
    fixture_uuid(0, &uuid);
    METRIC *metric = mrg_metric_get_and_acquire_by_uuid(main_mrg, &uuid, (Word_t)ctx);
    if(!metric) return 1;
    RRDENG_READ_OPERATION operation = {.context = account, .poll = read_test_poll, .acquire = read_test_acquire};
    RRDENG_READER *reader = NULL;
    RRDENG_READ_ERROR error;
    RRDENG_DEMAND_PAGE geometry;
    int errors = 0;
    if(!rrdeng_reader_open(metric, NULL, fixture_time(0, 0, 1), fixture_time(0, 0, 64),
                          &operation, &reader, &error) ||
       rrdeng_reader_acquire(reader, fixture_time(0, 0, 1), &operation, &geometry, &error) != RRDENG_DEMAND_READY)
        errors++;
    rrdeng_reader_close(reader, &operation);
    read_test_drain(account);
    struct extent_io_data extent = {0};
    PGC_PAGE *descriptor = !errors ? pgc_page_get_and_acquire(open_cache, (Word_t)ctx,
        mrg_metric_id(main_mrg, metric), geometry.first, PGC_SEARCH_EXACT) : NULL;
    if(descriptor) {
        extent = *(struct extent_io_data *)pgc_page_custom_data(open_cache, descriptor);
        pgc_page_release(open_cache, descriptor);
    }
    else errors++;
    const READ_TEST_CASE cases[] = {
        {.name = "warm-success"},
        {.name = "cancel-before-open", .before_open = true, .expected = RRDENG_READ_CANCELLED},
        {.name = "refuse-open", .before_open = true, .mode = READ_TEST_REFUSE_ACQUIRE,
         .kind = RRDENG_READ_MEMORY_WORKSPACE, .expected = RRDENG_READ_ACCOUNTING},
        {.name = "refuse-partial-page", .mode = READ_TEST_REFUSE_ACQUIRE,
         .kind = RRDENG_READ_MEMORY_PAGE, .expected = RRDENG_READ_ACCOUNTING},
        {.name = "refuse-warm-view", .mode = READ_TEST_REFUSE_ACQUIRE, .positive_acquisition = true,
         .kind = RRDENG_READ_MEMORY_BORROWED, .expected = RRDENG_READ_ACCOUNTING},
        {.name = "cancel-point", .cancel_point = true, .expected = RRDENG_READ_CANCELLED},
        {.name = "cold-success", .cold = true},
        {.name = "refuse-cold-routing", .cold = true, .mode = READ_TEST_REFUSE_GROWTH,
         .kind = RRDENG_READ_MEMORY_WORKSPACE, .expected = RRDENG_READ_ACCOUNTING},
        {.name = "refuse-worker-view", .cold = true, .mode = READ_TEST_REFUSE_GROWTH,
         .kind = RRDENG_READ_MEMORY_BORROWED, .expected = RRDENG_READ_ACCOUNTING},
        {.name = "cancel-worker-view", .cold = true, .mode = READ_TEST_CANCEL_GROWTH,
         .kind = RRDENG_READ_MEMORY_BORROWED, .expected = RRDENG_READ_CANCELLED},
        {.name = "private-collision-success", .cold = true, .collision = true},
        {.name = "private-refusal-precedes-cancel", .cold = true, .collision = true,
         .mode = READ_TEST_REFUSE_AND_CANCEL_GROWTH, .kind = RRDENG_READ_MEMORY_PAGE,
         .expected = RRDENG_READ_ACCOUNTING},
    };
    for(size_t i = 0; !errors && i < sizeof(cases) / sizeof(cases[0]); i++)
        errors += fixture_failure_case(ctx, metric, geometry, extent, &cases[i], account);
    mrg_metric_release(main_mrg, metric);
    return errors;
}

int promql_dbengine_storage_fixture(void) {
    const char *phase = getenv("PROMQL_STORAGE_FIXTURE_PHASE");
    const char *codec = getenv("PROMQL_STORAGE_FIXTURE_CODEC");
    if(!phase || (strcmp(phase, "write") && strcmp(phase, "read") && strcmp(phase, "append")) ||
       !codec || (strcmp(codec, "array") && strcmp(codec, "gorilla"))) return 1;
    tier_page_type[0] = !strcmp(codec, "array") ? RRDENG_PAGE_TYPE_ARRAY_32BIT : RRDENG_PAGE_TYPE_GORILLA_32BIT;
    nd_profile.storage_tiers = RRD_STORAGE_TIERS;
    struct rrdengine_instance *contexts[RRD_STORAGE_TIERS] = {0};
    READ_TEST_ACCOUNT account = {0};
    netdata_mutex_init(&account.lock);
    fatal_assert(netdata_cond_init(&account.drained) == 0);
    int errors = 0;
    unsigned initialized = 0;
    for(unsigned tier = 0; tier < RRD_STORAGE_TIERS; tier++) {
        char path[PATH_MAX];
        snprintfz(path, sizeof(path) - 1, "%s/storage-tier%u", netdata_configured_cache_dir, tier);
        if(rrdeng_init(NULL, path, 128, tier, 0)) { errors++; break; }
        contexts[tier] = multidb_ctx[tier];
        initialized++;
        rrdeng_readiness_wait(contexts[tier]);
    }
    bool writing = strcmp(phase, "read") != 0;
    unsigned last = !strcmp(phase, "append") ? FIXTURE_POINTS + 1 : FIXTURE_POINTS;
    for(unsigned tier = 0; !errors && tier < initialized; tier++) {
        if(writing && !strcmp(phase, "write")) errors += fixture_singletons_write(contexts[tier]);
        if(writing) errors += fixture_write(contexts[tier], tier, !strcmp(phase, "append") ? last : 1, last);
        for(unsigned metric = 0; !errors && metric < FIXTURE_METRICS; metric++)
            errors += fixture_read_metric(contexts[tier], tier, metric, last, &account);
    }
    if(!errors && !writing)
        errors += fixture_singleton_geometry();
    if(!errors && !writing)
        for(unsigned tier = 0; tier < initialized; tier++)
            errors += fixture_singletons_read(contexts[tier], tier, &account);
    if(!errors && !writing)
        errors += fixture_reader_failures(contexts[0], &account);
    if(!errors && !writing)
        errors += promql_dbengine_collection_fixture();
    // Keep the static tier contexts live until their caches and retained file references are destroyed.
    bool saved_unittest = unittest_running;
    unittest_running = false;
    for(unsigned tier = 0; tier < initialized; tier++) {
        rrdeng_quiesce(contexts[tier]);
        errors += rrdeng_exit(contexts[tier]);
    }
    unittest_running = saved_unittest;
    if(initialized) {
        dbengine_shutdown();
        pgc_destroy(extent_cache, false);
        pgc_destroy(open_cache, false);
        pgc_destroy(main_cache, false);
        errors += !!mrg_destroy(main_mrg);
        for(unsigned tier = 0; tier < initialized; tier++) finalize_rrd_files(contexts[tier]);
    }
    netdata_mutex_lock(&account.lock);
    errors += !!(account.handles || account.current[0] || account.current[1] || account.current[2]);
    printf("{\"storage_phase\":\"%s\",\"codec\":\"%s\",\"errors\":%d,\"lease_acquisitions\":%" PRIu64
           ",\"workspace_peak\":%" PRIu64 ",\"private_peak\":%" PRIu64 ",\"borrowed_peak\":%" PRIu64 "}\n",
           phase, codec, errors, account.acquisitions, account.peak[0], account.peak[1], account.peak[2]);
    netdata_mutex_unlock(&account.lock);
    netdata_cond_destroy(&account.drained);
    netdata_mutex_destroy(&account.lock);
    return errors != 0;
}
