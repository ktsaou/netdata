#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the actual backing I/O, including sparse offsets above 2 GiB."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time

REPO = Path(__file__).resolve().parents[2]

# Keep the I/O owner unchanged while isolating it from daemon accounting and control.
MEMORY_SHIM = r"""
#ifndef NETDATA_PROMQL_PROVIDER_MEMORY_H
#define NETDATA_PROMQL_PROVIDER_MEMORY_H
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#define MIN(a, b) ((a) < (b) ? (a) : (b))
#define MAX(a, b) ((a) > (b) ? (a) : (b))
typedef struct netdata_promql_accounting_context netdata_promql_accounting_context_t;
typedef struct { bool ready; } PROMQL_READ_FRAME;
static inline void *promql_malloc(const netdata_promql_accounting_context_t *account,
                                 uint32_t kind, size_t bytes) {
    (void)account; (void)kind;
    return calloc(1, bytes);
}
static inline void promql_free(void *memory) { free(memory); }
static inline bool promql_read_poll(PROMQL_READ_FRAME *frame) { return frame->ready; }
#endif
"""

PROBE = r"""
// The fixture's stat view is independent of the production I/O translation unit.
#define _FILE_OFFSET_BITS 64
#include "backing.h"
#include <inttypes.h>
#include <stdio.h>
#include <sys/stat.h>

static int failed(const char *stage, int error) {
    printf("{\"passed\":false,\"stage\":\"%s\",\"error\":%d}\n", stage, error);
    return 1;
}

int main(int argc, char **argv) {
    if(argc != 2) return failed("arguments", EINVAL);
    int error = 0, result = 1;
    PROMQL_FILE *file = promql_file_open(argv[1], NULL, 0, &error);
    if(!file) return failed("open", error);
    const uint64_t boundary = UINT64_C(1) << 31;
    const uint64_t large = UINT64_C(3) << 30;
    const char marker[] = "retained-query-window";
    const uint64_t offsets[] = {0, boundary - 4, boundary + 123, large + 7};
    if(!promql_file_size(file, large + 4096, &error)) {
        result = failed("resize-above-2GiB", error); goto done;
    }
    for(size_t i = 0; i < sizeof(offsets) / sizeof(offsets[0]); i++) {
        if(!promql_file_write(file, offsets[i], marker, sizeof(marker), NULL, &error)) {
            result = failed("write-above-2GiB", error); goto done;
        }
        char window[sizeof(marker)] = {0};
        if(!promql_file_read(file, offsets[i], window, sizeof(window), NULL, &error) ||
           memcmp(window, marker, sizeof(marker))) {
            result = failed("read-above-2GiB", error); goto done;
        }
    }
    struct stat retained;
    if(fstat(file->fd, &retained) || (uint64_t)retained.st_size != large + 4096 ||
       retained.st_blocks < 0 || (uint64_t)retained.st_blocks > 2048) {
        result = failed("sparse-storage", errno); goto done;
    }
    uint64_t length = file->length;
    if(promql_file_size(file, UINT64_MAX, &error) || error != EOVERFLOW || file->length != length) {
        result = failed("offset-overflow", error); goto done;
    }
    printf("{\"passed\":true,\"length\":%"PRIu64",\"allocated_bytes\":%"PRIu64
           ",\"windows\":%zu}\n", length, (uint64_t)retained.st_blocks * 512,
           sizeof(offsets) / sizeof(offsets[0]));
    result = 0;
done:
    promql_file_close(file);
    return result;
}
"""


def safe_directory(path):
    value = path.resolve(strict=True)
    if not value.is_dir() or len(value.parts) < 4 or len(str(value)) < 10:
        raise RuntimeError(f"unsafe fixture directory: {path}")
    return value


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def execute(run, name, command):
    with (run / f"{name}.stdout").open("w") as stdout, (run / f"{name}.stderr").open("w") as stderr:
        process = subprocess.Popen(command, cwd=run, stdout=stdout, stderr=stderr)
        (run / f"{name}.process.json").write_text(json.dumps({
            "pid": process.pid, "argv": command, "started": time.time(),
        }, indent=2) + "\n")
        status = process.wait()
    (run / f"{name}.exit.json").write_text(json.dumps({"exit": status}) + "\n")
    return status


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default=os.environ.get("CC", "cc"), help="Compiler command")
    parser.add_argument("--runner", default="", help="Optional emulator command and arguments")
    args = parser.parse_args()
    parent = safe_directory(REPO / ".tmp")
    run = safe_directory(Path(tempfile.mkdtemp(prefix="backing-offset-", dir=parent)))
    if run.parent != parent or run.stat().st_uid != os.getuid() or run.stat().st_mode & 0o077:
        raise RuntimeError("fixture ownership or permissions differ from the required private directory")
    sources = {str(Path(__file__).resolve().relative_to(REPO)): digest(Path(__file__).resolve())}
    for name in ("backing.c", "backing.h"):
        source = REPO / "src/web/api/promql" / name
        sources[str(source.relative_to(REPO))] = digest(source)
        shutil.copyfile(source, run / name)
    (run / "memory.h").write_text(MEMORY_SHIM)
    (run / "probe.c").write_text(PROBE)
    receipt = {"sources": sources, "passed": False,
               "scope": "actual backing I/O with isolated allocation/control shim; no daemon accounting assertion"}
    compiler = shlex.split(args.cc)
    status = execute(run, "compile", compiler + ["-D_GNU_SOURCE", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
                                                 "backing.c", "probe.c", "-o", "probe"])
    if not status:
        receipt["binary_sha256"] = digest(run / "probe")
        status = execute(run, "probe", shlex.split(args.runner) + [str(run / "probe"), str(run)])
        receipt["result"] = json.loads((run / "probe.stdout").read_text())
        if not status:
            receipt["passed"] = receipt["result"].get("passed") is True
    receipt["exit"] = status
    (run / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"receipt": str(run / "receipt.json"), "passed": receipt["passed"], "exit": status}))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
