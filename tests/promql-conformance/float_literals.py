#!/usr/bin/env python3
"""Qualify float-only conversion with original Go inputs and independent boundary cases."""
import argparse
import base64
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import tempfile
from pathlib import Path

from build import directory
from float_literal_cases import generate
from numerical_reference import PROFILE

HERE = Path(__file__).resolve().parent
COMMIT = "862c888e612ac346c7c4d99c9392bdfd265f33b0"
SOURCES = {
    "src/internal/strconv/atof.go": "b3322257fd15836b2ed9355599a767915382c040665391e01a6af35548b9b643",
    "src/internal/strconv/atof_test.go": "13883818e4eba65d99da2b4d1806782ebc1243fdbeeb1bdb90e44b1c75c11500",
    "src/internal/strconv/atoi.go": "bd69756b79c2f007ce7900bea400051f9f9da600c9c6bad656e46900331663d8",
    "src/strconv/number.go": "dda43e8c91d95f9dea0c46b2fec6637e5520cae099e40c5965dca0e41bbaab9e",
    "VERSION": "25eb74b09036e1ad894fb63b86cb0f0ee53342df8d2e2e860da4ff5079607758",
    "LICENSE": "911f8f5782931320f5b8d1160a76365b83aea6447ee6c04fa6d5591467db9dad",
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def invoke(command, task, env, name, data=None):
    print(f"{task} > {shlex.join(map(str, command))}", flush=True)
    result = subprocess.run(list(map(str, command)), input=data, cwd=task, env=env,
                            capture_output=True, timeout=600, check=False)
    (task / (name + ".stdout")).write_bytes(result.stdout)
    (task / (name + ".stderr")).write_bytes(result.stderr)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}); evidence: {task}")
    return result.stdout


def strict_object(items):
    result = {}
    for key, value in items:
        if key in result:
            raise ValueError("duplicate JSON field")
        result[key] = value
    return result


def decode(raw):
    return json.loads(raw, object_pairs_hook=strict_object)


def capture():
    parent = HERE / ".tmp"
    parent.mkdir(exist_ok=True)
    parent = directory(parent)
    task = directory(tempfile.mkdtemp(prefix="float-literals-capture-", dir=parent))
    private = task / "tmp"
    private.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(GOTOOLCHAIN="local", GO111MODULE="off", GOWORK="off", GOFLAGS="", GOAMD64="v1",
               GOPROXY="off", GOSUMDB="off", GOCACHE=str(parent / "go-build"),
               TMPDIR=str(directory(private)), GOTMPDIR=str(private))
    profile = decode(invoke(["go", "env", "-json", "GOROOT", *PROFILE], task, env, "profile"))
    runtime = directory(profile.pop("GOROOT"))
    if profile != PROFILE:
        raise ValueError("float conversion reference requires the pinned Go 1.27.1 / AMD64 profile")
    for relative, expected in SOURCES.items():
        source = runtime / relative
        if digest(source) != expected:
            raise ValueError(f"pinned Go owner changed: {relative}")
        destination = task / "go-source" / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    probe = task / "float_literals.go"
    shutil.copyfile(HERE / "capture/float_literals.go", probe)
    binary = task / "parse-float-go"
    invoke(["go", "build", "-p", "4", "-o", binary, probe], task, env, "build")
    original = invoke([binary, "--cases", runtime / "src/internal/strconv/atof_test.go"], task, env, "original")
    (task / "original-inputs.json").write_bytes(original)
    requests, counts = generate(decode(original))
    payload = json.dumps(requests, sort_keys=True, separators=(",", ":")).encode() + b"\n"
    (task / "requests.json").write_bytes(payload)
    reference = invoke([binary], task, env, "reference", payload)
    (task / "reference.json").write_bytes(reference)
    if any(digest(runtime / relative) != expected for relative, expected in SOURCES.items()):
        raise ValueError("Go owners changed during capture")
    files = ["original-inputs.json", "requests.json", "reference.json", "float_literals.go", "parse-float-go"]
    manifest = {"schema_version": 1, "runtime_commit": COMMIT, "profile": profile,
                "counts": counts, "required": len(requests), "sources": SOURCES,
                "generator_sha256": digest(HERE / "float_literal_cases.py"),
                "files": {name: digest(task / name) for name in files}}
    (task / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    load(task)
    print(json.dumps({"capture": str(task), "required": len(requests), "counts": counts}), flush=True)
    return task


def load(task):
    task = directory(task)
    manifest = decode((task / "manifest.json").read_bytes())
    if (manifest["schema_version"] != 1 or manifest["runtime_commit"] != COMMIT
            or manifest["profile"] != PROFILE or manifest["sources"] != SOURCES
            or manifest["generator_sha256"] != digest(HERE / "float_literal_cases.py")):
        raise ValueError("float reference source/input contract changed")
    expected_files = {"original-inputs.json", "requests.json", "reference.json", "float_literals.go", "parse-float-go"}
    if set(manifest["files"]) != expected_files:
        raise ValueError("incomplete float reference file ledger")
    if any(digest(task / name) != expected for name, expected in manifest["files"].items()):
        raise ValueError("float reference file changed")
    if any(digest(task / "go-source" / relative) != expected for relative, expected in SOURCES.items()):
        raise ValueError("captured Go owner changed")
    if digest(task / "float_literals.go") != digest(HERE / "capture/float_literals.go"):
        raise ValueError("float source probe changed")
    requests, counts = generate(decode((task / "original-inputs.json").read_bytes()))
    if (requests != decode((task / "requests.json").read_bytes()) or counts != manifest["counts"]
            or len(requests) != manifest["required"] or counts["original_go"] != 264):
        raise ValueError("incomplete float source/input identities")
    reference = decode((task / "reference.json").read_bytes())
    if (set(reference) != {"go_version", "architecture", "runtime_commit", "cases"}
            or reference["go_version"] != PROFILE["GOVERSION"] or reference["architecture"] != PROFILE["GOARCH"]
            or reference["runtime_commit"] != COMMIT or len(reference["cases"]) != len(requests)):
        raise ValueError("float result source/profile contract changed")
    for request, case in zip(requests, reference["cases"], strict=True):
        if (set(case) != {"id", "input", "origin", "value_bits", "error"}
                or any(case[key] != request[key] for key in ("id", "input", "origin"))
                or type(case["value_bits"]) is not str or not re.fullmatch(r"[0-9a-f]{16}", case["value_bits"])
                or case["error"] not in ("", "syntax", "range")):
            raise ValueError("malformed or unbound float result")
        base64.b64decode(case["input"], validate=True)
    if len({case["id"] for case in reference["cases"]}) != len(requests):
        raise ValueError("duplicate float reference identity")
    return reference, manifest


def compare(task, binary):
    reference, manifest = load(task)
    binary = Path(binary).expanduser().resolve(strict=True)
    if not binary.is_file():
        raise ValueError("float candidate must be an existing standalone frontend binary")
    output = directory(tempfile.mkdtemp(prefix="float-literals-compare-", dir=directory(HERE / ".tmp")))
    payload = "".join(f"{case['id']}\tf\t0\t{base64.b64decode(case['input'], validate=True).hex()}\n"
                      for case in reference["cases"]).encode()
    (output / "requests.tsv").write_bytes(payload)
    raw = invoke([binary], output, os.environ.copy(), "candidate", payload)
    actual = {}
    for line in raw.splitlines():
        case = decode(line)
        if (type(case) is not dict or set(case) != {"id", "value_bits", "error"}
                or type(case["id"]) is not str or case["id"] in actual
                or type(case["value_bits"]) is not str or not re.fullmatch(r"[0-9a-f]{16}", case["value_bits"])
                or case["error"] not in ("", "syntax", "range")):
            raise ValueError("malformed or duplicate float candidate result")
        actual[case["id"]] = case
    if set(actual) != {case["id"] for case in reference["cases"]}:
        raise ValueError("missing or extra float candidate identity")
    failures = []
    for case in reference["cases"]:
        expected = {key: case[key] for key in ("id", "value_bits", "error")}
        if expected != actual[case["id"]]:
            failures.append({"input": case["input"], "expected": expected, "actual": actual[case["id"]]})
    report = {"scope": "finite float-only parser qualification", "profile": manifest["profile"],
              "required": len(actual), "passed": len(actual)-len(failures), "failures": failures,
              "status": "FAIL" if failures else "PASS", "binary_sha256": digest(binary),
              "reference_sha256": manifest["files"]["reference.json"], "source_counts": manifest["counts"]}
    (output / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key != "failures"}
                     | {"failures": len(failures), "evidence": str(output)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", help="reuse an unchanged source/input-bound Go capture")
    parser.add_argument("--binary", default=HERE / ".tmp/core-target/release/promql-frontend-fixture",
                        help="existing standalone Rust frontend binary")
    parser.add_argument("--reference-only", action="store_true", help="capture and verify only the original Go reference")
    args = parser.parse_args()
    task = directory(args.capture) if args.capture else capture()
    if args.reference_only:
        load(task)
    else:
        raise SystemExit(compare(task, args.binary)["status"] != "PASS")
