// SPDX-License-Identifier: GPL-3.0-or-later

// Reuse the real owners and isolated controls; this probe owns no Rust reports.
#define main output_records_reader_main
#define netdata_promql_report_release point_test_report_release
#define __wrap_pwrite64 point_test_pwrite64
#include "output_records.c"
#undef __wrap_pwrite64
#undef netdata_promql_report_release
#undef main
#include "point-originals.h"
#include <limits.h>

int web_gzip_level, web_gzip_strategy;
static uint64_t float_calls, time_calls, fail_write_after;
static unsigned formatter_failure;
static bool cancel_after_float, interrupt_write, write_error_cancel;
static int write_error;

uint32_t __real_netdata_promql_api_float_text(uint64_t, netdata_promql_numeric_text_t *);
uint32_t __real_netdata_promql_api_timestamp_text(int64_t, netdata_promql_numeric_text_t *);
uint32_t __wrap_netdata_promql_api_float_text(uint64_t bits, netdata_promql_numeric_text_t *out) {
    float_calls++;
    if(formatter_failure == 2) return NETDATA_PROMQL_INTERNAL_ERROR;
    uint32_t status = __real_netdata_promql_api_float_text(bits, out);
    if(cancel_after_float) active->ready = false;
    return status;
}
uint32_t __wrap_netdata_promql_api_timestamp_text(int64_t time, netdata_promql_numeric_text_t *out) {
    time_calls++;
    if(formatter_failure == 1) return NETDATA_PROMQL_INTERNAL_ERROR;
    return __real_netdata_promql_api_timestamp_text(time, out);
}
ssize_t __wrap_pwrite64(int fd, const void *data, size_t length, off64_t offset) {
    if(interrupt_write) { interrupt_write = false; writes++; errno = EINTR; return -1; }
    if(fail_write_after && writes + 1 >= fail_write_after) {
        writes++;
        if(write_error_cancel) active->ready = false;
        errno = write_error;
        return -1;
    }
    return point_test_pwrite64(fd, data, length, offset);
}

typedef struct {
    const char *name;
    uint64_t count;
    size_t prefix;
    unsigned streams, fault;
    bool large, mixed, overwrite;
} APPEND_CASE;

static netdata_promql_sample_t sample(uint64_t index, bool large) {
    return (netdata_promql_sample_t){
        .time_ms = large ? INT64_C(1700002800000) + (int64_t)index * 1000 :
            original_times[(index / 21) % 10].milliseconds,
        .value = {.kind = NETDATA_PROMQL_VALUE_FLOAT, .payload.float_bits =
            large ? UINT64_C(0x3ff0000000000000) : original_floats[index % 21].bits}};
}

static bool expected(SINK *sink, uint64_t index, bool large) {
    char time[32];
    if(large) snprintf(time, sizeof(time), "%"PRIu64, UINT64_C(1700002800) + index);
    return text(sink, "[") && text(sink, large ? time : original_times[(index / 21) % 10].json) &&
        text(sink, ",") && text(sink, large ? "\"1\"" : original_floats[index % 21].json) && text(sink, "]");
}

static PROMQL_OUTPUT *create_output(const char *directory, unsigned streams, PROMQL_READ_FRAME *frame) {
    int error;
    PROMQL_OUTPUT *output = promql_output_create(frame->account, directory, 0, false, false, &error);
    if(!output) abort();
    output->records = promql_body_create(frame->account, directory, 0, &error);
    output->window = promql_malloc(frame->account, 0, PROMQL_BODY_WINDOW);
    output->series = promql_malloc(frame->account, 0, streams * sizeof(*output->series));
    if(!output->records || !output->window || !output->series) abort();
    output->count = streams;
    for(unsigned i = 0; i < streams; i++)
        output->series[i] = (OUTPUT_SERIES){.id = i,
            .floats = {NO_RECORD, NO_RECORD, 0}, .histograms = {NO_RECORD, NO_RECORD, 0}};
    return output;
}

static bool seed_histogram(PROMQL_OUTPUT *output, OUTPUT_SERIES *series, uint64_t index,
                            PROMQL_READ_FRAME *frame) {
    POINT_CHAIN *chain = &series->histograms;
    uint64_t offset = promql_body_length(output->records);
    POINT_RECORD record = {.next = NO_RECORD};
    SINK sink = {.body = output->records, .frame = frame};
    if(!put(&sink, &record, sizeof(record)) || !payload(&sink, index, true, 0)) return false;
    record.length = promql_body_length(output->records) - offset - sizeof(record);
    if(!promql_body_write(output->records, offset, &record, sizeof(record), frame, &sink.error) ||
       (chain->last != NO_RECORD && !promql_body_write(output->records, chain->last, &offset, sizeof(offset), frame, &sink.error)))
        return false;
    if(chain->first == NO_RECORD) chain->first = offset;
    chain->last = offset;
    chain->count++;
    return true;
}

static int check_chain(PROMQL_OUTPUT *output, const POINT_CHAIN *chain, bool histogram, bool large,
                        uint64_t *total) {
    uint64_t offset = chain->first;
    int errors = 0, error = 0;
    for(uint64_t i = 0; i < chain->count; i++) {
        POINT_RECORD record;
        if(!promql_body_read(output->records, offset, &record, sizeof(record), NULL, &error)) return errors + 1;
        SINK sink = {.body = output->records, .offset = offset + sizeof(record), .compare = true};
        if(!(histogram ? payload(&sink, i, true, 0) : expected(&sink, i, large)) || sink.errors ||
           record.length != sink.offset - offset - sizeof(record)) errors++;
        *total += sizeof(record) + sink.offset - offset - sizeof(record);
        if(i + 1 == chain->count) {
            if(record.next != NO_RECORD || chain->last != offset) errors++;
        }
        else if(record.next <= offset || record.next >= promql_body_length(output->records)) return errors + 1;
        offset = record.next;
    }
    if(!chain->count && (chain->first != NO_RECORD || chain->last != NO_RECORD)) errors++;
    return errors;
}

static void reset_controls(void) {
    if(held) abort();
    peak = reads = read_bytes = writes = write_bytes = polls = float_calls = time_calls = 0;
    record_fd = -1;
    short_io = interrupt_read = zero_read = refuse_write = cancel_error = false;
    error_after_read = cancel_after_read = 0;
    cancel_poll_delay = formatter_failure = 0;
    cancel_after_float = interrupt_write = write_error_cancel = false;
    fail_write_after = 0;
    write_error = 0;
}

static unsigned terminal_polls(const char *directory, bool predecessor) {
    reset_controls();
    PROMQL_READ_FRAME frame = {.ready = true, .account = (void *)(uintptr_t)1};
    active = &frame;
    PROMQL_OUTPUT *output = create_output(directory, 1, &frame);
    netdata_promql_bytes_t error;
    if(predecessor) {
        netdata_promql_sample_t first = sample(0, false);
        if(point_append(output, output->series, &first, &frame, &error) != NETDATA_PROMQL_OK) abort();
    }
    polls = 0;
    netdata_promql_sample_t point = sample(predecessor ? 1 : 0, false);
    if(point_append(output, output->series, &point, &frame, &error) != NETDATA_PROMQL_OK || polls > UINT_MAX) abort();
    unsigned count = (unsigned)polls;
    promql_output_destroy(output);
    if(held) abort();
    return count;
}

static int append_case(const char *directory, APPEND_CASE test, bool legacy) {
    unsigned terminal = test.fault == 3 || test.fault == 4 ? terminal_polls(directory, test.fault == 4) : 0;
    reset_controls();
    PROMQL_READ_FRAME frame = {.ready = true, .account = (void *)(uintptr_t)1};
    active = &frame;
    PROMQL_OUTPUT *output = create_output(test.fault == 8 ? "" : directory, test.streams, &frame);
    SINK sink = {.body = output->records, .frame = &frame};
    char prefix[256];
    memset(prefix, ' ', sizeof(prefix));
    for(size_t left = test.prefix; left;) {
        size_t count = MIN(left, sizeof(prefix));
        if(!put(&sink, prefix, count)) abort();
        left -= count;
    }
    netdata_promql_bytes_t message = {0};
    uint64_t completed = 0;
    if(test.fault == 4) {
        netdata_promql_sample_t first = sample(0, false);
        if(point_append(output, output->series, &first, &frame, &message) != NETDATA_PROMQL_OK) abort();
        completed = 1;
    }
    if(test.fault == 1) frame.ready = false;
    if(test.fault == 2) cancel_after_float = true;
    if(terminal) cancel_poll_delay = terminal;
    if(test.fault == 5 || test.fault == 6) formatter_failure = test.fault - 4;
    if(test.fault == 9) output->records->sealed = true;
    if(test.fault == 10) output->records->length = UINT64_MAX - 8;
    if(test.fault == 11) short_io = true;
    if(test.fault == 12) interrupt_write = true;
    if(test.fault >= 13) {
        fail_write_after = test.fault == 15 ? 2 : 1;
        write_error = test.fault == 14 ? EIO : ENOSPC;
        write_error_cancel = test.fault == 14;
    }
    reads = read_bytes = writes = write_bytes = polls = float_calls = time_calls = 0;
    uint32_t status = NETDATA_PROMQL_OK;
    int errors = 0;
    struct timespec start, end;
    if(clock_gettime(CLOCK_MONOTONIC, &start)) abort();
    for(uint64_t i = completed; i < test.count; i++) {
        unsigned stream = (unsigned)(i % test.streams);
        uint64_t index = i / test.streams;
        OUTPUT_SERIES *series = &output->series[stream];
        if(test.mixed && (!index || index == test.count / (2 * test.streams)) &&
           !seed_histogram(output, series, series->histograms.count, &frame)) abort();
        POINT_CHAIN before = series->floats;
        netdata_promql_sample_t point = sample(index, test.large);
        if(test.fault == 7) point.value.kind = UINT32_MAX;
        if(test.overwrite) memset(output->window, 0xa5, PROMQL_BODY_WINDOW);
        status = point_append(output, series, &point, &frame, &message);
        if(status != NETDATA_PROMQL_OK) {
            if(memcmp(&series->floats, &before, sizeof(before))) errors++;
            break;
        }
        completed++;
    }
    if(clock_gettime(CLOCK_MONOTONIC, &end)) abort();
    uint64_t elapsed = (uint64_t)(end.tv_sec - start.tv_sec) * UINT64_C(1000000000) +
        (uint64_t)end.tv_nsec - (uint64_t)start.tv_nsec;
    uint64_t measured_polls = polls, measured_writes = writes, measured_write_bytes = write_bytes;
    uint64_t measured_floats = float_calls, measured_times = time_calls;
    uint64_t length = promql_body_length(output->records);
    bool failure = test.fault && test.fault != 11 && test.fault != 12;
    uint32_t wanted = test.fault >= 1 && test.fault <= 4 ? NETDATA_PROMQL_CANCELLED :
        failure ? NETDATA_PROMQL_OUTPUT_ERROR : NETDATA_PROMQL_OK;
    int wanted_error = test.fault >= 1 && test.fault <= 4 ? ECANCELED :
        test.fault == 5 || test.fault == 6 || test.fault == 9 ? EINVAL :
        test.fault == 8 ? ENOENT : test.fault == 10 ? EOVERFLOW :
        test.fault == 14 ? EIO : test.fault == 13 || test.fault == 15 ? ENOSPC : 0;
    if(status != wanted || output->failure != wanted_error || (!failure && completed != test.count)) errors++;
    if(!failure && (measured_floats != test.count || measured_times != test.count)) errors++;
    if(test.fault == 1 && (measured_floats || measured_times)) errors++;
    if(test.fault == 5 && measured_floats) errors++;
    if(terminal) {
        SINK expected_length = {.body = promql_body_create(frame.account, directory, 0, &sink.error), .frame = NULL};
        if(!expected_length.body || !expected(&expected_length, test.fault == 4 ? 1 : 0, false)) abort();
        uint64_t payload_length = promql_body_length(expected_length.body);
        promql_body_release(expected_length.body);
        if(test.fault == 3 && length != sizeof(POINT_RECORD) + payload_length) errors++;
        if(test.fault == 4) {
            POINT_RECORD first;
            if(!promql_body_read(output->records, output->series[0].floats.first, &first, sizeof(first), NULL, &sink.error) ||
               first.next != NO_RECORD || length != 2 * sizeof(POINT_RECORD) + first.length + payload_length) errors++;
        }
    }
    uint64_t total = test.prefix;
    for(unsigned i = 0; i < test.streams; i++) {
        uint64_t count = completed / test.streams + (i < completed % test.streams);
        if(output->series[i].floats.count != count) errors++;
        else errors += check_chain(output, &output->series[i].floats, false, test.large, &total);
        if(output->series[i].histograms.count != (test.mixed ? 2 : 0)) errors++;
        else errors += check_chain(output, &output->series[i].histograms, true, false, &total);
    }
    if(!failure && total != length) errors++;
    if(test.large && !legacy && measured_polls > 5 * test.count + 50) errors++;
    uint64_t measured_peak = peak;
    int actual_error = output->failure;
    if(promql_output_take_body(output)) abort();
    promql_output_destroy(output);
    if(held) errors++;
    printf("{\"case\":\"%s\",\"errors\":%d,\"points_requested\":%"PRIu64",\"points_completed\":%"PRIu64
           ",\"status\":%u,\"failure\":%d,\"point_polls\":%"PRIu64",\"time_calls\":%"PRIu64",\"float_calls\":%"PRIu64
           ",\"record_writes\":%"PRIu64",\"record_write_bytes\":%"PRIu64",\"elapsed_ns\":%"PRIu64
           ",\"peak\":%"PRIu64",\"held_after\":%"PRIu64",\"body_length\":%"PRIu64",\"terminal_poll\":%u}\n",
           test.name, errors, test.count, completed, status, actual_error, measured_polls, measured_times, measured_floats,
           measured_writes, measured_write_bytes, elapsed, measured_peak, held, length, terminal);
    return errors;
}

int main(int argc, char **argv) {
    if(argc < 2 || argc > 3 || (argc == 3 && strcmp(argv[2], "--allow-legacy-polls"))) return 2;
    bool legacy = argc == 3;
    const APPEND_CASE cases[] = {
        {.name = "corpus", .count = 210, .streams = 1},
        {.name = "first", .count = 1, .streams = 1},
        {.name = "repeated-overwrite", .count = 3, .streams = 1, .overwrite = true},
        {.name = "interleaved", .count = 420, .streams = 2},
        {.name = "mixed", .count = 210, .streams = 1, .mixed = true},
        {.name = "interleaved-mixed", .count = 420, .streams = 2, .mixed = true},
        {.name = "prefix8191", .count = 210, .prefix = 8191, .streams = 1},
        {.name = "prefix8192", .count = 210, .prefix = 8192, .streams = 1},
        {.name = "prefix8193", .count = 210, .prefix = 8193, .streams = 1},
        {.name = "large", .count = 100000, .streams = 1, .large = true},
        {.name = "cancel-before-format", .count = 1, .streams = 1, .fault = 1},
        {.name = "cancel-before-copy", .count = 1, .streams = 1, .fault = 2},
        {.name = "cancel-after-copy", .count = 1, .streams = 1, .fault = 3},
        {.name = "cancel-before-link", .count = 2, .streams = 1, .fault = 4},
        {.name = "time-failure", .count = 1, .streams = 1, .fault = 5},
        {.name = "float-failure", .count = 1, .streams = 1, .fault = 6},
        {.name = "invalid-kind", .count = 1, .streams = 1, .fault = 7},
        {.name = "no-backing", .count = 1, .prefix = 8192, .streams = 1, .fault = 8},
        {.name = "sealed", .count = 1, .streams = 1, .fault = 9},
        {.name = "overflow", .count = 1, .streams = 1, .fault = 10},
        {.name = "short-write", .count = 2, .prefix = 8192, .streams = 1, .fault = 11},
        {.name = "eintr-write", .count = 1, .prefix = 8192, .streams = 1, .fault = 12},
        {.name = "enospc-first", .count = 1, .prefix = 8192, .streams = 1, .fault = 13},
        {.name = "eio-plus-cancel", .count = 1, .prefix = 8192, .streams = 1, .fault = 14},
        {.name = "enospc-late", .count = 1024, .streams = 1, .fault = 15},
    };
    int errors = 0;
    for(size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) errors += append_case(argv[1], cases[i], legacy);
    return errors ? 1 : 0;
}
