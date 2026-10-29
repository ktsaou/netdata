// SPDX-License-Identifier: GPL-3.0-or-later

#include "demand.h"

struct rrdeng_reader {
    PDC *pdc;
    const RRDENG_READ_PROMISE *promise;
    RRDENG_READ_PROMISE own_promise;
    RRDENG_DEMAND_PAGE page;
    PGDC cursor;
    time_t first, last;
    RRDENG_READ_ERROR error;
    RRDENG_READ_LEASE allocation, borrowed;
    uint32_t verified_file, verified_page;
};

typedef struct {
    time_t first, last;
    uint32_t cadence, block, bytes;
    struct rrdengine_datafile *file;
    bool main;
} DEMAND_CANDIDATE;

bool rrdeng_read_promise_valid(const RRDENG_READ_PROMISE *promise) {
    return promise && !__atomic_load_n(&promise->expired, __ATOMIC_ACQUIRE);
}

void rrdeng_read_expire_metric_unsafe(METRIC *metric, time_t first, time_t last) {
    struct rrdengine_instance *ctx = (struct rrdengine_instance *)mrg_metric_section(main_mrg, metric);
    for(RRDENG_READ_PROMISE *p = ctx->read_promises.head; p; p = p->next)
        if(p->metric == metric && rrdeng_read_promise_overlaps(p, first, last))
            __atomic_store_n(&p->expired, true, __ATOMIC_RELEASE);
}

static bool journal_copy(struct rrdengine_journalfile *journal, size_t offset, void *out, size_t bytes) {
    if(offset > journal->mmap.size || bytes > journal->mmap.size - offset)
        return false;
    volatile bool copied = false;
    {
        PROTECTED_ACCESS_SETUP(journal->mmap.data, journal->mmap.size, "dbengine journal", "read");
        if(no_signal_received) {
            memcpy(out, (uint8_t *)journal->mmap.data + offset, bytes);
            copied = true;
        }
    }
    return copied;
}

static bool journal_metric(struct rrdengine_journalfile *journal, const struct journal_v2_header *header,
                            const nd_uuid_t uuid, struct journal_metric_list *metric, bool *found) {
    *found = false;
    size_t size = journal->mmap.size;
    if(header->magic != JOURVAL_V2_MAGIC || header->metric_offset > size ||
       header->metric_count > (size - header->metric_offset) / sizeof(*metric))
        return false;
    if(!header->metric_count)
        return true;

    volatile bool complete = false;
    {
        // The acquired mapping and checked directory bounds cover every search probe.
        PROTECTED_ACCESS_SETUP(journal->mmap.data, journal->mmap.size, "dbengine journal", "read");
        if(no_signal_received) {
            uint32_t lo = 0, hi = header->metric_count;
            while(lo < hi) {
                uint32_t mid = lo + (hi - lo) / 2;
                memcpy(metric, (uint8_t *)journal->mmap.data + header->metric_offset + (size_t)mid * sizeof(*metric),
                       sizeof(*metric));
                int comparison = memcmp(metric->uuid, uuid, sizeof(nd_uuid_t));
                if(comparison < 0) lo = mid + 1;
                else if(comparison > 0) hi = mid;
                else { *found = true; break; }
            }
            complete = true;
        }
    }
    return complete;
}

static bool journal_directory_verified(struct rrdengine_journalfile *journal,
                                       const struct journal_v2_header *header,
                                       const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error);
static bool journal_pages_verified(struct rrdengine_journalfile *journal, const struct journal_metric_list *metric,
                                   bool verify_crc, const RRDENG_READ_OPERATION *operation,
                                   RRDENG_READ_ERROR *error, size_t *offset);

static bool journal_metric_overlaps(struct rrdengine_journalfile *journal,
                                    const struct journal_v2_header *header, const struct journal_metric_list *metric,
                                    time_t first, time_t last, bool *overlaps) {
    *overlaps = false;
    size_t offset;
    RRDENG_READ_ERROR error = {0};
    if(!journal_pages_verified(journal, metric, true, NULL, &error, &offset))
        return false;
    uint64_t base = header->start_time_ut / USEC_PER_SEC;
    uint32_t lo = 0, hi = metric->entries;
    struct journal_page_list page;
    while(lo < hi) {
        uint32_t mid = lo + (hi - lo) / 2;
        if(!journal_copy(journal, offset + (size_t)mid * sizeof(page), &page, sizeof(page)))
            return false;
        if(base + page.delta_end_s < (uint64_t)first) lo = mid + 1;
        else hi = mid;
    }
    if(lo < metric->entries) {
        if(!journal_copy(journal, offset + (size_t)lo * sizeof(page), &page, sizeof(page)))
            return false;
        *overlaps = base + page.delta_start_s <= (uint64_t)last;
    }
    return true;
}

void rrdeng_read_expire_file(struct rrdengine_datafile *datafile) {
    struct rrdengine_instance *ctx = datafile->ctx;
    struct rrdengine_journalfile *journal = datafile->journalfile;
    spinlock_tracked_lock(&journal->data_spinlock);
    time_t first = journal->v2.first_time_s, last = journal->v2.last_time_s;
    if(journal->failed_first > 0 && (!first || journal->failed_first < first))
        first = journal->failed_first;
    last = MAX(last, journal->failed_last);
    spinlock_tracked_unlock(&journal->data_spinlock);
    bool acquired = journalfile_v2_data_acquire_checked(journal) != NULL;
    struct journal_v2_header header;
    RRDENG_READ_ERROR error = {0};
    bool usable = acquired && journal_copy(journal, 0, &header, sizeof(header)) &&
                  journal_directory_verified(journal, &header, NULL, &error);
    if(usable && !first) {
        uint64_t header_first = header.start_time_ut / USEC_PER_SEC;
        uint64_t header_last = header.end_time_ut / USEC_PER_SEC;
        if((time_t)header_first > 0 && (uint64_t)(time_t)header_first == header_first &&
           (time_t)header_last >= (time_t)header_first && (uint64_t)(time_t)header_last == header_last) {
            first = (time_t)header_first;
            last = (time_t)header_last;
        }
    }
    bool known_range = first > 0 && last >= first;
    spinlock_lock(&ctx->read_promises.lock);
    for(RRDENG_READ_PROMISE *p = ctx->read_promises.head; p; p = p->next) {
        if(!p->required_count || (known_range && !rrdeng_read_promise_overlaps(p, first, last)))
            continue;
        struct journal_metric_list metric;
        bool found = false;
        bool complete = usable && journal_metric(journal, &header, p->uuid, &metric, &found);
        bool overlaps = false;
        for(uint32_t i = 0; complete && found && !overlaps && i < p->required_count; i++)
            complete = journal_metric_overlaps(journal, &header, &metric,
                                               p->required[i].first, p->required[i].last, &overlaps);
        if(!complete || overlaps)
            __atomic_store_n(&p->expired, true, __ATOMIC_RELEASE);
    }
    spinlock_unlock(&ctx->read_promises.lock);
    if(acquired) journalfile_v2_data_release(journal);
    spinlock_tracked_lock(&datafile->users.spinlock);
    bool deleting = datafile->users.pending_deletion;
    spinlock_tracked_unlock(&datafile->users.spinlock);
    if(!deleting) {
        spinlock_tracked_lock(&journal->data_spinlock);
        if(!journal->read_failure.code) {
            journal->read_failure = (RRDENG_READ_ERROR){ .code = RRDENG_READ_DESCRIPTOR, .file = datafile->fileno };
            journal->failed_first = first;
            journal->failed_last = last;
        }
        spinlock_tracked_unlock(&journal->data_spinlock);
    }
}

static bool journal_crc(struct rrdengine_journalfile *journal, size_t offset, size_t bytes,
                        uint32_t expected, const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error) {
    uint8_t buffer[1024];
    uLong crc = crc32(0L, Z_NULL, 0);
    while(bytes) {
        size_t count = MIN(bytes, sizeof(buffer));
        if(!rrdeng_read_operation_poll(operation)) {
            error->code = RRDENG_READ_CANCELLED;
            return false;
        }
        if(!journal_copy(journal, offset, buffer, count)) {
            error->code = RRDENG_READ_IO;
            return false;
        }
        crc = crc32(crc, buffer, count);
        offset += count;
        bytes -= count;
    }
    if(crc != expected) { error->code = RRDENG_READ_CHECKSUM; return false; }
    return true;
}

static bool journal_pages_verified(struct rrdengine_journalfile *journal, const struct journal_metric_list *metric,
                                   bool verify_crc, const RRDENG_READ_OPERATION *operation,
                                   RRDENG_READ_ERROR *error, size_t *offset) {
    size_t size = journal->mmap.size;
    struct journal_page_header page_header;
    if(metric->page_offset > size || sizeof(page_header) > size - metric->page_offset ||
       !journal_copy(journal, metric->page_offset, &page_header, sizeof(page_header))) {
        error->code = RRDENG_READ_DESCRIPTOR;
        return false;
    }
    uint32_t header_crc = page_header.crc;
    page_header.crc = JOURVAL_V2_MAGIC;
    if(header_crc != crc32(0L, (uint8_t *)&page_header, sizeof(page_header)) ||
       !nd_uuid_eq(page_header.uuid, metric->uuid) || page_header.entries != metric->entries) {
        error->code = RRDENG_READ_CHECKSUM;
        return false;
    }
    *offset = (size_t)metric->page_offset + sizeof(page_header);
    if(page_header.entries > (size - *offset) / sizeof(struct journal_page_list)) {
        error->code = RRDENG_READ_DESCRIPTOR;
        return false;
    }
    size_t bytes = (size_t)page_header.entries * sizeof(struct journal_page_list);
    struct journal_v2_block_trailer trailer;
    if(!journal_copy(journal, *offset + bytes, &trailer, sizeof(trailer))) {
        error->code = RRDENG_READ_DESCRIPTOR;
        return false;
    }
    return !verify_crc || journal_crc(journal, *offset, bytes, trailer.crc, operation, error);
}

static bool journal_directory_verified(struct rrdengine_journalfile *journal,
                                       const struct journal_v2_header *header,
                                       const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error) {
    size_t size = journal->mmap.size;
    struct journal_v2_block_trailer trailer;
    if(size < sizeof(*header) + sizeof(trailer) || header->magic != JOURVAL_V2_MAGIC ||
       header->journal_v2_file_size != size ||
       !journal_copy(journal, size - sizeof(trailer), &trailer, sizeof(trailer))) {
        error->code = RRDENG_READ_DESCRIPTOR;
        return false;
    }
    if(trailer.crc != crc32(0L, (const uint8_t *)header, sizeof(*header))) {
        error->code = RRDENG_READ_CHECKSUM;
        return false;
    }
    spinlock_tracked_lock(&journal->data_spinlock);
    bool verified = journal->read_directory_verified;
    spinlock_tracked_unlock(&journal->data_spinlock);
    if(verified) return true;
    const size_t offsets[] = {header->extent_offset, header->metric_offset};
    const size_t trailers[] = {header->extent_trailer_offset, header->metric_trailer_offset};
    const uint32_t counts[] = {header->extent_count, header->metric_count};
    const size_t strides[] = {sizeof(struct journal_extent_list), sizeof(struct journal_metric_list)};
    for(size_t i = 0; i < 2; i++) {
        if(offsets[i] > size || counts[i] > (size - offsets[i]) / strides[i] ||
           trailers[i] != offsets[i] + (size_t)counts[i] * strides[i] ||
           !journal_copy(journal, trailers[i], &trailer, sizeof(trailer))) {
            error->code = RRDENG_READ_DESCRIPTOR;
            return false;
        }
        if(!journal_crc(journal, offsets[i], (size_t)counts[i] * strides[i], trailer.crc, operation, error))
            return false;
    }
    spinlock_tracked_lock(&journal->data_spinlock);
    journal->read_directory_verified = true;
    spinlock_tracked_unlock(&journal->data_spinlock);
    return true;
}

static bool better_candidate(const DEMAND_CANDIDATE *candidate, const DEMAND_CANDIDATE *best, time_t minimum) {
    if(!best->first) return true;
    bool covers = candidate->first <= minimum, best_covers = best->first <= minimum;
    if(covers != best_covers) return covers;
    if(covers && candidate->cadence != best->cadence)
        return candidate->cadence && (!best->cadence || candidate->cadence < best->cadence);
    return candidate->first < best->first;
}

static void candidate_release(DEMAND_CANDIDATE *candidate) {
    if(candidate->file)
        datafile_release(candidate->file, DATAFILE_ACQUIRE_PAGE_DETAILS);
    *candidate = (DEMAND_CANDIDATE){0};
}

static bool cache_candidate(RRDENG_READER *reader, PGC *cache, bool main, time_t minimum,
                            DEMAND_CANDIDATE *best, RRDENG_READ_ERROR *error,
                            const RRDENG_READ_OPERATION *operation) {
    PDC *pdc = reader->pdc;
    time_t cursor = minimum;
    PGC_SEARCH search = PGC_SEARCH_CLOSEST;
    PGC_PAGE *page;
    while((page = pgc_page_get_and_acquire(cache, (Word_t)pdc->ctx, mrg_metric_id(main_mrg, pdc->metric), cursor, search))) {
        DEMAND_CANDIDATE candidate = {
            .first = pgc_page_start_time_s(page), .last = pgc_page_end_time_s(page),
            .cadence = pgc_page_update_every_s(page), .main = main,
        };
        cursor = candidate.first;
        search = PGC_SEARCH_NEXT;
        if(!rrdeng_read_operation_poll(operation)) {
            pgc_page_release(cache, page);
            error->code = RRDENG_READ_CANCELLED;
            return false;
        }
        if(candidate.first > reader->last) {
            pgc_page_release(cache, page);
            break;
        }
        if(candidate.last < minimum || (main && pgc_page_data(page) == PGD_EMPTY)) {
            pgc_page_release(cache, page);
            continue;
        }
        bool keep = better_candidate(&candidate, best, minimum);
        if(keep && !main) {
            struct extent_io_data data = *(struct extent_io_data *)pgc_page_custom_data(open_cache, page);
            candidate.file = pgc_page_data(page);
            candidate.block = data.block;
            candidate.bytes = data.bytes;
            if(!datafile_acquire(candidate.file, DATAFILE_ACQUIRE_PAGE_DETAILS)) {
                pgc_page_release(cache, page);
                error->code = RRDENG_READ_EXPIRED;
                return false;
            }
        }
        pgc_page_release(cache, page);
        if(keep) {
            candidate_release(best);
            *best = candidate;
        }
        break;
    }
    return true;
}

static bool journal_candidate(RRDENG_READER *reader, struct rrdengine_datafile *file, time_t minimum,
                              DEMAND_CANDIDATE *best, const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error) {
    struct rrdengine_journalfile *journal = file->journalfile;
    struct journal_v2_header header;
    struct journal_metric_list metric;
    bool found = false;
    if(!journal_copy(journal, 0, &header, sizeof(header)) ||
       !journal_directory_verified(journal, &header, operation, error) ||
       !journal_metric(journal, &header, reader->promise->uuid, &metric, &found))
        return false;
    if(!found)
        return true;
    size_t offset;
    if(!journal_pages_verified(journal, &metric,
                               reader->verified_file != file->fileno || reader->verified_page != metric.page_offset,
                               operation, error, &offset))
        return false;
    reader->verified_file = file->fileno;
    reader->verified_page = metric.page_offset;
    time_t base = header.start_time_ut / USEC_PER_SEC;
    uint32_t lo = 0, hi = metric.entries;
    struct journal_page_list page;
    while(lo < hi) {
        if(!rrdeng_read_operation_poll(operation) ||
           !journal_copy(journal, offset + (size_t)(lo + (hi - lo) / 2) * sizeof(page), &page, sizeof(page)))
            return false;
        uint32_t mid = lo + (hi - lo) / 2;
        if((uint64_t)base + page.delta_end_s < (uint64_t)minimum) lo = mid + 1;
        else hi = mid;
    }
    if(lo == metric.entries)
        return true;
    if(!journal_copy(journal, offset + (size_t)lo * sizeof(page), &page, sizeof(page)))
        return false;
    uint64_t first = (uint64_t)base + page.delta_start_s, last = (uint64_t)base + page.delta_end_s;
    if((time_t)first < 0 || (uint64_t)(time_t)first != first ||
       (time_t)last < 0 || (uint64_t)(time_t)last != last || first > last ||
       (!page.update_every_s && first != last))
        return false;
    if(first > (uint64_t)reader->last)
        return true;
    struct journal_extent_list extent;
    size_t size = journal->mmap.size;
    if(header.extent_offset > size || header.extent_count > (size - header.extent_offset) / sizeof(extent) ||
       page.extent_index >= header.extent_count ||
       !journal_copy(journal, header.extent_offset + (size_t)page.extent_index * sizeof(extent), &extent, sizeof(extent)) ||
       extent.datafile_offset % RRDENG_BLOCK_SIZE ||
       extent.datafile_offset / RRDENG_BLOCK_SIZE > UINT32_MAX || !rrdeng_valid_extent_disk_size(extent.datafile_size))
        return false;
    DEMAND_CANDIDATE candidate = {
        .first = first, .last = last, .cadence = page.update_every_s,
        .file = file, .block = OFFSET_TO_BLOCK(extent.datafile_offset), .bytes = extent.datafile_size,
    };
    if(better_candidate(&candidate, best, minimum)) {
        if(!datafile_acquire(file, DATAFILE_ACQUIRE_PAGE_DETAILS))
            return false;
        candidate_release(best);
        *best = candidate;
    }
    return true;
}

static bool failed_journal(RRDENG_READER *reader, time_t minimum, RRDENG_READ_ERROR *error) {
    struct rrdengine_instance *ctx = reader->pdc->ctx;
    bool failed = false;
    netdata_rwlock_rdlock(&ctx->datafiles.rwlock);
    Word_t index = 0;
    Pvoid_t *value;
    bool first = true;
    while((value = JudyLFirstThenNext(ctx->datafiles.JudyL, &index, &first))) {
        struct rrdengine_journalfile *journal = (*(struct rrdengine_datafile **)value)->journalfile;
        spinlock_tracked_lock(&journal->data_spinlock);
        bool known_range = journal->failed_first > 0 && journal->failed_last >= journal->failed_first;
        if(journal->read_failure.code &&
           (!known_range || (journal->failed_first <= reader->last && journal->failed_last >= minimum))) {
            *error = journal->read_failure;
            failed = true;
        }
        spinlock_tracked_unlock(&journal->data_spinlock);
        if(failed) break;
    }
    netdata_rwlock_rdunlock(&ctx->datafiles.rwlock);
    return failed;
}

static bool select_candidate(RRDENG_READER *reader, time_t minimum, DEMAND_CANDIDATE *candidate,
                             const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error) {
    if(!cache_candidate(reader, main_cache, true, minimum, candidate, error, operation))
        return false;
    if(candidate->main && candidate->first <= minimum)
        return true;
    if(!cache_candidate(reader, open_cache, false, minimum, candidate, error, operation) ||
       failed_journal(reader, minimum, error))
        return false;
    NJFV2IDX_FIND_STATE state = {
        .ctx = reader->pdc->ctx, .wanted_start_time_s = minimum, .wanted_end_time_s = reader->last, .checked = true,
    };
    struct rrdengine_datafile *file;
    while((file = njfv2idx_find_and_acquire_j2_header(&state))) {
        uint32_t file_id = file->fileno;
        bool ok = rrdeng_read_operation_poll(operation) && journal_candidate(reader, file, minimum, candidate, operation, error);
        journalfile_v2_data_release(file->journalfile);
        datafile_release(file, DATAFILE_ACQUIRE_PAGE_DETAILS);
        if(!ok) {
            if(!error->code)
                error->code = rrdeng_read_operation_poll(operation) ? RRDENG_READ_DESCRIPTOR : RRDENG_READ_CANCELLED;
            error->file = file_id;
            return false;
        }
    }
    if(state.failed) {
        *error = state.error;
        return false;
    }
    return true;
}

static void drain_reader(RRDENG_READER *reader, const RRDENG_READ_OPERATION *operation) {
    PDC *pdc = reader->pdc;
    unsigned jobs = 0;
    while(!pdc_demand_drained(pdc)) {
        if(!rrdeng_read_operation_poll(operation))
            __atomic_store_n(&pdc->workers_should_stop, true, __ATOMIC_RELEASE);
        jobs = completion_wait_for_a_job_with_timeout(&pdc->page_completion, jobs, 100);
    }
}

bool rrdeng_reader_open(METRIC *metric, const RRDENG_READ_PROMISE *promise,
                       time_t first, time_t last, const RRDENG_READ_OPERATION *operation,
                       RRDENG_READER **out, RRDENG_READ_ERROR *error) {
    *out = NULL;
    *error = (RRDENG_READ_ERROR){0};
    if(!rrdeng_read_operation_poll(operation)) { error->code = RRDENG_READ_CANCELLED; return false; }
    RRDENG_READ_LEASE allocation;
    if(!rrdeng_read_lease_acquire(operation, RRDENG_READ_MEMORY_WORKSPACE, sizeof(RRDENG_READER), &allocation)) {
        error->code = RRDENG_READ_ACCOUNTING;
        return false;
    }
    RRDENG_READER *reader = callocz(1, sizeof(*reader));
    reader->allocation = allocation;
    reader->pdc = pdc_create(metric, first, last, STORAGE_PRIORITY_NORMAL, operation, error);
    if(!reader->pdc) {
        freez(reader);
        rrdeng_read_lease_release(&allocation);
        return false;
    }
    if(!promise) {
        rrdeng_read_freeze_metric(metric, &reader->own_promise, first, last);
        promise = &reader->own_promise;
    }
    reader->promise = promise;
    reader->first = MAX(first, promise->first);
    reader->last = MIN(last, promise->last);
    *out = reader;
    return true;
}

RRDENG_DEMAND_STATUS rrdeng_reader_acquire(RRDENG_READER *reader, time_t minimum,
                       const RRDENG_READ_OPERATION *operation, RRDENG_DEMAND_PAGE *out,
                       RRDENG_READ_ERROR *error) {
    *out = (RRDENG_DEMAND_PAGE){0};
    *error = reader->error;
    if(error->code) return RRDENG_DEMAND_ERROR;
    if(!rrdeng_read_operation_poll(operation)) error->code = RRDENG_READ_CANCELLED;
    else if(!rrdeng_read_promise_valid(reader->promise)) error->code = RRDENG_READ_EXPIRED;
    if(error->code) goto failed;
    minimum = MAX(minimum, reader->first);
    if(reader->first <= 0 || minimum > reader->last) return RRDENG_DEMAND_END;
    if(reader->page.data && minimum >= reader->page.first && minimum <= reader->page.last) {
        *out = reader->page;
        return RRDENG_DEMAND_READY;
    }
    drain_reader(reader, operation);
    rrdeng_read_lease_release(&reader->borrowed);
    pdc_demand_release_page(reader->pdc);
    reader->page = (RRDENG_DEMAND_PAGE){0};
    DEMAND_CANDIDATE candidate = {0};
    if(!select_candidate(reader, minimum, &candidate, operation, error)) {
        candidate_release(&candidate);
        goto failed;
    }
    if(!rrdeng_read_promise_valid(reader->promise) || !rrdeng_read_operation_poll(operation)) {
        candidate_release(&candidate);
        error->code = rrdeng_read_promise_valid(reader->promise) ? RRDENG_READ_CANCELLED : RRDENG_READ_EXPIRED;
        goto failed;
    }
    if(!candidate.first) return RRDENG_DEMAND_END;
    if(candidate.first > minimum) {
        out->first = minimum; out->last = candidate.first - 1;
        candidate_release(&candidate);
        return RRDENG_DEMAND_GAP;
    }
    if(!pdc_demand_page(reader->pdc, operation)) {
        candidate_release(&candidate);
        error->code = RRDENG_READ_ACCOUNTING;
        goto failed;
    }
    struct page_details *pd = reader->pdc->demand_page;
    // Legacy singletons do not encode an interval. Match the ordinary cache reader fallback.
    if(!candidate.cadence && candidate.first == candidate.last) {
        candidate.cadence = mrg_metric_get_update_every_s(main_mrg, reader->pdc->metric);
        if(!candidate.cadence) candidate.cadence = nd_profile.update_every;
    }
    pd->metric_id = mrg_metric_id(main_mrg, reader->pdc->metric);
    pd->first_time_s = candidate.first; pd->last_time_s = candidate.last;
    pd->update_every_s = candidate.cadence;
    if(candidate.main) {
        pd->page = pgc_page_get_and_acquire(main_cache, (Word_t)reader->pdc->ctx, pd->metric_id, candidate.first, PGC_SEARCH_EXACT);
        if(!pd->page) { error->code = RRDENG_READ_EXPIRED; goto failed; }
        pdc_page_status_set(pd, PDC_PAGE_READY | PDC_PAGE_SOURCE_MAIN_CACHE);
    }
    else {
        pd->datafile.ptr = candidate.file;
        pd->datafile.block = candidate.block; pd->datafile.bytes = candidate.bytes;
        candidate.file = NULL;
        pd->status = PDC_PAGE_DATAFILE_ACQUIRED | PDC_PAGE_DISK_PENDING;
        pdc_acquire(reader->pdc);
        if(!rrdeng_enq_cmd_allocated(reader->pdc->ctx, RRDENG_OPCODE_QUERY, reader->pdc, NULL,
                       STORAGE_PRIORITY_NORMAL, NULL, NULL, &pd->router_allocation)) {
            pd->error.code = RRDENG_READ_ACCOUNTING;
            pdc_page_status_set(pd, PDC_PAGE_FAILED);
            pdc_release_and_destroy_if_unreferenced(reader->pdc, true, true);
        }
        drain_reader(reader, operation);
    }
    candidate_release(&candidate);
    if(pd->error.code) { *error = pd->error; goto failed; }
    if(!rrdeng_read_operation_poll(operation)) { error->code = RRDENG_READ_CANCELLED; goto failed; }
    if(!rrdeng_read_promise_valid(reader->promise)) { error->code = RRDENG_READ_EXPIRED; goto failed; }
    PGD *data = pd->private_page ? pd->private_page : pd->page ? pgc_page_data(pd->page) : NULL;
    if(!data || data == PGD_EMPTY || !pdc_page_status_check(pd, PDC_PAGE_READY)) {
        error->code = RRDENG_READ_PAYLOAD; goto failed;
    }
    if(pd->page && !pd->private_page) {
        RRDENG_PAGE_PROVENANCE *provenance = pgc_page_custom_data(main_cache, pd->page);
        rrdeng_page_provenance_lock(provenance);
        *error = provenance->error;
        rrdeng_page_provenance_unlock(provenance);
        if(error->code) goto failed;
    }
    uint64_t entries = pd->update_every_s && pd->last_time_s >= pd->first_time_s ?
        (uint64_t)(pd->last_time_s - pd->first_time_s) / pd->update_every_s + 1 : 0;
    if(!entries || entries > UINT32_MAX || entries > pgd_slots_used(data) ||
       (pd->last_time_s - pd->first_time_s) % pd->update_every_s) {
        error->code = RRDENG_READ_DESCRIPTOR; goto failed;
    }
    if(!pd->private_page && !rrdeng_read_lease_acquire(operation, RRDENG_READ_MEMORY_BORROWED,
                                                     pgd_memory_footprint(data), &reader->borrowed)) {
        error->code = RRDENG_READ_ACCOUNTING; goto failed;
    }
    reader->page = (RRDENG_DEMAND_PAGE){
        .data = data, .first = pd->first_time_s, .last = pd->last_time_s,
        .cadence = pd->update_every_s, .entries = entries,
    };
    pgdc_reset(&reader->cursor, data, 0);
    *out = reader->page;
    return RRDENG_DEMAND_READY;
failed:
    reader->error = *error;
    return RRDENG_DEMAND_ERROR;
}

bool rrdeng_reader_point(RRDENG_READER *reader, uint32_t position, STORAGE_POINT *point,
                        const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error) {
    *error = reader->error;
    if(error->code) return false;
    if(!rrdeng_read_operation_poll(operation)) {
        error->code = RRDENG_READ_CANCELLED;
        goto failed;
    }
    if(position >= reader->page.entries || !rrdeng_read_promise_valid(reader->promise)) {
        error->code = RRDENG_READ_EXPIRED;
        goto failed;
    }
    if(reader->cursor.position != position)
        pgdc_reset(&reader->cursor, reader->page.data, position);
    point->end_time_s = reader->page.first + (time_t)position * reader->page.cadence;
    point->start_time_s = point->end_time_s - reader->page.cadence;
    if(!pgdc_get_next_point(&reader->cursor, position, point) || (point->flags & SP_FLAG_COUNTER_INVALID)) {
        error->code = RRDENG_READ_PAYLOAD;
        error->slot = position;
        struct page_details *pd = reader->pdc->demand_page;
        if(pd) {
            if(pd->datafile.ptr) error->file = pd->datafile.ptr->fileno;
            error->block = pd->datafile.block;
            if(pd->page && !pd->private_page) {
                RRDENG_PAGE_PROVENANCE *provenance = pgc_page_custom_data(main_cache, pd->page);
                rrdeng_page_provenance_lock(provenance);
                if(!provenance->error.code) {
                    provenance->error = *error;
                    provenance->origin = RRDENG_PAGE_FAILED;
                }
                rrdeng_page_provenance_unlock(provenance);
            }
        }
        goto failed;
    }
    return true;
failed:
    reader->error = *error;
    return false;
}

bool rrdeng_reader_counter_before(RRDENG_READER *reader, uint32_t position, double *counter,
                                 const RRDENG_READ_OPERATION *operation, RRDENG_READ_ERROR *error) {
    *error = reader->error;
    if(error->code) return false;
    if(!rrdeng_read_operation_poll(operation)) {
        error->code = RRDENG_READ_CANCELLED;
        goto failed;
    }
    if(!rrdeng_read_promise_valid(reader->promise)) {
        error->code = RRDENG_READ_EXPIRED;
        goto failed;
    }
    if(position > reader->page.entries || !rrdeng_page_type_is_counter(pgd_type(reader->page.data))) {
        error->code = RRDENG_READ_PAYLOAD;
        error->slot = position;
        goto failed;
    }
    double value = pgd_counter_anchor(reader->page.data);
    if(position) {
        STORAGE_POINT prefix = STORAGE_POINT_UNSET;
        if(!rrdeng_reader_point(reader, position - 1, &prefix, operation, error)) return false;
        // Missing samples hide their value, but the recurrence still carries resets and the finite origin.
        value = reader->cursor.counter_value;
    }
    if(!isfinite(value)) {
        error->code = RRDENG_READ_PAYLOAD;
        error->slot = position;
        goto failed;
    }
    *counter = value;
    return true;
failed:
    reader->error = *error;
    return false;
}

void rrdeng_reader_close(RRDENG_READER *reader, const RRDENG_READ_OPERATION *operation) {
    if(!reader) return;
    __atomic_store_n(&reader->pdc->workers_should_stop, true, __ATOMIC_RELEASE);
    drain_reader(reader, operation);
    rrdeng_read_lease_release(&reader->borrowed);
    rrdeng_read_unfreeze_metric(&reader->own_promise);
    pdc_release_and_destroy_if_unreferenced(reader->pdc, false, false);
    RRDENG_READ_LEASE allocation = reader->allocation;
    freez(reader);
    rrdeng_read_lease_release(&allocation);
}

bool rrdeng_counter_recover(STORAGE_METRIC_HANDLE *smh, double *counter) {
    METRIC *metric = (METRIC *)smh;
    time_t first, last;
    mrg_metric_get_retention(main_mrg, metric, &first, &last, NULL);
    if(first <= 0 || last < first) return false;
    RRDENG_READER *reader = NULL;
    RRDENG_READ_ERROR error;
    if(!rrdeng_reader_open(metric, NULL, last, last, NULL, &reader, &error)) return false;
    RRDENG_DEMAND_PAGE page;
    bool usable = rrdeng_reader_acquire(reader, last, NULL, &page, &error) == RRDENG_DEMAND_READY &&
                  rrdeng_page_type_is_counter(pgd_type(page.data));
    STORAGE_POINT point = STORAGE_POINT_UNSET;
    for(uint32_t position = 0; usable && position < page.entries; position++)
        usable = rrdeng_reader_point(reader, position, &point, NULL, &error);
    if(usable && isfinite(reader->cursor.counter_value)) *counter = reader->cursor.counter_value;
    else usable = false;
    rrdeng_reader_close(reader, NULL);
    return usable;
}
