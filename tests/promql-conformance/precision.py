#!/usr/bin/env python3
"""Replay precise engine requests against frozen original Go outcomes."""
import argparse
import json
import subprocess
import tempfile
from copy import deepcopy
from pathlib import Path

from protocol_primitives import digest
from query import result_base64

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def source_identity():
    return {path.relative_to(ROOT).as_posix(): digest(path)
            for path in sorted((ROOT / "src").rglob("*.rs"))}


def compare(binary, reference):
    binary, reference = binary.resolve(strict=True), reference.resolve(strict=True)
    original = json.loads(reference.read_bytes())
    if (original["schema_version"] != 1 or original["reference"]["commit"] !=
            "5241a27fe3c6983549fccc32f6e65917408c63cd"):
        raise ValueError("original reference identity changed")
    cases = original["cases"]
    if len(cases) != 9 or len({case["request"]["id"] for case in cases}) != len(cases):
        raise ValueError("original precision roster changed")
    parent = HERE / ".tmp"
    parent.mkdir(exist_ok=True)
    parent = parent.resolve(strict=True)
    if not parent.is_dir() or not parent.is_relative_to(HERE.resolve()):
        raise ValueError("invalid project scratch directory")
    task = Path(tempfile.mkdtemp(prefix="precision-", dir=parent))
    sources, identity = source_identity(), digest(binary)
    failures = []
    for page_size in (1, 17, 1024):
        packets = []
        for case in cases:
            dataset = deepcopy(case["dataset"])
            dataset["page_size"] = page_size
            packets.extend((dataset, case["request"]))
        raw = b"".join(json.dumps(value).encode() + b"\n" for value in packets)
        (task / f"page-{page_size}.input.jsonl").write_bytes(raw)
        process = subprocess.run([str(binary)], input=raw, capture_output=True)
        (task / f"page-{page_size}.output.jsonl").write_bytes(process.stdout)
        (task / f"page-{page_size}.stderr").write_bytes(process.stderr)
        if process.returncode:
            raise ValueError(f"query fixture exited {process.returncode}; see {task}")
        actual = [json.loads(value) for value in process.stdout.splitlines()]
        if len(actual) != len(packets):
            raise ValueError("incomplete candidate precision output")
        for case, acknowledgement, got in zip(cases, actual[::2], actual[1::2]):
            ident = case["request"]["id"]
            if acknowledgement.get("dataset_id") != ident or got.get("id") != ident:
                raise ValueError("candidate precision request identity changed")
            result = result_base64(got["result"])
            if result != case["expected"]:
                failures.append({"id": ident, "page_size": page_size,
                                 "original": case["expected"], "candidate": result})
    if digest(binary) != identity or source_identity() != sources:
        raise ValueError("candidate source or binary changed during precision replay")
    result = {"schema_version": 1, "cases": len(cases), "comparisons": len(cases) * 3,
              "page_sizes": [1, 17, 1024], "failures": failures,
              "source_sha256": sources, "binary_sha256": identity,
              "reference_sha256": digest(reference), "reference": original["reference"],
              "scope": original["scope"], "http_router_executed": False}
    (task / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
    return task, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--reference", type=Path, default=HERE / "precision-original.json")
    args = parser.parse_args()
    task, result = compare(args.binary, args.reference)
    print(json.dumps({"receipt": str(task / "comparison.json"),
                      "comparisons": result["comparisons"], "failures": result["failures"]}, indent=2))
    return bool(result["failures"])


if __name__ == "__main__":
    raise SystemExit(main())
