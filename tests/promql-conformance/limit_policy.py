"""Qualify the declared LIMIT COUNT maps under approved label-byte order."""

import argparse
import json
import subprocess
import tempfile
from copy import deepcopy
from pathlib import Path

import cursor_calls
import limit_calls
from build import directory
from compare import compare, labels
from query import checked_candidate_packets
from topk_policy import BINDINGS, FILES, injected_inputs

HERE = Path(__file__).resolve().parent


def declarations(inputs):
    by_id = {row["request"]["id"]: row for row in inputs}
    declared, children = [], []
    for operator in ("limitk", "limit_ratio"):
        for window in (0, 1, 2):
            ident = (
                f"limits:{operator}:topk:body-producers-topk:upstream-tests:3:{window}"
            )
            inner, outer = 'count_values("v",m)', f'{operator}(2,{{v!=""}})'
            row = deepcopy(by_id[ident])
            if row.get("faults") or row["request"]["query"] != f"{operator}(2,{inner})":
                raise ValueError("LIMIT map boundary or fault-free declaration changed")
            declared.append((ident, inner, outer))
            row["request"].update(id="inner:" + ident, query=inner)
            children.append(row)
    return declared, children


def inject(declared, children, inner):
    output = injected_inputs(declared, children, inner)
    for result in output:
        if any(
            len(row["labels"]) != 1 or row["labels"][0]["name"] != "dg=="
            for row in result["dataset"]["series"]
        ):
            raise ValueError(
                "LIMIT injection requires unchanged COUNT v labels without deferred names"
            )
    return output


def load(reference, base, sampler):
    reference, base = directory(reference), directory(base)
    inputs, _ = limit_calls.load_reference(base, sampler)
    declared, children = declarations(inputs)
    manifest = json.loads((reference / "manifest.json").read_bytes())
    if (
        type(manifest) is not dict
        or manifest.keys() != {"required", "base_ids", "original_bindings", "files"}
        or type(manifest["required"]) is not int
        or manifest["required"] != len(declared)
        or manifest["base_ids"] != [row[0] for row in declared]
        or type(manifest["original_bindings"]) is not dict
        or manifest["original_bindings"].keys() != BINDINGS
        or type(manifest["files"]) is not dict
        or manifest["files"].keys() != FILES
    ):
        raise ValueError("LIMIT policy source binding or roster changed")
    if any(
        cursor_calls.sha(base / name) != digest
        for name, digest in manifest["original_bindings"].items()
    ):
        raise ValueError("LIMIT policy original executable or source changed")
    for name, digest in manifest["files"].items():
        if (reference / name).is_symlink() or cursor_calls.sha(
            reference / name
        ) != digest:
            raise ValueError("LIMIT policy source artifact changed")
    if json.loads((reference / "inner-inputs.json").read_bytes()) != children:
        raise ValueError("LIMIT policy child scope or dataset changed")
    inner = cursor_calls.checked_original(
        json.loads((reference / "inner-original.json").read_bytes()), children
    )
    outer_inputs = inject(declared, children, inner)
    if json.loads((reference / "outer-inputs.json").read_bytes()) != outer_inputs:
        raise ValueError("LIMIT policy raw labels, child order or parent changed")
    outer = cursor_calls.checked_original(
        json.loads((reference / "outer-original.json").read_bytes()), outer_inputs
    )
    return declared, outer


def capture(base, sampler):
    base = directory(base)
    inputs, _ = limit_calls.load_reference(base, sampler)
    declared, children = declarations(inputs)
    task = directory(
        tempfile.mkdtemp(prefix="limits-policy-source-", dir=directory(HERE / ".tmp"))
    )
    bindings = {name: cursor_calls.sha(base / name) for name in BINDINGS}
    expected = None
    for phase in ("inner", "outer"):
        inputs = children if phase == "inner" else inject(declared, children, expected)
        (task / f"{phase}-inputs.json").write_text(json.dumps(inputs, indent=2) + "\n")
        with (
            (task / f"{phase}-original.json").open("xb") as out,
            (task / f"{phase}-original.stderr").open("xb") as err,
        ):
            process = subprocess.run(
                [str(base / "cursor-oracle"), str(task / f"{phase}-inputs.json")],
                stdout=out,
                stderr=err,
                timeout=600,
                check=False,
            )
        if process.returncode:
            raise ValueError(f"LIMIT policy original control failed; evidence: {task}")
        expected = cursor_calls.checked_original(
            json.loads((task / f"{phase}-original.json").read_bytes()), inputs
        )
    if any(
        cursor_calls.sha(base / name) != digest for name, digest in bindings.items()
    ):
        raise ValueError("LIMIT policy source changed during capture")
    (task / "manifest.json").write_text(
        json.dumps(
            {
                "required": len(declared),
                "base_ids": [row[0] for row in declared],
                "original_bindings": bindings,
                "files": {
                    name: cursor_calls.sha(task / name) for name in sorted(FILES)
                },
            },
            indent=2,
        )
        + "\n"
    )
    load(task, base, sampler)
    print(json.dumps({"capture": str(task), "controls": len(declared)}), flush=True)
    return task


def expected_policy(raw, declared, outer):
    expected = deepcopy(raw)
    replacements = {
        row[0]: result | {"id": row[0]}
        for row, result in zip(declared, outer["results"], strict=True)
    }
    count_ids = {
        f"limits:{operator}:topk:parents-topk:upstream-tests:5:{window}"
        for operator in ("limitk", "limit_ratio")
        for window in (0, 1)
    }
    found = set()
    for index, result in enumerate(expected["results"]):
        ident = result["id"]
        if ident in replacements:
            replacement = replacements[ident]
            if any(
                replacement[key] != result[key] for key in ("warnings", "infos", "kind")
            ):
                raise ValueError("LIMIT injection changed diagnostics or result type")
            expected["results"][index] = replacement
            found.add(ident)
        elif ident in count_ids:
            if result["kind"] != "vector":
                raise ValueError("LIMIT COUNT boundary is not an instant vector")
            result["rows"].sort(key=labels)
            found.add(ident)
    if found != replacements.keys() | count_ids:
        raise ValueError("LIMIT policy outcome roster is incomplete")
    return expected, sorted(found)


def qualify(base, reference, candidate, sampler):
    base, candidate = directory(base), directory(candidate)
    inputs, raw = limit_calls.load_reference(base, sampler)
    declared, outer = load(reference, base, sampler)
    expected, boundaries = expected_policy(raw, declared, outer)
    report = json.loads((candidate / "report.json").read_bytes())
    if (
        report["cases"] != len(inputs)
        or report["reference_sha256"] != cursor_calls.sha(base / "original.json")
        or report["output_sha256"] != cursor_calls.sha(candidate / "candidate.jsonl")
        or report["input_sha256"] != cursor_calls.sha(candidate / "requests.jsonl")
    ):
        raise ValueError("LIMIT policy candidate packet changed")
    groups, requests = cursor_calls.candidate_packets(inputs, report["page_size"])
    if [
        json.loads(line)
        for line in (candidate / "requests.jsonl").read_text().splitlines()
    ] != requests:
        raise ValueError("LIMIT policy candidate scope or dataset changed")
    actual = checked_candidate_packets(
        [
            json.loads(line)
            for line in (candidate / "candidate.jsonl").read_text().splitlines()
        ],
        groups,
        "cursor",
    )
    cases = [
        {"id": row["request"]["id"], "ordered": True, "exact_numeric": True}
        for row in inputs
    ]
    original_report = compare(raw, actual, cases)
    if any(
        report[field] != original_report[field]
        for field in (
            "semantic_pass",
            "bitwise_pass",
            "semantic_failures",
            "bitwise_differences",
        )
    ):
        raise ValueError("LIMIT policy raw report changed")
    qualified = compare(expected, actual, cases)
    qualified.update(
        cases=len(inputs),
        policy_boundaries=boundaries,
        raw_failures=original_report["semantic_failures"],
        source_manifest_sha256=cursor_calls.sha(directory(reference) / "manifest.json"),
        candidate_output_sha256=report["output_sha256"],
    )
    (candidate / "policy-expected.json").write_text(
        json.dumps(expected, indent=2) + "\n"
    )
    (candidate / "policy-report.json").write_text(
        json.dumps(qualified, indent=2) + "\n"
    )
    print(
        json.dumps(
            {
                "cases": len(inputs),
                "boundaries": len(boundaries),
                "failures": len(qualified["semantic_failures"]),
                "raw_failures": len(original_report["semantic_failures"]),
                "bitwise_pass": qualified["bitwise_pass"],
                "evidence": str(candidate),
            }
        ),
        flush=True,
    )
    return qualified


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--sampler-reference", required=True)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--reference")
    parser.add_argument("--candidate")
    arguments = parser.parse_args()
    if arguments.capture:
        capture(arguments.base, arguments.sampler_reference)
    else:
        if not arguments.reference or not arguments.candidate:
            parser.error("qualification requires --reference and --candidate")
        raise SystemExit(
            not qualify(
                arguments.base,
                arguments.reference,
                arguments.candidate,
                arguments.sampler_reference,
            )["bitwise_pass"]
        )
