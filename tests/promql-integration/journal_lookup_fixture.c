// SPDX-License-Identifier: GPL-3.0-or-later

#include "lookup.inc"
#include <sys/mman.h>
#include <time.h>
#include <inttypes.h>

static unsigned checks, errors;
static uint64_t outcome_hash = UINT64_C(14695981039346656037);
static void check(bool pass) { checks++; errors += !pass; }

static void key(nd_uuid_t out, uint32_t value) {
    memset(out, 0, sizeof(nd_uuid_t));
    for(unsigned i = 0; i < 4; i++) out[i] = value >> (24 - i * 8);
}

static void populate(void *data, unsigned count) {
    struct journal_metric_list *records = data;
    for(unsigned i = 0; i < count; i++) {
        memset(&records[i], 0, sizeof(records[i]));
        key(records[i].uuid, 2 * i + 2);
        records[i].entries = i + 1;
        records[i].page_offset = i * 100;
        records[i].delta_start_s = i;
        records[i].delta_end_s = i + 99;
        records[i].update_every_s = i % 30 + 1;
    }
}

// Linear membership is independent of the production binary search.
static void membership(struct rrdengine_journalfile *journal, struct journal_v2_header *header, unsigned value) {
    nd_uuid_t uuid;
    key(uuid, value);
    struct journal_metric_list output, expected;
    memset(&output, 0xa5, sizeof(output));
    bool found = true, wanted = false;
    for(unsigned i = 0; i < header->metric_count; i++) {
        struct journal_metric_list row;
        memcpy(&row, (char *)journal->mmap.data + header->metric_offset + i * sizeof(row), sizeof(row));
        if(!memcmp(row.uuid, uuid, sizeof(uuid))) { expected = row; wanted = true; break; }
    }
    int depth = protected_access_state.depth;
    bool ok = journal_metric(journal, header, uuid, &output, &found);
    check(ok && found == wanted && (!wanted || !memcmp(&output, &expected, sizeof(output))) &&
          protected_access_state.depth == depth);
    const unsigned char *bytes = (const unsigned char *)&output;
    for(size_t i = 0; i < sizeof(output); i++) outcome_hash = (outcome_hash ^ bytes[i]) * UINT64_C(1099511628211);
    outcome_hash = (outcome_hash ^ found) * UINT64_C(1099511628211);
}

static void geometry(void) {
    unsigned sizes[] = {0, 1, 2, 3, 127, 1024};
    for(unsigned c = 0; c < sizeof(sizes) / sizeof(*sizes); c++) {
        unsigned count = sizes[c];
        size_t bytes = (count + 1) * sizeof(struct journal_metric_list);
        void *data = calloc(1, bytes);
        if(!data) abort();
        populate(data, count);
        struct rrdengine_journalfile journal = {.mmap = {data, bytes}};
        struct journal_v2_header header = {.magic = JOURVAL_V2_MAGIC, .metric_count = count};
        for(unsigned i = 0; i <= count * 2 + 3; i++) membership(&journal, &header, i);
        nd_uuid_t uuid = {0};
        struct journal_metric_list output;
        for(unsigned bad = 0; bad < 3; bad++) {
            struct journal_v2_header invalid = header;
            if(bad == 0) invalid.magic = 0;
            if(bad == 1) invalid.metric_offset = bytes + 1;
            if(bad == 2) invalid.metric_count = UINT32_MAX;
            bool found = true;
            check(!journal_metric(&journal, &invalid, uuid, &output, &found) && !found &&
                  protected_access_state.depth == 0);
        }
        header.metric_offset = bytes;
        header.metric_count = 0;
        membership(&journal, &header, 0);
        free(data);
    }
}

static void fault_handler(int sig, siginfo_t *info, void *context) {
    signal_protected_access_check(sig, info, context);
    _exit(90);
}

static void mapping_faults(void) {
    size_t page = sysconf(_SC_PAGESIZE), bytes = page * 3;
    int fd = memfd_create("journal-lookup", 0);
    if(fd < 0 || ftruncate(fd, bytes)) abort();
    void *data = mmap(NULL, bytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if(data == MAP_FAILED) abort();
    unsigned count = bytes / sizeof(struct journal_metric_list);
    populate(data, count);
    struct rrdengine_journalfile journal = {.mmap = {data, bytes}};
    struct journal_v2_header header = {.magic = JOURVAL_V2_MAGIC, .metric_count = count};
    struct sigaction action = {.sa_sigaction = fault_handler, .sa_flags = SA_SIGINFO};
    sigemptyset(&action.sa_mask);
    if(sigaction(SIGBUS, &action, NULL) || sigaction(SIGSEGV, &action, NULL)) abort();
    sigset_t blocked, before, after;
    sigemptyset(&blocked);
    sigaddset(&blocked, SIGUSR1);
    if(sigprocmask(SIG_BLOCK, &blocked, &before)) abort();
    nd_uuid_t uuid;
    key(uuid, 2 * count);
    for(unsigned fault = 0; fault < 2; fault++) {
        if(fault == 0 && ftruncate(fd, page * 2)) abort();
        if(fault == 1 && mprotect((char *)data + page * 2, page, PROT_NONE)) abort();
        struct journal_metric_list output = {0};
        bool found = true;
        {
            // An enclosing caller's frame must survive the lookup's recovery.
            PROTECTED_ACCESS_SETUP(data, bytes, "fixture", "read");
            if(!no_signal_received) abort();
            check(!journal_metric(&journal, &header, uuid, &output, &found) && !found &&
                  protected_access_state.depth == 1);
        }
        // The first successful probe is in the middle page; a later probe faults.
        check(output.entries == count / 2 + 1 && protected_access_state.depth == 0);
        if(sigprocmask(SIG_SETMASK, NULL, &after)) abort();
        check(sigismember(&after, SIGUSR1) == 1 && sigismember(&after, SIGBUS) == sigismember(&before, SIGBUS));
        if(fault == 0 && ftruncate(fd, bytes)) abort();
        if(fault == 1 && mprotect((char *)data + page * 2, page, PROT_READ | PROT_WRITE)) abort();
        populate(data, count);
        membership(&journal, &header, 2 * count);
    }
    if(sigprocmask(SIG_SETMASK, &before, NULL) || munmap(data, bytes) || close(fd)) abort();
}

static uint64_t now(void) {
    struct timespec t;
    if(clock_gettime(CLOCK_MONOTONIC, &t)) abort();
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}

static int benchmark(unsigned count) {
    if(!count || count > 65535) return 2;
    size_t bytes = count * sizeof(struct journal_metric_list);
    void *data = calloc(1, bytes);
    if(!data) abort();
    populate(data, count);
    struct rrdengine_journalfile journal = {.mmap = {data, bytes}};
    struct journal_v2_header header = {.magic = JOURVAL_V2_MAGIC, .metric_count = count};
    uint64_t checksum = 0, started = now();
    for(unsigned i = 0; i < 100000; i++) {
        nd_uuid_t uuid;
        unsigned index = (i * 7919U) % count;
        key(uuid, 2 * index + 2);
        struct journal_metric_list output;
        bool found = false;
        if(!journal_metric(&journal, &header, uuid, &output, &found) || !found || output.entries != index + 1)
            abort();
        checksum += output.entries;
    }
    printf("{\"records\":%u,\"lookups\":100000,\"elapsed_ns\":%"PRIu64",\"checksum\":%"PRIu64"}\n",
           count, now() - started, checksum);
    free(data);
    return 0;
}

int main(int argc, char **argv) {
    if(argc == 2) return benchmark(strtoul(argv[1], NULL, 10));
    // Keep the general copy helper covered when metric lookup no longer uses it.
    char input = 1, output = 0;
    struct rrdengine_journalfile journal = {.mmap = {&input, 1}};
    check(journal_copy(&journal, 0, &output, 1) && output == input);
    check(!journal_copy(&journal, 2, &output, 1));
    geometry();
    mapping_faults();
    printf("{\"checks\":%u,\"errors\":%u,\"outcome_hash\":\"%016"PRIx64"\"}\n", checks, errors, outcome_hash);
    return errors ? 1 : 0;
}
