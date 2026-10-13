"""Qualify cursor originals and the approved physical-Seek reachability policy."""

import argparse
from copy import deepcopy
import json
from pathlib import Path

import cursor_calls
from compare import compare
from native_matrix_calls import recorded_run
from query import checked_candidate_packets

PROFILES = ("server-default", "upstream-tests")
VECTOR_CONTROLS = {
    f"{fault}-{profile}": f"{control}-{profile}"
    for profile in PROFILES
    for fault, control in (
        ("seek-fault", "ordinary-direct-delta"),
        ("clock-seek-fault", "clock-direct-delta"),
    )
}
PHASE_IDS = {
    f"{prefix}-{profile}"
    for profile in PROFILES
    for prefix in ("phase-ordinary", "phase-clock-unary")
}
POLICY_IDS = VECTOR_CONTROLS.keys() | PHASE_IDS


def harness_identity():
    here = Path(__file__).resolve(strict=True).parent
    return {name: cursor_calls.sha(here / name) for name in (
        "cursor_policy_calls.py", "cursor_calls.py", "compare.py",
        "native_matrix_calls.py", "query.py", "cursor-original-inputs.json",
    )}


def controls():
    result = []
    for source in cursor_calls.cases():
        if source["request"]["id"] not in PHASE_IDS:
            continue
        control = deepcopy(source)
        control["request"]["id"] += ":without-redundant-seek"
        fault = control["faults"].pop("m/u")
        if fault != {"seek_targets": [1900, 1901], "message": "u-late-seek"}:
            raise ValueError("declared physical Seek control changed")
        result.append(control)
    if len(result) != 4:
        raise ValueError("physical Seek control roster changed")
    return result


def capture(upstream):
    return cursor_calls.capture(
        upstream,
        controls(),
        subject="physical-seek-policy",
        process_runner=recorded_run,
    )


def qualify(reference, control_reference, binary, page_size=1024):
    harness = harness_identity()
    inputs, expected = cursor_calls.load_reference(reference)
    _, independent = cursor_calls.load_reference(control_reference, controls())
    by_input = {row["request"]["id"]: row for row in inputs}
    for fault, control in VECTOR_CONTROLS.items():
        left, right = deepcopy(by_input[fault]), deepcopy(by_input[control])
        left["request"].pop("id")
        right["request"].pop("id")
        left.pop("faults")
        if right.pop("faults"):
            raise ValueError("fault-free original unexpectedly declares a fault")
        if left != right:
            raise ValueError("fault-free original is not the identical expression input")
    binary = Path(binary).resolve(strict=True)
    identity = cursor_calls.sha(binary)
    outputs = []

    def run(command, **options):
        outputs.append(Path(options["stdout"].name).resolve(strict=True))
        return recorded_run(command, **options)

    strict = cursor_calls.qualify(
        reference, binary, page_size, exact_numeric=True, process_runner=run
    )
    original_output = outputs[-1]
    control_report = cursor_calls.qualify(
        control_reference, binary, page_size, controls(),
        subject="physical-seek-policy", exact_numeric=True, process_runner=run,
    )
    if not control_report["semantic_pass"] or not control_report["bitwise_pass"]:
        raise ValueError("independent physical Seek controls differ")
    failures = set(strict["semantic_failures"])
    if failures != (POLICY_IDS if page_size in (17, 1024) else set()):
        raise ValueError("cursor original failure roster differs from the declared policy")
    if strict["unexpected_results"] or strict["bitwise_differences"]:
        raise ValueError("cursor original has an undeclared outcome or paired-bit difference")

    groups, _ = cursor_calls.candidate_packets(inputs, page_size)
    actual = checked_candidate_packets(
        [json.loads(line) for line in original_output.read_text().splitlines()],
        groups, "physical Seek policy",
    )
    adjusted = deepcopy(expected)
    original_rows = {row["id"]: row for row in expected["results"]}
    control_rows = {row["id"]: row for row in independent["results"]}
    mappings = {}
    if page_size in (17, 1024):
        for position, row in enumerate(adjusted["results"]):
            ident = row["id"]
            if ident in VECTOR_CONTROLS:
                control = VECTOR_CONTROLS[ident]
                replacement = deepcopy(original_rows[control])
                mappings[ident] = {"original_control_id": control}
            elif ident in PHASE_IDS:
                control = ident + ":without-redundant-seek"
                replacement = deepcopy(control_rows[control])
                mappings[ident] = {"independent_control_id": control}
            else:
                continue
            replacement["id"] = ident
            adjusted["results"][position] = replacement
    policy = compare(
        adjusted, actual,
        [{"id": row["request"]["id"], "ordered": True, "exact_numeric": True}
         for row in inputs],
    )
    if cursor_calls.sha(binary) != identity:
        raise ValueError("physical Seek candidate changed during execution")
    if harness_identity() != harness:
        raise ValueError("physical Seek qualification sources changed during execution")
    report = {
        "decision": "30.A",
        "scope": "Errors from actual physical cursor operations; skipped callbacks cannot return faults.",
        "cases": len(inputs), "additional_controls": 4, "page_size": page_size,
        "raw_semantic_pass": strict["semantic_pass"],
        "raw_bitwise_pass": strict["bitwise_pass"],
        "raw_failures": strict["semantic_failures"],
        "policy_mappings": mappings, "policy_comparison": policy,
        "independent_control_comparison": control_report,
        "semantic_pass": policy["semantic_pass"], "bitwise_pass": policy["bitwise_pass"],
        "binary_sha256": identity, "reference_sha256": strict["reference_sha256"],
        "control_reference_sha256": control_report["reference_sha256"],
        "original_output_sha256": strict["output_sha256"],
        "control_output_sha256": control_report["output_sha256"],
        "harness_sha256": harness,
    }
    (original_output.parent / "physical-seek-policy.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n"
    )
    print(json.dumps({key: report[key] for key in (
        "cases", "additional_controls", "page_size", "raw_semantic_pass", "semantic_pass", "bitwise_pass"
    )} | {"evidence": str(original_output.parent)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--controls")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, choices=(1, 17, 1024), default=1024)
    args = parser.parse_args()
    if args.capture:
        if not args.upstream:
            parser.error("capture requires --upstream")
        capture(args.upstream)
    else:
        if not args.reference or not args.controls or not args.binary:
            parser.error("comparison requires --reference, --controls and --binary")
        report = qualify(args.reference, args.controls, args.binary, args.page_size)
        raise SystemExit(not report["semantic_pass"] or not report["bitwise_pass"])
