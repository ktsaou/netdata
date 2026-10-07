#!/usr/bin/env python3
"""Qualify exact metadata permutations and comparator calls against original Go."""
import argparse
import json
import os
import re
import shutil
import tempfile
from pathlib import Path

from build import directory
from float_literals import COMMIT, decode, digest, invoke
from go_sort_cases import generate
from numerical_reference import PROFILE

HERE = Path(__file__).resolve().parent
SOURCES = {
    "src/slices/sort.go": "e6cd273b637e7dd2f547596c22e6236b92ae9b376f31ceb7b3c3269dd60a38df",
    "src/slices/zsortanyfunc.go": "50398783e085ac9cc1d7c6249ebe75a5cc4425dcdc6a4cc448ea6e7179126492",
    "src/slices/sort_test.go": "6f2fbfa62503650b6e76b380068587d581bfa0aa21335aa425fa21677fe02207",
    "src/math/bits/bits.go": "694e0114bda396589e8c993a3d7880a0b461da23ee64c272929a3a5b69c2acf2",
    "VERSION": "25eb74b09036e1ad894fb63b86cb0f0ee53342df8d2e2e860da4ff5079607758",
    "LICENSE": "911f8f5782931320f5b8d1160a76365b83aea6447ee6c04fa6d5591467db9dad",
}
FILES = {"go_sort.go", "sort-go", "requests.json", "reference.jsonl",
         "original-tests.stdout", "original-tests.stderr"}


def capture():
    parent = directory(HERE / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="go-sort-capture-", dir=parent))
    private = task / "tmp"
    private.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(GOTOOLCHAIN="local", GO111MODULE="off", GOWORK="off", GOFLAGS="", GOAMD64="v1",
               GOPROXY="off", GOSUMDB="off", GOCACHE=str(parent / "go-build"),
               TMPDIR=str(directory(private)), GOTMPDIR=str(private))
    profile = decode(invoke(["go", "env", "-json", "GOROOT", *PROFILE], task, env, "profile"))
    runtime = directory(profile.pop("GOROOT"))
    if profile != PROFILE:
        raise ValueError("sort reference requires the pinned Go 1.27.1 / AMD64 profile")
    for relative, expected in SOURCES.items():
        if digest(runtime / relative) != expected:
            raise ValueError(f"pinned original owner changed: {relative}")
        destination = task / "go-source" / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(runtime / relative, destination)
    shutil.copyfile(HERE / "capture/go_sort.go", task / "go_sort.go")
    invoke(["go", "test", "-count=1", "-p", "4", "-timeout=10m", "slices"], task, env, "original-tests")
    invoke(["go", "build", "-p", "4", "-o", task / "sort-go", task / "go_sort.go"], task, env, "build")
    requests, counts = generate()
    payload = json.dumps(requests, separators=(",", ":")).encode() + b"\n"
    (task / "requests.json").write_bytes(payload)
    (task / "reference.jsonl").write_bytes(invoke([task / "sort-go"], task, env, "reference", payload))
    if any(digest(runtime / relative) != expected for relative, expected in SOURCES.items()):
        raise ValueError("original sort owners changed during capture")
    manifest = {"schema_version": 1, "runtime_commit": COMMIT, "profile": profile,
                "sources": SOURCES, "counts": counts, "required": len(requests),
                "generator_sha256": digest(HERE / "go_sort_cases.py"),
                "files": {name: digest(task / name) for name in sorted(FILES)}}
    (task / "manifest.json").write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n")
    load(task)
    print(json.dumps({"capture": str(task), "required": len(requests), "counts": counts}), flush=True)
    return task


def outcomes(raw, requests):
    results = [decode(line) for line in raw.splitlines()]
    if len(results) != len(requests):
        raise ValueError("incomplete sort outcomes")
    for case, request in zip(results, requests, strict=True):
        if set(case) != {"id", "actual"} or case["id"] != request["id"]:
            raise ValueError("unbound sort identity")
        result = case["actual"]
        if (set(result) != {"items", "comparisons", "trace"} or type(result["items"]) is not list
                or type(result["comparisons"]) is not int or result["comparisons"] < 0
                or not re.fullmatch(r"[0-9a-f]{16}", result["trace"])):
            raise ValueError("malformed sort outcome")
        bounds, items = request["bounds"], result["items"]
        if len(items) != len(bounds):
            raise ValueError("changed sort item count")
        seen = set()
        for item in items:
            ordinal = item["ordinal"]
            if (set(item) != {"bound", "ordinal"} or type(ordinal) is not int
                    or not 0 <= ordinal < len(bounds) or ordinal in seen
                    or item["bound"] != bounds[ordinal]):
                raise ValueError("sort changed a bound/ordinal or duplicated an item")
            seen.add(ordinal)
    return results


def load(task):
    task = directory(task)
    manifest = decode((task / "manifest.json").read_bytes())
    if (manifest["schema_version"] != 1 or manifest["runtime_commit"] != COMMIT
            or manifest["profile"] != PROFILE or manifest["sources"] != SOURCES
            or manifest["generator_sha256"] != digest(HERE / "go_sort_cases.py")
            or set(manifest["files"]) != FILES):
        raise ValueError("sort source contract changed")
    if (any(digest(task / name) != expected for name, expected in manifest["files"].items())
            or any(digest(task / "go-source" / name) != expected for name, expected in SOURCES.items())
            or digest(task / "go_sort.go") != digest(HERE / "capture/go_sort.go")):
        raise ValueError("sort source capture changed")
    requests, counts = generate()
    if (decode((task / "requests.json").read_bytes()) != requests or manifest["counts"] != counts
            or manifest["required"] != len(requests)):
        raise ValueError("sort input identities changed")
    return requests, outcomes((task / "reference.jsonl").read_bytes(), requests)


def compare_outcomes(reference, actual):
    if [case["id"] for case in reference] != [case["id"] for case in actual]:
        raise ValueError("candidate sort identities changed")
    return [{"id": expected["id"], "path": "actual." + key}
            for expected, candidate in zip(reference, actual, strict=True)
            for key in ("items", "comparisons", "trace") if expected["actual"][key] != candidate["actual"][key]]


def compare(task, binary):
    requests, reference = load(task)
    binary = Path(binary).expanduser().resolve(strict=True)
    binary_hash = digest(binary)
    output = directory(tempfile.mkdtemp(prefix="go-sort-compare-", dir=directory(HERE / ".tmp")))
    payload = b"".join(json.dumps({"id": case["id"], "mode": "sort-metadata", "borrowed": True,
                                 "request": {"bounds": case["bounds"]}},
                                separators=(",", ":")).encode() + b"\n" for case in requests)
    (output / "requests.jsonl").write_bytes(payload)
    actual = outcomes(invoke([binary], output, os.environ.copy(), "candidate", payload), requests)
    if digest(binary) != binary_hash:
        raise ValueError("sort candidate changed during comparison")
    failures = compare_outcomes(reference, actual)
    report = {"required": len(requests), "pass": not failures, "failures": failures,
              "binary_sha256": binary_hash, "reference_sha256": digest(Path(task) / "reference.jsonl")}
    (output / "report.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key != "failures"} |
                     {"failures": len(failures), "evidence": str(output)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", help="reuse a pinned original capture; omit to capture original Go")
    parser.add_argument("--binary", help="existing Rust kernel fixture; omit to capture only")
    args = parser.parse_args()
    captured = args.capture or capture()
    if args.binary:
        raise SystemExit(not compare(captured, args.binary)["pass"])
