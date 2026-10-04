#!/usr/bin/env python3
"""Capture computed upstream cases offline without modifying reference sources."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile

from build import directory, run
from upstream_inventory import inventory

HERE = Path(__file__).resolve().parent


def verify_facade(source, facade):
    """Check every reference file, including embeds and module metadata."""
    digests = {}
    for path in sorted(source.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"reference source contains a symlink: {path.relative_to(source)}")
        if not path.is_file():
            continue
        relative = path.relative_to(source)
        raw = path.read_bytes()
        if (facade / relative).read_bytes() != raw:
            raise ValueError(f"source facade differs: {relative}")
        digests[relative.as_posix()] = hashlib.sha256(raw).hexdigest()
    return digests


def validate_frontend(data):
    if data.get("schema_version") != 1 or data.get("original_assertions_passed") is not True:
        raise ValueError("capture has no passing original assertions")
    if data.get("commit") != "5241a27fe3c6983549fccc32f6e65917408c63cd":
        raise ValueError("capture reference changed")
    lexers, expressions = data["lexers"], data["expressions"]
    if len(lexers) != 157 or len(expressions) != 356:
        raise ValueError("incomplete frontend capture")
    if sum(c["declared_failure"] for c in lexers) != 52:
        raise ValueError("lexer failure coverage changed")
    if sum(c["series_description"] for c in lexers) != 9:
        raise ValueError("series-description lexer coverage changed")
    if sum(c["declared_failure"] for c in expressions) != 148:
        raise ValueError("parser failure coverage changed")
    if [c["id"] for c in expressions] != [f"parser:{i:03d}" for i in range(356)]:
        raise ValueError("parser ordinals missing or reordered")
    ids = [c["id"] for c in lexers]
    if len(set(ids)) != len(ids):
        raise ValueError("duplicate lexer ordinal")
    def check_bytes(value):
        if not isinstance(value, str):
            raise ValueError("captured bytes must be base64 strings")
        base64.b64decode(value, validate=True)
    for case in lexers:
        check_bytes(case["input"])
        for token in case["tokens"] + case["declared_tokens"]:
            if str(token["type"]) not in data["token_catalog"]:
                raise ValueError("unknown captured token identity")
            check_bytes(token["value"])
        if case["declared_failure"] and not any(
                data["token_catalog"][str(token["type"])] == "ERROR" for token in case["tokens"]):
            raise ValueError("declared lexer failure has no error token")
        if not case["declared_failure"] and case["tokens"][:-1] != case["declared_tokens"]:
            raise ValueError("captured successful token stream differs from declared expectations")
        if not case["declared_failure"] and (not case["tokens"] or
                data["token_catalog"][str(case["tokens"][-1]["type"])] != "EOF"):
            raise ValueError("successful lexer stream has no terminal EOF")
    for case in expressions:
        check_bytes(case["input"])
        if case["declared_failure"] != bool(case["errors"]):
            raise ValueError("parser success/failure attribution changed")
        if not case["declared_failure"] and (case["ast"] is None or case["declared_ast"] is None):
            raise ValueError("successful parser case has no AST")
        for error in case["errors"] + case["declared_errors"]:
            check_bytes(error["message"])
            if "query_ref" in error:
                if error["query_ref"] != "input" or "query" in error:
                    raise ValueError("invalid parser-error query reference")
            else:
                check_bytes(error["query"])
        if case["errors"] != case["declared_errors"]:
            raise ValueError("captured parser errors differ from declared expectations")
        if not case["declared_failure"] and case["ast"] != case["declared_ast"]:
            raise ValueError("captured AST differs from declared expectations")
    return {"lexer_cases": len(lexers), "parser_cases": len(expressions),
            "lexer_declared_failures": 52, "parser_failures": 148}


def capture(upstream, mod_cache):
    source, cache = directory(upstream), directory(mod_cache)
    if source != directory(cache / "github.com/prometheus/prometheus@v0.315.0"):
        raise ValueError("source must be the pinned module in the verified module cache")
    inventory(source)
    output = HERE / ".tmp"
    output.mkdir(exist_ok=True)
    directory(output)
    task = directory(tempfile.mkdtemp(prefix="capture-", dir=output))
    facade = task / "reference"
    env = os.environ.copy()
    env.update(GOMODCACHE=str(cache), GOCACHE=str(output / "go-build"), GOPROXY="off",
               GOSUMDB="off", GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="-mod=readonly")
    run(["go", "mod", "verify"], HERE / "oracle", env)
    print(f"Copying reference source into private facade: {facade}", flush=True)
    shutil.copytree(source, facade)
    directory(facade)
    # The Go module cache is deliberately read-only to the capture workflow.
    for path in facade.rglob("*"):
        path.chmod(path.stat().st_mode | 0o200)
    hashes = verify_facade(source, facade)
    hook = HERE / "capture" / "parser_test.go"
    target = facade / "promql/parser/netdata_capture_test.go"
    if target.exists():
        raise ValueError("capture hook would replace an upstream file")
    hook_bytes = hook.read_bytes()
    target.write_bytes(hook_bytes)
    env["NETDATA_PROMQL_CAPTURE_FRONTEND"] = str(task / "frontend.json")
    run(["go", "test", "-p", "4", "-count=1", "-run", "^TestNetdataCaptureFrontend$",
         "./promql/parser"], facade, env)
    if target.read_bytes() != hook_bytes:
        raise ValueError("capture hook changed during execution")
    # Verify production sources again after tests, not just after copying.
    if verify_facade(source, facade) != hashes:
        raise ValueError("reference source changed during capture")
    frontend_path = task / "frontend.json"
    frontend = json.loads(frontend_path.read_text())
    generated = (source / "promql/parser/generated_parser.y.go").read_text()
    frontend["token_catalog"] = {code: name for name, code in re.findall(
        r"^const (\w+) = (\d+)$", generated, re.M)}
    counts = validate_frontend(frontend)
    frontend_path.write_text(json.dumps(frontend, indent=2) + "\n")
    manifest = {"schema_version": 1, "reference": "prometheus/prometheus",
                "commit": "5241a27fe3c6983549fccc32f6e65917408c63cd",
                "source_sha256": hashes, "hook_sha256": hashlib.sha256(hook_bytes).hexdigest(),
                "frontend": "frontend.json", "frontend_sha256": hashlib.sha256(frontend_path.read_bytes()).hexdigest(),
                "counts": counts,
                "qualification": "Original upstream assertions and fixture capture; no Rust candidate pass."}
    (task / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"capture_directory": str(task), **counts}), flush=True)
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, help="existing pinned Prometheus module source")
    parser.add_argument("--mod-cache", required=True, help="existing populated module cache; downloads disabled")
    args = parser.parse_args()
    capture(args.upstream, args.mod_cache)
