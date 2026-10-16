// SPDX-License-Identifier: GPL-3.0-or-later

// Include the real owners so internal records and the pending/file seam are exercised unchanged.
#include "output.c"
#include "body.c"
#include <inttypes.h>
#include <time.h>

typedef union { max_align_t alignment; size_t bytes; } TEST_ALLOCATION;
static uint64_t held, peak, reads, read_bytes, writes, write_bytes, polls;
static int record_fd = -1, error_after_read, cancel_after_read;
static unsigned cancel_poll_delay;
static bool short_io, interrupt_read, zero_read, refuse_write, cancel_error;
static PROMQL_READ_FRAME *active;

void *output_test_allocate(size_t length) {
    if(length > SIZE_MAX - sizeof(TEST_ALLOCATION)) return NULL;
    TEST_ALLOCATION *allocation = calloc(1, sizeof(*allocation) + length);
    if(!allocation) return NULL;
    allocation->bytes = length;
    held += length + sizeof(*allocation);
    peak = MAX(peak, held);
    return allocation + 1;
}
size_t output_test_size(void *data) { return ((TEST_ALLOCATION *)data - 1)->bytes; }
void output_test_free(void *data) {
    if(!data) return;
    TEST_ALLOCATION *allocation = (TEST_ALLOCATION *)data - 1;
    held -= allocation->bytes + sizeof(*allocation);
    free(allocation);
}
bool output_test_poll(PROMQL_READ_FRAME *frame) {
    polls++;
    if(cancel_poll_delay && !--cancel_poll_delay) frame->ready = false;
    if(frame && !frame->ready) { frame->interrupted = NETDATA_PROMQL_CANCELLED; return false; }
    return true;
}
// Probe bodies never own Rust reports. Real report/account leases are qualified by the Agent fixture.
uint32_t netdata_promql_report_release(netdata_promql_report_t *report __maybe_unused) { abort(); }

ssize_t __real_pread64(int, void *, size_t, off64_t);
ssize_t __real_pwrite64(int, const void *, size_t, off64_t);
ssize_t __wrap_pread64(int fd, void *data, size_t length, off64_t offset) {
    if(fd == record_fd) {
        reads++;
        if(interrupt_read) { interrupt_read = false; errno = EINTR; return -1; }
        if(error_after_read && reads >= (uint64_t)error_after_read) {
            if(cancel_error) active->ready = false;
            errno = EIO; return -1;
        }
        if(zero_read) return 0;
    }
    ssize_t count = __real_pread64(fd, data, short_io ? MIN(length, (size_t)7) : length, offset);
    if(fd == record_fd && count > 0) {
        read_bytes += (uint64_t)count;
        if(cancel_after_read && reads >= (uint64_t)cancel_after_read) cancel_poll_delay = 2;
    }
    return count;
}
ssize_t __wrap_pwrite64(int fd, const void *data, size_t length, off64_t offset) {
    writes++;
    if(refuse_write) { errno = ENOSPC; return -1; }
    ssize_t count = __real_pwrite64(fd, data, short_io ? MIN(length, (size_t)7) : length, offset);
    if(count > 0) write_bytes += (uint64_t)count;
    return count;
}

typedef struct { PROMQL_BODY *body; uint64_t offset; PROMQL_READ_FRAME *frame; int error, errors; bool compare; } SINK;
static bool put(SINK *sink, const void *bytes, size_t length) {
    if(!sink->compare) return promql_body_append(sink->body, bytes, length, sink->frame, &sink->error);
    uint8_t actual[256];
    const uint8_t *expected = bytes;
    while(length) {
        size_t count = MIN(length, sizeof(actual));
        if(!promql_body_read(sink->body, sink->offset, actual, count, NULL, &sink->error) ||
           memcmp(actual, expected, count)) { sink->errors++; return false; }
        sink->offset += count; expected += count; length -= count;
    }
    return true;
}
static bool text(SINK *sink, const char *value) { return put(sink, value, strlen(value)); }
static bool payload(SINK *sink, uint64_t index, bool histogram, size_t padding) {
    char buffer[128];
    snprintf(buffer, sizeof(buffer), "[%"PRIu64",", UINT64_C(1700002800) + index);
    if(!text(sink, buffer)) return false;
    if(histogram) {
        if(!text(sink, "{\"count\":\"512\",\"sum\":\"131328\",\"buckets\":[")) return false;
        for(unsigned i = 0; i < 512; i++) {
            snprintf(buffer, sizeof(buffer), "%s[0,\"%u\",\"%u\",\"1\"]", i ? "," : "", i, i + 1);
            if(!text(sink, buffer)) return false;
        }
        if(!text(sink, "]}")) return false;
    }
    else if(!text(sink, "\"1\"")) return false;
    memset(buffer, ' ', sizeof(buffer));
    while(padding) {
        size_t count = MIN(padding, sizeof(buffer));
        if(!put(sink, buffer, count)) return false;
        padding -= count;
    }
    return text(sink, "]");
}

typedef struct {
    const char *name;
    uint64_t count;
    unsigned streams;
    size_t prefix, padding;
    bool histogram, matrix;
    unsigned fault;
} CASE;

static int run_case(const char *directory, CASE test, bool legacy) {
    int errors = 0, error = 0;
    peak = held = reads = read_bytes = writes = write_bytes = polls = 0;
    record_fd = -1; short_io = interrupt_read = zero_read = refuse_write = cancel_error = false;
    error_after_read = cancel_after_read = 0;
    cancel_poll_delay = 0;
    PROMQL_READ_FRAME frame = {.ready = true, .account = (void *)(uintptr_t)1};
    active = &frame;
    PROMQL_OUTPUT output = {0};
    output.records = promql_body_create(frame.account, directory, 0, &error);
    output.body = promql_body_create(frame.account, directory, 0, &error);
    output.window = promql_malloc(frame.account, 0, PROMQL_BODY_WINDOW);
    POINT_CHAIN chains[4];
    for(unsigned i = 0; i < 4; i++) chains[i] = (POINT_CHAIN){NO_RECORD, NO_RECORD, 0};
    if(!output.records || !output.body || !output.window) abort();
    if(test.fault == 14) {
        output.records->file = promql_file_open(directory, frame.account, 0, &error);
        if(!output.records->file || !promql_file_size(output.records->file, UINT64_C(3) << 30, &error)) abort();
        output.records->length = output.records->file->length;
    }
    SINK input = {.body = output.records, .frame = &frame};
    for(size_t i = 0; i < test.prefix; i++) if(!text(&input, " ")) abort();
    for(uint64_t i = 0; i < test.count; i++) {
        POINT_CHAIN *chain = &chains[i % test.streams];
        uint64_t offset = promql_body_length(output.records);
        POINT_RECORD record = {.next = NO_RECORD};
        if(!promql_body_append(output.records, &record, sizeof(record), &frame, &error) ||
           !payload(&input, i, test.histogram && i % test.streams == test.streams - 1, test.padding)) abort();
        record.length = promql_body_length(output.records) - offset - sizeof(record);
        if(!promql_body_write(output.records, offset, &record, sizeof(record), &frame, &error) ||
           (chain->last != NO_RECORD && !promql_body_write(output.records, chain->last, &offset, sizeof(offset), &frame, &error))) abort();
        if(chain->first == NO_RECORD) chain->first = offset;
        chain->last = offset; chain->count++;
        if(test.fault == 15 || (test.fault == 17 && i % 2)) {
            char gap[128]; memset(gap, ' ', sizeof(gap));
            for(unsigned part = 0; part < 128; part++)
                if(!promql_body_append(output.records, gap, sizeof(gap), &frame, &error)) abort();
        }
    }
    if(test.fault == 1) chains[0].count++;
    if(test.fault == 2) {
        uint64_t invalid = UINT64_MAX - 1;
        if(!promql_body_write(output.records, chains[0].first, &invalid, sizeof(invalid), &frame, &error)) abort();
    }
    if(test.fault == 3) {
        uint64_t invalid = UINT64_MAX;
        if(!promql_body_write(output.records, chains[0].first + sizeof(uint64_t), &invalid, sizeof(invalid), &frame, &error)) abort();
    }
    if(test.fault == 4) chains[0].count--;
    if(output.records->file) record_fd = output.records->file->fd;
    if(test.fault == 5) short_io = true;
    if(test.fault == 6) interrupt_read = true;
    if(test.fault == 7) error_after_read = 1;
    if(test.fault == 8) error_after_read = 2;
    if(test.fault == 9) zero_read = true;
    if(test.fault == 10) cancel_after_read = 2;
    if(test.fault == 11) frame.ready = false;
    if(test.fault == 12) { error_after_read = 1; cancel_error = true; }
    if(test.fault == 13) refuse_write = true;
    OUTPUT_SERIES metadata[4] = {0};
    output.series = metadata;
    output.count = test.fault == 15 || test.fault == 17 ? 2 : test.streams;
    if(test.fault == 16) {
        output.count = 1;
        metadata[0].floats = chains[0]; metadata[0].histograms = chains[1];
    }
    else {
        for(unsigned i = 0; i < test.streams; i++) {
            if(test.histogram && i == test.streams - 1) metadata[i].histograms = chains[i];
            else metadata[i].floats = chains[i];
        }
    }
    reads = read_bytes = writes = write_bytes = polls = 0;
    struct timespec start, finish;
    clock_gettime(CLOCK_MONOTONIC, &start);
    bool success = true;
    for(unsigned stream = 0; success && stream < test.streams; stream++) {
        memset(output.window, 0xa5, PROMQL_BODY_WINDOW);
        success = (!stream || promql_json_text(output.body, "|", &frame, &output.failure)) &&
                  write_chain(&output, chains[stream], test.matrix, &frame);
    }
    clock_gettime(CLOCK_MONOTONIC, &finish);
    uint64_t nanos = (uint64_t)(finish.tv_sec - start.tv_sec) * UINT64_C(1000000000) + finish.tv_nsec - start.tv_nsec;
    uint64_t acquired = reads, acquired_bytes = read_bytes, sent = writes, sent_bytes = write_bytes, polled = polls;
    int failure = output.failure;
    bool wanted = !test.fault || test.fault == 5 || test.fault == 6 || test.fault >= 14;
    errors += success != wanted;
    if(test.fault >= 7 && test.fault <= 9) errors += failure != EIO;
    if(test.fault == 10 || test.fault == 11) errors += failure != ECANCELED;
    if(test.fault == 12) errors += failure != EIO;
    if(test.fault == 13) errors += failure != ENOSPC;
    frame.ready = true; frame.interrupted = NETDATA_PROMQL_OK;
    short_io = interrupt_read = zero_read = refuse_write = cancel_error = false;
    error_after_read = cancel_after_read = 0;
    cancel_poll_delay = 0;
    if(success) {
        SINK expected = {.body = output.body, .compare = true};
        for(unsigned stream = 0; stream < test.streams; stream++) {
            if((stream && !text(&expected, "|")) || (test.matrix && !text(&expected, "["))) errors++;
            uint64_t index = stream;
            for(uint64_t point = 0; point < chains[stream].count; point++, index += test.streams) {
                if((point && !text(&expected, ",")) ||
                   !payload(&expected, index, test.histogram && stream == test.streams - 1, test.padding)) { errors++; break; }
            }
            if(test.matrix && !text(&expected, "]")) errors++;
        }
        errors += expected.errors || expected.offset != promql_body_length(output.body);
    }
    bool bound = true;
    if(test.count == 100000 && test.streams == 1 && !test.fault) {
        bound = acquired <= 512;
        if(!legacy) errors += !bound;
    }
    if(test.fault == 15 || test.fault == 17) {
        bound = acquired_bytes <= test.count * 32;
        errors += !bound;
    }
    if(test.histogram && test.streams == 1 && test.count == 1 && output.records->file)
        errors += acquired_bytes > output.records->file->length;
    uint64_t body_bytes = promql_body_length(output.body), observed_peak = peak;
    promql_body_release(output.records); promql_body_release(output.body); promql_free(output.window);
    errors += held != 0;
    printf("{\"case\":\"%s\",\"errors\":%d,\"success\":%s,\"failure\":%d,\"read_bound\":%s,\"record_reads\":%"PRIu64
           ",\"record_bytes\":%"PRIu64",\"writes\":%"PRIu64",\"write_bytes\":%"PRIu64",\"polls\":%"PRIu64
           ",\"chain_ns\":%"PRIu64",\"body_bytes\":%"PRIu64",\"isolated_peak\":%"PRIu64",\"held_after\":%"PRIu64"}\n",
           test.name, errors, success ? "true" : "false", failure, bound ? "true" : "false", acquired,
           acquired_bytes, sent, sent_bytes, polled, nanos, body_bytes, observed_peak, held);
    return errors;
}

int main(int argc, char **argv) {
    if(argc < 2 || argc > 3) return 2;
    bool legacy = argc == 3 && strcmp(argv[2], "--allow-legacy-reads") == 0;
    CASE cases[] = {
        {"empty", 0, 1, 0, 0, false, true, 0},
        {"instant", 1, 1, 0, 0, false, false, 0},
        {"tiny", 4, 1, 0, 0, false, true, 0},
        {"inline-8191", 1, 1, 8159, 0, false, true, 0},
        {"inline-8192", 1, 1, 8160, 0, false, true, 0},
        {"spill-8193", 1, 1, 8161, 0, false, true, 0},
        {"split-header", 5, 1, 8185, 0, false, true, 0},
        {"split-payload", 5, 1, 8170, 9, false, true, 0},
        {"long-histogram", 1, 1, 0, 0, true, true, 0},
        {"interleaved-floats", 513, 2, 7, 0, false, true, 0},
        {"mixed-chains", 36, 4, 8187, 0, true, true, 0},
        {"wide-offset", 5, 1, 8185, 0, false, true, 14},
        {"sparse-chain", 513, 1, 2, 0, false, true, 15},
        {"mixed-one-series", 18, 2, 7, 0, true, true, 16},
        {"bursty-chain", 513, 1, 2, 0, false, true, 17},
        {"large", 100000, 1, 2, 0, false, true, 0},
    };
    int errors = 0;
    for(size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) errors += run_case(argv[1], cases[i], legacy);
    const char *faults[] = {"count-long", "bad-next", "bad-length", "count-short", "short-io", "interrupted-read",
        "first-eio", "late-eio", "eof", "cancel-hit", "cancel-initial", "eio-before-cancel", "output-full"};
    for(unsigned i = 0; i < sizeof(faults) / sizeof(faults[0]); i++)
        errors += run_case(argv[1], (CASE){faults[i], 2049, 1, 2, 0, false, true, i + 1}, legacy);
    return errors ? 1 : 0;
}
