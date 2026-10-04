#!/usr/bin/env python3
"""Compare in-place classic kernels with independent Go helper sequences."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from build import directory
from build_frontend import build
from classic_reference import load, validate_declaration, validate_outcome
from histogram_reference import compare_value, fields


def compare_outcomes(cases, output):
    expected = {row["id"]: row for row in cases}
    if type(output) is not list or len(output) != len(expected):
        raise ValueError("incomplete classic candidate output")
    failures, differences, seen = [], [], set()
    for row in output:
        fields(row, ("id", "actual"))
        identity = row["id"]
        if type(identity) is not str or identity not in expected or identity in seen:
            raise ValueError("unknown or duplicate classic candidate identity")
        seen.add(identity)
        source, actual = expected[identity], row["actual"]
        try:
            validate_outcome(source["mode"], source["request"], actual)
            validate_declaration(source | {"actual": actual})
        except ValueError as error:
            failures.append({"id": identity, "path": "actual", "reason": str(error)})
            continue
        if source["mode"] == "almost-equal":
            if actual != source["actual"]:
                failures.append({"id": identity, "path": "actual", "reason": "changed almost.Equal outcome"})
            continue
        for index, (go, candidate) in enumerate(zip(source["actual"]["steps"], actual["steps"])):
            path = f"actual.steps[{index}]"
            for key in ("panicked", "input_after"):
                if go[key] != candidate[key]:
                    failures.append({"id": identity, "path": path + "." + key, "reason": "changed exact backing mutation or panic"})
            if go["result"] is None or candidate["result"] is None:
                if go["result"] != candidate["result"]:
                    failures.append({"id": identity, "path": path + ".result", "reason": "changed result existence"})
                continue
            for key in ("value", "reciprocal"):
                compare_value(go["result"][key], candidate["result"][key], identity, path + ".result." + key, failures, differences)
            for key in ("quantile", "negative_reciprocal_infinite", "positive_reciprocal_infinite", "less_than_positive_infinity"):
                if go["result"][key] != candidate["result"][key]:
                    failures.append({"id": identity, "path": path + ".result." + key, "reason": "changed diagnostic or composed outcome"})
    return {"semantic_pass": not failures, "bitwise_pass": not failures and not differences,
            "cases": len(expected), "failures": failures, "bit_differences": differences}


def compare(capture, binary=None):
    cases, counts = load(capture)
    binary = Path(binary).resolve(strict=True) if binary else build().with_name("promql-kernel-fixture")
    parent = directory(Path(__file__).resolve().parent / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="classic-compare-", dir=parent))
    inputs = [{key: row[key] for key in ("id", "mode", "request")} | {"borrowed": True} for row in cases]
    payload = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in inputs)
    (task / "requests.jsonl").write_text(payload)
    result = subprocess.run([str(binary)], input=payload, text=True, capture_output=True, timeout=600, check=False)
    (task / "candidate.jsonl").write_text(result.stdout)
    (task / "candidate.stderr").write_text(result.stderr)
    if result.returncode:
        raise SystemExit(f"classic candidate failed ({result.returncode}); evidence: {task}")
    report = compare_outcomes(cases, [json.loads(line) for line in result.stdout.splitlines()])
    report["source_counts"], report["declared_bit_differences"] = counts, []
    for row in cases:
        if row["mode"] == "classic-buckets" and row["declared"] is not None:
            compare_value(row["declared"]["value"], row["actual"]["steps"][0]["result"]["value"], row["id"], "declared.value", [], report["declared_bit_differences"])
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in {"failures", "bit_differences", "declared_bit_differences"}} |
                     {"bit_differences": len(report["bit_differences"]), "declared_bit_differences": len(report["declared_bit_differences"]), "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, help="verified classic Go capture directory")
    parser.add_argument("--binary", help="existing standalone Rust kernel fixture binary")
    parser.add_argument("--strict-bits", action="store_true", help="fail every numeric bit difference")
    args = parser.parse_args()
    report = compare(args.capture, args.binary)
    raise SystemExit(not report["bitwise_pass" if args.strict_bits else "semantic_pass"])
