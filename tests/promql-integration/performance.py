#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Measure a fixed warm HTTP/DBengine panel in owned Linux internal-check Agents."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import sys
import tempfile
from threading import Barrier

from live import fixture_expressions, qualify, request, target
from storage import REPO, safe_parent, sha256, source_binding

PLAN = {"warmups": 1, "serial_samples": 7, "concurrent_clients": 2, "concurrent_batches": 4}
TIMINGS = {"evalTotalTime", "resultSortTime", "queryPreparationTime", "innerEvalTime",
           "execQueueTime", "execTotalTime"}


class ProcessMemory:
    def __init__(self, pid, binary):
        if type(pid) is not int or pid <= 0:
            raise RuntimeError("invalid owned process identity")
        self.process = Path("/proc") / str(pid)
        stat = binary.stat()
        self.executable = stat.st_dev, stat.st_ino
        self.started = self.identity()

    def identity(self):
        stat = (self.process / "exe").stat()
        if (stat.st_dev, stat.st_ino) != self.executable:
            raise RuntimeError("owned process executable changed")
        fields = (self.process / "stat").read_text().rsplit(") ", 1)[1].split()
        if fields[0] == "Z":
            raise RuntimeError("owned process exited before observation")
        return int(fields[19])

    def read(self):
        if self.identity() != self.started:
            raise RuntimeError("owned process start identity changed")
        fields = dict(line.split(":", 1) for line in (self.process / "status").read_text().splitlines() if ":" in line)
        result = {}
        for name, key in [("VmRSS", "rss_bytes"), ("VmHWM", "lifetime_high_water_bytes")]:
            number, unit = fields[name].split()
            if unit != "kB":
                raise RuntimeError("unexpected process memory unit")
            result[key] = int(number) * 1024
        result["threads"] = int(fields["Threads"])
        if self.identity() != self.started:
            raise RuntimeError("owned process identity changed during observation")
        return result


def cases(ready):
    expressions = fixture_expressions()
    instant = ready["base"] + 4
    result = []
    for name, expression, value, local, reads in [
        ("scalar", "1", "1", False, False),
        ("fleet-gauge", f'scalar({expressions["gauge"]})', "12", False, True),
        ("local-counter-rate", expressions["rate"], "2", True, True),
        ("fleet-average", f'scalar({expressions["average"]})', "12", False, True),
        ("six-group-histogram", f'scalar({expressions["grouped_quantile"]})', "3", False, True),
    ]:
        expected = {"status": "success", "data": {"resultType": "scalar", "result": [instant, value]}}
        if name == "local-counter-rate":
            expected["infos"] = ['PromQL info: metric might not be a counter, name does not end in '
                                 '_total/_sum/_count/_bucket: "fixture_http" (1:13)']
        result.append({"name": name, "endpoint": "query", "compressed": False,
                       "prefix": "/host/" + ready["host"] if local else "",
                       "parameters": {"query": expression, "time": instant, "stats": "all"},
                       "expected": expected, "reads_dbengine": reads})
    for compressed in (False, True):
        result.append({"name": "delivery-gzip" if compressed else "delivery-plain", "endpoint": "query_range",
                       "compressed": compressed, "prefix": "", "reads_dbengine": False,
                       "parameters": {"query": "vector(1)", "start": ready["base"],
                                      "end": ready["base"] + 99999, "step": 1, "stats": "all"}})
    return result


def validate(case, response, body):
    if response["status"] != 200:
        raise RuntimeError(f'{case["name"]}: unexpected HTTP status {response["status"]}')
    if case["compressed"] and response["headers"].get("Content-Encoding") != "gzip":
        raise RuntimeError("requested gzip delivery was not encoded")
    value = json.loads(body)
    raw_data = value.get("data", {})
    observed = value | {"data": {key: item for key, item in raw_data.items() if key != "stats"}}
    if "expected" in case:
        correct = observed == case["expected"]
    else:
        data = observed.get("data", {})
        rows = data.get("result", [])
        correct = (set(observed) == {"status", "data"} and observed["status"] == "success" and
                   set(data) == {"resultType", "result"} and data["resultType"] == "matrix" and len(rows) == 1 and
                   set(rows[0]) == {"metric", "values"} and rows[0]["metric"] == {} and
                   len(rows[0]["values"]) == 100000 and
                   all(point == [case["parameters"]["start"] + index, "1"]
                       for index, point in enumerate(rows[0]["values"])))
    if not correct:
        raise RuntimeError(f'{case["name"]}: complete HTTP result differs from the fixed expectation')
    stats = raw_data.get("stats", {})
    timings = stats.get("timings", {})
    samples = stats.get("samples", {})
    if set(stats) != {"timings", "samples", "netdata"} or set(timings) != TIMINGS:
        raise RuntimeError("missing or unexpected HTTP timing statistics")
    if any(type(value) not in (int, float) or not math.isfinite(value) or value < 0 for value in timings.values()):
        raise RuntimeError("invalid server timing")
    if set(samples) != {"totalQueryableSamples", "peakSamples"} or any(type(n) is not int or n < 0 for n in samples.values()):
        raise RuntimeError("invalid sample statistics")
    if bool(samples["totalQueryableSamples"]) != case["reads_dbengine"]:
        raise RuntimeError("query's logical reads differ from its DBengine/control purpose")
    resources = stats["netdata"]
    if resources.get("snapshot") != "evaluation-before-output-commit" or len(resources.get("memoryBytes", {})) != 8:
        raise RuntimeError("missing evaluation resource snapshot")
    return stats


def measure(root, ready, binary):
    memory = ProcessMemory(ready["pid"], binary)
    directory = root / "measurements"
    directory.mkdir(mode=0o700)
    directory = safe_parent(directory)
    if directory.parent != root:
        raise RuntimeError("measurement directory escaped the fixture")
    result = {"plan": PLAN, "baseline_phase": "after existing HTTP/readiness qualification",
              "process_before": memory.read(), "cases": []}

    def sample(case, mode, index, client=0, barrier=None):
        before = memory.read()
        if barrier is not None:
            barrier.wait(timeout=30)
        response, body = request(ready["port"], target(case["endpoint"], case["parameters"], case["prefix"]),
                                 compressed=case["compressed"])
        artifact = directory / f'{case["name"]}-{mode}-{index}-{client}.json.gz'
        with artifact.open("xb") as output:
            output.write(gzip.compress(body, compresslevel=1, mtime=0))
        record = {"mode": mode, "index": index, "client": client, **response, "process_before": before,
                  "body_sha256": hashlib.sha256(body).hexdigest(), "passed": False,
                  "body_artifact": artifact.relative_to(REPO).as_posix(), "artifact_sha256": sha256(artifact)}
        try:
            record["process_after"] = memory.read()
            record["statistics"] = validate(case, response, body)
            record["passed"] = True
        finally:
            with artifact.with_suffix(".record.json").open("x") as output:
                json.dump(record, output, indent=2)
                output.write("\n")
        return record

    for case in cases(ready):
        item = {"case": case, "requests": [], "batches": []}
        result["cases"].append(item)
        item["requests"].append(sample(case, "warmup", 0))
        for index in range(PLAN["serial_samples"]):
            item["requests"].append(sample(case, "serial", index))
        with ThreadPoolExecutor(max_workers=PLAN["concurrent_clients"]) as executor:
            for index in range(PLAN["concurrent_batches"]):
                barrier = Barrier(PLAN["concurrent_clients"])
                futures = [executor.submit(sample, case, "concurrent", index, client, barrier)
                           for client in range(PLAN["concurrent_clients"])]
                rows = [future.result() for future in futures]
                item["requests"].extend(rows)
                item["batches"].append({
                    "index": index,
                    "wire_span_ns": max(row["wire_finished_ns"] for row in rows) - min(row["wire_started_ns"] for row in rows),
                    "client_wire_overlap_ns": max(0, min(row["wire_finished_ns"] for row in rows) -
                                                    max(row["wire_started_ns"] for row in rows)),
                })
        serial = [row for row in item["requests"] if row["mode"] == "serial"]
        durations = [row["wire_elapsed_ns"] for row in serial]
        item["summary"] = {
            "serial_wire_ns": {"min": min(durations), "median": statistics.median(durations), "max": max(durations)},
            "serial_server_timing_medians_seconds": {
                name: statistics.median(row["statistics"]["timings"][name] for row in serial) for name in sorted(TIMINGS)},
            "two_client_span_median_ns": statistics.median(row["wire_span_ns"] for row in item["batches"]),
            "client_overlapping_batches": sum(row["client_wire_overlap_ns"] > 0 for row in item["batches"]),
        }
        samples = [row["statistics"]["samples"] for row in item["requests"]]
        if any(value != samples[0] for value in samples):
            raise RuntimeError("same fixed query returned different logical/peak sample counts")
        item["summary"]["sample_statistics"] = samples[0]
        print(f'{root.name}: {case["name"]}: median {item["summary"]["serial_wire_ns"]["median"] / 1e6:.3f} ms', flush=True)
    result["process_after"] = memory.read()
    result["passed"] = True
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--codecs", nargs="+", choices=["array", "gorilla"], default=["array", "gorilla"])
    args = parser.parse_args()
    if not sys.platform.startswith("linux"):
        raise RuntimeError("process memory observations require Linux /proc")
    binary = args.binary.resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError("Agent binary is not executable")
    parent = safe_parent(REPO / ".tmp")
    run = Path(tempfile.mkdtemp(prefix="live-speed-", dir=parent)).resolve(strict=True)
    if run.parent != parent or run.stat().st_uid != os.getuid() or run.stat().st_mode & 0o077:
        raise RuntimeError("invalid private measurement directory")
    print(f"measurement_root: {run.relative_to(REPO)}", flush=True)
    names = [Path(__file__), REPO / "tests/promql-integration/live.py", REPO / "tests/promql-integration/test_live.py",
             REPO / "tests/promql-integration/test_live_lifecycle.py", REPO / "tests/promql-integration/test_performance.py"]
    sources = source_binding() | {path.relative_to(REPO).as_posix(): sha256(path) for path in names}
    binary_hash = sha256(binary)
    receipt = {"binary_sha256": binary_hash, "sources": sources, "plan": PLAN, "runs": [], "passed": False,
               "limits": ["Tiny fixed-clock two-host data fits configured DBengine caches; warm API evidence only.",
                          "Client request overlap does not establish simultaneous Rust evaluators.",
                          "RSS is the whole Agent; high-water includes startup and earlier qualification.",
                          "Wire duration includes connection establishment, server work and final body delivery; excludes client decoding/validation.",
                          "HTTP sample counts are logical and do not count fetched pages or allocated bytes.",
                          "Shared host; no CPU/frequency isolation, cold-disk, high-cardinality, IoT or physical ARM performance claim."]}
    try:
        for codec in args.codecs:
            receipt["runs"].append(qualify(binary, codec, run, observe=lambda root, ready: measure(root, ready, binary)))
        current = source_binding() | {path.relative_to(REPO).as_posix(): sha256(path) for path in names}
        if current != sources or sha256(binary) != binary_hash:
            raise RuntimeError("source or binary changed during measurement")
        receipt["passed"] = True
    finally:
        (run / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"receipt: {run.relative_to(REPO) / 'receipt.json'}", flush=True)


if __name__ == "__main__":
    main()
