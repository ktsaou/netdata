#!/usr/bin/env python3
"""Qualify single-call, bounded classic count replay against original Go states."""
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
from classic_reference import load as load_classic
from classic_reference import validate_outcome
from classic_replay_cases import requests
from histogram_capture import COMMIT, MODULE
from histogram_functions_capture import digest
from histogram_reference import compare_value, fields, input_ledger
from numerical_reference import PROFILE

HERE = Path(__file__).resolve().parent


def capture(mod_cache):
    cache = directory(mod_cache)
    source = directory(cache / MODULE)
    manifest = verify_facade(source, source)
    if digest(manifest) != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original replay module pin changed")
    task = directory(tempfile.mkdtemp(prefix="classic-replay-capture-", dir=directory(HERE / ".tmp")))
    env = os.environ.copy()
    env.update(GOMODCACHE=str(cache), GOCACHE=str(HERE / ".tmp/go-build"), GOPROXY="off", GOSUMDB="off",
               GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="-mod=readonly", GOAMD64="v1")
    profile = json.loads(subprocess.check_output(["go", "env", "-json", *PROFILE], cwd=HERE / "oracle", env=env))
    if profile != PROFILE:
        raise ValueError("original replay runtime changed")
    shutil.copy2(HERE / "capture/classic_replay.go", task / "capture.go")
    inputs = requests()
    (task / "original-requests.jsonl").write_text("".join(json.dumps(row, separators=(",", ":")) + "\n" for row in inputs))
    commands = (("verify", ["go", "mod", "verify"]),
                ("build", ["go", "build", "-p", "4", "-o", str(task / "original-probe"), str(task / "capture.go")]))
    for label, command in commands:
        result = subprocess.run(command, cwd=HERE / "oracle", env=env, capture_output=True, timeout=600, check=False)
        (task / (label + ".stdout")).write_bytes(result.stdout)
        (task / (label + ".stderr")).write_bytes(result.stderr)
        if result.returncode:
            raise ValueError(f"original replay {label} failed ({result.returncode}); evidence: {task}")
    with ((task / "original-requests.jsonl").open("rb") as stdin,
          (task / "original-results.jsonl").open("xb") as stdout,
          (task / "original-results.stderr").open("xb") as stderr):
        result = subprocess.run([str(task / "original-probe")], stdin=stdin, stdout=stdout, stderr=stderr,
                                timeout=600, check=False)
    if result.returncode or verify_facade(source, source) != manifest:
        raise ValueError(f"original replay capture failed or source changed; evidence: {task}")
    names = ("capture.go", "original-probe", "original-requests.jsonl", "original-results.jsonl")
    metadata = {"commit": COMMIT, "profile": profile, "cases": len(inputs), "source_files_verified": len(manifest),
                "source_manifest_sha256": digest(manifest),
                "files": {name: hashlib.sha256((task / name).read_bytes()).hexdigest() for name in names}}
    (task / "capture-binding.json").write_text(json.dumps(metadata, indent=2) + "\n")
    (task / "source-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    load_supplement(task)
    print(json.dumps({"original_cases": len(inputs), "capture": str(task)}), flush=True)
    return task


def load_supplement(task):
    task = directory(task)
    metadata = json.loads((task / "capture-binding.json").read_text())
    if metadata["commit"] != COMMIT or metadata["profile"] != PROFILE or metadata["cases"] != 15000:
        raise ValueError("original replay supplement binding changed")
    manifest = json.loads((task / "source-manifest.json").read_text())
    if (metadata["source_manifest_sha256"] != input_ledger()["source_manifest_sha256"]
            or digest(manifest) != metadata["source_manifest_sha256"]
            or len(manifest) != metadata["source_files_verified"]
            or metadata["files"]["capture.go"] != hashlib.sha256((HERE / "capture/classic_replay.go").read_bytes()).hexdigest()):
        raise ValueError("original replay supplement source pin changed")
    for name, expected in metadata["files"].items():
        if Path(name).name != name or hashlib.sha256((task / name).read_bytes()).hexdigest() != expected:
            raise ValueError("original replay supplement file changed")
    inputs = [json.loads(line) for line in (task / "original-requests.jsonl").read_text().splitlines()]
    if inputs != requests():
        raise ValueError("independent replay supplement inputs changed")
    output = [json.loads(line) for line in (task / "original-results.jsonl").read_text().splitlines()]
    if [row["id"] for row in output] != [row["id"] for row in inputs]:
        raise ValueError("incomplete original replay supplement")
    cases = []
    for row, original in zip(output, inputs):
        expected = row["actual"]
        validate_outcome("classic-buckets", {"buckets": original["buckets"], "operations": [original["operation"]]},
                         {"steps": [expected]})
        cases.append({"id": row["id"], "request": {key: original[key] for key in ("buckets", "operation")},
                      "original_after": expected["input_after"],
                      "expected": {key: expected[key] for key in ("panicked", "result")}})
    return cases


def single_calls(cases):
    rows = []
    for case in cases:
        if case["mode"] != "classic-buckets":
            continue
        before = case["request"]["buckets"]
        for index, (operation, outcome) in enumerate(zip(case["request"]["operations"], case["actual"]["steps"])):
            rows.append({"id": case["id"] + f":single:{index}", "request": {"buckets": before, "operation": operation},
                         "original_after": outcome["input_after"],
                         "expected": {key: outcome[key] for key in ("panicked", "result")}})
            # Only original Go mutations can supply another single-call input.
            before = outcome["input_after"]
    return rows


def compare_outcomes(cases, output):
    expected = {row["id"]: row for row in cases}
    if type(output) is not list or len(output) != len(expected):
        raise ValueError("incomplete replay output")
    seen, failures, differences = set(), [], []
    for row in output:
        fields(row, ("id", "actual"))
        identity = row["id"]
        if type(identity) is not str or identity not in expected or identity in seen:
            raise ValueError("unknown or duplicate replay identity")
        seen.add(identity)
        actual, source = row["actual"], expected[identity]
        fields(actual, ("panicked", "result", "reads", "polls"))
        count = len(source["request"]["buckets"] or [])
        bound = 2 * count if source["request"]["operation"]["kind"] == "fraction" else count * (2 + (count - 1).bit_length())
        if (type(actual["panicked"]) is not bool or type(actual["reads"]) is not int or type(actual["polls"]) is not int
                or not 0 <= actual["reads"] <= bound or actual["polls"] < 0):
            failures.append({"id": identity, "path": "actual", "reason": "invalid shape or count-read bound"})
            continue
        try:
            validate_outcome("classic-buckets", {"buckets": source["request"]["buckets"], "operations": [source["request"]["operation"]]},
                             {"steps": [{"panicked": actual["panicked"], "result": actual["result"],
                                         "input_after": source["original_after"]}]})
        except ValueError as error:
            failures.append({"id": identity, "path": "actual", "reason": str(error)})
            continue
        go = source["expected"]
        if actual["panicked"] != go["panicked"] or (actual["result"] is None) != (go["result"] is None):
            failures.append({"id": identity, "path": "actual", "reason": "changed panic or result existence"})
            continue
        if go["result"] is None:
            continue
        for key in ("value", "reciprocal"):
            compare_value(go["result"][key], actual["result"][key], identity, "actual.result." + key, failures, differences)
        for key in ("quantile", "negative_reciprocal_infinite", "positive_reciprocal_infinite", "less_than_positive_infinity"):
            if go["result"][key] != actual["result"][key]:
                failures.append({"id": identity, "path": "actual.result." + key, "reason": "changed notes or composed outcome"})
    return {"cases": len(cases), "semantic_pass": not failures, "bitwise_pass": not failures and not differences,
            "failures": failures, "bit_differences": differences}


def compare(reference, supplement, binary):
    cases = single_calls(load_classic(reference)[0]) + load_supplement(supplement)
    binary = Path(binary).resolve(strict=True)
    task = directory(tempfile.mkdtemp(prefix="classic-replay-compare-", dir=directory(HERE / ".tmp")))
    payload = "".join(json.dumps({"id": row["id"], "mode": "classic-replay", "borrowed": True,
                                  "request": row["request"]}, separators=(",", ":")) + "\n" for row in cases)
    (task / "requests.jsonl").write_text(payload)
    result = subprocess.run([str(binary)], input=payload, text=True, capture_output=True, timeout=600, check=False)
    (task / "candidate.jsonl").write_text(result.stdout)
    (task / "candidate.stderr").write_text(result.stderr)
    if result.returncode:
        raise ValueError(f"replay candidate failed ({result.returncode}); evidence: {task}")
    report = compare_outcomes(cases, [json.loads(line) for line in result.stdout.splitlines()])
    report["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in ("failures", "bit_differences")}
                     | {"failures": len(report["failures"]), "bit_differences": len(report["bit_differences"]),
                        "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", required=True, help="unchanged original classic helper capture")
    parser.add_argument("--supplement", help="existing original operand capture")
    parser.add_argument("--capture", action="store_true", help="capture original operand inputs first")
    parser.add_argument("--mod-cache", help="existing pinned Go module cache for capture")
    parser.add_argument("--binary", required=True, help="existing standalone Rust kernel fixture")
    args = parser.parse_args()
    if args.capture == bool(args.supplement) or (args.capture and not args.mod_cache):
        parser.error("choose --supplement DIR or --capture --mod-cache DIR")
    supplement = capture(args.mod_cache) if args.capture else args.supplement
    raise SystemExit(not compare(args.reference, supplement, args.binary)["bitwise_pass"])
