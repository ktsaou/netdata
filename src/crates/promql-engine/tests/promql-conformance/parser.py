#!/usr/bin/env python3
"""Execute every captured expression case and compare full ASTs and ordered errors."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess

from build import directory
from capture import validate_frontend

HERE = Path(__file__).resolve().parent


def difference(expected, actual):
    pending = [("$", expected, actual)]
    while pending:
        path, left, right = pending.pop()
        if type(left) is not type(right):
            return {"path": path, "expected": left, "actual": right}
        if isinstance(left, dict):
            if left.keys() != right.keys():
                return {"path": path, "expected_keys": sorted(left), "actual_keys": sorted(right)}
            pending.extend((f"{path}.{key}", left[key], right[key]) for key in reversed(list(left)))
        elif isinstance(left, list):
            if len(left) != len(right):
                return {"path": path, "expected_length": len(left), "actual_length": len(right)}
            pending.extend((f"{path}[{index}]", left[index], right[index]) for index in range(len(left)-1, -1, -1))
        elif left != right:
            return {"path": path, "expected": left, "actual": right}
    return None


def qualify(capture_directory):
    capture_directory = directory(capture_directory)
    manifest = json.loads((capture_directory / "manifest.json").read_text())
    raw = (capture_directory / "frontend.json").read_bytes()
    if hashlib.sha256(raw).hexdigest() != manifest["outputs"]["frontend"]["sha256"]:
        raise ValueError("captured frontend digest changed")
    frontend = json.loads(raw)
    validate_frontend(frontend)
    options = frontend["parser_options"]
    flags = (int(options["EnableExperimentalFunctions"]) | int(options["EnableBinopFillModifiers"]) << 1
             | int(options["EnableExtendedRangeSelectors"]) << 2 | int(options["ExperimentalDurationExpr"]) << 3)
    requests = [f'{case["id"]}\tp\t{flags}\t' + base64.b64decode(case["input"], validate=True).hex()
                for case in frontend["expressions"]]
    candidate = HERE / ".tmp/core-target/release/promql-frontend-fixture"
    result = subprocess.run([candidate], input=("\n".join(requests)+"\n").encode(),
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=600)
    if result.returncode:
        raise RuntimeError(f"candidate failed ({result.returncode}): {result.stderr.decode(errors='replace')}")
    expected_ids = {case["id"] for case in frontend["expressions"]}
    actual = {}
    for line in result.stdout.splitlines():
        item = json.loads(line)
        ident = item["id"]
        if ident not in expected_ids or ident in actual:
            raise ValueError("extra or duplicate parser candidate case")
        actual[ident] = item
    if set(actual) != expected_ids:
        raise ValueError("missing parser candidate case")
    failures = []
    for case in frontend["expressions"]:
        expected = {"ast": case["ast"], "errors": case["errors"]}
        item = actual[case["id"]]
        diff = difference(expected, {key: item[key] for key in expected})
        if diff:
            failures.append({"id": case["id"], "input": case["input"], "difference": diff})
    report = {"parser_cases": len(expected_ids), "exact_passed": len(expected_ids)-len(failures),
              "failed": len(failures),
              "status": ("Captured expression table matches; complete regex/profile/evaluator gates remain required."
                         if not failures else "Incomplete: every captured AST/error mismatch remains required."),
              "failures": failures}
    private = HERE / ".tmp/parser-differences.json"
    private.write_text(json.dumps(report, indent=2)+"\n")
    print(json.dumps({key: value for key, value in report.items() if key != "failures"}
                     | {"failing_ids": [failure["id"] for failure in failures]}))
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--capture", required=True)
    qualify(cli.parse_args().capture)
