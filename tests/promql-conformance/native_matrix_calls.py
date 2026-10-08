#!/usr/bin/env python3
"""Capture and qualify matrix/native source contracts without normalizing results."""

import argparse
import json
import subprocess
import tempfile
import time
from copy import deepcopy
from pathlib import Path

import cursor_calls
from build import directory
from compare import checked_results
from native_matrix_cases import PANELS, cases
from query import checked_candidate_ids, qualify_outputs, result_base64


def _encoding(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def exact_trim_results():
    """Bind the four complete malformed outputs retained from the original source."""
    inputs = cases("native-checked")
    exact = {}
    for index in (34, 35, 40, 41):
        source = inputs[index]
        if source["request"]["query"] != ("h </ 1" if index % 2 == 0 else "h >/ 1"):
            raise ValueError("source trim control identity changed")
        record = source["dataset"]["series"][0]
        histogram = deepcopy(record["points"][0]["float_histogram"])
        histogram["positive_spans"] = [
            {"Offset": span["offset"], "Length": span["length"]}
            for span in histogram["positive_spans"]
        ]
        if index == 35:
            histogram.update(
                count="0000000000000000",
                sum="0000000000000000",
                positive_spans=[{"Offset": 1, "Length": 1}],
                positive_buckets=[],
            )
        elif index == 41:
            histogram.update(
                count="0000000000000000",
                sum="0000000000000000",
                positive_spans=None,
                positive_buckets=["4000000000000000"],
            )
        ident = source["request"]["id"]
        exact[ident] = {
            "id": ident,
            "kind": "vector",
            "rows": [
                {
                    "labels": deepcopy(record["labels"]),
                    "points": [
                        {
                            "time_ms": 2000,
                            "kind": "float_histogram",
                            "float_histogram": histogram,
                        }
                    ],
                }
            ],
            "warnings": [],
            "infos": [],
        }
    return exact


def result_validator(panel):
    inputs = cases(panel)
    identities = [source["request"]["id"] for source in inputs]
    exact = exact_trim_results() if panel == "native-checked" else {}

    def validate(document):
        if (
            type(document) is not dict
            or document.keys() != {"schema_version", "results"}
            or type(document["schema_version"]) is not int
            or document["schema_version"] != 1
            or type(document["results"]) is not list
            or len(document["results"]) != len(identities)
        ):
            raise ValueError("native matrix result document changed")
        for ident, result in zip(identities, document["results"], strict=True):
            if type(result) is not dict or result.get("id") != ident:
                raise ValueError("native matrix result roster changed")
            if ident in exact:
                if _encoding(result) != _encoding(exact[ident]):
                    raise ValueError("complete source trim output changed")
            else:
                checked_results({"schema_version": 1, "results": [result]})

    return validate


def recorded_run(command, **options):
    """Keep child identity and completion; a deadline never sends a signal."""
    options.pop("timeout", None)
    check = options.pop("check", False)
    output = Path(options["stdout"].name).resolve(strict=True)
    task = directory(output.parent)
    process_path = task / (output.name + ".process.json")
    status_path = task / (output.name + ".status.json")
    if process_path.exists() or status_path.exists():
        raise ValueError("native matrix process evidence already exists")
    started = time.time_ns()
    process = subprocess.Popen(command, **options)
    process_path.write_text(
        json.dumps(
            {
                "pid": process.pid,
                "command": list(map(str, command)),
                "cwd": str(Path(options.get("cwd", Path.cwd())).resolve(strict=True)),
                "started_unix_ns": started,
                "automatic_signal": False,
            }
        )
        + "\n"
    )
    out, err = process.communicate()
    status_path.write_text(
        json.dumps(
            {
                "pid": process.pid,
                "returncode": process.returncode,
                "elapsed_seconds": (time.time_ns() - started) / 1e9,
            }
        )
        + "\n"
    )
    returned = subprocess.CompletedProcess(command, process.returncode, out, err)
    if check:
        returned.check_returncode()
    return returned


def capture(upstream, panel):
    return cursor_calls.capture(
        upstream,
        cases(panel),
        subject="native-matrix-" + panel,
        construction_errors=True,
        result_validator=result_validator(panel),
        process_runner=recorded_run,
    )


def load_reference(reference, panel):
    return cursor_calls.load_reference(
        reference,
        cases(panel),
        construction_errors=True,
        result_validator=result_validator(panel),
    )


def qualify_outcomes(inputs, expected, outcomes, panel):
    """Compare complete payloads for source-only trims; preserve required OPEN paths."""
    exact = exact_trim_results() if panel == "native-checked" else {}
    if len(outcomes) != len(inputs) or len(expected["results"]) != len(inputs):
        raise ValueError("native matrix comparison roster changed")
    report = {
        "required": len(inputs),
        "completed": 0,
        "passed": 0,
        "failed": 0,
        "open": {},
        "dispositions": [],
        "semantic_failures": {},
        "bitwise_differences": [],
        "semantic_pass": True,
        "bitwise_pass": True,
    }
    for source, original, outcome in zip(
        inputs, expected["results"], outcomes, strict=True
    ):
        ident = source["request"]["id"]
        if outcome.get("id") != ident or original.get("id") != ident:
            raise ValueError("native matrix comparison identity changed")
        if ident in exact and _encoding(original) != _encoding(exact[ident]):
            raise ValueError("source trim expectation changed")
        if ident in exact and "unsupported" not in outcome:
            if set(outcome) not in ({"id", "result"}, {"id", "result", "observations"}):
                raise ValueError("incomplete source trim outcome")
            actual = result_base64(outcome["result"])
            passed = _encoding(actual) == _encoding(original)
            report["completed"] += 1
            report["passed" if passed else "failed"] += 1
            report["dispositions"].append(
                {"id": ident, "status": "pass" if passed else "fail"}
            )
            if not passed:
                report["semantic_failures"][ident] = {
                    "comparison": "complete source trim payload",
                    "expected": original,
                    "actual": actual,
                }
                report["semantic_pass"] = report["bitwise_pass"] = False
        else:
            entry = {
                "request": source["request"],
                "result": original,
                "source_command": {"ordered": True},
                "exact_numeric": True,
            }
            case, _ = qualify_outputs([entry], [outcome])
            for key in ("completed", "passed", "failed"):
                report[key] += case[key]
            for key in ("open", "semantic_failures"):
                report[key].update(case[key])
            for key in ("dispositions", "bitwise_differences"):
                report[key].extend(case[key])
            for key in ("semantic_pass", "bitwise_pass"):
                report[key] &= case[key]
    return report


def qualify(reference, binary, panel, page_size=1024):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    inputs, expected = load_reference(reference, panel)
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be positive")
    task = directory(
        tempfile.mkdtemp(
            prefix="native-matrix-" + panel + "-compare-",
            dir=directory(cursor_calls.HERE / ".tmp"),
        )
    )
    groups, packets = cursor_calls.candidate_packets(inputs, page_size)
    (task / "requests.jsonl").write_text(
        "".join(json.dumps(row) + "\n" for row in packets)
    )
    with (
        (task / "requests.jsonl").open("rb") as supplied,
        (task / "candidate.jsonl").open("xb") as out,
        (task / "candidate.stderr").open("xb") as err,
    ):
        process = recorded_run([str(binary)], stdin=supplied, stdout=out, stderr=err)
    if process.returncode:
        raise ValueError(f"native matrix candidate failed; evidence: {task}")
    responses = [
        json.loads(row) for row in (task / "candidate.jsonl").read_text().splitlines()
    ]
    outcomes = checked_candidate_ids(responses, groups, "native matrix")
    report = qualify_outcomes(inputs, expected, outcomes, panel)
    report.update(
        panel=panel,
        page_size=page_size,
        binary_sha256=cursor_calls.sha(binary),
        reference_sha256=cursor_calls.sha(reference / "original.json"),
        reference_binding_sha256=cursor_calls.sha(reference / "binding.json"),
        input_sha256=cursor_calls.sha(task / "requests.jsonl"),
        output_sha256=cursor_calls.sha(task / "candidate.jsonl"),
    )
    (task / "report.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n"
    )
    print(
        json.dumps(
            {key: report[key] for key in ("required", "completed", "passed", "failed")}
            | {
                "open": len(report["open"]),
                "bits": len(report["bitwise_differences"]),
                "evidence": str(task),
            }
        ),
        flush=True,
    )
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--panel", choices=PANELS, required=True)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    arguments = parser.parse_args()
    if arguments.capture:
        if not arguments.upstream:
            parser.error("capture requires --upstream")
        capture(arguments.upstream, arguments.panel)
    else:
        if not arguments.reference or not arguments.binary:
            parser.error("comparison requires reference and binary")
        report = qualify(
            arguments.reference, arguments.binary, arguments.panel, arguments.page_size
        )
        raise SystemExit(not report["bitwise_pass"])
