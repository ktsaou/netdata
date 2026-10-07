#!/usr/bin/env python3
"""Qualify request-specific invariant boundaries against unchanged pinned Prometheus."""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from build import directory
from capture import verify_facade
from histogram_capture import COMMIT
from histogram_reference import input_ledger
from query import dataset_hex, request_hex

HERE = Path(__file__).resolve().parent


def requests():
    queries = [
        "info(m @ 120 or n @ 480)",
        'info(m @ 120 or n @ 480, {foo="bar"} @ 120)',
        "info(m @ 120 or n @ 120)",
        'info(m @ 120 or n @ 120, {foo="bar"})',
        "info(m @ 180 offset 1m or n @ 120)",
        'info(vector(1), {version=~".*"})',
        'info(vector(1) or n @ 120, {version=~".*"})',
        'info(n @ 120 or vector(1), {version=~".*"})',
        "info(m @ start() or n @ end())",
        "info(m @ end() offset step() or n @ start())",
        "info(m @ 1 * scalar(n @ 2))",
        "info(timestamp(m @ 1))",
        "info(timestamp(abs(m @ 1)))",
        "info(vector(time()))",
        "info(info(m @ start()) or n @ end())",
        'info(m @ start(), {foo="bar"} @ end())',
        "abs(info(m @ start() or n @ end()))",
        "sum(info(m @ start() or n @ end()))",
        "info(m @ start() or n @ end()) * 1",
        "info(m @ end() offset step() or n @ start()) + vector(1)",
        "info(last_over_time((m)[1s:1s] @ end() offset step()) or n @ start())",
        "info(last_over_time((m @ start())[1s:1s] @ end() offset step()) or n @ start())",
        "info(last_over_time(m[1ms] @ start()))",
        "timestamp(m @ start())",
        "timestamp(abs(m @ start()))",
        "sum((m @ start()))",
        "topk(scalar(info(m @ start())), n @ end())",
        "(1)",
        "-(1)",
        "sum_over_time((m @ start())[1s:1s])",
    ]
    contexts = [{"mode": "instant", "time_ms": 1000}]
    contexts += [{"mode": "range", "start_ms": start, "end_ms": end, "step_ms": step}
                 for start, end, step in [(1000, 1000, 1000), (1000, 2000, 1000),
                                          (1000, 3000, 1000), (1000000, 9999000, 1000)]]
    return [{"id": f"preprocessing/{index}/{context}", "query": query, "profile": "upstream-tests",
             "lookback_ms": 300000, "subquery_step_ms": 60000, **request}
            for index, query in enumerate(queries) for context, request in enumerate(contexts)]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def same_boundaries(source, target):
    if "error" in source or "error" in target:
        return source.get("error") == target.get("error")
    for value in (source, target):
        boundaries = value.get("boundaries")
        if (type(boundaries) is not list or any(type(span) is not list or len(span) != 2
                or any(type(position) is not int for position in span) for span in boundaries)):
            return False
    return sorted(source["boundaries"]) == sorted(target["boundaries"])


def capture(upstream):
    source = directory(upstream)
    manifest = verify_facade(source, source)
    digest = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if digest != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original preprocessing source changed")
    task = directory(tempfile.mkdtemp(prefix="preprocessing-capture-", dir=directory(HERE / ".tmp")))
    shutil.copy2(HERE / "capture/preprocessing.go", task / "preprocessing.go")
    for name in ("go.mod", "go.sum"):
        shutil.copy2(HERE / "oracle" / name, task / name)
    private = task / "tmp"
    private.mkdir(mode=0o700)
    env = os.environ.copy() | {"GOMODCACHE": str(directory(source.parents[2])),
                              "GOCACHE": str(directory(HERE / ".tmp/go-build")), "GOPROXY": "off",
                              "GOSUMDB": "off", "GOTOOLCHAIN": "local", "GOWORK": "off",
                              "GOFLAGS": "-mod=readonly", "GOAMD64": "v1", "TMPDIR": str(directory(private)),
                              "GOTMPDIR": str(private)}
    toolchain = subprocess.check_output(["go", "version"], env=env, text=True).strip()
    if toolchain != "go version go1.27.1 linux/amd64":
        raise ValueError("original preprocessing compiler profile changed")
    (task / "toolchain.txt").write_text(toolchain + "\n")
    command = ["go", "build", "-p", "4", "-o", str(task / "preprocessing"), str(task / "preprocessing.go")]
    (task / "build-command.json").write_text(json.dumps(command) + "\n")
    result = subprocess.run(command, cwd=HERE / "oracle", env=env, capture_output=True, timeout=600, check=False)
    (task / "build.stdout").write_bytes(result.stdout)
    (task / "build.stderr").write_bytes(result.stderr)
    if result.returncode:
        raise ValueError(f"original preprocessing build failed; evidence: {task}")
    for name, value in [("requests.json", requests()), ("source.json", manifest)]:
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    result = subprocess.run([str(task / "preprocessing"), str(task / "requests.json")],
                            env=env, capture_output=True, timeout=600, check=False)
    (task / "original.json").write_bytes(result.stdout)
    (task / "original.stderr").write_bytes(result.stderr)
    if result.returncode or verify_facade(source, source) != manifest:
        raise ValueError(f"original preprocessing capture failed or source changed; evidence: {task}")
    original = json.loads(result.stdout)
    if original["commit"] != COMMIT or [row["id"] for row in original["results"]] != [r["id"] for r in requests()]:
        raise ValueError("original preprocessing result roster changed")
    names = ["preprocessing.go", "preprocessing", "go.mod", "go.sum", "toolchain.txt", "build-command.json",
             "requests.json", "source.json", "original.json"]
    binding = {"commit": COMMIT, "cases": len(requests()), "source_manifest_sha256": digest,
               "files": {name: sha(task / name) for name in names}}
    (task / "binding.json").write_text(json.dumps(binding, indent=2) + "\n")
    print(json.dumps({"original_cases": len(requests()), "capture": str(task)}), flush=True)


def qualify(reference, binary):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    binding = json.loads((reference / "binding.json").read_text())
    if binding["commit"] != COMMIT or binding["source_manifest_sha256"] != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original preprocessing binding changed")
    for name, expected in binding["files"].items():
        if Path(name).name != name or sha(reference / name) != expected:
            raise ValueError("original preprocessing artifact changed")
    if set(binding["files"]) != {"preprocessing.go", "preprocessing", "go.mod", "go.sum", "toolchain.txt",
                                  "build-command.json", "requests.json", "source.json", "original.json"}:
        raise ValueError("original preprocessing owner roster changed")
    manifest = json.loads((reference / "source.json").read_text())
    digest = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if (sha(reference / "preprocessing.go") != sha(HERE / "capture/preprocessing.go")
            or digest != binding["source_manifest_sha256"]
            or (reference / "toolchain.txt").read_text() != "go version go1.27.1 linux/amd64\n"
            or any(sha(reference / name) != sha(HERE / "oracle" / name) for name in ("go.mod", "go.sum"))
            or json.loads((reference / "requests.json").read_text()) != requests()
            or binding["cases"] != len(requests())):
        raise ValueError("original preprocessing inputs changed")
    original = json.loads((reference / "original.json").read_text())
    if original["commit"] != COMMIT:
        raise ValueError("original preprocessing output reference changed")
    task = directory(tempfile.mkdtemp(prefix="preprocessing-compare-", dir=directory(HERE / ".tmp")))
    rows = [{"op": "dataset", "dataset_id": "flags", "page_size": 1,
             "dataset": dataset_hex({"schema_version": 1, "series": []})}]
    rows += [dict(request_hex(request, "flags"), op="prepare") for request in requests()]
    (task / "requests.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
    with ((task / "requests.jsonl").open("rb") as inputs, (task / "candidate.jsonl").open("xb") as outputs,
          (task / "candidate.stderr").open("xb") as errors):
        result = subprocess.run([str(binary)], stdin=inputs, stdout=outputs, stderr=errors, timeout=600, check=False)
    if result.returncode:
        raise ValueError(f"candidate preprocessing failed; evidence: {task}")
    actual = [json.loads(line) for line in (task / "candidate.jsonl").read_text().splitlines()]
    if actual[0] != {"dataset_id": "flags"} or [row["id"] for row in actual[1:]] != [r["id"] for r in requests()]:
        raise ValueError("candidate preprocessing result roster changed")
    if [row["id"] for row in original["results"]] != [r["id"] for r in requests()]:
        raise ValueError("original preprocessing result roster changed")
    failures = []
    for expected, candidate in zip(original["results"], actual[1:], strict=True):
        source, target = expected["scope"], candidate["scope"]
        if not same_boundaries(source, target):
            failures.append({"id": expected["id"], "original": source, "candidate": target})
    report = {"cases": len(requests()), "failed": len(failures), "pass": not failures, "failures": failures,
              "binary_sha256": sha(binary), "reference_sha256": sha(reference / "original.json"),
              "input_sha256": sha(task / "requests.jsonl"), "output_sha256": sha(task / "candidate.jsonl")}
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("cases", "failed", "pass")} | {"evidence": str(task)}), flush=True)
    return report["pass"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    args = parser.parse_args()
    if args.capture:
        if not args.upstream:
            parser.error("capture requires --upstream")
        capture(args.upstream)
    else:
        if not args.reference or not args.binary:
            parser.error("comparison requires --reference and --binary")
        raise SystemExit(not qualify(args.reference, args.binary))
