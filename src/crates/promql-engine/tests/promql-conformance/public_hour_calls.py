"""Generate and check the six public, exact hour-expression examples."""

import argparse
import gzip
import hashlib
import json
import sys
from pathlib import Path

import cursor_calls
import query

HERE = Path(__file__).resolve().parent
FIXTURE_SHA256 = "c1a870615bd5fe2daccebf5df87e267abc548dd0beec36865971691143a94108"


def cases():
    path = HERE / "public-hour-originals.json"
    raw = path.read_bytes()
    if path.is_symlink() or hashlib.sha256(raw).hexdigest() != FIXTURE_SHA256:
        raise ValueError("independently declared public hour originals changed")
    fixture = json.loads(raw)
    dataset = fixture["dataset"]
    inputs = [
        {"request": row["request"], "dataset": dataset, "faults": {}}
        for row in fixture["default_original"]["cases"]
    ]
    original = cursor_calls.checked_original(fixture["default_original"], inputs)
    entries = [
        {
            "request": source["request"],
            "result": result,
            "source_command": {"ordered": False},
            "exact_numeric": True,
        }
        for source, result in zip(inputs, original["results"], strict=True)
    ]
    for row in fixture["upstream_originals"]:
        inputs.append({"request": row["request"], "dataset": dataset, "faults": {}})
        entries.append(row | {"exact_numeric": True})
    return inputs, entries


def packets(page_size):
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be a positive integer")
    inputs, _ = cases()
    return cursor_calls.candidate_packets(inputs, page_size)


def qualify(responses):
    groups, _ = packets(17)
    query.checked_candidate_packets(responses, groups, "public hour")
    outputs = query.checked_candidate_ids(responses, groups, "public hour")
    _, entries = cases()
    report, _ = query.qualify_outputs(entries, outputs)
    passed = report["semantic_pass"] and report["bitwise_pass"]
    return {"status": "pass" if passed else "fail", **report}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--packets", action="store_true", help="write fixture input JSONL")
    action.add_argument("--results", type=Path, help="check complete fixture output JSONL or gzip")
    parser.add_argument("--page-size", type=int, default=17)
    args = parser.parse_args()
    try:
        if args.packets:
            _, payload = packets(args.page_size)
            for packet in payload:
                print(json.dumps(packet, separators=(",", ":")))
            return 0
        opener = gzip.open if args.results.suffix == ".gz" else open
        with opener(args.results, "rb") as stream:
            responses = [json.loads(line) for line in stream]
        report = qualify(responses)
        print(json.dumps(report, indent=2))
        return 0 if report["status"] == "pass" else 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(2, f"public hour check failed: {error}\n")


if __name__ == "__main__":
    sys.exit(main())
