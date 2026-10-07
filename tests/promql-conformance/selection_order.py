"""Capture original boolean sorting/heap operations with immutable source bindings."""
import argparse
import json
import os
import re
import shutil
import tempfile
from pathlib import Path

from build import directory
from float_literals import COMMIT, decode, digest, invoke
from histogram_reference import COMMIT as PROMETHEUS_COMMIT
from numerical_reference import PROFILE
from selection_order_cases import generate

HERE = Path(__file__).resolve().parent
SOURCES = {
    "src/sort/sort.go": "100e49103d23dd6b8335a50b17ae6e9db2b056cb1217d964dcf58852c3c6f8a1",
    "src/sort/zsortinterface.go": "8978a49cc4174b0c35a84a2ddf73175d41254eb340ebeda29f21d3e546088a3b",
    "src/container/heap/heap.go": "5a04882f60417c2535138f37daeaeb254a77487d0abd0bc1958290d50b8043ec",
    "VERSION": "25eb74b09036e1ad894fb63b86cb0f0ee53342df8d2e2e860da4ff5079607758",
    "LICENSE": "911f8f5782931320f5b8d1160a76365b83aea6447ee6c04fa6d5591467db9dad",
}
PROMETHEUS_SOURCE = "07e4c19d39b7112b24b55e999c16981bff9e719b9fd7a05f488d8b582821bbbc"
FILES = {"selection_order.go", "original_helpers.go", "order-go", "requests.json", "reference.jsonl",
         "original-tests.stdout", "original-tests.stderr", "prometheus-functions.go"}


def original_helpers(sort_source, generated, prometheus):
    # Copy the private helpers and comparators verbatim; only their package/type bindings are supplied.
    helpers = generated[generated.index("// insertionSort"):generated.index("func swapRange")]
    definitions = sort_source[sort_source.index("type sortedHint"):sort_source.index("// lessSwap")]
    comparators = prometheus[prometheus.index("type vectorByValueHeap Vector"):
                             prometheus.index("// createLabelsForAbsentFunction")]
    return ('// Source-derived private helper bindings; see capture manifest for owners.\n'
            'package main\nimport ("math"; "math/bits"; "sort")\n'
            'type Interface = sort.Interface\n' + definitions + helpers + comparators).encode()


def capture(upstream):
    upstream = directory(upstream)
    owner = upstream / "promql/functions.go"
    if digest(owner) != PROMETHEUS_SOURCE:
        raise ValueError("pinned Prometheus heap comparators changed")
    parent = directory(HERE / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="selection-order-capture-", dir=parent))
    private = task / "tmp"
    private.mkdir(mode=0o700)
    env = os.environ.copy() | {
        "GOTOOLCHAIN": "local", "GO111MODULE": "off", "GOWORK": "off", "GOFLAGS": "", "GOAMD64": "v1",
        "GOPROXY": "off", "GOSUMDB": "off", "GOCACHE": str(parent / "go-build"),
        "TMPDIR": str(directory(private)), "GOTMPDIR": str(private)}
    profile = decode(invoke(["go", "env", "-json", "GOROOT", *PROFILE], task, env, "profile"))
    runtime = directory(profile.pop("GOROOT"))
    if profile != PROFILE:
        raise ValueError("selection ordering requires the pinned Go 1.27.1 AMD64 profile")
    for name, expected in SOURCES.items():
        if digest(runtime / name) != expected:
            raise ValueError(f"pinned original ordering owner changed: {name}")
        destination = task / "go-source" / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        directory(destination.parent)
        shutil.copyfile(runtime / name, destination)
    shutil.copyfile(owner, task / "prometheus-functions.go")
    shutil.copyfile(HERE / "capture/selection_order.go", task / "selection_order.go")
    helpers = original_helpers((runtime / "src/sort/sort.go").read_text(),
                               (runtime / "src/sort/zsortinterface.go").read_text(), owner.read_text())
    (task / "original_helpers.go").write_bytes(helpers)
    invoke(["go", "test", "-count=1", "-p", "4", "-timeout=10m", "sort", "container/heap"],
           task, env, "original-tests")
    invoke(["go", "build", "-p", "4", "-o", task / "order-go", task / "selection_order.go",
            task / "original_helpers.go"], task, env, "build")
    requests = generate()
    payload = json.dumps(requests, separators=(",", ":")).encode() + b"\n"
    (task / "requests.json").write_bytes(payload)
    (task / "reference.jsonl").write_bytes(invoke([task / "order-go"], task, env, "reference", payload))
    if any(digest(runtime / name) != expected for name, expected in SOURCES.items()) or digest(owner) != PROMETHEUS_SOURCE:
        raise ValueError("original ordering source changed during capture")
    manifest = {"schema_version": 1, "runtime_commit": COMMIT, "prometheus_commit": PROMETHEUS_COMMIT,
                "profile": profile, "sources": SOURCES, "prometheus_source": PROMETHEUS_SOURCE,
                "required": len(requests), "generator_sha256": digest(HERE / "selection_order_cases.py"),
                "files": {name: digest(task / name) for name in sorted(FILES)}}
    (task / "manifest.json").write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n")
    load(task)
    print(json.dumps({"capture": str(task), "required": len(requests)}), flush=True)
    return task


def outcomes(raw, requests):
    results = [decode(line) for line in raw.splitlines()]
    if len(results) != len(requests):
        raise ValueError("incomplete selection ordering outcomes")
    for case, request in zip(results, requests, strict=True):
        if set(case) != {"id", "actual"} or case["id"] != request["id"]:
            raise ValueError("unbound selection ordering identity")
        actual = case["actual"]
        if (set(actual) != {"items", "comparisons", "swaps", "trace", "result"}
                or type(actual["items"]) is not list
                or any(type(actual[key]) is not int or actual[key] < 0 for key in ("comparisons", "swaps"))
                or type(actual["trace"]) is not str or not re.fullmatch(r"[0-9a-f]{16}", actual["trace"])
                or actual["result"] is not None and (type(actual["result"]) is not list
                    or any(type(value) is not int or value < 0 for value in actual["result"]))):
            raise ValueError("malformed selection ordering outcome")
        items = actual["items"]
        if len(items) != len(request["values"]):
            raise ValueError("changed selection ordering item count")
        seen = set()
        changed = []
        for item in items:
            ordinal = item["ordinal"]
            if (set(item) != {"bits", "ordinal"} or type(ordinal) is not int
                    or not 0 <= ordinal < len(items) or ordinal in seen
                    or type(item["bits"]) is not str or not re.fullmatch(r"[0-9a-f]{16}", item["bits"])):
                raise ValueError("ordering changed identity or payload representation")
            if request["action"] != "fix" and item["bits"] != request["values"][ordinal]:
                raise ValueError("ordering changed a sample value")
            if item["bits"] != request["values"][ordinal]:
                changed.append(item["bits"])
            seen.add(ordinal)
        if request["action"] == "fix" and (len(changed) > 1 or any(
                value != request["replacement"] for value in changed)):
            raise ValueError("heap Fix changed more than its one declared value")
        result = actual["result"]
        action = request["action"]
        if action in {"partial", "equal", "partition", "pivot"}:
            width = 2 if action in {"partition", "pivot"} else 1
            if result is None or len(result) != width:
                raise ValueError("missing original helper return value")
            if (action == "partial" and result[0] > 1
                    or action in {"equal", "partition"} and result[0] > len(items)
                    or action == "partition" and result[1] > 1
                    or action == "pivot" and (result[0] >= len(items) or result[1] > 2)):
                raise ValueError("invalid original helper return value")
        elif result is not None:
            raise ValueError("unexpected original helper return value")
    return results


def load(task):
    task = directory(task)
    manifest = decode((task / "manifest.json").read_bytes())
    if (manifest["schema_version"] != 1 or manifest["runtime_commit"] != COMMIT
            or manifest["prometheus_commit"] != PROMETHEUS_COMMIT
            or manifest["profile"] != PROFILE or manifest["sources"] != SOURCES
            or manifest["prometheus_source"] != PROMETHEUS_SOURCE
            or manifest["generator_sha256"] != digest(HERE / "selection_order_cases.py")
            or set(manifest["files"]) != FILES):
        raise ValueError("selection ordering capture contract changed")
    if (any(digest(task / name) != expected for name, expected in manifest["files"].items())
            or any(digest(task / "go-source" / name) != expected for name, expected in SOURCES.items())
            or digest(task / "prometheus-functions.go") != PROMETHEUS_SOURCE
            or digest(task / "selection_order.go") != digest(HERE / "capture/selection_order.go")
            or (task / "original_helpers.go").read_bytes() != original_helpers(
                (task / "go-source/src/sort/sort.go").read_text(),
                (task / "go-source/src/sort/zsortinterface.go").read_text(),
                (task / "prometheus-functions.go").read_text())):
        raise ValueError("selection ordering source or helper binding changed")
    requests = generate()
    if decode((task / "requests.json").read_bytes()) != requests or manifest["required"] != len(requests):
        raise ValueError("selection ordering input roster changed")
    return requests, outcomes((task / "reference.jsonl").read_bytes(), requests)


def compare(reference, binary):
    requests, expected = load(reference)
    binary = Path(binary).expanduser().resolve(strict=True)
    binary_hash = digest(binary)
    task = directory(tempfile.mkdtemp(prefix="selection-order-compare-", dir=directory(HERE / ".tmp")))
    packets = [{"id": row["id"], "mode": "selection-order", "borrowed": True,
                "request": {key: value for key, value in row.items() if key != "id"}} for row in requests]
    payload = b"".join(json.dumps(row, separators=(",", ":")).encode() + b"\n" for row in packets)
    (task / "requests.jsonl").write_bytes(payload)
    actual = outcomes(invoke([binary], task, os.environ.copy(), "candidate", payload), requests)
    if digest(binary) != binary_hash:
        raise ValueError("candidate ordering binary changed during comparison")
    failures = [{"id": source["id"], "path": "actual." + key}
                for source, candidate in zip(expected, actual, strict=True)
                for key in ("items", "comparisons", "swaps", "trace", "result")
                if source["actual"][key] != candidate["actual"][key]]
    report = {"required": len(requests), "pass": not failures, "failures": failures,
              "binary_sha256": binary_hash, "reference_sha256": digest(Path(reference) / "reference.jsonl"),
              "input_sha256": digest(task / "requests.jsonl"), "output_sha256": digest(task / "candidate.stdout")}
    (task / "report.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key != "failures"} |
                     {"failures": len(failures), "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", help="existing pinned Prometheus source, needed for a new capture")
    parser.add_argument("--reference", help="reuse an original capture; omit to capture")
    parser.add_argument("--binary", help="compare an existing Rust kernel fixture binary")
    args = parser.parse_args()
    if not args.reference and not args.upstream:
        parser.error("a new capture requires --upstream")
    reference = args.reference or capture(args.upstream)
    if args.binary:
        raise SystemExit(not compare(reference, args.binary)["pass"])
