#!/usr/bin/env python3
"""Capture and compare complete original histogram-call results, including map aliases."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

from build import directory
from capture import verify_facade
from compare import checked_results, compare
from histogram_call_cases import cases
from histogram_capture import COMMIT
from histogram_reference import input_ledger
from query import dataset_hex, request_hex, result_base64

HERE = Path(__file__).resolve().parent
ORACLE_SHA256 = "28f31b32475f05ca6ff47e9dc79a65df4811155fec8d80c4eab8e266d76e7f62"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(upstream, oracle):
    source = directory(upstream)
    oracle = Path(oracle).resolve(strict=True)
    if sha(oracle) != ORACLE_SHA256:
        raise ValueError("canonical original query binary changed")
    manifest = verify_facade(source, source)
    digest = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if digest != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original histogram-call source changed")
    task = directory(tempfile.mkdtemp(prefix="histogram-call-capture-", dir=directory(HERE / ".tmp")))
    dataset, requests = cases()
    for name, value in [("dataset.json", dataset), ("requests.json", requests), ("source.json", manifest)]:
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    with (task / "original.json").open("xb") as out, (task / "original.stderr").open("xb") as err:
        result = subprocess.run([str(oracle), str(task / "dataset.json"), str(task / "requests.json")],
                                stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode or verify_facade(source, source) != manifest:
        raise ValueError(f"original histogram-call capture failed; evidence: {task}")
    expected = json.loads((task / "original.json").read_text())
    checked_results(expected)
    if expected["commit"] != COMMIT or [row["id"] for row in expected["results"]] != [row["id"] for row in requests]:
        raise ValueError("original histogram-call result roster changed")
    names = ("dataset.json", "requests.json", "source.json", "original.json")
    binding = {"commit": COMMIT, "oracle_sha256": sha(oracle), "cases": len(requests),
               "files": {name: sha(task / name) for name in names}, "source_manifest_sha256": digest}
    (task / "binding.json").write_text(json.dumps(binding, indent=2) + "\n")
    print(json.dumps({"original_cases": len(requests), "capture": str(task)}), flush=True)
    return task


def qualify(reference, binary, page_size):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    binding = json.loads((reference / "binding.json").read_text())
    if binding["commit"] != COMMIT or binding["oracle_sha256"] != ORACLE_SHA256:
        raise ValueError("original histogram-call binding changed")
    for name, digest in binding["files"].items():
        if Path(name).name != name or sha(reference / name) != digest:
            raise ValueError("original histogram-call artifact changed")
    dataset, requests = cases()
    if (json.loads((reference / "dataset.json").read_text()) != dataset
            or json.loads((reference / "requests.json").read_text()) != requests
            or binding["cases"] != len(requests)):
        raise ValueError("independent histogram-call inputs changed")
    manifest = json.loads((reference / "source.json").read_text())
    digest = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if digest != binding["source_manifest_sha256"] or digest != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original histogram-call source pin changed")
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be positive")
    expected = json.loads((reference / "original.json").read_text())
    checked_results(expected)
    task = directory(tempfile.mkdtemp(prefix="histogram-call-compare-", dir=directory(HERE / ".tmp")))
    rows = [{"op": "dataset", "dataset_id": "call", "page_size": page_size, "dataset": dataset_hex(dataset)}]
    rows.extend(request_hex(request, "call") for request in requests)
    (task / "requests.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
    with ((task / "requests.jsonl").open("rb") as inputs,
          (task / "candidate.jsonl").open("xb") as out, (task / "candidate.stderr").open("xb") as err):
        result = subprocess.run([str(binary)], stdin=inputs, stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode:
        raise ValueError(f"histogram-call candidate failed; evidence: {task}")
    responses = [json.loads(line) for line in (task / "candidate.jsonl").read_text().splitlines()]
    if responses[0] != {"dataset_id": "call"} or [row.get("id") for row in responses[1:]] != [row["id"] for row in requests]:
        raise ValueError("candidate histogram-call result roster changed")
    actual = {"schema_version": 1, "results": [result_base64(row["result"]) for row in responses[1:]]}
    checked_results(actual)
    report = compare(expected, actual, [{"id": row["id"], "ordered": True, "exact_numeric": False} for row in requests])
    report.update(cases=len(requests), binary_sha256=sha(binary), reference_sha256=sha(reference / "original.json"),
                  page_size=page_size)
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ["cases", "semantic_pass", "bitwise_pass"]}
                     | {"failures": len(report["semantic_failures"]),
                        "bit_differences": len(report["bitwise_differences"]), "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--oracle")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=17)
    args = parser.parse_args()
    if args.capture:
        if not args.upstream or not args.oracle:
            parser.error("capture requires --upstream and --oracle")
        capture(args.upstream, args.oracle)
    else:
        if not args.reference or not args.binary:
            parser.error("comparison requires --reference and --binary")
        result = qualify(args.reference, args.binary, args.page_size)
        raise SystemExit(not result["bitwise_pass"])
