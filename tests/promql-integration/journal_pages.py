#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise actual journal page searches with bounded mappings and reference controls."""

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import tempfile

from backing import digest, execute, safe_directory
from journal_lookup import REPO, SHIM, declaration

TYPES = r"""
#include "protected-access.h"
#include <time.h>
#include <limits.h>
#include <zlib.h>
#define MIN(a,b) ((a) < (b) ? (a) : (b))
#define USEC_PER_SEC 1000000U
#define RRDENG_BLOCK_SIZE 4096U
#define OFFSET_TO_BLOCK(value) ((value) / RRDENG_BLOCK_SIZE)
#define DATAFILE_ACQUIRE_PAGE_DETAILS 1
#define CHECKSUM_SZ 4
enum { RRDENG_READ_CANCELLED = 1, RRDENG_READ_IO, RRDENG_READ_CHECKSUM, RRDENG_READ_DESCRIPTOR };
typedef struct { unsigned code; } RRDENG_READ_ERROR;
typedef struct { unsigned polls, cancel_on; } TEST_CONTROL;
typedef struct { TEST_CONTROL *context; } RRDENG_READ_OPERATION;
static bool rrdeng_read_operation_poll(const RRDENG_READ_OPERATION *op) {
    if(!op) return true;
    return ++op->context->polls != op->context->cancel_on;
}
typedef struct { nd_uuid_t uuid; } RRDENG_READ_PROMISE;
typedef struct { const RRDENG_READ_PROMISE *promise; time_t last; uint32_t verified_file, verified_page; } RRDENG_READER;
struct rrdengine_journalfile {
    struct { void *data; uint32_t size; } mmap;
    int data_spinlock;
    bool read_directory_verified;
};
struct rrdengine_datafile { struct rrdengine_journalfile *journalfile; unsigned fileno, references; };
static bool datafile_acquire(struct rrdengine_datafile *file, int reason __maybe_unused) { file->references++; return true; }
static void datafile_release(struct rrdengine_datafile *file, int reason __maybe_unused) {
    if(!file->references) abort();
    file->references--;
}
static void spinlock_tracked_lock(int *lock) { if((*lock)++) abort(); }
static void spinlock_tracked_unlock(int *lock) { if(--*lock) abort(); }
static bool nd_uuid_eq(const nd_uuid_t a, const nd_uuid_t b) { return !memcmp(a,b,sizeof(nd_uuid_t)); }
// Extent decoding is outside this fixture; the complete Agent checks its disk geometry.
static bool rrdeng_valid_extent_disk_size(size_t size) { return size == 4096; }
static size_t trace_first, trace_bytes;
static unsigned trace_count, trace_indices[64];
"""

COPY = r"""
static bool journal_copy(struct rrdengine_journalfile *journal, size_t offset, void *out, size_t bytes) {
#ifdef TRACE_COPIES
    if(bytes == sizeof(struct journal_page_list) && offset >= trace_first &&
       offset - trace_first < trace_bytes && (offset - trace_first) % sizeof(struct journal_page_list) == 0) {
        if(trace_count == sizeof(trace_indices) / sizeof(*trace_indices)) abort();
        trace_indices[trace_count++] = (offset - trace_first) / sizeof(struct journal_page_list);
    }
#endif
    return page_test_copy(journal, offset, out, bytes);
}
"""


def function(source, name):
    for match in re.finditer(r"^static (?:bool|void) " + name + r"\(", source, re.M):
        begin = match.start()
        if source.index(";", begin) < source.index("{", begin):
            continue
        return source[begin:source.index("\n}", begin) + 2]
    raise RuntimeError("missing function: " + name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default=os.environ.get("CC", "cc"))
    expectation = parser.add_mutually_exclusive_group()
    expectation.add_argument("--expect-reuse", dest="expect_reuse", action="store_true", default=True,
                             help="expect the current reused-descriptor behavior (default)")
    expectation.add_argument("--expect-baseline", dest="expect_reuse", action="store_false",
                             help="expect the original duplicate read in a baseline checkout")
    args = parser.parse_args()
    parent = safe_directory(REPO / ".tmp")
    run = safe_directory(Path(tempfile.mkdtemp(prefix="journal-pages-", dir=parent)))
    if run.parent != parent or run.stat().st_uid != os.getuid() or run.stat().st_mode & 0o077:
        raise RuntimeError("fixture directory ownership differs")
    paths = [REPO / name for name in (
        "src/database/engine/demand.c", "src/database/engine/journalfile.h",
        "src/daemon/protected-access.h", "src/daemon/protected-access.c",
        "tests/promql-integration/journal_pages.py", "tests/promql-integration/journal_pages_fixture.c",
        "tests/promql-integration/journal_lookup.py", "tests/promql-integration/backing.py")]
    sources = {str(p.relative_to(REPO)): digest(p) for p in paths}
    demand, journal, header, handler, _, fixture, _, _ = [p.read_text() for p in paths]
    magic = re.search(r"^#define JOURVAL_V2_MAGIC\s+\S+", journal, re.M)
    candidate = re.search(r"typedef struct \{\n    time_t first, last;.*?\n\} DEMAND_CANDIDATE;", demand, re.S)
    if not magic or not candidate:
        raise RuntimeError("missing source declarations")
    names = ["journal_v2_header", "journal_extent_list", "journal_metric_list", "journal_page_header",
             "journal_page_list", "journal_v2_block_trailer"]
    code = TYPES + magic.group(0) + "\n" + "\n".join(declaration(journal, n) for n in names)
    code += "\n" + candidate.group(0) + "\n"
    code += function(demand, "journal_copy").replace("journal_copy(", "page_test_copy(", 1) + COPY
    for name in ["journal_metric", "journal_crc", "journal_pages_verified", "journal_directory_verified",
                 "better_candidate", "candidate_release", "journal_metric_overlaps", "journal_candidate"]:
        code += "\n" + function(demand, name) + "\n"
    (run / "libnetdata").mkdir()
    (run / "libnetdata/libnetdata.h").write_text(SHIM)
    (run / "protected-access.h").write_text(header)
    (run / "protected-access.c").write_text(handler)
    (run / "pages.inc").write_text(code)
    (run / "fixture.c").write_text(fixture)
    receipt = {"sources": sources, "passed": False, "expect_reuse": args.expect_reuse,
               "scope": "actual page/metric searches, CRC checks and mapped access; isolated locks, references, "
                        "control and types; extent geometry accepts fixture size4096, not full disk decoding"}
    status = execute(run, "compile", shlex.split(args.cc) + ["-D_GNU_SOURCE", "-DTRACE_COPIES", "-std=gnu11", "-O2",
                     "-Wall", "-Wextra", "-Werror", "-I.", "fixture.c", "protected-access.c", "-lz", "-o", "fixture"])
    if not status:
        receipt["binary_sha256"] = digest(run / "fixture")
        status = execute(run, "check", [str(run / "fixture"), "reuse" if args.expect_reuse else "baseline"])
        receipt["records"] = [json.loads(line) for line in (run / "check.stdout").read_text().splitlines()]
        receipt["passed"] = not status and len(receipt["records"]) >= 10 and all(not r["errors"] for r in receipt["records"])
    if sources != {str(p.relative_to(REPO)): digest(p) for p in paths}:
        raise RuntimeError("sources changed during qualification")
    receipt["exit"] = status
    receipt["artifacts"] = {str(p.relative_to(run)): digest(p) for p in sorted(run.rglob("*")) if p.is_file()}
    (run / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"receipt": str(run / "receipt.json"), "passed": receipt["passed"]}))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
