#!/usr/bin/env python3
"""Compare borrowed native histogram helpers with independently executed Go outcomes."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from build import directory
from build_frontend import build
from histogram_reference import compare_value, fields
from histogram_functions_reference import load, validate_declaration, validate_outcome


def compare_outcomes(cases, output):
    expected = {row["id"]: row for row in cases}
    if type(output) is not list or len(output) != len(expected):
        raise ValueError("incomplete native helper candidate output")
    failures, bit_differences, seen = [], [], set()
    for result in output:
        fields(result, ("id", "actual"))
        identity = result["id"]
        if type(identity) is not str or identity not in expected or identity in seen:
            raise ValueError("unknown or duplicate native helper candidate identity")
        seen.add(identity)
        source, actual = expected[identity], result["actual"]
        try:
            validate_outcome(source["request"], actual)
        except ValueError as error:
            failures.append({"id": identity, "path": "actual", "reason": str(error)})
            continue
        for key in ("value", "reciprocal"):
            compare_value(source["actual"][key], actual[key], identity, "actual." + key,
                          failures, bit_differences)
        for key in ("annotations", "negative_reciprocal_infinite", "positive_reciprocal_infinite"):
            if source["actual"][key] != actual[key]:
                failures.append({"id": identity, "path": "actual." + key, "reason": "changed discrete outcome"})
        try:
            validate_declaration(source | {"actual": actual})
        except ValueError as error:
            failures.append({"id": identity, "path": "declared", "reason": str(error)})
    return {"semantic_pass": not failures, "bitwise_pass": not failures and not bit_differences,
            "cases": len(expected), "failures": failures, "bit_differences": bit_differences}


def compare(capture, binary=None):
    cases, counts = load(capture)
    binary = Path(binary).resolve(strict=True) if binary else build().with_name("promql-kernel-fixture")
    parent = directory(Path(__file__).resolve().parent / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="histogram-functions-compare-", dir=parent))
    inputs = [{key: row[key] for key in ("id", "mode", "request")} | {"borrowed": True} for row in cases]
    payload = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in inputs)
    (task / "requests.jsonl").write_text(payload)
    result = subprocess.run([str(binary)], input=payload, text=True, capture_output=True, timeout=600, check=False)
    (task / "candidate.jsonl").write_text(result.stdout)
    (task / "candidate.stderr").write_text(result.stderr)
    if result.returncode:
        raise SystemExit(f"native helper candidate failed ({result.returncode}); evidence: {task}")
    report = compare_outcomes(cases, [json.loads(line) for line in result.stdout.splitlines()])
    report["source_counts"] = counts
    report["declared_bit_differences"] = []
    for row in cases:
        declaration = row["declared"]
        if declaration is not None and declaration["comparison"] in {"exact", "almost"}:
            compare_value(declaration["value"], row["actual"]["value"], row["id"], "declared.value", [],
                          report["declared_bit_differences"])
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in
                     {"failures", "bit_differences", "declared_bit_differences"}} |
                     {"bit_differences": len(report["bit_differences"]),
                      "declared_bit_differences": len(report["declared_bit_differences"]), "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, help="verified native histogram helper capture directory")
    parser.add_argument("--binary", help="existing standalone Rust kernel fixture binary")
    parser.add_argument("--strict-bits", action="store_true", help="also fail every reported candidate numerical bit difference")
    args = parser.parse_args()
    report = compare(args.capture, args.binary)
    raise SystemExit(not report["bitwise_pass" if args.strict_bits else "semantic_pass"])
