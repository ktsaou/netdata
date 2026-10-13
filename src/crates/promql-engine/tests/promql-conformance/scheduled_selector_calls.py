"""Compare single-selector schedules and retained replay against unchanged Go."""

import argparse
import json
from pathlib import Path

import cursor_calls
from build import directory
from compare import checked_results, compare, labels
from native_matrix_calls import recorded_run
from query import checked_candidate_packets
from scheduled_selector_cases import cases


def capture(upstream):
    return cursor_calls.capture(
        upstream,
        cases(),
        subject="scheduled-selector",
        construction_errors=True,
        process_runner=recorded_run,
    )


def qualify(reference, binary, page_size=1024):
    binary = Path(binary).resolve(strict=True)
    before = cursor_calls.sha(binary)
    inputs = cases()
    # These four-step child maps reach the existing unspecified-order boundary.
    policy_ids = {
        f"scheduled:projection:{profile}:{index}:fold"
        for profile in ("server-default", "upstream-tests")
        for index in range(4)
    }
    output_path = None

    def run(command, **options):
        nonlocal output_path
        output_path = Path(options["stdout"].name)
        return recorded_run(command, **options)

    report = cursor_calls.qualify(
        reference,
        binary,
        page_size,
        inputs,
        subject="scheduled-selector",
        exact_numeric=True,
        ordered=lambda request: request["id"] not in policy_ids,
        construction_errors=True,
        process_runner=run,
    )
    if cursor_calls.sha(binary) != before:
        raise ValueError("scheduled selector candidate changed during execution")
    if output_path is None:
        raise ValueError("scheduled selector output was not captured")
    groups, _ = cursor_calls.candidate_packets(inputs, page_size)
    actual = checked_candidate_packets(
        [json.loads(line) for line in output_path.read_text().splitlines()],
        groups,
        "scheduled selector ordering",
    )
    results = checked_results(actual)
    canonical_failures = []
    for ident in sorted(policy_ids):
        result = results[ident]
        if result["kind"] != "vector":
            canonical_failures.append(ident)
            continue
        row_labels = [labels(row) for row in result["rows"]]
        if row_labels != sorted(row_labels):
            canonical_failures.append(ident)
    _, expected = cursor_calls.load_reference(
        reference, inputs, construction_errors=True
    )
    strict = compare(
        expected,
        actual,
        [
            {"id": source["request"]["id"], "ordered": True, "exact_numeric": True}
            for source in inputs
        ],
    )
    (output_path.parent / "strict-order-report.json").write_text(
        json.dumps(strict, indent=2) + "\n"
    )
    policy = {
        "decision": "9.A",
        "source": "promql/engine.go:1489,1579,1622,2103,2439,857,869",
        "cases": sorted(policy_ids),
        "canonical_failures": canonical_failures,
        "binary_sha256": report["binary_sha256"],
        "reference_sha256": report["reference_sha256"],
        "output_sha256": report["output_sha256"],
    }
    (output_path.parent / "ordering-policy.json").write_text(
        json.dumps(policy, indent=2) + "\n"
    )
    if canonical_failures:
        raise ValueError("scheduled selector canonical row order differs")
    print(
        json.dumps(
            {key: report[key] for key in ("semantic_pass", "bitwise_pass")}
            | {"cases": report["cases"]}
        ),
        flush=True,
    )
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
        capture(directory(arguments.upstream))
    else:
        if not arguments.reference or not arguments.binary:
            parser.error("comparison requires reference and binary")
        raise SystemExit(
            not qualify(arguments.reference, arguments.binary, arguments.page_size)[
                "bitwise_pass"
            ]
        )
