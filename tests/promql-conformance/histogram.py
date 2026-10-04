#!/usr/bin/env python3
"""Compare owned/borrowed Rust kernels with independently captured pinned Go results."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from build import directory
from build_frontend import build
from histogram_reference import compare_declared, compare_value, fields, load

BORROWED = {"float-copy", "float-copy-to", "integer-copy", "integer-copy-to", "integer-to-float", "float-copy-to-schema",
            "float-iterator-set"}


def requests(data):
    inputs, expected = [], {}
    for case in data["cases"]:
        for borrowed in ([False, True] if case["mode"] in BORROWED else [False]):
            identity = case["id"] + (":borrowed" if borrowed else "")
            if identity in expected:
                raise ValueError("duplicate generated kernel identity")
            inputs.append({"id": identity, "mode": case["mode"], "request": case["request"], "borrowed": borrowed})
            expected[identity] = case["actual"]
    return inputs, expected


def compare_results(expected, output, exact_bits_ids=()):
    failures, bit_differences, seen = [], [], set()
    if type(output) is not list or len(output) != len(expected):
        raise ValueError("incomplete candidate kernel output")
    for result in output:
        fields(result, ("id", "actual"))
        identity = result["id"]
        if type(identity) is not str or identity not in expected or identity in seen:
            raise ValueError("unknown or duplicate candidate kernel identity")
        seen.add(identity)
        compare_value(expected[identity], result["actual"], identity, "actual", failures, bit_differences,
                      exact_bits=identity in exact_bits_ids)
    return {"semantic_pass": not failures, "bitwise_pass": not bit_differences and not failures,
            "cases": len(expected), "failures": failures, "bit_differences": bit_differences}


def compare(capture, binary=None):
    data, counts = load(capture)
    binary = Path(binary).resolve(strict=True) if binary else build().with_name("promql-kernel-fixture")
    inputs, expected = requests(data)
    parent = directory(Path(__file__).resolve().parent / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="histogram-compare-", dir=parent))
    payload = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in inputs)
    (task / "requests.jsonl").write_text(payload)
    result = subprocess.run([str(binary)], input=payload, text=True, capture_output=True, timeout=600, check=False)
    (task / "candidate.jsonl").write_text(result.stdout)
    (task / "candidate.stderr").write_text(result.stderr)
    if result.returncode:
        raise SystemExit(f"kernel candidate failed ({result.returncode}); evidence: {task}")
    exact_bits_ids = {row["id"] for row in inputs
                         if row["mode"] in {"exponential-bound", "exponential-bound-table"}}
    report = compare_results(expected, [json.loads(line) for line in result.stdout.splitlines()], exact_bits_ids)
    report["source_counts"] = counts
    report["declared_bit_differences"] = []
    declared_failures = []
    for case in data["cases"]:
        compare_declared(case, declared_failures, report["declared_bit_differences"])
    if declared_failures:
        raise ValueError("captured Go kernel differs from its original declared assertion")
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items()
                      if key not in {"failures", "bit_differences", "declared_bit_differences"}}
                     | {"bit_differences": len(report["bit_differences"]),
                        "declared_bit_differences": len(report["declared_bit_differences"]),
                        "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, help="verified histogram capture directory")
    parser.add_argument("--binary", help="existing standalone Rust kernel fixture binary")
    parser.add_argument("--strict-bits", action="store_true", help="also fail reported numeric bit differences")
    args = parser.parse_args()
    result = compare(args.capture, args.binary)
    raise SystemExit(not result["bitwise_pass" if args.strict_bits else "semantic_pass"])
