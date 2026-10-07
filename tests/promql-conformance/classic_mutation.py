#!/usr/bin/env python3
"""Qualify reference-based bucket mutations against actual original Go backing states."""
import argparse
import hashlib
import json
import math
import subprocess
import tempfile
from pathlib import Path

from build import directory
from classic import compare_outcomes
from classic_reference import load

HERE = Path(__file__).resolve().parent


def quantile_runs(original):
    cases = []
    for row in original:
        if row["mode"] != "classic-buckets":
            continue
        before = row["request"]["buckets"]
        run = None
        for index, (operation, outcome) in enumerate(zip(row["request"]["operations"], row["actual"]["steps"], strict=True)):
            if operation["kind"] != "quantile":
                run = None
            else:
                if run is None:
                    run = {"id": row["id"] + f":quantile-run:{index}", "mode": "classic-buckets",
                           "request": {"buckets": before, "operations": []}, "actual": {"steps": []},
                           "declared": row["declared"] if index == 0 else None, "source": row["source"]}
                    cases.append(run)
                run["request"]["operations"].append(operation)
                run["actual"]["steps"].append(outcome)
            # A fraction boundary supplies its captured original state, never a candidate state.
            before = outcome["input_after"]
    return cases


def qualify(reference, binary):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    original, counts = load(reference)
    cases = quantile_runs(original)
    task = directory(tempfile.mkdtemp(prefix="classic-mutation-compare-", dir=directory(HERE / ".tmp")))
    payload = [{"id": row["id"], "mode": "classic-mutating-replay", "borrowed": True,
                "request": {"buckets": row["request"]["buckets"],
                            "quantiles": [op["quantile"] for op in row["request"]["operations"]]}}
               for row in cases]
    raw = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in payload).encode()
    (task / "requests.jsonl").write_bytes(raw)
    binary_sha = hashlib.sha256(binary.read_bytes()).hexdigest()
    with (task / "candidate.jsonl").open("xb") as out, (task / "candidate.stderr").open("xb") as err:
        result = subprocess.run([str(binary)], input=raw, stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode or hashlib.sha256(binary.read_bytes()).hexdigest() != binary_sha:
        raise ValueError(f"mutation candidate failed or binary changed; evidence: {task}")
    responses = [json.loads(line) for line in (task / "candidate.jsonl").read_bytes().splitlines()]
    if [row["id"] for row in responses] != [row["id"] for row in cases]:
        raise ValueError("mutation candidate identity roster changed")
    observations = []
    for row, source in zip(responses, cases, strict=True):
        actual = row["actual"]
        n = len(source["request"]["buckets"] or [])
        q = len(source["request"]["operations"])
        for key in ["reads", "producers", "work", "workspace_peak"]:
            if type(actual[key]) is not int or actual[key] < 0:
                raise ValueError("invalid mutation resource observation")
        if (actual["producers"] > q * (n // 2)
                or actual["reads"] > q * (2 * n + math.ceil(math.log2(max(n, 1))) + 3)):
            raise ValueError("mutation producer or raw-read bound changed")
        observations.append({"id": row["id"], **{key: actual[key] for key in ["reads", "producers", "work", "workspace_peak"]}})
    report = compare_outcomes(cases, [{"id": row["id"], "actual": {"steps": row["actual"]["steps"]}}
                                    for row in responses])
    load(reference)
    report.update(source_counts=counts, steps=sum(len(row["request"]["operations"]) for row in cases),
                  binary_sha256=binary_sha,
                  reference_sha256=hashlib.sha256((reference / "observations.json").read_bytes()).hexdigest(),
                  observations=observations)
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ["cases", "steps", "semantic_pass", "bitwise_pass"]}
                     | {"failures": len(report["failures"]), "paired_bits": len(report["bit_differences"]),
                        "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", required=True)
    parser.add_argument("--binary", required=True)
    args = parser.parse_args()
    report = qualify(args.reference, args.binary)
    raise SystemExit(not report["bitwise_pass"])
