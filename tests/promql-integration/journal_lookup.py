#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check the actual journal metric lookup and mapped-file recovery in isolation."""

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import tempfile

from backing import digest, execute, safe_directory

REPO = Path(__file__).resolve().parents[2]
SHIM = r"""
#pragma once
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#define __maybe_unused __attribute__((unused))
typedef int SIGNAL_CODE;
typedef uint64_t usec_t;
typedef unsigned char nd_uuid_t[16];
#define fatal(...) abort()
#define nd_log_limit_static_thread_var(name, ...) int name = 0
#define nd_log_limit(...) ((void)_erl)
static inline int signal_code(int sig, int code) { return sig * 256 + code; }
static inline void SIGNAL_CODE_2str_h(int code, char *out, size_t size) {
    snprintf(out, size, "%d", code);
}
"""


def declaration(source, name):
    match = re.search(r"struct " + name + r" \{.*?\n\};", source, re.S)
    if not match:
        raise RuntimeError("missing declaration: " + name)
    return match.group(0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default=os.environ.get("CC", "cc"))
    args = parser.parse_args()
    parent = safe_directory(REPO / ".tmp")
    run = safe_directory(Path(tempfile.mkdtemp(prefix="journal-lookup-", dir=parent)))
    if run.parent != parent or run.stat().st_uid != os.getuid() or run.stat().st_mode & 0o077:
        raise RuntimeError("fixture directory ownership differs")
    paths = [REPO / name for name in (
        "src/database/engine/demand.c", "src/database/engine/journalfile.h",
        "src/daemon/protected-access.h", "src/daemon/protected-access.c",
        "tests/promql-integration/journal_lookup.py", "tests/promql-integration/journal_lookup_fixture.c",
        "tests/promql-integration/backing.py")]
    sources = {str(p.relative_to(REPO)): digest(p) for p in paths}
    demand, journal, header, handler, _, fixture, _ = [p.read_text() for p in paths]
    begin = demand.index("static bool journal_copy(")
    end = demand.index("static bool journal_directory_verified(", begin)
    magic = re.search(r"^#define JOURVAL_V2_MAGIC\s+\S+", journal, re.M)
    if not magic:
        raise RuntimeError("missing journal magic")
    (run / "libnetdata").mkdir()
    (run / "libnetdata/libnetdata.h").write_text(SHIM)
    (run / "protected-access.h").write_text(header)
    (run / "protected-access.c").write_text(handler)
    (run / "lookup.inc").write_text(
        '#include "protected-access.h"\n' + magic.group(0) + "\n" +
        declaration(journal, "journal_v2_header") + "\n" + declaration(journal, "journal_metric_list") +
        "\nstruct rrdengine_journalfile { struct { void *data; size_t size; } mmap; };\n" + demand[begin:end])
    (run / "fixture.c").write_text(fixture)
    receipt = {"sources": sources, "passed": False,
               "scope": "actual lookup, journal structs, mapped-access macros and handler; "
                        "isolated mapping and logging/type shims, not full Agent execution"}
    status = execute(run, "compile", shlex.split(args.cc) + ["-D_GNU_SOURCE", "-std=gnu11", "-O2",
                     "-Wall", "-Wextra", "-Werror", "-I.", "fixture.c", "protected-access.c", "-o", "fixture"])
    if not status:
        receipt["binary_sha256"] = digest(run / "fixture")
        status = execute(run, "check", [str(run / "fixture")])
        rows = [json.loads(line) for line in (run / "check.stdout").read_text().splitlines()]
        receipt["records"] = rows
        receipt["passed"] = not status and len(rows) == 1 and rows[0]["errors"] == 0 and rows[0]["checks"] > 2000
    if sources != {str(p.relative_to(REPO)): digest(p) for p in paths}:
        raise RuntimeError("sources changed during qualification")
    receipt["exit"] = status
    receipt["artifacts"] = {str(p.relative_to(run)): digest(p) for p in sorted(run.rglob("*")) if p.is_file()}
    (run / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"receipt": str(run / "receipt.json"), "passed": receipt["passed"]}))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
