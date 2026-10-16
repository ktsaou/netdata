#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check actual point serialization against frozen protocol originals and a supplied Rust archive."""

import argparse
import base64
import json
import os
from pathlib import Path
import shlex
import shutil
import tempfile

from backing import digest, execute, safe_directory

REPO = Path(__file__).resolve().parents[2]


def originals(path):
    rows = json.loads(path.read_text())["helpers"]
    floats = [row for row in rows if row["op"] == "sample_float"]
    times = [row for row in rows if row["op"] == "sample_timestamp"]
    if len(floats) != 21 or len(times) != 10 or not all(row["success"] for row in floats + times):
        raise RuntimeError("unexpected frozen protocol roster")

    def quoted(row):
        text = base64.b64decode(row["raw_json"], validate=True).decode("ascii")
        if "\0" in text:
            raise RuntimeError("unexpected NUL in original")
        return json.dumps(text)

    def integer(value):
        number = int(value)
        if number == -(1 << 63):
            return "(-INT64_C(9223372036854775807) - 1)"
        return ("-" if number < 0 else "") + f"INT64_C({abs(number)})"

    lines = ["// Generated from the committed independent protocol originals.",
             "static const struct { uint64_t bits; const char *json; } original_floats[] = {"]
    lines += [f"    {{UINT64_C(0x{row['bits']}), {quoted(row)}}}," for row in floats]
    lines += ["};", "static const struct { int64_t milliseconds; const char *json; } original_times[] = {"]
    lines += [f"    {{{integer(row['time_ms'])}, {quoted(row)}}}," for row in times]
    return "\n".join(lines + ["};", ""])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", type=Path, required=True, help="Existing in-repository Rust static archive")
    parser.add_argument("--cc", default=os.environ.get("CC", "cc"))
    parser.add_argument("--runner", default="", help="Optional emulator and arguments")
    parser.add_argument("--output-source", type=Path, help="Captured in-repository output.c for comparisons")
    parser.add_argument("--allow-legacy-polls", action="store_true", help="Record old work without the improvement bound")
    args = parser.parse_args()
    parent = safe_directory(REPO / ".tmp")
    run = safe_directory(Path(tempfile.mkdtemp(prefix="point-append-", dir=parent)))
    if run.parent != parent or run.stat().st_uid != os.getuid() or run.stat().st_mode & 0o077:
        raise RuntimeError("unexpected fixture ownership or permissions")
    source = REPO / "src/web/api/promql"
    library = args.library.resolve(strict=True)
    output_source = args.output_source.resolve(strict=True) if args.output_source else source / "output.c"
    if any(not path.is_file() or not path.is_relative_to(REPO) for path in (library, output_source)):
        raise RuntimeError("library and output source must be files in this repository")
    names = ("output.c", "output.h", "body.c", "body.h", "backing.c", "backing.h",
             "json.c", "json.h", "keys.h", "notes.c", "notes.h")
    core = REPO / "src/crates/promql-engine"
    corpus = core / "tests/promql-conformance/protocol-original.json"
    inputs = [output_source if name == "output.c" else source / name for name in names]
    inputs += [Path(__file__).resolve(), Path(__file__).with_name("point_append.c"),
               Path(__file__).with_name("output_records.c"), Path(__file__).with_name("output_records_memory.h"),
               Path(__file__).with_name("backing.py"), core / "include/netdata-promql.h", corpus]
    pins = {path.relative_to(REPO).as_posix(): digest(path) for path in inputs}
    library_pin = digest(library)
    for name in names:
        shutil.copyfile(output_source if name == "output.c" else source / name, run / name)
    for name in ("output_records.c", "backing.py"):
        shutil.copyfile(Path(__file__).with_name(name), run / name)
    shutil.copyfile(Path(__file__).with_name("point_append.c"), run / "probe.c")
    shutil.copyfile(Path(__file__).with_name("output_records_memory.h"), run / "memory.h")
    shutil.copyfile(Path(__file__), run / "runner.py")
    shutil.copyfile(core / "include/netdata-promql.h", run / "netdata-promql.h")
    shutil.copyfile(corpus, run / "protocol-original.json")
    (run / "point-originals.h").write_text(originals(corpus))
    system_headers = {}
    for name in ("zlib.h", "zconf.h"):
        header = Path("/usr/include") / name
        system_headers[name] = digest(header)
        shutil.copyfile(header, run / name)
    web = run / "web/server"
    web.mkdir(parents=True)
    (web / "web_client.h").write_text("extern int web_gzip_level, web_gzip_strategy;\n")
    receipt = {"sources": pins, "library": {"path": library.relative_to(REPO).as_posix(), "sha256": library_pin},
               "installed_header_sha256": system_headers, "passed": False,
               "scope": "Actual point_append/body/backing/json and real supplied Rust formatters; frozen original bytes. Isolated allocation/control, synthetic histogram coexistence; no daemon accounting or native histogram-iteration claim."}
    command = shlex.split(args.cc) + ["-D_GNU_SOURCE", "-D_FILE_OFFSET_BITS=64", "-std=c11", "-O3",
        "-Wall", "-Wextra", "-Werror", "-ffunction-sections", "-fdata-sections", "-fstack-usage", "-I", str(run),
        "probe.c", "backing.c", "json.c", "notes.c", str(library), "-Wl,--gc-sections",
        "-Wl,--wrap=pread64", "-Wl,--wrap=pwrite64", "-Wl,--wrap=netdata_promql_api_float_text",
        "-Wl,--wrap=netdata_promql_api_timestamp_text", "-lpthread", "-ldl", "-lm", "-lrt", "-lutil", "-o", "probe"]
    print(shlex.join(command), flush=True)
    status = execute(run, "compile", command)
    if not status:
        receipt["binary_sha256"] = digest(run / "probe")
        command = shlex.split(args.runner) + [str(run / "probe"), str(run)]
        if args.allow_legacy_polls:
            command.append("--allow-legacy-polls")
        print(shlex.join(command), flush=True)
        status = execute(run, "probe", command)
        receipt["records"] = [json.loads(line) for line in (run / "probe.stdout").read_text().splitlines()]
        receipt["passed"] = not status and len(receipt["records"]) == 25 and len({
            row["case"] for row in receipt["records"]}) == 25 and all(not row["errors"] for row in receipt["records"])
    if digest(library) != library_pin or {path.relative_to(REPO).as_posix(): digest(path) for path in inputs} != pins:
        raise RuntimeError("fixture inputs changed during execution")
    receipt["exit"] = status
    (run / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"receipt": str(run / "receipt.json"), "passed": receipt["passed"], "exit": status}), flush=True)
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
