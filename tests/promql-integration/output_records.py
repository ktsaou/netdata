#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise actual output-chain/body/backing code with isolated allocation and I/O controls."""

import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import tempfile

from backing import digest, execute, safe_directory

REPO = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", default=os.environ.get("CC", "cc"))
    parser.add_argument("--runner", default="", help="Optional emulator and arguments")
    parser.add_argument("--output-source", type=Path, help="Retained in-repository output.c for comparisons")
    parser.add_argument("--allow-legacy-reads", action="store_true", help="Record old read counts without the improvement bound")
    args = parser.parse_args()
    parent = safe_directory(REPO / ".tmp")
    run = safe_directory(Path(tempfile.mkdtemp(prefix="output-records-", dir=parent)))
    if run.parent != parent or run.stat().st_uid != os.getuid() or run.stat().st_mode & 0o077:
        raise RuntimeError("unexpected fixture ownership or permissions")
    source = REPO / "src/web/api/promql"
    names = ("output.c", "output.h", "body.c", "body.h", "backing.c", "backing.h",
             "json.c", "json.h", "keys.h", "notes.h")
    output_source = args.output_source.resolve(strict=True) if args.output_source else source / "output.c"
    if not output_source.is_file() or not output_source.is_relative_to(REPO):
        raise RuntimeError("output source must be a file within this repository")
    inputs = [output_source if name == "output.c" else source / name for name in names]
    inputs += [Path(__file__).resolve(), Path(__file__).with_name("backing.py"),
               Path(__file__).with_name("output_records.c"),
               Path(__file__).with_name("output_records_memory.h"),
               REPO / "src/crates/promql-engine/include/netdata-promql.h"]
    pins = {path.relative_to(REPO).as_posix(): digest(path) for path in inputs}
    for name in names:
        shutil.copyfile(output_source if name == "output.c" else source / name, run / name)
    shutil.copyfile(Path(__file__).with_name("output_records.c"), run / "probe.c")
    shutil.copyfile(Path(__file__).with_name("output_records_memory.h"), run / "memory.h")
    shutil.copyfile(Path(__file__), run / "runner.py")
    shutil.copyfile(Path(__file__).with_name("backing.py"), run / "backing.py")
    shutil.copyfile(REPO / "src/crates/promql-engine/include/netdata-promql.h", run / "netdata-promql.h")
    # Unused gzip code still needs its installed portable declarations when compiling for ARM.
    system_headers = {}
    for name in ("zlib.h", "zconf.h"):
        header = Path("/usr/include") / name
        system_headers[name] = digest(header)
        shutil.copyfile(header, run / name)
    web = run / "web/server"
    web.mkdir(parents=True)
    (web / "web_client.h").write_text("extern int web_gzip_level, web_gzip_strategy;\n")
    receipt = {"sources": pins, "installed_header_sha256": system_headers, "passed": False,
               "scope": "Actual production output.c/body.c/backing.c/json.c with isolated allocation/control and counted real I/O. No daemon accounting, numerical formatting, gzip, engine or transport claim."}
    command = shlex.split(args.cc) + ["-D_GNU_SOURCE", "-D_FILE_OFFSET_BITS=64", "-std=c11", "-O3",
        "-Wall", "-Wextra", "-Werror", "-ffunction-sections", "-fdata-sections", "-I", str(run),
        "probe.c", "backing.c", "json.c", "-Wl,--gc-sections", "-Wl,--wrap=pread64",
        "-Wl,--wrap=pwrite64", "-o", "probe"]
    print(shlex.join(command), flush=True)
    status = execute(run, "compile", command)
    if not status:
        receipt["binary_sha256"] = digest(run / "probe")
        command = shlex.split(args.runner) + [str(run / "probe"), str(run)]
        if args.allow_legacy_reads:
            command.append("--allow-legacy-reads")
        print(shlex.join(command), flush=True)
        status = execute(run, "probe", command)
        receipt["records"] = [json.loads(line) for line in (run / "probe.stdout").read_text().splitlines()]
        receipt["passed"] = not status and len(receipt["records"]) == 29 and len({
            row["case"] for row in receipt["records"]}) == 29 and all(
            not row["errors"] for row in receipt["records"])
    if {path.relative_to(REPO).as_posix(): digest(path) for path in inputs} != pins:
        raise RuntimeError("fixture sources changed during execution")
    receipt["exit"] = status
    (run / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"receipt": str(run / "receipt.json"), "passed": receipt["passed"], "exit": status}), flush=True)
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
