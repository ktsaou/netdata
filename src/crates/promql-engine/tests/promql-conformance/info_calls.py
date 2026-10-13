#!/usr/bin/env python3
"""Capture unchanged original INFO queries and compare complete candidate results."""
import argparse
import base64
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

from build import directory
from capture import verify_facade
from compare import checked_results, compare
from histogram_reference import COMMIT, MODULE, input_ledger
from info_cases import cases
from query import checked_candidate_packets, dataset_hex, request_hex

HERE = Path(__file__).resolve().parent
ORACLE_SHA256 = "28f31b32475f05ca6ff47e9dc79a65df4811155fec8d80c4eab8e266d76e7f62"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest_digest(manifest):
    return hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def artifact_names(groups):
    return {"inputs.json", "source.json", "original.json"} | {
        f"{stem}-{index}.{suffix}" for index in range(len(groups))
        for stem, suffix in (("dataset", "json"), ("requests", "json"), ("original", "json"), ("original", "stderr"))}


def checked_original(document, requests):
    checked_results(document)
    attribution = {"commit": COMMIT, "reference": MODULE, "go_version": "go1.27.1",
                   "go_os": "linux", "go_arch": "amd64", "unicode_version": "17.0.0"}
    if (any(document.get(key) != value for key, value in attribution.items())
            or [row["id"] for row in document["results"]] != [row["id"] for row in requests]):
        raise ValueError("original INFO attribution or result roster changed")


def checked_timestamp_outcomes(groups, document):
    results = {row["id"]: row for row in document["results"]}
    for group in groups:
        if not group["id"].startswith("timestamp-fixed-"):
            continue
        duplicate = group["id"].endswith("-duplicate")
        encoded = b"-9007199254740992" if "-negative-" in group["id"] else b"9007199254740992"
        message = (b'found duplicate series for info metric: existing {__name__="target_info", data="one", '
                   b'instance="a", job="worker"} @ ' + encoded
                   + b', new {__name__="target_info", data="two", instance="a", job="worker"} @ ' + encoded)
        for request in group["requests"]:
            expected = {"id": request["id"], "kind": "error" if duplicate else
                        "vector" if request["mode"] == "instant" else "matrix",
                        "rows": [], "warnings": [], "infos": []}
            if duplicate:
                expected["error"] = {"phase": "execution", "type": "*errors.errorString",
                                     "message": base64.b64encode(message).decode()}
            else:
                times = [request["time_ms"]] if request["mode"] == "instant" else range(
                    request["start_ms"], request["end_ms"] + 1, request["step_ms"])
                expected["rows"] = [{"labels": [{"name": base64.b64encode(name).decode(),
                                                 "value": base64.b64encode(value).decode()}
                                                for name, value in ((b"data", b"new"), (b"instance", b"a"),
                                                                    (b"job", b"worker"))],
                                     "points": [{"time_ms": time, "kind": "float", "float": "3ff0000000000000"}
                                                for time in times]}]
            if results.get(request["id"]) != expected:
                raise ValueError("extreme INFO timestamp case did not exercise its declared join")


def load_reference(reference):
    reference = directory(reference)
    binding = json.loads((reference / "manifest.json").read_bytes())
    groups = cases()
    requests = [request for group in groups for request in group["requests"]]
    if (type(binding) is not dict or binding.keys() != {
            "cases", "groups", "source_manifest_sha256", "oracle_sha256", "case_source_sha256", "artifacts"}
            or type(binding["cases"]) is not int or binding["cases"] != len(requests)
            or type(binding["groups"]) is not int or binding["groups"] != len(groups)
            or binding["oracle_sha256"] != ORACLE_SHA256
            or binding["case_source_sha256"] != sha(HERE / "info_cases.py")
            or binding["source_manifest_sha256"] != input_ledger()["source_manifest_sha256"]):
        raise ValueError("original INFO capture binding changed")
    if type(binding["artifacts"]) is not dict or binding["artifacts"].keys() != artifact_names(groups):
        raise ValueError("original INFO artifact roster changed")
    for name, digest in binding["artifacts"].items():
        path = reference / name
        if path.is_symlink() or sha(path) != digest:
            raise ValueError("original INFO artifact changed")
    if json.loads((reference / "inputs.json").read_bytes()) != groups:
        raise ValueError("independently declared INFO inputs changed")
    if manifest_digest(json.loads((reference / "source.json").read_bytes())) != binding["source_manifest_sha256"]:
        raise ValueError("original INFO source roster changed")
    expected = json.loads((reference / "original.json").read_bytes())
    checked_results(expected)
    results = []
    for index, group in enumerate(groups):
        if (json.loads((reference / f"dataset-{index}.json").read_bytes()) != group["dataset"]
                or json.loads((reference / f"requests-{index}.json").read_bytes()) != group["requests"]):
            raise ValueError("original INFO group inputs changed")
        document = json.loads((reference / f"original-{index}.json").read_bytes())
        checked_original(document, group["requests"])
        results.extend(document["results"])
    if expected != {"schema_version": 1, "results": results}:
        raise ValueError("original INFO combined result changed")
    checked_timestamp_outcomes(groups, expected)
    return groups, expected


def checked_candidate(responses, groups):
    return checked_candidate_packets(responses, groups, "INFO")


def capture(upstream, oracle):
    upstream = directory(upstream)
    oracle = Path(oracle).resolve(strict=True)
    if sha(oracle) != ORACLE_SHA256:
        raise ValueError("canonical original query binary changed")
    source = verify_facade(upstream, upstream)
    if manifest_digest(source) != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original INFO source changed")
    groups = cases()
    task = directory(tempfile.mkdtemp(prefix="info-call-capture-", dir=directory(HERE / ".tmp")))
    results = []
    for name, value in (("inputs.json", groups), ("source.json", source)):
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    for index, group in enumerate(groups):
        dataset, requests = f"dataset-{index}.json", f"requests-{index}.json"
        output, errors = f"original-{index}.json", f"original-{index}.stderr"
        for name, value in ((dataset, group["dataset"]), (requests, group["requests"])):
            (task / name).write_text(json.dumps(value, indent=2) + "\n")
        with (task / output).open("xb") as out, (task / errors).open("xb") as err:
            process = subprocess.run([str(oracle), str(task / dataset), str(task / requests)],
                                     stdout=out, stderr=err, timeout=600, check=False)
        if process.returncode:
            raise ValueError(f"original INFO query failed; evidence: {task}")
        document = json.loads((task / output).read_bytes())
        checked_original(document, group["requests"])
        results.extend(document["results"])
    if verify_facade(upstream, upstream) != source:
        raise ValueError("original INFO source changed during capture")
    (task / "original.json").write_text(json.dumps({"schema_version": 1, "results": results}, indent=2) + "\n")
    (task / "manifest.json").write_text(json.dumps({"cases": len(results), "groups": len(groups),
        "source_manifest_sha256": manifest_digest(source), "oracle_sha256": sha(oracle),
        "case_source_sha256": sha(HERE / "info_cases.py"),
        "artifacts": {name: sha(task / name) for name in sorted(artifact_names(groups))}}, indent=2) + "\n")
    load_reference(task)
    print(json.dumps({"capture": str(task), "groups": len(groups), "cases": len(results)}), flush=True)
    return task


def qualify(reference, binary, page_size=1024):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    groups, expected = load_reference(reference)
    requests = [request for group in groups for request in group["requests"]]
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be positive")
    task = directory(tempfile.mkdtemp(prefix="info-call-compare-", dir=directory(HERE / ".tmp")))
    rows = []
    for group in groups:
        rows.append({"op": "dataset", "dataset_id": group["id"], "page_size": page_size,
                     "dataset": dataset_hex(group["dataset"])})
        rows.extend(request_hex(request, group["id"]) for request in group["requests"])
    (task / "requests.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
    with ((task / "requests.jsonl").open("rb") as inputs,
          (task / "candidate.jsonl").open("xb") as out, (task / "candidate.stderr").open("xb") as err):
        process = subprocess.run([str(binary)], stdin=inputs, stdout=out, stderr=err, timeout=600, check=False)
    if process.returncode:
        raise ValueError(f"INFO candidate failed; evidence: {task}")
    responses = [json.loads(line) for line in (task / "candidate.jsonl").read_text().splitlines()]
    actual = checked_candidate(responses, groups)
    report = compare(expected, actual, [{"id": row["id"], "ordered": False, "exact_numeric": False}
                                       for row in requests])
    report.update(cases=len(requests), groups=len(groups), page_size=page_size,
                  binary_sha256=sha(binary), reference_sha256=sha(reference / "original.json"),
                  input_sha256=sha(task / "requests.jsonl"), output_sha256=sha(task / "candidate.jsonl"))
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("cases", "semantic_pass", "bitwise_pass")}
                     | {"failures": len(report["semantic_failures"]),
                        "paired_bits": len(report["bitwise_differences"]), "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--oracle")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    arguments = parser.parse_args()
    if arguments.capture:
        capture(arguments.upstream, arguments.oracle)
    else:
        report = qualify(arguments.reference, arguments.binary, arguments.page_size)
        raise SystemExit(not report["bitwise_pass"])
