#!/usr/bin/env python3
"""Compare every captured regex case; unsupported required fields remain failures."""
import argparse
import base64
from collections import Counter
import json
from pathlib import Path
import subprocess

from build import directory
from parser import difference
from regex_reference import load

HERE = Path(__file__).resolve().parent


def qualify(capture):
    reference = directory(capture)
    cases = load(reference)
    requests = []
    for case in cases:
        request = {"mode": case["mode"], "flags": case.get("flags", 0)}
        for field in ("pattern", "subject"):
            request[field] = base64.b64decode(case.get(field, ""), validate=True).hex()
        for field in ("templates", "lookups"):
            request[field] = [base64.b64decode(value, validate=True).hex() for value in case.get(field, [])]
        requests.append(f'{case["id"]}\tr\t0\t' + json.dumps(request).encode().hex())
    result = subprocess.run([HERE / ".tmp/core-target/release/promql-frontend-fixture"],
                            input=("\n".join(requests)+"\n").encode(), capture_output=True, timeout=600)
    if result.returncode:
        raise RuntimeError(f"regex candidate failed ({result.returncode}): {result.stderr.decode(errors='replace')}")
    actual = {}
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if item["id"] in actual:
            raise ValueError("duplicate regex candidate identity")
        actual[item["id"]] = item["expected"]
    if set(actual) != {case["id"] for case in cases}:
        raise ValueError("missing or extra regex candidate result")
    failures = []
    totals, passed, fields = Counter(), Counter(), Counter()
    for case in cases:
        candidate = actual[case["id"]]
        totals[case["mode"]] += 1
        for field, value in case["expected"].items():
            if field in candidate and difference(value, candidate[field]) is None:
                fields[f'{case["mode"]}:{field}:exact'] += 1
            else:
                fields[f'{case["mode"]}:{field}:failed'] += 1
        diff = difference(case["expected"], candidate)
        if diff:
            failures.append({"id": case["id"], "mode": case["mode"], "pattern": case["pattern"],
                             "subject": case.get("subject"), "difference": diff})
        else:
            passed[case["mode"]] += 1
    destination = HERE / ".tmp/regex-differences.json"
    destination.write_text(json.dumps(failures, indent=2)+"\n")
    print(json.dumps({"cases": len(cases), "exact_passed": len(cases)-len(failures), "failed": len(failures),
                      "mode_cases": totals, "mode_exact_passed": passed, "field_results": fields,
                      "first_failure_ids": [row["id"] for row in failures[:20]],
                      "failure_report": ".tmp/regex-differences.json",
                      "status": "Finite regex corpus qualification only; complete core gates remain required."}))
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--capture", required=True, help="independent regex_capture.py output")
    qualify(cli.parse_args().capture)
