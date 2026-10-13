#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Qualify the actual checkpoint/backing owners with isolated allocation and I/O controls."""

import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import tempfile

from backing import digest, execute, safe_directory

REPO = Path(__file__).resolve().parents[2]

MEMORY_SHIM = r"""
#ifndef NETDATA_PROMQL_PROVIDER_MEMORY_H
#define NETDATA_PROMQL_PROVIDER_MEMORY_H
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#define MIN(a, b) ((a) < (b) ? (a) : (b))
#define MAX(a, b) ((a) > (b) ? (a) : (b))
#define NETDATA_PROMQL_MEMORY_PROVIDER_HELD 3U
typedef struct netdata_promql_accounting_context netdata_promql_accounting_context_t;
typedef struct { const netdata_promql_accounting_context_t *account; bool ready; } PROMQL_READ_FRAME;
void *checkpoint_test_allocate(size_t);
void checkpoint_test_free(void *);
static inline void *promql_malloc(const netdata_promql_accounting_context_t *account,
                                 uint32_t kind, size_t bytes) {
    (void)account; (void)kind;
    return checkpoint_test_allocate(bytes);
}
static inline void promql_free(void *memory) { checkpoint_test_free(memory); }
static inline bool promql_read_poll(PROMQL_READ_FRAME *frame) { return !frame || frame->ready; }
#endif
"""

PROBE = r"""
#define _FILE_OFFSET_BITS 64
#include "checkpoint.c"
#include <inttypes.h>
#include <stdio.h>
#include <stdarg.h>
#include <sys/stat.h>

typedef union { max_align_t alignment; size_t bytes; } TEST_ALLOCATION;
static uint64_t held, peak, read_calls, write_calls, read_bytes, write_bytes;
static unsigned files, file_peak;
static int file_descriptors[8];
static bool refuse_allocation, refuse_open, refuse_size, refuse_read, refuse_write, partial_write, short_io;
static bool cancel_write, cancel_error;
static PROMQL_READ_FRAME *active;

void *checkpoint_test_allocate(size_t bytes) {
    if(refuse_allocation) { refuse_allocation = false; return NULL; }
    if(bytes > SIZE_MAX - sizeof(TEST_ALLOCATION)) return NULL;
    TEST_ALLOCATION *block = calloc(1, sizeof(*block) + bytes);
    if(!block) return NULL;
    block->bytes = sizeof(*block) + bytes;
    held += block->bytes;
    peak = MAX(peak, held);
    return block + 1;
}
void checkpoint_test_free(void *memory) {
    if(!memory) return;
    TEST_ALLOCATION *block = (TEST_ALLOCATION *)memory - 1;
    held -= block->bytes;
    free(block);
}
static int retain_file(int fd) {
    if(fd >= 0) {
        if(files == 8) abort();
        file_descriptors[files++] = fd;
        file_peak = MAX(file_peak, files);
    }
    return fd;
}
int __real_open64(const char *, int, ...);
int __real_mkstemp64(char *);
int __real_close(int);
int __real_ftruncate64(int, off64_t);
ssize_t __real_pread64(int, void *, size_t, off64_t);
ssize_t __real_pwrite64(int, const void *, size_t, off64_t);
int __wrap_open64(const char *name, int flags, ...) {
    va_list arguments;
    va_start(arguments, flags);
    mode_t mode = va_arg(arguments, mode_t);
    va_end(arguments);
    if(refuse_open) { errno = ENOENT; return -1; }
    return retain_file(__real_open64(name, flags, mode));
}
int __wrap_mkstemp64(char *name) {
    if(refuse_open) { errno = ENOENT; return -1; }
    return retain_file(__real_mkstemp64(name));
}
int __wrap_close(int fd) {
    for(unsigned i = 0; i < files; i++)
        if(file_descriptors[i] == fd) { file_descriptors[i] = file_descriptors[--files]; break; }
    return __real_close(fd);
}
int __wrap_ftruncate64(int fd, off64_t length) {
    if(refuse_size) { errno = ENOSPC; return -1; }
    return __real_ftruncate64(fd, length);
}
ssize_t __wrap_pread64(int fd, void *data, size_t length, off64_t offset) {
    read_calls++;
    if(refuse_read) { errno = EIO; return -1; }
    ssize_t bytes = __real_pread64(fd, data, short_io ? MIN(length, (size_t)7) : length, offset);
    if(bytes > 0) read_bytes += (uint64_t)bytes;
    return bytes;
}
ssize_t __wrap_pwrite64(int fd, const void *data, size_t length, off64_t offset) {
    write_calls++;
    if(refuse_write) {
        if(cancel_error) active->ready = false;
        errno = ENOSPC;
        return -1;
    }
    bool partial = partial_write;
    ssize_t bytes = __real_pwrite64(fd, data, (short_io || partial) ? MIN(length, (size_t)7) : length, offset);
    if(bytes > 0) {
        write_bytes += (uint64_t)bytes;
        if(partial) { partial_write = false; refuse_write = true; }
        if(cancel_write) active->ready = false;
    }
    return bytes;
}
static uint64_t bits(uint64_t index, unsigned stream) {
    const uint64_t values[] = {0, UINT64_C(0x8000000000000000), UINT64_C(0x7ff800000000002a),
        UINT64_C(0x7ff0000000000002), UINT64_C(0xfff0000000000000), UINT64_C(0x7ff0000000000000),
        UINT64_C(0x4079300000000001), UINT64_C(0x4340000000000001)};
    return values[index % 8] ^ stream;
}
static PROMQL_CHECKPOINT record(uint64_t index, unsigned stream) {
    return (PROMQL_CHECKPOINT){.next = 1700000000 + (int64_t)index * 3, .counter_bits = bits(index, stream)};
}
static bool append(PROMQL_CHECKPOINTS **owner, PROMQL_CHECKPOINT_BACKING *bank, const char *directory,
                    uint64_t index, unsigned stream, PROMQL_READ_FRAME *frame, int *error) {
    return promql_checkpoints_append(owner, bank, directory, record(index, stream), frame, error);
}
static int check_prefix(PROMQL_CHECKPOINTS *owner, PROMQL_CHECKPOINT_BACKING *bank,
                         uint64_t count, unsigned stream, PROMQL_READ_FRAME *frame) {
    int failures = !owner || owner->count != count;
    for(uint64_t i = 0; i < count; i++) {
        PROMQL_CHECKPOINT point;
        bool found;
        int error;
        failures += !promql_checkpoints_find(owner, bank, record(i, stream).next + 1, &point, &found, frame, &error) ||
            !found || point.next != record(i, stream).next || point.counter_bits != bits(i, stream);
    }
    return failures;
}
static int finish(const char *name, PROMQL_CHECKPOINTS **owners, unsigned count,
                   PROMQL_CHECKPOINT_BACKING *bank, int failures) {
    size_t scratch = bank->window_bytes;
    uint64_t length = bank->file ? bank->file->length : 0;
    for(unsigned i = 0; i < count; i++) promql_checkpoints_free(owners[i]);
    promql_checkpoint_backing_close(bank);
    failures += held != 0 || files != 0 || peak > 12 * 1024 || file_peak > 1;
    printf("{\"checkpoint_case\":\"%s\",\"errors\":%d,\"memory_peak\":%"PRIu64
           ",\"remaining_bytes\":%"PRIu64",\"remaining_files\":%u,\"file_peak\":%u"
           ",\"scratch_bytes\":%zu,\"backing_length\":%"PRIu64",\"read_calls\":%"PRIu64
           ",\"write_calls\":%"PRIu64",\"read_bytes\":%"PRIu64",\"write_bytes\":%"PRIu64"}\n",
           name, failures, peak, held, files, file_peak, scratch, length, read_calls, write_calls, read_bytes, write_bytes);
    peak = read_calls = write_calls = read_bytes = write_bytes = file_peak = 0;
    return failures;
}
static int basic(const char *directory) {
    PROMQL_READ_FRAME frame = {.ready = true}; active = &frame;
    PROMQL_CHECKPOINT_BACKING bank = {0};
    PROMQL_CHECKPOINTS *owners[3] = {0};
    int error = 0, failures = 0;
    for(uint64_t i = 0; i < 16; i++) failures += !append(&owners[0], &bank, NULL, i, 0, &frame, &error);
    failures += bank.file != NULL || bank.window != NULL;
    PROMQL_CHECKPOINT first; bool found;
    failures += !promql_checkpoints_find(owners[0], &bank, 1, &first, &found, &frame, &error) || found ||
        first.next != record(0, 0).next || first.counter_bits != bits(0, 0);
    failures += check_prefix(owners[0], &bank, 16, 0, &frame);
    failures += !append(&owners[0], &bank, NULL, 0, 1, &frame, &error) || owners[0]->count != 16;
    failures += !append(&owners[0], &bank, directory, 16, 0, &frame, &error) || bank.window != NULL;
    for(unsigned stream = 1; stream < 3; stream++)
        for(uint64_t i = 0; i < 17; i++) failures += !append(&owners[stream], &bank, directory, i, stream, &frame, &error);
    for(uint64_t i = 17; i < 129; i++)
        for(unsigned stream = 0; stream < 3; stream++)
            failures += !append(&owners[stream], &bank, directory, i, stream, &frame, &error);
    for(unsigned stream = 0; stream < 3; stream++) failures += check_prefix(owners[stream], &bank, 129, stream, &frame);
    return finish("shared-exact-bits", owners, 3, &bank, failures);
}
static int long_history(const char *directory) {
    PROMQL_READ_FRAME frame = {.ready = true}; active = &frame;
    PROMQL_CHECKPOINT_BACKING bank = {0}; PROMQL_CHECKPOINTS *owner = NULL;
    int error = 0, failures = 0;
    for(uint64_t i = 0; i < 65537; i++) failures += !append(&owner, &bank, directory, i, 0, &frame, &error);
    for(uint64_t i = 0; i < 65537; i += 67) {
        PROMQL_CHECKPOINT point; bool found;
        failures += !promql_checkpoints_find(owner, &bank, record(i, 0).next + 1, &point, &found, &frame, &error) ||
            !found || point.counter_bits != bits(i, 0) || point.next != record(i, 0).next;
    }
    uint64_t reads = read_calls;
    PROMQL_CHECKPOINT point; bool found;
    failures += !promql_checkpoints_find(owner, &bank, 1, &point, &found, &frame, &error) || found;
    failures += !promql_checkpoints_find(owner, &bank, INT64_MAX, &point, &found, &frame, &error) || !found;
    failures += read_calls != reads || bank.window_bytes != 8192;
    return finish("bounded-history", &owner, 1, &bank, failures);
}
static int exact_first(const char *directory) {
    PROMQL_READ_FRAME frame = {.ready = true}; active = &frame;
    PROMQL_CHECKPOINT_BACKING bank = {0}; PROMQL_CHECKPOINTS *owner = NULL;
    int error = 0, failures = 0;
    for(unsigned i = 0; i < 2049; i++) failures += !append(&owner, &bank, directory, i, 0, &frame, &error);
    uint64_t reads = read_calls;
    refuse_read = true;
    PROMQL_CHECKPOINT point; bool found;
    failures += !promql_checkpoints_find(owner, &bank, record(0, 0).next, &point, &found, &frame, &error) ||
        !found || point.next != record(0, 0).next || point.counter_bits != bits(0, 0) || read_calls != reads;
    refuse_read = false;
    return finish("exact-first-no-io", &owner, 1, &bank, failures);
}
static int fault(const char *directory, const char *name, unsigned mode) {
    PROMQL_READ_FRAME frame = {.ready = true}; active = &frame;
    PROMQL_CHECKPOINT_BACKING bank = {0}; PROMQL_CHECKPOINTS *owner = NULL;
    int error = 0, failures = 0;
    unsigned count = mode <= 4 ? 16 : mode == 5 || mode == 6 ? 17 : 32;
    for(unsigned i = 0; i < count; i++) failures += !append(&owner, &bank, directory, i, 0, &frame, &error);
    PROMQL_CHECKPOINTS before = *owner; PROMQL_CHECKPOINTS *pointer = owner;
    if(mode == 0) refuse_allocation = true;
    if(mode == 1) refuse_open = true;
    if(mode == 2) refuse_size = true;
    if(mode == 3) partial_write = true;
    if(mode == 4 || mode == 5) cancel_write = true;
    if(mode == 6) { refuse_write = true; cancel_error = true; }
    if(mode == 7) refuse_read = true;
    if(mode == 8) refuse_allocation = true;
    failures += append(&owner, &bank, directory, count, 0, &frame, &error);
    int expected = mode == 0 || mode == 8 ? ENOMEM : mode == 1 ? ENOENT : mode == 4 || mode == 5 ? ECANCELED :
        mode == 7 ? EIO : ENOSPC;
    failures += error != expected || owner != pointer || memcmp(owner, &before, sizeof(before)) != 0;
    refuse_allocation = refuse_open = refuse_size = refuse_read = refuse_write = partial_write = cancel_write = cancel_error = false;
    frame.ready = true;
    failures += check_prefix(owner, &bank, count, 0, &frame);
    failures += !append(&owner, &bank, directory, count, 0, &frame, &error);
    failures += check_prefix(owner, &bank, count + 1, 0, &frame);
    return finish(name, &owner, 1, &bank, failures);
}
static int large_offsets(const char *directory) {
    PROMQL_READ_FRAME frame = {.ready = true}; active = &frame;
    PROMQL_CHECKPOINT_BACKING bank = {0}; PROMQL_CHECKPOINTS *owner = NULL;
    int error = 0, failures = 0;
    bank.file = promql_file_open(directory, NULL, 0, &error);
    if(!bank.file) return 1;
    failures += !promql_file_size(bank.file, UINT64_C(3) << 30, &error);
    short_io = true;
    for(unsigned i = 0; i < 65; i++) failures += !append(&owner, &bank, directory, i, 0, &frame, &error);
    failures += check_prefix(owner, &bank, 65, 0, &frame);
    short_io = false;
    struct stat details;
    failures += fstat(bank.file->fd, &details) || details.st_size <= (off_t)(UINT64_C(3) << 30) ||
        details.st_blocks > 128;
    PROMQL_CHECKPOINTS before = *owner; PROMQL_CHECKPOINTS *pointer = owner;
    uint64_t actual_length = bank.file->length;
    // Exercise signed-offset overflow without requiring the filesystem to admit an enormous file.
    bank.file->length = INT64_MAX;
    // Region addition fails before any copy, leaving the earlier records usable.
    owner->count = owner->capacity;
    PROMQL_CHECKPOINT last = owner->last;
    failures += append(&owner, &bank, directory, 129, 0, &frame, &error) || error != EOVERFLOW || owner != pointer ||
        owner->last.next != last.next || owner->last.counter_bits != last.counter_bits;
    *owner = before;
    bank.file->length = actual_length;
    failures += check_prefix(owner, &bank, 65, 0, &frame);
    failures += __real_ftruncate64(bank.file->fd, 0) != 0;
    PROMQL_CHECKPOINT point; bool found;
    failures += promql_checkpoints_find(owner, &bank, record(20, 0).next, &point, &found, &frame, &error) || error != EIO;
    return finish("wide-short-truncated", &owner, 1, &bank, failures);
}
int main(int argc, char **argv) {
    if(argc != 2) return 1;
    printf("{\"checkpoint_layout\":true,\"record_bytes\":%zu,\"owner_bytes\":%zu,\"bank_bytes\":%zu,\"wrapper_bytes\":%zu}\n",
           sizeof(PROMQL_CHECKPOINT), sizeof(PROMQL_CHECKPOINTS), sizeof(PROMQL_CHECKPOINT_BACKING), sizeof(TEST_ALLOCATION));
    int failures = basic(argv[1]) + long_history(argv[1]) + exact_first(argv[1]);
    const char *names[] = {"compact-refusal", "missing-directory", "full-reservation", "partial-prefix-write",
        "cancel-prefix", "cancel-after-append", "io-error-before-cancel", "relocation-read-error", "scratch-refusal"};
    for(unsigned i = 0; i < sizeof(names) / sizeof(names[0]); i++) failures += fault(argv[1], names[i], i);
    failures += large_offsets(argv[1]);
    return failures ? 1 : 0;
}
"""

CASES = {
    "shared-exact-bits", "bounded-history", "exact-first-no-io", "compact-refusal", "missing-directory", "full-reservation",
    "partial-prefix-write", "cancel-prefix", "cancel-after-append", "io-error-before-cancel",
    "relocation-read-error", "scratch-refusal", "wide-short-truncated",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default=os.environ.get("CC", "cc"))
    parser.add_argument("--runner", default="")
    args = parser.parse_args()
    parent = safe_directory(REPO / ".tmp")
    run = safe_directory(Path(tempfile.mkdtemp(prefix="checkpoint-owner-", dir=parent)))
    if run.parent != parent or run.stat().st_uid != os.getuid() or run.stat().st_mode & 0o077:
        raise RuntimeError("checkpoint fixture ownership or permissions differ")
    source = REPO / "src/web/api/promql"
    paths = [source / name for name in ("checkpoint.c", "checkpoint.h", "backing.c", "backing.h", "memory.h")]
    paths += [Path(__file__).resolve(), Path(__file__).resolve().with_name("backing.py")]
    sources = {str(path.relative_to(REPO)): digest(path) for path in paths}
    for path in paths[:4]:
        shutil.copyfile(path, run / path.name)
    (run / "memory.h").write_text(MEMORY_SHIM)
    (run / "probe.c").write_text(PROBE)
    receipt = {"sources": sources, "passed": False,
               "scope": "actual checkpoint/backing code with isolated allocation/control and I/O wrappers; "
                        "not daemon allocation-handle accounting or complete Agent execution"}
    wrap = [f"-Wl,--wrap={name}" for name in ("open64", "mkstemp64", "close", "ftruncate64", "pread64", "pwrite64")]
    status = execute(run, "compile", shlex.split(args.cc) + ["-D_GNU_SOURCE", "-D_FILE_OFFSET_BITS=64", "-std=c11",
                     "-O2", "-Wall", "-Wextra", "-Werror", "backing.c", "probe.c", *wrap, "-o", "probe"])
    if not status:
        receipt["binary_sha256"] = digest(run / "probe")
        status = execute(run, "probe", shlex.split(args.runner) + [str(run / "probe"), str(run)])
        rows = [json.loads(line) for line in (run / "probe.stdout").read_text().splitlines() if line.startswith("{")]
        cases = [row for row in rows if "checkpoint_case" in row]
        layouts = [row for row in rows if "checkpoint_layout" in row]
        receipt["records"] = rows
        receipt["passed"] = (not status and len(cases) == len(CASES) and len(layouts) == 1
                             and {row["checkpoint_case"] for row in cases} == CASES
                             and all(row["errors"] == 0 for row in cases))
    if sources != {str(path.relative_to(REPO)): digest(path) for path in paths}:
        raise RuntimeError("checkpoint source changed during qualification")
    receipt["exit"] = status
    receipt["artifacts"] = {path.name: digest(path) for path in sorted(run.iterdir()) if path.is_file()}
    (run / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"receipt": str(run / "receipt.json"), "passed": receipt["passed"], "exit": status}))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
