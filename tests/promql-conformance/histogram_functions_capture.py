#!/usr/bin/env python3
"""Capture native histogram helper owners and original Go/DSL assertions offline."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile

from build import directory, run
from capture import verify_facade
from histogram_capture import COMMIT, MODULE
from histogram_reference import input_ledger as histogram_ledger
from upstream_inventory import inventory

HERE = Path(__file__).resolve().parent
HOOKS = {"promql": "promql/netdata_histogram_functions_capture_test.go",
         "promqltest": "promql/promqltest/netdata_histogram_functions_capture_test.go"}
TEMPLATES = ("histogram_functions_common.go.in", "histogram_functions_dsl.go.in",
             "histogram_functions_supplemental.go.in")
TEST = "TestTrimBuckets_HistogramFractionCrossCheck"


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def replace(source, anchor, replacement):
    if source.count(anchor) != 1:
        raise ValueError("changed native helper source observation anchor")
    return source.replace(anchor, replacement)


def instrument(definition):
    source = definition["body"]
    source = replace(source, f"func {TEST}(t *testing.T) {{", f"func netdata{TEST}(t *testing.T, output *[]map[string]any) {{")
    source = replace(source, "for _, tc := range testCases {", "for netdataCase, tc := range testCases {")
    source = replace(source, "for _, x := range tc.thresholds {", "for netdataThreshold, x := range tc.thresholds {")
    for direction, call, bucket, line, lower, upper in (
        ("below", "fractionBelow", "lower", 439, "math.Inf(-1)", "x"),
        ("above", "fractionAbove", "upper", 448, "x", "math.Inf(1)"),
    ):
        anchor = f'{call}, annos := HistogramFraction({lower}, {upper}, tc.h, "", posrange.PositionRange{{}})'
        source = replace(source, anchor,
                         f'netdataRequest{direction} := netdataHelperRequest(tc.h, {lower}, {upper}, false, "", "", posrange.PositionRange{{}})\n' + anchor)
        anchor = f"require.InEpsilon(t, {call}*total, {bucket}.Count, 1e-9)\n\t\t\t\t\t}}"
        source = replace(source, anchor, anchor + f'''
            netdataHelperRecord(output, fmt.Sprintf("{TEST}:%03d:%03d:{direction}", netdataCase, netdataThreshold),
                netdataRequest{direction}, tc.h, {call}, annos, "", false,
                map[string]any{{"comparison": "trim_crosscheck", "trim_count": netdataHelperBits({bucket}.Count),
                    "total": netdataHelperBits(total), "annotations_empty": true}},
                map[string]any{{"test": "{TEST}", "call_line": {line}, "case_index": netdataCase,
                    "threshold_index": netdataThreshold, "direction": "{direction}"}})''')
    anchor = 'fraction, annos := HistogramFraction(math.Inf(-1), 0.5, h, "", posrange.PositionRange{})'
    source = replace(source, anchor, 'request := netdataHelperRequest(h, math.Inf(-1), 0.5, false, "", "", posrange.PositionRange{})\n' + anchor)
    anchor = "require.True(t, math.IsNaN(fraction))"
    source = replace(source, anchor, anchor + f'''
        netdataHelperRecord(output, "{TEST}:zero", request, h, fraction, annos, "", false,
            map[string]any{{"comparison": "nan", "annotations_empty": true}},
            map[string]any{{"test": "{TEST}", "call_line": 462, "case_index": -1,
                "threshold_index": -1, "direction": "zero"}})''')
    source = replace(source, "for _, tc := range divergences {", "for netdataIndex, tc := range divergences {")
    anchor = 'fraction, annos := HistogramFraction(lower, upper, tc.h, "", posrange.PositionRange{})'
    source = replace(source, anchor, 'request := netdataHelperRequest(tc.h, lower, upper, false, "", "", posrange.PositionRange{})\n' + anchor)
    anchor = 'require.Equal(t, tc.expectedFraction, fraction, "HistogramFraction handles the %s differently", tc.why)'
    return replace(source, anchor, anchor + f'''
        netdataHelperRecord(output, fmt.Sprintf("{TEST}:divergence:%03d", netdataIndex), request, tc.h, fraction, annos, "", false,
            map[string]any{{"comparison": "exact", "value": netdataHelperBits(tc.expectedFraction), "annotations_empty": true}},
            map[string]any{{"test": "{TEST}", "call_line": 529, "case_index": netdataIndex,
                "threshold_index": -1, "direction": "divergence"}})''')


def generate_hook(package, definitions):
    template = (HERE / "capture" / TEMPLATES[0]).read_text()
    if package == "promql":
        imports = ""
        aliases = "var netdataHelperFraction = HistogramFraction\nvar netdataHelperQuantile = HistogramQuantile"
        functions = instrument(definitions[TEST]) + "\n\n" + (HERE / "capture" / TEMPLATES[2]).read_text()
        body = f'''t.Run("{TEST}", {TEST})
            if t.Failed() {{ t.Fatal("original fraction assertions failed") }}
            cases := []map[string]any{{}}
            netdata{TEST}(t, &cases)
            netdataHelperSupplemental(&cases)
            netdataHelperWrite(t, cases, []any{{}})'''
    elif package == "promqltest":
        imports = '''"io/fs"
            "time"
            "github.com/prometheus/prometheus/model/labels"
            "github.com/prometheus/prometheus/schema"
            "github.com/prometheus/prometheus/promql"
            "github.com/prometheus/prometheus/promql/parser"'''
        aliases = "var netdataHelperFraction = promql.HistogramFraction\nvar netdataHelperQuantile = promql.HistogramQuantile"
        functions = (HERE / "capture" / TEMPLATES[1]).read_text()
        body = '''raw, err := fs.ReadFile(testsFs, "testdata/native_histograms.test")
            require.NoError(t, err)
            engine := NewTestEngine(t, false, 5*time.Minute, DefaultMaxSamplesPerQuery)
            t.Run("original_native_histograms", func(t *testing.T) { RunTest(t, string(raw), engine) })
            if t.Failed() { t.Fatal("original native histogram DSL assertions failed") }
            cases := []map[string]any{}
            commands := netdataHelperCaptureDSL(t, string(raw), engine, &cases)
            netdataHelperWrite(t, cases, commands)'''
    else:
        raise ValueError("unknown native helper capture package")
    for name, value in {"PACKAGE": package, "IMPORTS": imports, "HELPER_ALIASES": aliases,
                        "CAPTURE_FUNCTIONS": functions, "CAPTURE_BODY": body}.items():
        marker = "/* " + name + " */"
        if marker not in template:
            raise ValueError("missing native helper capture marker")
        template = template.replace(marker, value)
    if "/* " in template:
        raise ValueError("unfilled native helper capture marker")
    return template.encode()


def capture(mod_cache, write_input_ledger=False):
    cache = directory(mod_cache)
    source = directory(cache / MODULE)
    inventory(source)
    parent = directory(HERE / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="histogram-functions-capture-", dir=parent))
    private_tmp = task / "tmp"
    private_tmp.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(GOMODCACHE=str(cache), GOCACHE=str(parent / "go-build"), GOPROXY="off", GOSUMDB="off",
               GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="-mod=readonly", GOAMD64="v1",
               TMPDIR=str(directory(private_tmp)), GOTMPDIR=str(private_tmp))
    profile = json.loads(subprocess.check_output(["go", "env", "-json", "GOVERSION", "GOOS", "GOARCH",
                                                "GOAMD64", "GOEXPERIMENT"], cwd=HERE / "oracle", env=env))
    if profile != {"GOVERSION": "go1.27.1", "GOOS": "linux", "GOARCH": "amd64", "GOAMD64": "v1", "GOEXPERIMENT": ""}:
        raise ValueError("unexpected native helper compiler profile")
    run(["go", "mod", "verify"], HERE / "oracle", env)
    facade = task / "reference"
    shutil.copytree(source, facade)
    directory(facade)
    for path in [facade, *facade.rglob("*")]:
        if path.is_dir():
            path.chmod(path.stat().st_mode | 0o200)
    manifest = verify_facade(source, facade)
    if digest(manifest) != histogram_ledger()["source_manifest_sha256"]:
        raise ValueError("native helper reference pin changed")
    generator = HERE / "capture/histogram_functions_sources.go"
    command = ["go", "run", generator, facade]
    print(f"{HERE / 'oracle'} > {shlex.join(map(str, command))}", flush=True)
    sources = json.loads(subprocess.check_output(command, cwd=HERE / "oracle", env=env))
    definitions, source_commands = sources["definitions"], sources["commands"]
    hooks = {key: generate_hook(key, definitions) for key in HOOKS}
    for package, relative in HOOKS.items():
        destination = facade / relative
        if destination.exists():
            raise ValueError("native helper hook would replace reference source")
        destination.write_bytes(hooks[package])
        env["NETDATA_HISTOGRAM_FUNCTIONS_" + package] = str(task / (package + ".json"))
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-run", "^TestNetdataCaptureHistogramFunctions$",
         "./promql", "./promql/promqltest"], facade, env)
    if verify_facade(source, facade) != manifest:
        raise ValueError("native helper source changed during capture")
    files = {path.relative_to(facade).as_posix() for path in facade.rglob("*") if path.is_file()}
    if files != manifest.keys() | set(HOOKS.values()) or any(path.is_symlink() for path in facade.rglob("*")):
        raise ValueError("unlisted native helper facade source")
    if any((facade / HOOKS[key]).read_bytes() != value for key, value in hooks.items()):
        raise ValueError("native helper hook changed during execution")
    documents = {key: json.loads((task / (key + ".json")).read_bytes()) for key in HOOKS}
    ledger = {"schema_version": 1, "commit": COMMIT, "module": MODULE, "source_manifest_sha256": digest(manifest),
              "definitions": definitions, "source_commands": source_commands, "commands": documents["promqltest"]["commands"],
              "cases": [{key: row[key] for key in ("id", "mode", "request", "declared", "source")}
                        for data in documents.values() for row in data["cases"]]}
    ledger_path = HERE / "histogram-functions-original-inputs.json"
    if not write_input_ledger:
        previous = json.loads(ledger_path.read_bytes())
        if ledger != previous:
            raise ValueError("native helper input bindings changed")
        ledger = previous
    from histogram_functions_reference import validate_cases
    counts = validate_cases(documents, ledger)
    if write_input_ledger:
        if ledger_path.exists():
            previous = json.loads(ledger_path.read_bytes())
            bindings = {row["id"]: row for row in ledger["cases"]}
            if any(bindings.get(row["id"]) != row for row in previous["cases"]):
                raise ValueError("earlier native helper bindings changed")
        ledger_path.write_text(json.dumps(ledger, indent=2) + "\n")
    (task / "definitions.json").write_text(json.dumps(definitions, indent=2) + "\n")
    (task / "source-commands.json").write_text(json.dumps(source_commands, indent=2) + "\n")
    (task / "source-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    metadata = {"schema_version": 1, "commit": COMMIT, "profile": profile, "counts": counts,
                "generator_sha256": hashlib.sha256(generator.read_bytes()).hexdigest(),
                "templates": {name: hashlib.sha256((HERE / "capture" / name).read_bytes()).hexdigest() for name in TEMPLATES},
                "definitions_sha256": digest(definitions), "source_commands_sha256": digest(source_commands), "source_files": len(manifest),
                "hooks": {key: hashlib.sha256(value).hexdigest() for key, value in hooks.items()},
                "outputs": {key: hashlib.sha256((task / (key + ".json")).read_bytes()).hexdigest() for key in HOOKS}}
    (task / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(json.dumps({"capture": str(task), "counts": counts}), flush=True)
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mod-cache", required=True, help="existing pinned module cache; downloads disabled")
    parser.add_argument("--write-input-ledger", action="store_true", help="explicitly extend verified independent Go bindings")
    args = parser.parse_args()
    capture(args.mod_cache, args.write_input_ledger)
