// SPDX-License-Identifier: GPL-3.0-or-later

#include "pages.inc"

typedef struct {
    uint8_t bytes[4096];
    struct journal_v2_header header;
    struct journal_metric_list metric;
    struct rrdengine_journalfile journal;
    struct rrdengine_datafile file;
    RRDENG_READ_PROMISE promise;
    RRDENG_READER reader;
} FIXTURE;

static void put(FIXTURE *f, size_t offset, const void *data, size_t bytes) {
    if(offset > sizeof(f->bytes) || bytes > sizeof(f->bytes) - offset) abort();
    memcpy(f->bytes + offset, data, bytes);
}

static void trailer(FIXTURE *f, size_t offset, size_t bytes) {
    struct journal_v2_block_trailer t = {.crc = crc32(0, f->bytes + offset, bytes)};
    put(f, offset + bytes, &t, sizeof(t));
}

static void initialize(FIXTURE *f, unsigned count, int invalid_index) {
    memset(f, 0, sizeof(*f));
    f->header.magic = JOURVAL_V2_MAGIC;
    f->header.start_time_ut = 1000 * USEC_PER_SEC;
    f->header.end_time_ut = 1100 * USEC_PER_SEC;
    f->header.extent_count = 1;
    f->header.extent_offset = sizeof(f->header);
    struct journal_extent_list extent = {.datafile_offset = 4096, .datafile_size = 4096, .pages = count};
    put(f, f->header.extent_offset, &extent, sizeof(extent));
    f->header.extent_trailer_offset = f->header.extent_offset + sizeof(extent);
    trailer(f, f->header.extent_offset, sizeof(extent));
    f->header.metric_count = 1;
    f->header.metric_offset = f->header.extent_trailer_offset + sizeof(struct journal_v2_block_trailer);
    f->metric.uuid[0] = 17;
    f->metric.entries = count;
    f->header.metric_trailer_offset = f->header.metric_offset + sizeof(f->metric);
    f->metric.page_offset = f->header.metric_trailer_offset + sizeof(struct journal_v2_block_trailer);
    put(f, f->header.metric_offset, &f->metric, sizeof(f->metric));
    trailer(f, f->header.metric_offset, sizeof(f->metric));
    struct journal_page_header h = {.crc = JOURVAL_V2_MAGIC, .entries = count, .uuid_offset = f->header.metric_offset};
    memcpy(h.uuid, f->metric.uuid, sizeof(h.uuid));
    h.crc = crc32(0, (uint8_t *)&h, sizeof(h));
    put(f, f->metric.page_offset, &h, sizeof(h));
    trace_first = f->metric.page_offset + sizeof(h);
    trace_bytes = count * sizeof(struct journal_page_list);
    f->header.page_offset = f->metric.page_offset;
    f->header.page_count = count;
    for(unsigned i = 0; i < count; i++) {
        struct journal_page_list page = {.delta_start_s = 10 * (i + 1), .delta_end_s = 10 * (i + 1),
            .extent_index = (int)i == invalid_index ? 99 : 0, .update_every_s = 10, .page_length = 4, .type = 1};
        put(f, trace_first + i * sizeof(page), &page, sizeof(page));
    }
    trailer(f, trace_first, trace_bytes);
    f->header.journal_v2_file_size = trace_first + trace_bytes + 2 * sizeof(struct journal_v2_block_trailer);
    put(f, 0, &f->header, sizeof(f->header));
    struct journal_v2_block_trailer last = {.crc = crc32(0, (uint8_t *)&f->header, sizeof(f->header))};
    put(f, f->header.journal_v2_file_size - sizeof(last), &last, sizeof(last));
    f->journal.mmap.data = f->bytes;
    f->journal.mmap.size = f->header.journal_v2_file_size;
    f->file = (struct rrdengine_datafile){.journalfile = &f->journal, .fileno = 7, .references = 1};
    memcpy(f->promise.uuid, f->metric.uuid, sizeof(nd_uuid_t));
    f->reader = (RRDENG_READER){.promise = &f->promise, .last = 1100, .verified_file = 7,
                               .verified_page = f->metric.page_offset};
    RRDENG_READ_ERROR error = {0};
    size_t offset;
    if(!journal_directory_verified(&f->journal, &f->header, NULL, &error) ||
       !journal_pages_verified(&f->journal, &f->metric, true, NULL, &error, &offset) || offset != trace_first)
        abort();
    trace_count = 0;
}

typedef struct {
    const char *name;
    unsigned count, minimum, maximum;
    int selected, invalid_index;
    unsigned probes, indices[4];
    bool cancel;
} CASE;

static int check_case(const CASE *c, bool overlap, bool reuse) {
    FIXTURE f;
    initialize(&f, c->count, c->invalid_index);
    f.reader.last = 1000 + c->maximum;
    TEST_CONTROL control = {.cancel_on = c->cancel ? 2 : 0};
    RRDENG_READ_OPERATION operation = {.context = &control};
    RRDENG_READ_ERROR error = {0};
    DEMAND_CANDIDATE candidate = {0};
    bool found = false;
    bool ok = overlap ? journal_metric_overlaps(&f.journal, &f.header, &f.metric,
                                                1000 + c->minimum, 1000 + c->maximum, &found) :
                        journal_candidate(&f.reader, &f.file, 1000 + c->minimum, &candidate, &operation, &error);
    bool selected = c->selected >= 0 && 10U * (c->selected + 1) <= c->maximum;
    bool valid = overlap || c->selected != c->invalid_index || c->invalid_index < 0;
    int errors = ok != (!c->cancel && valid);
    if(overlap) errors += found != selected;
    else {
        bool expected = !c->cancel && valid && selected;
        errors += !!candidate.file != expected;
        errors += f.file.references != (expected ? 2U : 1U);
        if(expected)
            errors += candidate.first != 1000 + 10 * (c->selected + 1) || candidate.last != candidate.first ||
                      candidate.cadence != 10 || candidate.block != 1 || candidate.bytes != 4096 || candidate.main;
        if(c->cancel) errors += control.polls != 2;
        candidate_release(&candidate);
    }
    unsigned expected[5], count = 0;
    // A singleton overlap's actual CRC check also copies its sole twenty-byte record.
    if(overlap && c->count == 1) expected[count++] = 0;
    unsigned probes = c->cancel ? 1 : c->probes - (reuse && c->selected >= 0 ? 1 : 0);
    for(unsigned i = 0; i < probes; i++) expected[count++] = c->indices[i];
    errors += trace_count != count;
    for(unsigned i = 0; i < trace_count && i < count; i++) errors += trace_indices[i] != expected[i];
    errors += f.file.references != 1 || f.journal.data_spinlock || protected_access_state.depth;
    printf("{\"case\":\"%s\",\"owner\":\"%s\",\"copies\":%u,\"polls\":%u,\"errors\":%d}\n",
           c->name, overlap ? "overlap" : "candidate", trace_count, control.polls, errors);
    return errors;
}

int main(int argc, char **argv) {
    bool reuse = argc == 2 && !strcmp(argv[1], "reuse");
    const CASE cases[] = {
        {.name="before", .count=4, .minimum=5, .maximum=100, .selected=0, .invalid_index=-1, .probes=4, .indices={2,1,0,0}},
        {.name="middle", .count=4, .minimum=15, .maximum=100, .selected=1, .invalid_index=-1, .probes=4, .indices={2,1,0,1}},
        {.name="gap", .count=4, .minimum=25, .maximum=100, .selected=2, .invalid_index=-1, .probes=3, .indices={2,1,2}},
        {.name="last", .count=4, .minimum=35, .maximum=100, .selected=3, .invalid_index=-1, .probes=3, .indices={2,3,3}},
        {.name="past", .count=4, .minimum=45, .maximum=100, .selected=-1, .invalid_index=-1, .probes=2, .indices={2,3}},
        {.name="upper-gap", .count=4, .minimum=25, .maximum=29, .selected=2, .invalid_index=-1, .probes=3, .indices={2,1,2}},
        {.name="upper-exact", .count=4, .minimum=25, .maximum=30, .selected=2, .invalid_index=-1, .probes=3, .indices={2,1,2}},
        {.name="invalid-selected", .count=4, .minimum=25, .maximum=100, .selected=2, .invalid_index=2, .probes=3, .indices={2,1,2}},
        {.name="invalid-discarded", .count=4, .minimum=25, .maximum=100, .selected=2, .invalid_index=1, .probes=3, .indices={2,1,2}},
        {.name="empty", .minimum=5, .maximum=100, .selected=-1, .invalid_index=-1},
        {.name="singleton", .count=1, .minimum=5, .maximum=100, .selected=0, .invalid_index=-1, .probes=2, .indices={0,0}},
        {.name="cancel-second", .count=4, .minimum=25, .maximum=100, .selected=2, .invalid_index=-1, .probes=3, .indices={2,1,2}, .cancel=true},
    };
    int errors = 0;
    for(unsigned i = 0; i < sizeof(cases) / sizeof(*cases); i++) {
        errors += check_case(&cases[i], false, reuse);
        if(!cases[i].cancel) errors += check_case(&cases[i], true, reuse);
    }
    return errors ? 1 : 0;
}
