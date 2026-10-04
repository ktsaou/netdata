#!/usr/bin/env python3
"""Compare Rust numerical primitives with independently captured public Go math."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from build import directory
from build_frontend import build
from histogram import compare_results
from histogram_reference import compare_value
from numerical_reference import load


def compare(capture, binary=None):
    data, counts = load(capture)
    binary = Path(binary).resolve(strict=True) if binary else build().with_name("promql-kernel-fixture")
    inputs = [{key: row[key] for key in ("id", "mode", "request")} | {"borrowed": False} for row in data["cases"]]
    expected = {row["id"]: row["actual"] for row in data["cases"]}
    parent = directory(Path(__file__).resolve().parent / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="numerical-compare-", dir=parent))
    payload = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in inputs)
    (task / "requests.jsonl").write_text(payload)
    result = subprocess.run([str(binary)], input=payload, text=True, capture_output=True, timeout=600, check=False)
    (task / "candidate.jsonl").write_text(result.stdout)
    (task / "candidate.stderr").write_text(result.stderr)
    if result.returncode:
        raise SystemExit(f"numerical candidate failed ({result.returncode}); evidence: {task}")
    exact = {row["id"] for row in inputs if row["mode"] == "math-frexp"
             or row["id"].startswith(("TestLog2:power:", "TestExp2:power:"))}
    report = compare_results(expected, [json.loads(line) for line in result.stdout.splitlines()], exact)
    report["source_counts"] = counts
    report["declared_bit_differences"] = []
    for row in data["cases"]:
        if row["declared"] is not None:
            # load() already enforces the exact original assertion's comparison rule.
            compare_value(row["declared"]["value"], row["actual"], row["id"], "declared", [],
                          report["declared_bit_differences"])
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in {"failures", "bit_differences", "declared_bit_differences"}}
                     | {"bit_differences": len(report["bit_differences"]),
                        "declared_bit_differences": len(report["declared_bit_differences"]), "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, help="verified Go numerical capture")
    parser.add_argument("--binary", help="existing standalone Rust kernel fixture")
    parser.add_argument("--strict-bits", action="store_true", help="also fail reported numeric bit differences")
    args = parser.parse_args()
    report = compare(args.capture, args.binary)
    raise SystemExit(not report["bitwise_pass" if args.strict_bits else "semantic_pass"])
