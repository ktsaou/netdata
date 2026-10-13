#!/usr/bin/env python3
"""Capture and compare complete original histogram-call results, including map aliases."""
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
from histogram_call_cases import cases, provider_cases, provider_messages
from histogram_capture import COMMIT
from histogram_reference import input_ledger
from query import dataset_hex, request_hex, result_base64

HERE = Path(__file__).resolve().parent
ORACLE_SHA256 = "28f31b32475f05ca6ff47e9dc79a65df4811155fec8d80c4eab8e266d76e7f62"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(upstream, oracle, provider=False):
    source = directory(upstream)
    oracle = Path(oracle).resolve(strict=True) if oracle else None
    if not provider and (oracle is None or sha(oracle) != ORACLE_SHA256):
        raise ValueError("canonical original query binary changed")
    manifest = verify_facade(source, source)
    digest = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if digest != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original histogram-call source changed")
    task = directory(tempfile.mkdtemp(prefix="histogram-call-capture-", dir=directory(HERE / ".tmp")))
    if provider:
        shutil.copy2(HERE / "capture/histogram_provider.go", task / "provider.go")
        for name in ("go.mod", "go.sum"):
            shutil.copy2(HERE / "oracle" / name, task / name)
        private = task / "tmp"
        private.mkdir(mode=0o700)
        env = os.environ.copy() | {"GOMODCACHE": str(directory(source.parents[2])),
                                  "GOCACHE": str(directory(HERE / ".tmp/go-build")),
                                  "GOPROXY": "off", "GOSUMDB": "off", "GOTOOLCHAIN": "local",
                                  "GOWORK": "off", "GOFLAGS": "-mod=readonly", "GOAMD64": "v1",
                                  "TMPDIR": str(directory(private)), "GOTMPDIR": str(private)}
        toolchain = subprocess.check_output(["go", "version"], env=env, text=True).strip()
        if toolchain != "go version go1.27.1 linux/amd64":
            raise ValueError("original provider-query compiler profile changed")
        (task / "toolchain.txt").write_text(toolchain + "\n")
        oracle = task / "provider-query"
        command = ["go", "build", "-p", "4", "-o", str(oracle), str(task / "provider.go")]
        (task / "build-command.json").write_text(json.dumps(command) + "\n")
        build = subprocess.run(command, cwd=HERE / "oracle", env=env, capture_output=True, timeout=600, check=False)
        (task / "build.stdout").write_bytes(build.stdout)
        (task / "build.stderr").write_bytes(build.stderr)
        if build.returncode:
            raise ValueError(f"original provider-query build failed; evidence: {task}")
    dataset, requests = provider_cases() if provider else cases()
    for name, value in [("dataset.json", dataset), ("requests.json", requests), ("source.json", manifest)]:
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    with (task / "original.json").open("xb") as out, (task / "original.stderr").open("xb") as err:
        result = subprocess.run([str(oracle), str(task / "dataset.json"), str(task / "requests.json")],
                                stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode or verify_facade(source, source) != manifest:
        raise ValueError(f"original histogram-call capture failed; evidence: {task}")
    expected = json.loads((task / "original.json").read_text())
    checked_results(expected)
    if provider and expected["provider_messages"] != provider_messages():
        raise ValueError("original provider diagnostic input changed")
    if expected["commit"] != COMMIT or [row["id"] for row in expected["results"]] != [row["id"] for row in requests]:
        raise ValueError("original histogram-call result roster changed")
    names = ["dataset.json", "requests.json", "source.json", "original.json"]
    if provider:
        names += ["provider.go", "provider-query", "go.mod", "go.sum", "toolchain.txt", "build-command.json"]
    binding = {"commit": COMMIT, "oracle_sha256": sha(oracle), "cases": len(requests),
               "files": {name: sha(task / name) for name in names}, "source_manifest_sha256": digest}
    if provider:
        binding["provider"] = True
    (task / "binding.json").write_text(json.dumps(binding, indent=2) + "\n")
    print(json.dumps({"original_cases": len(requests), "capture": str(task)}), flush=True)
    return task


def qualify(reference, binary, page_size):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    binding = json.loads((reference / "binding.json").read_text())
    provider = binding.get("provider", False)
    if binding["commit"] != COMMIT or not provider and binding["oracle_sha256"] != ORACLE_SHA256:
        raise ValueError("original histogram-call binding changed")
    for name, digest in binding["files"].items():
        if Path(name).name != name or sha(reference / name) != digest:
            raise ValueError("original histogram-call artifact changed")
    if provider and (sha(reference / "provider.go") != sha(HERE / "capture/histogram_provider.go")
                     or sha(reference / "provider-query") != binding["oracle_sha256"]
                     or (reference / "toolchain.txt").read_text() != "go version go1.27.1 linux/amd64\n"):
        raise ValueError("original provider-query source/binary profile changed")
    dataset, requests = provider_cases() if provider else cases()
    if (json.loads((reference / "dataset.json").read_text()) != dataset
            or json.loads((reference / "requests.json").read_text()) != requests
            or binding["cases"] != len(requests)):
        raise ValueError("independent histogram-call inputs changed")
    manifest = json.loads((reference / "source.json").read_text())
    digest = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if digest != binding["source_manifest_sha256"] or digest != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original histogram-call source pin changed")
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be positive")
    expected = json.loads((reference / "original.json").read_text())
    checked_results(expected)
    task = directory(tempfile.mkdtemp(prefix="histogram-call-compare-", dir=directory(HERE / ".tmp")))
    identifiers = ["server-default", "upstream-tests"] if provider else ["call"]
    rows = []
    for ident in identifiers:
        row = {"op": "dataset", "dataset_id": ident, "page_size": page_size, "dataset": dataset_hex(dataset)}
        if provider:
            row["faults"] = {"m": {"warning": provider_messages()[ident]}}
        rows.append(row)
        rows.extend(request_hex(request, ident) for request in requests
                    if not provider or request["profile"] == ident)
    (task / "requests.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
    with ((task / "requests.jsonl").open("rb") as inputs,
          (task / "candidate.jsonl").open("xb") as out, (task / "candidate.stderr").open("xb") as err):
        result = subprocess.run([str(binary)], stdin=inputs, stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode:
        raise ValueError(f"histogram-call candidate failed; evidence: {task}")
    responses = [json.loads(line) for line in (task / "candidate.jsonl").read_text().splitlines()]
    acknowledgements = [row for row in responses if "dataset_id" in row]
    responses = [row for row in responses if "dataset_id" not in row]
    if (acknowledgements != [{"dataset_id": ident} for ident in identifiers]
            or [row.get("id") for row in responses] != [row["id"] for row in requests]):
        raise ValueError("candidate histogram-call result roster changed")
    actual = {"schema_version": 1, "results": [result_base64(row["result"]) for row in responses]}
    checked_results(actual)
    report = compare(expected, actual, [{"id": row["id"], "ordered": True, "exact_numeric": False} for row in requests])
    report.update(cases=len(requests), binary_sha256=sha(binary), reference_sha256=sha(reference / "original.json"),
                  page_size=page_size)
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ["cases", "semantic_pass", "bitwise_pass"]}
                     | {"failures": len(report["semantic_failures"]),
                        "bit_differences": len(report["bitwise_differences"]), "evidence": str(task)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--provider", action="store_true", help="capture ordinary provider warnings sharing a helper key")
    parser.add_argument("--upstream")
    parser.add_argument("--oracle")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=17)
    args = parser.parse_args()
    if args.capture:
        if not args.upstream or not args.provider and not args.oracle:
            parser.error("capture requires --upstream and, unless --provider, --oracle")
        capture(args.upstream, args.oracle, args.provider)
    else:
        if not args.reference or not args.binary:
            parser.error("comparison requires --reference and --binary")
        result = qualify(args.reference, args.binary, args.page_size)
        raise SystemExit(not result["bitwise_pass"])
