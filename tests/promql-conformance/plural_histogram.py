#!/usr/bin/env python3
"""Capture and qualify plural calls with independent name-removal options."""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from build import directory
from capture import verify_facade
from compare import checked_results, compare
from histogram_capture import COMMIT
from histogram_reference import input_ledger
from numerical_reference import PROFILE
from plural_histogram_cases import cases
from query import dataset_hex, request_hex, result_base64

HERE = Path(__file__).resolve().parent
PROBE = HERE / "capture/plural_histogram.go"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest_digest(manifest):
    return hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def capture(upstream):
    source = directory(upstream)
    manifest = verify_facade(source, source)
    digest = manifest_digest(manifest)
    if digest != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original plural source pin changed")
    task = directory(tempfile.mkdtemp(prefix="plural-histogram-capture-", dir=directory(HERE / ".tmp")))
    private = task / "build-tmp"
    private.mkdir(mode=0o700)
    directory(private)
    probe = task / "original-probe.go"
    shutil.copy2(PROBE, probe)
    env = os.environ.copy() | {"GOMODCACHE": str(directory(source.parents[2])),
                              "GOCACHE": str(directory(HERE / ".tmp/go-build")),
                              "GOPROXY": "off", "GOSUMDB": "off", "GOTOOLCHAIN": "local",
                              "GOWORK": "off", "GOFLAGS": "-mod=readonly", "GOAMD64": "v1",
                              "GODEBUG": "", "TMPDIR": str(private), "GOTMPDIR": str(private)}
    profile = json.loads(subprocess.check_output(["go", "env", "-json", *PROFILE], cwd=HERE / "oracle", env=env))
    if profile != PROFILE:
        raise ValueError("original plural runtime profile changed")
    binary = task / "original-probe"
    command = ["go", "build", "-p", "4", "-o", str(binary), str(probe)]
    (task / "build-command.json").write_text(json.dumps(command) + "\n")
    with (task / "build.stdout").open("xb") as out, (task / "build.stderr").open("xb") as err:
        result = subprocess.run(command, cwd=HERE / "oracle", env=env, stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode:
        raise ValueError(f"original plural build failed; evidence: {task}")
    dataset, requests = cases()
    for name, value in [("dataset.json", dataset), ("requests.json", requests), ("source.json", manifest)]:
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    with (task / "original.json").open("xb") as out, (task / "original.stderr").open("xb") as err:
        result = subprocess.run([str(binary), str(task / "dataset.json"), str(task / "requests.json")],
                                cwd=HERE / "oracle", env=env, stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode or verify_facade(source, source) != manifest or sha(probe) != sha(PROBE):
        raise ValueError(f"original plural capture failed or source changed; evidence: {task}")
    names = ["original-probe.go", "original-probe", "build-command.json", "dataset.json", "requests.json",
             "source.json", "original.json"]
    binding = {"commit": COMMIT, "profile": profile, "cases": len(requests), "source_manifest_sha256": digest,
               "oracle_sha256": sha(binary), "files": {name: sha(task / name) for name in names},
               "owners": {"case_generator": sha(HERE / "plural_histogram_cases.py"),
                          "driver": sha(Path(__file__)), "probe": sha(PROBE)}}
    (task / "binding.json").write_text(json.dumps(binding, indent=2) + "\n")
    load(task)
    print(json.dumps({"original_cases": len(requests), "capture": str(task)}), flush=True)
    return task


def load(reference):
    reference = directory(reference)
    binding = json.loads((reference / "binding.json").read_bytes())
    if (binding["commit"] != COMMIT or binding["profile"] != PROFILE
            or binding["source_manifest_sha256"] != input_ledger()["source_manifest_sha256"]):
        raise ValueError("original plural attribution changed")
    if binding["owners"] != {"case_generator": sha(HERE / "plural_histogram_cases.py"),
                              "driver": sha(Path(__file__)), "probe": sha(PROBE)}:
        raise ValueError("plural qualification owner changed")
    for name, digest in binding["files"].items():
        if Path(name).name != name or sha(reference / name) != digest:
            raise ValueError("original plural artifact changed")
    if (sha(reference / "original-probe.go") != sha(PROBE)
            or sha(reference / "original-probe") != binding["oracle_sha256"]):
        raise ValueError("original plural probe changed")
    manifest = json.loads((reference / "source.json").read_bytes())
    if manifest_digest(manifest) != binding["source_manifest_sha256"]:
        raise ValueError("original plural source binding changed")
    dataset, requests = cases()
    if (json.loads((reference / "dataset.json").read_bytes()) != dataset
            or json.loads((reference / "requests.json").read_bytes()) != requests
            or binding["cases"] != len(requests)):
        raise ValueError("independent plural inputs changed")
    expected = json.loads((reference / "original.json").read_bytes())
    checked_results(expected)
    for key, value in [("reference", "github.com/prometheus/prometheus@v0.315.0"), ("commit", COMMIT),
                       ("go_version", "go1.27.1"), ("go_arch", "amd64"), ("go_os", "linux"),
                       ("unicode_version", "17.0.0")]:
        if expected.get(key) != value:
            raise ValueError("original plural result profile changed")
    if [row["id"] for row in expected["results"]] != [row["id"] for row in requests]:
        raise ValueError("original plural result roster changed")
    return dataset, requests, expected


def qualify(reference, binary, page_size):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be positive")
    dataset, requests, expected = load(reference)
    task = directory(tempfile.mkdtemp(prefix="plural-histogram-compare-", dir=directory(HERE / ".tmp")))
    rows = [{"op": "dataset", "dataset_id": "plural", "page_size": page_size, "dataset": dataset_hex(dataset)}]
    for request in requests:
        payload = request_hex(request, "plural")
        payload["delayed_name_removal"] = request["delayed_name_removal"]
        rows.append(payload)
    raw = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows).encode()
    (task / "requests.jsonl").write_bytes(raw)
    binary_sha = sha(binary)
    with (task / "candidate.jsonl").open("xb") as out, (task / "candidate.stderr").open("xb") as err:
        result = subprocess.run([str(binary)], input=raw, stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode or binary_sha != sha(binary):
        raise ValueError(f"plural candidate failed or binary changed; evidence: {task}")
    responses = [json.loads(line) for line in (task / "candidate.jsonl").read_bytes().splitlines()]
    if responses[0] != {"dataset_id": "plural"} or [row["id"] for row in responses[1:]] != [row["id"] for row in requests]:
        raise ValueError("candidate plural identity roster changed")
    actual = {"schema_version": 1, "results": [result_base64(row["result"]) for row in responses[1:]]}
    checked_results(actual)
    report = compare(expected, actual, [{"id": row["id"], "ordered": row["mode"] == "range", "exact_numeric": True}
                                       for row in requests])
    load(reference)
    report.update(cases=len(requests), binary_sha256=binary_sha, reference_sha256=sha(reference / "original.json"),
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
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=17)
    args = parser.parse_args()
    if args.capture:
        if not args.upstream:
            parser.error("capture requires --upstream")
        capture(args.upstream)
    else:
        if not args.reference or not args.binary:
            parser.error("qualification requires --reference and --binary")
        report = qualify(args.reference, args.binary, args.page_size)
        raise SystemExit(not report["bitwise_pass"])
