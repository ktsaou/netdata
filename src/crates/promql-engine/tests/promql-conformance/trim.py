#!/usr/bin/env python3
"""Compare complete Go fraction/trim outcomes with the owned Rust kernels."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from build import directory
from build_frontend import build
from histogram import compare_results
from histogram_reference import compare_value
from trim_reference import load, validate_outcome


def compare(capture, binary=None):
    cases, counts = load(capture)
    binary = Path(binary).resolve(strict=True) if binary else build().with_name("promql-kernel-fixture")
    inputs = [{key: row[key] for key in ("id", "mode", "request")} | {"borrowed": False} for row in cases]
    expected = {row["id"]: row["actual"] for row in cases}
    parent = directory(Path(__file__).resolve().parent / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="trim-compare-", dir=parent))
    payload = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in inputs)
    (task / "requests.jsonl").write_text(payload)
    result = subprocess.run([str(binary)], input=payload, text=True, capture_output=True, timeout=600, check=False)
    (task / "candidate.jsonl").write_text(result.stdout)
    (task / "candidate.stderr").write_text(result.stderr)
    if result.returncode:
        raise SystemExit(f"fraction/trim candidate failed ({result.returncode}); evidence: {task}")
    output = [json.loads(line) for line in result.stdout.splitlines()]
    report = compare_results(expected, output)
    by_id = {row["id"]: row for row in cases}
    for row in output:
        source = by_id[row["id"]]
        if source["mode"] == "float-trim":
            try:
                validate_outcome(source["request"], row["actual"])
            except ValueError as failure:
                report["failures"].append({"id": row["id"], "path": "actual", "reason": str(failure)})
        elif source["mode"] == "float-trim-equality":
            try:
                validate_outcome(source["request"]["trim"], row["actual"]["trim"])
                if row["actual"]["peer_after"] != source["request"]["peer"]:
                    raise ValueError("trim equality consumer mutated its peer")
            except ValueError as failure:
                report["failures"].append({"id": row["id"], "path": "actual", "reason": str(failure)})
    report["semantic_pass"] = not report["failures"]
    report["bitwise_pass"] = not report["failures"] and not report["bit_differences"]
    report["source_counts"] = counts
    report["declared_bit_differences"] = []
    for row in cases:
        declared = row["declared"]
        if declared is not None and declared["comparison"] != "lifecycle":
            actual = row["actual"] if row["mode"] == "bucket-fraction" else row["actual"]["receiver_after"]["count"]
            compare_value(declared["value"], actual, row["id"], "declared", [], report["declared_bit_differences"])
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in
                     {"failures", "bit_differences", "declared_bit_differences"}} |
                     {"bit_differences": len(report["bit_differences"]),
                      "declared_bit_differences": len(report["declared_bit_differences"]), "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, help="verified fraction/trim capture directory")
    parser.add_argument("--binary", help="existing standalone Rust kernel fixture binary")
    parser.add_argument("--strict-bits", action="store_true", help="also fail every candidate numerical bit difference")
    args = parser.parse_args()
    result = compare(args.capture, args.binary)
    raise SystemExit(not result["bitwise_pass" if args.strict_bits else "semantic_pass"])
