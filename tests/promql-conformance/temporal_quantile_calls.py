#!/usr/bin/env python3
"""Qualify temporal quantiles against complete source-bound original expressions."""

import argparse
import json
import subprocess
import tempfile
from pathlib import Path

import cursor_calls
from build import directory
from query import checked_candidate_ids, qualify_outputs
from temporal_quantile_cases import cases


def capture(upstream):
    return cursor_calls.capture(upstream, cases(), subject="temporal-quantile", construction_errors=True)


def load_reference(reference):
    return cursor_calls.load_reference(reference, cases(), construction_errors=True)


def qualify(reference, binary, page_size=1024):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be positive")
    inputs, original = load_reference(reference)
    groups, rows = cursor_calls.candidate_packets(inputs, page_size)
    task = directory(tempfile.mkdtemp(prefix="temporal-quantile-compare-",
                                     dir=directory(cursor_calls.HERE / ".tmp")))
    before = cursor_calls.sha(binary)
    (task / "requests.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
    with ((task / "requests.jsonl").open("rb") as requests,
          (task / "candidate.jsonl").open("xb") as output,
          (task / "candidate.stderr").open("xb") as errors):
        process = subprocess.run([str(binary)], stdin=requests, stdout=output, stderr=errors,
                                 timeout=600, check=False)
    if process.returncode or cursor_calls.sha(binary) != before:
        raise ValueError(f"temporal candidate failed or changed; evidence: {task}")
    responses = [json.loads(line) for line in (task / "candidate.jsonl").read_text().splitlines()]
    packets = checked_candidate_ids(responses, groups, "temporal quantile")
    entries = [{"request": row["request"], "result": result,
                "source_command": {"ordered": True}, "exact_numeric": True}
               for row, result in zip(inputs, original["results"], strict=True)]
    report, _ = qualify_outputs(entries, packets)
    report.update(page_size=page_size, binary_sha256=before,
                  reference_sha256=cursor_calls.sha(reference / "original.json"),
                  input_sha256=cursor_calls.sha(task / "requests.jsonl"),
                  output_sha256=cursor_calls.sha(task / "candidate.jsonl"))
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("required", "completed", "passed", "failed",
                                                  "semantic_pass", "bitwise_pass")}
                     | {"open": len(report["open"]),
                        "paired_bits": len(report["bitwise_differences"]),
                        "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    arguments = parser.parse_args()
    if arguments.capture:
        if not arguments.upstream:
            parser.error("capture requires --upstream")
        capture(arguments.upstream)
    else:
        if not arguments.reference or not arguments.binary:
            parser.error("comparison requires reference and binary")
        raise SystemExit(not qualify(arguments.reference, arguments.binary, arguments.page_size)["bitwise_pass"])
