#!/usr/bin/env python3
"""Capture and qualify public parser regressions in all sixteen option combinations."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from build import directory, run
from capture import verify_facade
from parser import difference
from upstream_inventory import inventory

HERE = Path(__file__).resolve().parent
REQUESTS = HERE / "parser-requests.json"


def capture(cache):
    cache = directory(cache)
    source = directory(cache / "github.com/prometheus/prometheus@v0.315.0")
    inventory(source)
    parent = HERE / ".tmp"
    parent.mkdir(exist_ok=True)
    directory(parent)
    task = directory(tempfile.mkdtemp(prefix="parser-cases-", dir=parent))
    facade = task / "reference"
    shutil.copytree(source, facade)
    directory(facade)
    facade.chmod(facade.stat().st_mode | 0o200)
    for path in facade.rglob("*"):
        path.chmod(path.stat().st_mode | 0o200)
    hashes = verify_facade(source, facade)
    hook = facade / "promql/parser/netdata_capture_test.go"
    if hook.exists():
        raise ValueError("additional hook would replace a reference source")
    raw = (HERE / "capture/parser_test.go").read_bytes()
    hook.write_bytes(raw)
    env = os.environ.copy()
    destination = task / "parser-cases.json"
    env.update(GOMODCACHE=str(cache), GOCACHE=str(parent / "go-build"), GOPROXY="off", GOSUMDB="off",
               GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="-mod=readonly",
               NETDATA_PROMQL_CAPTURE_PARSER_CASES=str(destination), NETDATA_PROMQL_PARSER_REQUESTS=str(REQUESTS))
    run(["go", "mod", "verify"], HERE / "oracle", env)
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-run",
         "^TestNetdataCaptureParserCases$", "./promql/parser"], facade, env)
    if verify_facade(source, facade) != hashes or hook.read_bytes() != raw:
        raise ValueError("reference or hook changed during parser capture")
    data = load(destination)
    print(json.dumps({"capture": str(destination), "cases": len(data["cases"])}))
    return destination


def load(path):
    data = json.loads(Path(path).read_bytes())
    requests = json.loads(REQUESTS.read_bytes())
    if (data["schema_version"] != 1 or data["commit"] != "5241a27fe3c6983549fccc32f6e65917408c63cd"
            or data["go_version"] != "go1.27.1" or data["unicode_version"] != "17.0.0" or data["go_arch"] != "amd64"
            or data["requests_sha256"] != hashlib.sha256(REQUESTS.read_bytes()).hexdigest()):
        raise ValueError("additional parser capture changed")
    expected = [(flags, index, query.encode()) for flags in range(16) for index, query in enumerate(requests)]
    if len(data["cases"]) != len(expected):
        raise ValueError("incomplete parser profile coverage")
    for case, (flags, index, query) in zip(data["cases"], expected, strict=True):
        if (case["id"] != f"additional:{flags:02d}:{index:03d}" or case["flags"] != flags
                or base64.b64decode(case["input"], validate=True) != query):
            raise ValueError("parser case identity or input changed")
        options = case["parser_options"]
        if any(options[field] is not bool(flags & bit) for field, bit in (
                ("EnableExperimentalFunctions", 1), ("EnableBinopFillModifiers", 2),
                ("EnableExtendedRangeSelectors", 4), ("ExperimentalDurationExpr", 8))):
            raise ValueError("effective parser options changed")
    return data


def qualify(path):
    data = load(path)
    requests = [f'{case["id"]}\tp\t{case["flags"]}\t' + base64.b64decode(case["input"], validate=True).hex()
                for case in data["cases"]]
    candidate = HERE / ".tmp/core-target/release/promql-frontend-fixture"
    result = subprocess.run([candidate], input=("\n".join(requests)+"\n").encode(), capture_output=True, timeout=600)
    if result.returncode:
        raise RuntimeError(f"parser candidate failed ({result.returncode}): {result.stderr.decode(errors='replace')}")
    actual = {}
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if item["id"] in actual:
            raise ValueError("duplicate additional parser result")
        actual[item["id"]] = item
    if set(actual) != {case["id"] for case in data["cases"]}:
        raise ValueError("missing or extra additional parser result")
    failures = []
    for case in data["cases"]:
        diff = difference({key: case[key] for key in ("ast", "errors")},
                          {key: actual[case["id"]][key] for key in ("ast", "errors")})
        if diff:
            failures.append({"id": case["id"], "input": case["input"], "difference": diff})
    (HERE / ".tmp/parser-additional-differences.json").write_text(json.dumps(failures, indent=2)+"\n")
    print(json.dumps({"cases": len(data["cases"]), "exact_passed": len(data["cases"])-len(failures),
                      "failed": len(failures), "first_failure_ids": [item["id"] for item in failures[:20]],
                      "failure_report": ".tmp/parser-additional-differences.json"}))
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--mod-cache", help="capture from the existing pinned cache without downloads")
    cli.add_argument("--reference", help="qualify a capture; defaults to the committed public regression fixture")
    args = cli.parse_args()
    if args.mod_cache and args.reference:
        cli.error("choose --mod-cache or --reference")
    if args.mod_cache:
        capture(args.mod_cache)
    else:
        qualify(args.reference or HERE / "parser-cases.json")
