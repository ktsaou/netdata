#!/usr/bin/env python3
"""Qualify histogram annotation construction/merging against the original pinned Go helpers."""
import argparse
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from build import directory
from capture import verify_facade
from float_literals import COMMIT, decode, digest, invoke
from float_text import GO_SOURCES, PROMETHEUS_COMMIT, PROMETHEUS_SOURCES
from histogram_annotations_cases import generate
from numerical_reference import PROFILE

HERE = Path(__file__).resolve().parent
TIME_SOURCES = {'src/time/format.go': 'f2b75d1440a46231523d3be86925b52c59c5f23657658e24514f2daa588e575d',
 'src/time/format_rfc3339.go': '14b2a58fa295cca1a4f99467f5a5059418ed266a2298316124af97ffa014f271',
 'src/time/format_test.go': 'f8646ea90208c0904b5988685c2d78a5f90535ef98a2b4dcd8d53fbb45f0d9a4',
 'src/time/time.go': '7d992810b162f65e02cea4d60fd0d90af59b3cafa2c8926ca08083d5c73451dc',
 'src/time/zoneinfo.go': '198928e8d2573d88d8743db57b86777a26506f17ef2b0283882a9f5c884979b7'}
REQUIRED = {"histogram_annotations.go", "histogram_annotations_cases.py", "histogram_annotations.py",
            "annotation-go", "requests.json", "reference.json", "go.mod", "go.sum",
            "original-tests.stdout", "original-tests.stderr", "time-tests.stdout", "time-tests.stderr"}


def capture():
    parent = HERE / ".tmp"
    parent.mkdir(exist_ok=True)
    task = directory(tempfile.mkdtemp(prefix="histogram-annotations-capture-", dir=directory(parent)))
    private = task / "tmp"
    private.mkdir(mode=0o700)
    cache = directory(HERE.parents[1].parent / "netdata/tests/promql-prototype/oracle/.cache/go-mod")
    prometheus = directory(cache / "github.com/prometheus/prometheus@v0.315.0")
    env = os.environ.copy()
    env.update(GOTOOLCHAIN="local", GO111MODULE="on", GOWORK="off", GOFLAGS="-mod=readonly", GOAMD64="v1",
               GOPROXY="off", GOSUMDB="off", GOCACHE=str(parent / "go-build"), GOMODCACHE=str(cache),
               TMPDIR=str(directory(private)), GOTMPDIR=str(private))
    for name in ("go.mod", "go.sum"):
        shutil.copyfile(HERE / "oracle" / name, task / name)
    profile = decode(invoke(["go", "env", "-json", "GOROOT", *PROFILE], task, env, "profile"))
    runtime = directory(profile.pop("GOROOT"))
    if profile != PROFILE:
        raise ValueError("annotation reference requires the pinned Go 1.27.1 / AMD64 profile")
    for base, label, sources in ((runtime, "go-source", GO_SOURCES | TIME_SOURCES),
                                 (prometheus, "prometheus-source", PROMETHEUS_SOURCES)):
        for relative, expected in sources.items():
            if digest(base / relative) != expected:
                raise ValueError(f"pinned original owner changed: {label}/{relative}")
            destination = task / label / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(base / relative, destination)
    shutil.copyfile(HERE / "capture/histogram_annotations.go", task / "histogram_annotations.go")
    for name in ("histogram_annotations_cases.py", "histogram_annotations.py"):
        shutil.copyfile(HERE / name, task / name)
    invoke(["go", "test", "-count=1", "-p", "4", "-v", "-run", "^TestAnnotations_AsStrings$",
            "github.com/prometheus/prometheus/util/annotations"], task, env, "original-tests")
    invoke(["go", "test", "-count=1", "-p", "4", "-v", "-run", "^TestRFC3339Conversion$", "time"],
           task, env, "time-tests")
    invoke(["go", "build", "-p", "4", "-o", task / "annotation-go", task / "histogram_annotations.go"],
           task, env, "build")
    requests = generate()
    (task / "requests.json").write_text(json.dumps(requests, indent=2)+"\n")
    raw = invoke([task / "annotation-go"], task, env, "source", (task / "requests.json").read_bytes())
    (task / "reference.json").write_bytes(raw)
    source = verify_facade(prometheus, parent / "capture-1usdtup9/reference")
    (task / "prometheus-source.sha256.json").write_text(json.dumps(source, indent=2, sort_keys=True)+"\n")
    manifest = {"schema_version": 1, "prometheus_commit": PROMETHEUS_COMMIT, "runtime_commit": COMMIT,
                "profile": profile, "case_count": len(requests),
                "original_test": "util/annotations/annotations_test.go:27-62",
                "go_sources": GO_SOURCES | TIME_SOURCES, "prometheus_sources": PROMETHEUS_SOURCES,
                "files": {name: digest(task / name) for name in sorted(REQUIRED)}}
    (task / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True)+"\n")
    load(task)
    print(json.dumps({"reference": str(task), "cases": len(requests),
                      "reference_sha256": digest(task / "reference.json")}), flush=True)
    return task


def load(task):
    task = directory(task)
    manifest = decode((task / "manifest.json").read_bytes())
    if (manifest["schema_version"] != 1 or manifest["prometheus_commit"] != PROMETHEUS_COMMIT
            or manifest["runtime_commit"] != COMMIT or manifest["profile"] != PROFILE
            or manifest["go_sources"] != GO_SOURCES | TIME_SOURCES
            or manifest["prometheus_sources"] != PROMETHEUS_SOURCES
            or set(manifest["files"]) != REQUIRED):
        raise ValueError("annotation source contract changed")
    for name, expected in manifest["files"].items():
        if digest(task / name) != expected:
            raise ValueError(f"source artifact changed: {name}")
    for label, sources in (("go-source", GO_SOURCES | TIME_SOURCES),
                           ("prometheus-source", PROMETHEUS_SOURCES)):
        for name, expected in sources.items():
            if digest(task / label / name) != expected:
                raise ValueError(f"captured source owner changed: {label}/{name}")
    requests = decode((task / "requests.json").read_bytes())
    reference = decode((task / "reference.json").read_bytes())
    if (requests != generate() or manifest["case_count"] != len(requests)
            or reference["go_version"] != PROFILE["GOVERSION"]
            or reference["architecture"] != PROFILE["GOARCH"]
            or len(reference["results"]) != len(requests)):
        raise ValueError("incomplete annotation source reference")
    for request, result in zip(requests, reference["results"], strict=True):
        if set(result) != {"id", "notes"} or result["id"] != request["id"]:
            raise ValueError("annotation input/result identity changed")
        keys = []
        for note in result["notes"]:
            if (set(note) != {"raw", "message", "severity", "span", "occurrences"}
                    or note["severity"] not in ("info", "warning")
                    or type(note["occurrences"]) is not int or note["occurrences"] < 1):
                raise ValueError("malformed original annotation")
            raw = bytes.fromhex(note["raw"])
            bytes.fromhex(note["message"])
            keys.append(raw)
            if note["span"] is not None and (set(note["span"]) != {"start", "end"}
                    or any(type(n) is not int for n in note["span"].values())):
                raise ValueError("malformed original position")
        if keys != sorted(set(keys)):
            raise ValueError("source raw-key order or uniqueness changed")
    return requests, reference["results"]


def compare(reference, binary):
    requests, expected = load(reference)
    binary = Path(binary).expanduser().resolve(strict=True)
    task = directory(tempfile.mkdtemp(prefix="histogram-annotations-compare-", dir=directory(HERE / ".tmp")))
    payload = "".join(f"{r['id']}\ta\t0\t{json.dumps(r, separators=(',', ':')).encode().hex()}\n" for r in requests).encode()
    (task / "requests.txt").write_bytes(payload)
    process = subprocess.run([str(binary)], input=payload, capture_output=True, timeout=600, check=False)
    (task / "candidate.stdout").write_bytes(process.stdout)
    (task / "candidate.stderr").write_bytes(process.stderr)
    if process.returncode:
        raise RuntimeError(f"candidate failed ({process.returncode}); evidence: {task}")
    actual = [decode(line) for line in process.stdout.splitlines()]
    if len(actual) != len(expected):
        raise ValueError("incomplete candidate annotation results")
    differences = []
    for wanted, observed in zip(expected, actual, strict=True):
        observations = observed.pop("accounting")
        if (observations["result_current_after_drop"] != 0 or observations["result_peak"] < observations["result_held"]
                or observations["result_acquired"] != observations["result_released"]):
            raise ValueError("candidate annotation allocations are not paired")
        if observed != wanted:
            differences.append({"id": wanted["id"], "expected": wanted, "actual": observed})
    report = {"status": "PASS" if not differences else "FAIL", "cases": len(expected),
              "matching_cases": len(expected)-len(differences), "differences": differences,
              "reference_sha256": digest(Path(reference) / "reference.json"), "binary_sha256": digest(binary)}
    (task / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True)+"\n")
    print(json.dumps({key: value for key, value in report.items() if key != "differences"} | {"evidence": str(task)}))
    if differences:
        raise SystemExit(1)
    return task


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--reference")
    parser.add_argument("--binary", default=HERE / ".tmp/core-target/release/promql-frontend-fixture")
    args = parser.parse_args()
    if args.capture:
        capture()
    elif args.reference:
        compare(args.reference, args.binary)
    else:
        parser.error("select --capture or --reference")


if __name__ == "__main__":
    main()
