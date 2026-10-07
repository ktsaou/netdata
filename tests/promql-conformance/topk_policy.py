"""Qualify declared 9.A map boundaries through independently ordered Go child inputs."""
import argparse
import json
import subprocess
import tempfile
from copy import deepcopy
from itertools import pairwise
from pathlib import Path

import cursor_calls
import topk_calls
from build import directory
from compare import compare, labels
from query import checked_candidate_packets

HERE = Path(__file__).resolve().parent
FILES = {"inner-inputs.json", "inner-original.json", "inner-original.stderr",
         "outer-inputs.json", "outer-original.json", "outer-original.stderr"}
BINDINGS = {"cursor-oracle", "binding.json", "source.json"}


def declarations(inputs):
    by_id = {row["request"]["id"]: row for row in inputs}
    output = []
    for operator in ("topk", "bottomk"):
        for profile in ("server-default", "upstream-tests"):
            output.append((f"topk:parents-{operator}:{profile}:4:3", f"{operator}(3,m)",
                           f"{operator}(1,m)"))
        for profile in ("server-default", "upstream-tests"):
            for window in range(4):
                output.append((f"topk:body-producers-{operator}:{profile}:3:{window}",
                               'count_values("v",m)', f'{operator}(2,{{v!=""}})'))
        for profile in ("server-default", "upstream-tests"):
            output.append((f"topk:body-producers-{operator}:{profile}:5:3", "abs(m)",
                           f'{operator}(2,{{z!=""}})'))
    rows = []
    for ident, inner, outer in output:
        row = deepcopy(by_id[ident])
        if row.get("faults"):
            raise ValueError("policy injection requires a fault-free child")
        if row["request"]["query"] != outer.split(",", 1)[0] + "," + inner + ")":
            raise ValueError("declared map boundary no longer matches its parent")
        row["request"].update(id="inner:" + ident, query=inner)
        rows.append(row)
    return output, rows


def injected_inputs(declared, inner_inputs, inner_expected):
    output = []
    for (ident, _, query), original, result in zip(
            declared, inner_inputs, inner_expected["results"], strict=True):
        if (result["id"] != "inner:" + ident or result["kind"] not in {"matrix", "vector"}
                or result["warnings"] or result["infos"] or result.get("error")):
            raise ValueError("policy injection child has unsupported semantics")
        rows = sorted(result["rows"], key=labels)
        for row in rows:
            if (any(point["kind"] != "float" for point in row["points"])
                    or any(left["time_ms"] >= right["time_ms"]
                           for left, right in pairwise(row["points"]))):
                raise ValueError("policy injection requires distinct ordinary float points")
        output.append({"request": original["request"] | {"id": "outer:" + ident, "query": query},
                       "dataset": original["dataset"] | {"series": deepcopy(rows)}})
    return output


def load(reference, base):
    reference, base = directory(reference), directory(base)
    inputs, _ = topk_calls.load_reference(base)
    declared, inner_inputs = declarations(inputs)
    manifest = json.loads((reference / "manifest.json").read_bytes())
    if (manifest.get("required") != len(declared)
            or manifest.get("base_ids") != [row[0] for row in declared]
            or manifest.get("original_bindings", {}).keys() != BINDINGS
            or manifest.get("files", {}).keys() != FILES):
        raise ValueError("policy source binding or declaration roster changed")
    for name, digest in manifest["original_bindings"].items():
        if cursor_calls.sha(base / name) != digest:
            raise ValueError("policy original executable or source changed")
    for name, digest in manifest["files"].items():
        if (reference / name).is_symlink() or cursor_calls.sha(reference / name) != digest:
            raise ValueError("policy source artifact changed")
    if json.loads((reference / "inner-inputs.json").read_bytes()) != inner_inputs:
        raise ValueError("policy inner scope, profile or input changed")
    inner = cursor_calls.checked_original(
        json.loads((reference / "inner-original.json").read_bytes()), inner_inputs)
    outer_inputs = injected_inputs(declared, inner_inputs, inner)
    if json.loads((reference / "outer-inputs.json").read_bytes()) != outer_inputs:
        raise ValueError("policy child label order, points or parent changed")
    outer = cursor_calls.checked_original(
        json.loads((reference / "outer-original.json").read_bytes()), outer_inputs)
    return declared, outer


def capture(base):
    base = directory(base)
    inputs, _ = topk_calls.load_reference(base)
    declared, inner_inputs = declarations(inputs)
    task = directory(tempfile.mkdtemp(prefix="topk-policy-source-", dir=directory(HERE / ".tmp")))
    bindings = {name: cursor_calls.sha(base / name) for name in BINDINGS}
    expected = None
    for phase in ("inner", "outer"):
        phase_inputs = inner_inputs if phase == "inner" else injected_inputs(declared, inner_inputs, expected)
        input_path = task / f"{phase}-inputs.json"
        input_path.write_text(json.dumps(phase_inputs, indent=2) + "\n")
        with ((task / f"{phase}-original.json").open("xb") as out,
              (task / f"{phase}-original.stderr").open("xb") as err):
            process = subprocess.run([str(base / "cursor-oracle"), str(input_path)],
                                     stdout=out, stderr=err, timeout=600, check=False)
        if process.returncode:
            raise ValueError(f"policy original control failed; evidence: {task}")
        expected = cursor_calls.checked_original(
            json.loads((task / f"{phase}-original.json").read_bytes()), phase_inputs)
    if any(cursor_calls.sha(base / name) != digest for name, digest in bindings.items()):
        raise ValueError("policy original executable or source changed during capture")
    (task / "manifest.json").write_text(json.dumps({"required": len(declared),
        "base_ids": [row[0] for row in declared], "original_bindings": bindings,
        "canonicalization": "raw label-byte order only at declared derived matrix map boundary",
        "files": {name: cursor_calls.sha(task / name) for name in sorted(FILES)}}, indent=2) + "\n")
    load(task, base)
    print(json.dumps({"capture": str(task), "controls": len(declared)}), flush=True)
    return task


def expected_policy(raw, declared, outer):
    expected = deepcopy(raw)
    replacements = {row[0]: result | {"id": row[0]}
                    for row, result in zip(declared, outer["results"], strict=True)}
    count_ids = {f"topk:parents-{operator}:{profile}:5:{window}"
                 for operator in ("topk", "bottomk")
                 for profile in ("server-default", "upstream-tests") for window in (0, 1)}
    found = set()
    for index, result in enumerate(expected["results"]):
        ident = result["id"]
        if ident in replacements:
            expected["results"][index] = replacements[ident]
            found.add(ident)
        elif ident in count_ids:
            result["rows"].sort(key=labels)
            found.add(ident)
    if found != replacements.keys() | count_ids:
        raise ValueError("policy original result roster is incomplete")
    return expected, sorted(found)


def qualify(base, reference, candidate):
    base, candidate = directory(base), directory(candidate)
    inputs, raw = topk_calls.load_reference(base)
    declared, outer = load(reference, base)
    expected, boundaries = expected_policy(raw, declared, outer)
    report = json.loads((candidate / "report.json").read_bytes())
    if (report["cases"] != len(inputs) or report["reference_sha256"] != cursor_calls.sha(base / "original.json")
            or report["output_sha256"] != cursor_calls.sha(candidate / "candidate.jsonl")
            or report["input_sha256"] != cursor_calls.sha(candidate / "requests.jsonl")):
        raise ValueError("policy candidate packet changed")
    groups, requests = cursor_calls.candidate_packets(inputs, report["page_size"])
    if [json.loads(line) for line in (candidate / "requests.jsonl").read_text().splitlines()] != requests:
        raise ValueError("policy candidate scope, profile or dataset changed")
    actual = checked_candidate_packets(
        [json.loads(line) for line in (candidate / "candidate.jsonl").read_text().splitlines()], groups, "cursor")
    cases = [{"id": row["request"]["id"], "ordered": True, "exact_numeric": True} for row in inputs]
    original_report = compare(raw, actual, cases)
    for field in ("semantic_pass", "bitwise_pass", "semantic_failures", "bitwise_differences"):
        if report[field] != original_report[field]:
            raise ValueError("policy raw report no longer matches the immutable original comparison")
    qualified = compare(expected, actual, cases)
    qualified.update(cases=len(inputs), policy_boundaries=boundaries,
                     raw_failures=original_report["semantic_failures"],
                     source_manifest_sha256=cursor_calls.sha(Path(reference) / "manifest.json"),
                     candidate_output_sha256=report["output_sha256"])
    (candidate / "policy-expected.json").write_text(json.dumps(expected, indent=2) + "\n")
    (candidate / "policy-report.json").write_text(json.dumps(qualified, indent=2) + "\n")
    print(json.dumps({key: qualified[key] for key in ("cases", "semantic_pass", "bitwise_pass")}
                     | {"failures": len(qualified["semantic_failures"]), "boundaries": len(boundaries),
                        "raw_failures": len(original_report["semantic_failures"]),
                        "evidence": str(candidate)}), flush=True)
    return qualified


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--reference")
    parser.add_argument("--candidate")
    arguments = parser.parse_args()
    if arguments.capture:
        capture(arguments.base)
    else:
        if not arguments.reference or not arguments.candidate:
            parser.error("qualification requires --reference and --candidate")
        raise SystemExit(not qualify(arguments.base, arguments.reference, arguments.candidate)["bitwise_pass"])
