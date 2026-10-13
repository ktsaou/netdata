#!/usr/bin/env python3
"""Replay frozen original API primitive outcomes through the actual typed C facade."""
import argparse
import base64
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def hex_bytes(value):
    return base64.b64decode(value, validate=True).hex()


def requests(original):
    result = []
    expected = []
    for case in original["helpers"]:
        if case["op"] == "time_param":
            continue
        request = {"id": case["id"], "op": case["op"]}
        if case["op"] in ("time", "time_boundary", "duration", "limit"):
            request["input"] = hex_bytes(case.get("resolved_input", case["input"]))
        elif case["op"] in ("sample_float", "scalar_float"):
            request["bits"] = case["bits"]
        elif case["op"] in ("sample_timestamp", "scalar_timestamp"):
            request["time_ms"] = case["time_ms"]
        else:
            raise ValueError("unknown original helper operation")
        result.append(request)
        expected.append(case)
    labelsets = [{"id": value["ID"], "labels": [
        {key: hex_bytes(label[key]) for key in ("name", "value")}
        for label in value["Labels"]]} for value in original["selector_labelsets"]]
    for case in original["selectors"]:
        if case["op"] == "batch":
            continue
        if case["op"] != "single" or len(case["inputs"]) != 1:
            raise ValueError("unknown original selector operation")
        result.append({"id": case["id"], "op": "selector", "input": hex_bytes(case["inputs"][0]),
                       "labelsets": labelsets})
        expected.append(case)
    if len({value["id"] for value in result}) != len(result):
        raise ValueError("duplicate original request identity")
    return result, expected


def projections(want, got):
    if want["op"] == "single":
        expected = {"success": want["parse_success"]}
        actual = {"success": got["success"]}
        if expected["success"] and actual["success"]:
            expected.update(matchers=[{"kind": item["type"], "name": hex_bytes(item["name"]),
                                       "value": hex_bytes(item["value"])} for item in want["matchers"]],
                            nonempty=want["nonempty_per_set"][0],
                            labelset_matches=want["labelset_matches"][0]["probes"])
            actual.update({key: got[key] for key in ("matchers", "nonempty", "labelset_matches")})
        elif not expected["success"] and not actual["success"]:
            expected["errors"] = [{"message": hex_bytes(item["message"]), "start": item["start"],
                                    "end": item["end"]} for item in want["parse_error"]["parse_errors"]]
            actual["errors"] = [{key: item[key] for key in ("message", "start", "end")}
                                for item in got["errors"]]
        return expected, actual
    expected = {"success": want["success"]}
    actual = {"success": got["success"]}
    if expected["success"] != actual["success"]:
        return expected, actual
    if not expected["success"]:
        expected["error_message"] = hex_bytes(want["error_message"])
        actual["error_message"] = got["error_message"]
    elif "timestamp" in want:
        expected["timestamp"] = {key: value for key, value in want["timestamp"].items()
                                  if key != "rfc3339_nano"}
        actual["timestamp"] = got["timestamp"]
    elif "duration_ns" in want:
        expected["duration_ns"] = want["duration_ns"]
        actual["duration_ns"] = got["duration_ns"]
    elif "limit" in want:
        expected["limit"] = want["limit"]
        actual["limit"] = got["limit"]
    else:
        expected["raw_json"] = hex_bytes(want["raw_json"])
        actual["raw_json"] = got["raw_json"]
    return expected, actual


def compare(binary, reference):
    binary, reference = binary.resolve(strict=True), reference.resolve(strict=True)
    original = json.loads(reference.read_bytes())
    if (original["schema_version"] != 1 or original["reference"]["commit"] !=
            "5241a27fe3c6983549fccc32f6e65917408c63cd"):
        raise ValueError("original reference identity changed")
    inputs, expected = requests(original)
    parent = HERE / ".tmp"
    parent.mkdir(exist_ok=True)
    parent = parent.resolve(strict=True)
    if not parent.is_dir() or not parent.is_relative_to(HERE.resolve()):
        raise ValueError("invalid project scratch directory")
    task = Path(tempfile.mkdtemp(prefix="protocol-primitives-", dir=parent))
    raw = b"".join(json.dumps(value).encode() + b"\n" for value in inputs)
    (task / "candidate-input.jsonl").write_bytes(raw)
    sources = {path.relative_to(HERE.parent.parent).as_posix(): digest(path)
               for path in sorted((HERE.parent.parent / "src").rglob("*.rs"))}
    identity = digest(binary)
    process = subprocess.run([str(binary)], input=raw, capture_output=True)
    (task / "candidate-output.jsonl").write_bytes(process.stdout)
    (task / "candidate.stderr").write_bytes(process.stderr)
    if process.returncode:
        raise ValueError(f"protocol fixture exited {process.returncode}; see {task}")
    actual = [json.loads(value) for value in process.stdout.splitlines()]
    if len(actual) != len(expected):
        raise ValueError("incomplete candidate output")
    failures = []
    for want, got in zip(expected, actual):
        if want["id"] != got["id"]:
            raise ValueError("candidate request identity changed")
        left, right = projections(want, got)
        if left != right:
            failures.append({"id": want["id"], "original": left, "candidate": right})
    if digest(binary) != identity or sources != {
        path.relative_to(HERE.parent.parent).as_posix(): digest(path)
        for path in sorted((HERE.parent.parent / "src").rglob("*.rs"))
    }:
        raise ValueError("candidate source or binary changed during replay")
    result = {"schema_version": 1, "cases": len(actual), "failures": failures,
              "selector_labelset_comparisons": sum(len(value.get("labelset_matches", [])) for value in actual),
              "source_sha256": sources, "binary_sha256": identity, "reference_sha256": digest(reference),
              "scope": "Typed UTC/clock projections, exact durations/limits/formatter bytes, selector matcher order/errors/spans/nonempty guard and complete label conjunctions.",
              "caller_cases_not_executed": {
                  "time_param": sum(value["op"] == "time_param" for value in original["helpers"]),
                  "selector_batches": sum(value["op"] == "batch" for value in original["selectors"])},
              "http_router_executed": False}
    (task / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
    return task, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--reference", type=Path, default=HERE / "protocol-original.json")
    args = parser.parse_args()
    task, result = compare(args.binary, args.reference)
    print(json.dumps({"receipt": str(task / "comparison.json"), "cases": result["cases"],
                      "labelset_comparisons": result["selector_labelset_comparisons"],
                      "failures": result["failures"]}, indent=2))
    return bool(result["failures"])


if __name__ == "__main__":
    raise SystemExit(main())
