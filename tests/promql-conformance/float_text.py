#!/usr/bin/env python3
"""Qualify three float text projections against original Go and Prometheus owners."""
import argparse
import json
import os
import re
import shutil
import tempfile
from pathlib import Path

from build import directory
from float_literals import COMMIT, SOURCES, decode, digest, invoke
from float_text_cases import generate
from numerical_reference import PROFILE

HERE = Path(__file__).resolve().parent
PROMETHEUS_COMMIT = "5241a27fe3c6983549fccc32f6e65917408c63cd"
GO_SOURCES = SOURCES | {
    "src/internal/strconv/ftoa.go": "93b2dbb223f9c8928ab3c3b6f44a74de0378cf64f56e978c3431c9554de5d9be",
    "src/internal/strconv/ftoa_test.go": "69c768e6087c92ada29d22e3696ff503ca74c05705816e7f3f4cb7babab29c2d",
    "src/internal/strconv/itoa.go": "d84e249b4334bab64a916061a45366a2d77becf6f4e5cf9d917527efddfc245f",
    "src/fmt/print.go": "10d7cd625d83a15f70b6e61641d3cebd4e4d49d7e0f2a24d5d2b59988074d2f7",
    "src/fmt/format.go": "a668c0d6bb23f6ac929b73ef661a65be2225a9f665052a2ce02c4c58339c3335",
}
PROMETHEUS_SOURCES = {
    "model/labels/float.go": "fb5a202ed1969cab61f917958f4ae143c3bd8c8bab1d5187c62177c3ed2076d4",
    "util/annotations/annotations.go": "3fb4231061c241cacaaf79ad4784d3bcf95fb99abf902a29c143ed03b8f65c77",
    "util/annotations/annotations_test.go": "350c3034eb78c1431cf560dc92b163d5119c5fda1516ee6c5b9eec6d5c7f93b6",
    "promql/parser/posrange/posrange.go": "11534f13914e8e3a71b0562916bc1ce5ecbef978f4375afbabb12ac73beb4e02",
    "go.mod": "94e39d68d3abd10168c4b1944a63356a1d9a467ddeceee46aea75fc14e4381d1",
    "go.sum": "2b0f11cdccdc6605922c8f48d7afc68ae44fdc90df6b5d2f6f677b447b07d99e",
    "LICENSE": "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4",
}
MODES = {"shortest": 0, "two": 1, "openmetrics": 2}
REQUIRED_FILES = {"float_text.go", "float_text_extract.go", "original.go", "extract-go", "format-go",
                  "original-inputs.json", "requests.json", "reference.json", "go.mod", "go.sum",
                  "original-tests.stdout", "original-tests.stderr"}


def original_requests(original):
    if set(original) != {"rows", "selected"} or len(original["rows"]) != 148:
        raise ValueError("incomplete original formatting assertions")
    selected = []
    for index, row in enumerate(original["rows"]):
        if (set(row) != {"index", "line", "expression", "bits", "format", "precision",
                         "declared", "text", "append_text"}
                or row["index"] != index or type(row["line"]) is not int or row["line"] < 1
                or type(row["precision"]) is not int or not row["expression"]
                or not re.fullmatch(r"[0-9a-f]{16}", row["bits"])
                or row["text"] != row["declared"] or row["append_text"] != "abc" + row["declared"]):
            raise ValueError("malformed original formatting assertion")
        if row["format"] == "g" and row["precision"] in (-1, 2):
            selected.append({"id": f"original-go-{index:03d}", "bits": row["bits"],
                             "mode": "shortest" if row["precision"] == -1 else "two",
                             "origin": f"ftoa_test.go:{row['line']}:{index}"})
    if len(selected) != 38 or selected != original["selected"]:
        raise ValueError("original formatting selection changed")
    return selected


def capture():
    parent = HERE / ".tmp"
    parent.mkdir(exist_ok=True)
    parent = directory(parent)
    task = directory(tempfile.mkdtemp(prefix="float-text-capture-", dir=parent))
    private = task / "tmp"
    private.mkdir(mode=0o700)
    cache = directory(HERE.parents[1].parent / "netdata/tests/promql-prototype/oracle/.cache/go-mod")
    prometheus = directory(cache / "github.com/prometheus/prometheus@v0.315.0")
    env = os.environ.copy()
    env.update(GOTOOLCHAIN="local", GO111MODULE="on", GOWORK="off", GOFLAGS="-mod=readonly", GOAMD64="v1",
               GOPROXY="off", GOSUMDB="off", GOCACHE=str(parent / "go-build"), GOMODCACHE=str(cache),
               TMPDIR=str(directory(private)), GOTMPDIR=str(private))
    for name in ("go.mod", "go.sum"):
        shutil.copyfile(HERE / "oracle" / name, task / name)
    profile = decode(invoke(["go", "env", "-json", "GOROOT", *PROFILE], task, env, "profile"))
    runtime = directory(profile.pop("GOROOT"))
    if profile != PROFILE:
        raise ValueError("float rendering reference requires the pinned Go 1.27.1 / AMD64 profile")
    for base, label, sources in ((runtime, "go-source", GO_SOURCES), (prometheus, "prometheus-source", PROMETHEUS_SOURCES)):
        for relative, expected in sources.items():
            if digest(base / relative) != expected:
                raise ValueError(f"pinned original owner changed: {label}/{relative}")
            destination = task / label / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(base / relative, destination)
    for name in ("float_text.go", "float_text_extract.go"):
        shutil.copyfile(HERE / "capture" / name, task / name)
    invoke(["go", "test", "-count=1", "-p", "4", "internal/strconv", "-v", "-run", "^TestFtoa$"],
           task, env, "original-tests")
    invoke(["go", "build", "-p", "4", "-o", task / "extract-go", task / "float_text_extract.go"], task, env, "extract-build")
    invoke([task / "extract-go", task / "go-source/src/internal/strconv/ftoa_test.go", task / "original.go"],
           task, env, "extract")
    invoke(["go", "build", "-p", "4", "-o", task / "format-go", task / "float_text.go", task / "original.go"],
           task, env, "build")
    original = invoke([task / "format-go", "--cases"], task, env, "original")
    (task / "original-inputs.json").write_bytes(original)
    requests, counts = generate(original_requests(decode(original)))
    payload = json.dumps(requests, sort_keys=True, separators=(",", ":")).encode() + b"\n"
    (task / "requests.json").write_bytes(payload)
    reference = invoke([task / "format-go"], task, env, "reference", payload)
    (task / "reference.json").write_bytes(reference)
    for base, sources in ((runtime, GO_SOURCES), (prometheus, PROMETHEUS_SOURCES)):
        if any(digest(base / relative) != expected for relative, expected in sources.items()):
            raise ValueError("original formatting source changed during capture")
    manifest = {"schema_version": 1, "runtime_commit": COMMIT, "prometheus_commit": PROMETHEUS_COMMIT,
                "profile": profile, "counts": counts, "required": len(requests),
                "go_sources": GO_SOURCES, "prometheus_sources": PROMETHEUS_SOURCES,
                "generator_sha256": digest(HERE / "float_text_cases.py"),
                "files": {name: digest(task / name) for name in REQUIRED_FILES}}
    (task / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    load(task)
    print(json.dumps({"capture": str(task), "required": len(requests), "counts": counts}), flush=True)
    return task


def load(task):
    task = directory(task)
    manifest = decode((task / "manifest.json").read_bytes())
    if (manifest["schema_version"] != 1 or manifest["runtime_commit"] != COMMIT
            or manifest["prometheus_commit"] != PROMETHEUS_COMMIT or manifest["profile"] != PROFILE
            or manifest["go_sources"] != GO_SOURCES or manifest["prometheus_sources"] != PROMETHEUS_SOURCES
            or manifest["generator_sha256"] != digest(HERE / "float_text_cases.py")
            or set(manifest["files"]) != REQUIRED_FILES):
        raise ValueError("float formatting reference contract changed")
    if any(digest(task / name) != expected for name, expected in manifest["files"].items()):
        raise ValueError("float formatting reference file changed")
    for label, sources in (("go-source", GO_SOURCES), ("prometheus-source", PROMETHEUS_SOURCES)):
        if any(digest(task / label / name) != expected for name, expected in sources.items()):
            raise ValueError("captured float formatting owner changed")
    for name in ("float_text.go", "float_text_extract.go"):
        if digest(task / name) != digest(HERE / "capture" / name):
            raise ValueError("float formatting source probe changed")
    for name in ("go.mod", "go.sum"):
        if digest(task / name) != digest(HERE / "oracle" / name):
            raise ValueError("float formatting module lock changed")
    original = decode((task / "original-inputs.json").read_bytes())
    requests, counts = generate(original_requests(original))
    if (requests != decode((task / "requests.json").read_bytes()) or counts != manifest["counts"]
            or len(requests) != manifest["required"]):
        raise ValueError("incomplete float formatting source/input identities")
    reference = decode((task / "reference.json").read_bytes())
    if (set(reference) != {"go_version", "architecture", "runtime_commit", "prometheus_commit", "cases"}
            or reference["go_version"] != PROFILE["GOVERSION"] or reference["architecture"] != PROFILE["GOARCH"]
            or reference["runtime_commit"] != COMMIT or reference["prometheus_commit"] != PROMETHEUS_COMMIT
            or len(reference["cases"]) != len(requests)):
        raise ValueError("float formatting result source/profile contract changed")
    for req, case in zip(requests, reference["cases"], strict=True):
        if (set(case) != {"id", "bits", "mode", "origin", "text", "roundtrip_bits", "roundtrip_error"}
                or any(case[key] != req[key] for key in req) or type(case["text"]) is not str
                or not case["text"].isascii() or not 1 <= len(case["text"]) <= 32
                or not re.fullmatch(r"[0-9a-f]{16}", case["roundtrip_bits"])
                or case["roundtrip_error"] not in ("", "syntax", "range")):
            raise ValueError("malformed or unbound float formatting result")
        bits = int(req["bits"], 16)
        is_nan = bits & 0x7fffffffffffffff > 0x7ff0000000000000
        if req["mode"] != "two" and not is_nan:
            expected = 0 if req["mode"] == "openmetrics" and bits & 0x7fffffffffffffff == 0 else bits
            if case["roundtrip_error"] or int(case["roundtrip_bits"], 16) != expected:
                raise ValueError("shortest formatting does not preserve the original binary64 value")
    for req, row in zip(requests[:38], [r for r in original["rows"] if r["format"] == "g" and r["precision"] in (-1, 2)], strict=True):
        case = reference["cases"][requests.index(req)]
        if case["text"] != row["declared"]:
            raise ValueError("original formatting expectation differs from the reference")
    if len({case["id"] for case in reference["cases"]}) != len(requests):
        raise ValueError("duplicate float formatting reference identity")
    return reference, manifest


def compare(task, binary):
    reference, manifest = load(task)
    binary = Path(binary).expanduser().resolve(strict=True)
    if not binary.is_file():
        raise ValueError("float formatting candidate must be an existing standalone frontend binary")
    output = directory(tempfile.mkdtemp(prefix="float-text-compare-", dir=directory(HERE / ".tmp")))
    payload = "".join(f"{case['id']}\tg\t{MODES[case['mode']]}\t{case['bits']}\n" for case in reference["cases"]).encode()
    (output / "requests.tsv").write_bytes(payload)
    binary_sha256 = digest(binary)
    raw = invoke([binary], output, os.environ.copy(), "candidate", payload)
    if digest(binary) != binary_sha256:
        raise ValueError("float formatting candidate changed during comparison")
    actual = {}
    for line in raw.splitlines():
        case = decode(line)
        if (type(case) is not dict or set(case) != {"id", "text", "roundtrip_bits", "roundtrip_error"}
                or type(case["id"]) is not str or case["id"] in actual or type(case["text"]) is not str
                or not re.fullmatch(r"[0-9a-f]{16}", case["roundtrip_bits"])
                or case["roundtrip_error"] not in ("", "syntax", "range")):
            raise ValueError("malformed or duplicate float formatting candidate result")
        actual[case["id"]] = case
    if set(actual) != {case["id"] for case in reference["cases"]}:
        raise ValueError("missing or extra float formatting candidate identity")
    failures = []
    for case in reference["cases"]:
        expected = {key: case[key] for key in ("id", "text", "roundtrip_bits", "roundtrip_error")}
        if expected != actual[case["id"]]:
            failures.append({"bits": case["bits"], "mode": case["mode"], "expected": expected, "actual": actual[case["id"]]})
    report = {"scope": "finite source-derived binary64 text and roundtrip qualification", "profile": manifest["profile"],
              "required": len(actual), "passed": len(actual) - len(failures), "failures": failures,
              "status": "FAIL" if failures else "PASS", "binary_sha256": binary_sha256,
              "reference_sha256": manifest["files"]["reference.json"], "source_counts": manifest["counts"]}
    (output / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key != "failures"}
                     | {"failures": len(failures), "evidence": str(output)}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", help="reuse an unchanged source/input-bound original reference")
    parser.add_argument("--binary", default=HERE / ".tmp/core-target/release/promql-frontend-fixture",
                        help="existing standalone Rust frontend binary")
    parser.add_argument("--reference-only", action="store_true", help="capture and verify only the original reference")
    args = parser.parse_args()
    task = directory(args.capture) if args.capture else capture()
    if args.reference_only:
        load(task)
    else:
        raise SystemExit(compare(task, args.binary)["status"] != "PASS")
