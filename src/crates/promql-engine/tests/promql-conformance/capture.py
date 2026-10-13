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
from compare import checked_results
from upstream_inventory import inventory

HERE = Path(__file__).resolve().parent
BUILTIN_OPTIONS = {"max_samples": 10000, "timeout_ns": 100_000_000_000,
                   "lookback_ns": 300_000_000_000, "subquery_step_ms": 60000,
                   "enable_at": True, "enable_negative_offset": True,
                   "enable_delayed_name_removal": True, "use_start_timestamps": True,
                   "per_step_stats": False, "parser": {"EnableExperimentalFunctions": True,
                       "ExperimentalDurationExpr": True, "EnableExtendedRangeSelectors": True,
                       "EnableBinopFillModifiers": True}}


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


def validate_evaluation(metadata, task, expected_commands):
    if (metadata.get("schema_version") != 1 or metadata.get("original_assertions_passed") is not True
            or metadata.get("fixture_replays_exact") is not True
            or metadata.get("commit") != "5241a27fe3c6983549fccc32f6e65917408c63cd"):
        raise ValueError("evaluation capture lacks passing reference/replay assertions")
    if metadata.get("engine_options") != BUILTIN_OPTIONS:
        raise ValueError("upstream builtin engine options changed")
    commands = {c["id"]: c for c in metadata["commands"]}
    if len(commands) != 2243 or len(metadata["commands"]) != len(commands) or set(commands) != set(expected_commands):
        raise ValueError("incomplete or duplicate source command capture")
    if any(commands[ident]["mode"] != mode for ident, mode in expected_commands.items()):
        raise ValueError("source command query mode changed")
    entries, datasets, variants = 0, {}, {}
    for line in (task / "evaluation.jsonl").open():
        entry = json.loads(line)
        command, request, result = entry["source_command"], entry["request"], entry["result"]
        ident = command["id"]
        ordinal = variants.get(ident, 0)
        if commands.get(ident) != command or entry["variant_ordinal"] != ordinal:
            raise ValueError("unknown command, missing variant or duplicate ordinal")
        variants[ident] = ordinal + 1
        if request["id"] != f"{ident}:variant:{ordinal:04d}" or result["id"] != request["id"]:
            raise ValueError("variant result identity changed")
        if entry["fixture_replay_exact"] is not True:
            raise ValueError("storage fixture replay differs")
        query = base64.b64decode(request["query_bytes"], validate=True)
        if ordinal == 0 and (query != base64.b64decode(command["query"], validate=True)
                             or request["mode"] != command["mode"]):
            raise ValueError("source expression missing from generated variants")
        if request["profile"] != "upstream-tests" or request["mode"] not in {"instant", "range"}:
            raise ValueError("invalid evaluation request profile or mode")
        if (entry.get("query_options") != {"nil": True, "per_step": False, "lookback_ns": 0}
                or request["lookback_ms"] != 300000 or request["subquery_step_ms"] != 60000):
            raise ValueError("effective query options changed")
        context = entry.get("effective_context")
        if result.get("error", {}).get("phase") != "construction":
            expected_context = {"start_ms": request["time_ms"], "end_ms": request["time_ms"],
                                "interval_ns": 0, "lookback_ns": 300_000_000_000}
            if request["mode"] == "range":
                expected_context.update(start_ms=request["start_ms"], end_ms=request["end_ms"],
                                        interval_ns=request["step_ms"] * 1_000_000)
            if context != expected_context:
                raise ValueError("effective query time context changed")
        checked_results({"schema_version": 1, "results": [result]})
        dataset = entry["dataset_id"]
        if not isinstance(dataset, str) or not re.fullmatch("[0-9a-f]{64}", dataset):
            raise ValueError("invalid dataset content identity")
        if dataset not in datasets:
            raw = (task / "datasets" / (dataset + ".json")).read_bytes()
            if hashlib.sha256(raw).hexdigest() != dataset:
                raise ValueError("captured dataset digest changed")
            datasets[dataset] = len(raw)
        entries += 1
    if (entries != metadata["entries"] or len(datasets) != metadata["datasets"]
            or set(variants) != set(commands)):
        raise ValueError("incomplete evaluation variants or datasets")
    return {"source_commands": len(commands), "evaluation_variants": entries,
            "datasets": len(datasets), "dataset_bytes": sum(datasets.values())}


def source_commands(source):
    return {f"testdata/{path.name}:{line}": ("range" if re.match(r"^[ \t]*eval(?:_[a-z]+)?\s+range\s", text) else "instant")
            for path in sorted((source / "promql/promqltest/testdata").glob("*.test"))
            for line, text in enumerate(path.read_text().splitlines(), 1)
            if re.match(r"^[ \t]*eval(?:_[a-z]+)?(?:\s|$)", text)}


def capture(upstream, mod_cache, surface="all"):
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
    facade.chmod(facade.stat().st_mode | 0o200)
    for path in facade.rglob("*"):
        path.chmod(path.stat().st_mode | 0o200)
    hashes = verify_facade(source, facade)
    added = {}
    def add(relative, raw):
        target = facade / relative
        if target.exists():
            raise ValueError(f"capture hook would replace an upstream file: {relative}")
        directory(target.parent if target.parent.exists() else facade)
        target.parent.mkdir(parents=True, exist_ok=True)
        directory(target.parent)
        target.write_bytes(raw)
        added[relative] = raw
    if surface in {"frontend", "all"}:
        add("promql/parser/netdata_capture_test.go", (HERE / "capture/parser_test.go").read_bytes())
        env["NETDATA_PROMQL_CAPTURE_FRONTEND"] = str(task / "frontend.json")
    if surface in {"evaluation", "all"}:
        add("promql/promqltest/netdata_capture_test.go", (HERE / "capture/evaluation_test.go").read_bytes())
        for package, target in [("wire", "netdatawire"), ("codec", "netdatacodec")]:
            for path in sorted((HERE / "oracle" / package).glob("*.go")):
                raw = path.read_bytes().replace(b'"netdata-promql-conformance-oracle/wire"',
                                               b'"github.com/prometheus/prometheus/internal/netdatawire"')
                add(f"internal/{target}/{path.name}", raw)
        env["NETDATA_PROMQL_CAPTURE_EVALUATION"] = str(task)
        private_tmp = task / "tmp"
        private_tmp.mkdir(mode=0o700)
        env["TMPDIR"] = str(directory(private_tmp))
    packages = (["./promql/parser"] if surface == "frontend" else
                ["./promql/promqltest"] if surface == "evaluation" else ["./promql/parser", "./promql/promqltest"])
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-run", "^TestNetdataCapture(Frontend|Evaluation)$",
         *packages], facade, env)
    if any((facade / relative).read_bytes() != raw for relative, raw in added.items()):
        raise ValueError("capture hook changed during execution")
    # Verify production sources again after tests, not just after copying.
    if verify_facade(source, facade) != hashes:
        raise ValueError("reference source changed during capture")
    counts, outputs = {}, {}
    if surface in {"frontend", "all"}:
        frontend_path = task / "frontend.json"
        frontend = json.loads(frontend_path.read_text())
        generated = (source / "promql/parser/generated_parser.y.go").read_text()
        frontend["token_catalog"] = {code: name for name, code in re.findall(
            r"^const (\w+) = (\d+)$", generated, re.M)}
        counts.update(validate_frontend(frontend))
        frontend_path.write_text(json.dumps(frontend, indent=2) + "\n")
        outputs["frontend"] = {"path": "frontend.json", "sha256": hashlib.sha256(frontend_path.read_bytes()).hexdigest()}
    if surface in {"evaluation", "all"}:
        metadata = json.loads((task / "evaluation-metadata.json").read_text())
        counts.update(validate_evaluation(metadata, task, source_commands(source)))
        for name in ["evaluation-metadata.json", "evaluation.jsonl"]:
            with (task / name).open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            outputs[name] = {"path": name, "sha256": digest}
    manifest = {"schema_version": 1, "reference": "prometheus/prometheus",
                "commit": "5241a27fe3c6983549fccc32f6e65917408c63cd",
                "source_sha256": hashes, "added_sha256": {relative: hashlib.sha256(raw).hexdigest() for relative, raw in added.items()},
                "outputs": outputs,
                "counts": counts,
                "qualification": "Original upstream assertions and fixture capture; no Rust candidate pass."}
    (task / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"capture_directory": str(task), **counts}), flush=True)
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, help="existing pinned Prometheus module source")
    parser.add_argument("--mod-cache", required=True, help="existing populated module cache; downloads disabled")
    parser.add_argument("--surface", choices=["all", "frontend", "evaluation"], default="all")
    args = parser.parse_args()
    capture(args.upstream, args.mod_cache, args.surface)
