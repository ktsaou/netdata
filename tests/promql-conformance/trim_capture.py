#!/usr/bin/env python3
"""Capture original fraction/trim assertions in two unchanged pinned Go packages."""
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
HOOKS = {"histogram": "model/histogram/netdata_trim_capture_test.go", "promql": "promql/netdata_trim_capture_test.go"}
TESTS = {"histogram": ("TestBucketFractionBelow", "TestFloatHistogramTrimBucketsInPlace"),
         "promql": ("TestTrimBuckets_HistogramFractionCrossCheck",)}


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def replace(source, anchor, replacement):
    if source.count(anchor) != 1:
        raise ValueError("changed trim source observation anchor")
    return source.replace(anchor, replacement)


def instrument(name, definition):
    source = definition["body"]
    source = replace(source, f"func {name}(t *testing.T) {{", f"func netdata{name}(t *testing.T, output *[]map[string]any) {{")
    if name == "TestBucketFractionBelow":
        source = replace(source, "for _, s := range scenarios {", "for netdataIndex, s := range scenarios {")
        anchor = "require.InDelta(t, s.want, s.bucket.FractionBelow(s.v, s.linear), 1e-12)"
        observation = '''
            request := map[string]any{"bucket": map[string]any{"lower": netdataTrimNumber(s.bucket.Lower),
                "upper": netdataTrimNumber(s.bucket.Upper), "count": netdataTrimNumber(s.bucket.Count),
                "index": s.bucket.Index, "lower_inclusive": s.bucket.LowerInclusive, "upper_inclusive": s.bucket.UpperInclusive},
                "value": netdataTrimNumber(s.v), "linear": s.linear}
            netdataTrimRecord(output, fmt.Sprintf("TestBucketFractionBelow:%03d", netdataIndex), "bucket-fraction",
                request, netdataTrimNumber(s.bucket.FractionBelow(s.v, s.linear)),
                map[string]any{"comparison": "absolute", "value": netdataTrimNumber(s.want)})'''
        return replace(source, anchor, anchor + observation)
    if name == "TestFloatHistogramTrimBucketsInPlace":
        source = replace(source, "got := h.TrimBuckets(4, true)", "request := netdataTrimRequest(h, 4, true, false)\n\t\tgot := h.TrimBuckets(4, true)")
        anchor = "require.NotEqual(t, newHist(), h)"
        source = replace(source, anchor, anchor + '''
            netdataTrimRecord(output, "TestFloatHistogramTrimBucketsInPlace:000", "float-trim", request,
                netdataTrimActual(h, h, got, ""), map[string]any{"comparison": "lifecycle", "same_receiver": true,
                    "preserved_input": nil, "changed": true})''')
        source = replace(source, "trimmed := h.Copy().TrimBuckets(4, true)", "request := netdataTrimRequest(h, 4, true, true)\n\t\ttrimmed := h.Copy().TrimBuckets(4, true)")
        anchor = "require.NotEqual(t, h, trimmed)"
        return replace(source, anchor, anchor + '''
            netdataTrimRecord(output, "TestFloatHistogramTrimBucketsInPlace:001", "float-trim", request,
                netdataTrimActual(h, trimmed, trimmed, ""), map[string]any{"comparison": "lifecycle", "same_receiver": nil,
                    "preserved_input": true, "changed": true})''')
    if name != "TestTrimBuckets_HistogramFractionCrossCheck":
        raise ValueError("unknown trim source test")
    source = replace(source, "for _, tc := range testCases {", "for netdataCase, tc := range testCases {")
    source = replace(source, "for _, x := range tc.thresholds {", "for netdataThreshold, x := range tc.thresholds {")
    source = replace(source, "lower := tc.h.Copy().TrimBuckets(x, true)", "netdataLowerRequest := netdataTrimRequest(tc.h, x, true, true)\n\t\t\t\t\tlower := tc.h.Copy().TrimBuckets(x, true)")
    anchor = "upper := tc.h.Copy().TrimBuckets(x, false)"
    source = replace(source, anchor, '''netdataComparison := "relative"
                    if fractionBelow == 0 { netdataComparison = "exact" }
                    netdataTrimRecord(output, fmt.Sprintf("TestTrimBuckets_HistogramFractionCrossCheck:%03d:%03d:upper", netdataCase, netdataThreshold),
                        "float-trim", netdataLowerRequest, netdataTrimActual(tc.h, lower, lower, ""),
                        map[string]any{"comparison": netdataComparison, "value": netdataTrimNumber(fractionBelow * total)})
                    netdataUpperRequest := netdataTrimRequest(tc.h, x, false, true)
                    ''' + anchor)
    anchor = "require.InEpsilon(t, fractionAbove*total, upper.Count, 1e-9)\n\t\t\t\t\t}"
    source = replace(source, anchor, anchor + '''
                    netdataComparison = "relative"
                    if fractionAbove == 0 { netdataComparison = "exact" }
                    netdataTrimRecord(output, fmt.Sprintf("TestTrimBuckets_HistogramFractionCrossCheck:%03d:%03d:lower", netdataCase, netdataThreshold),
                        "float-trim", netdataUpperRequest, netdataTrimActual(tc.h, upper, upper, ""),
                        map[string]any{"comparison": netdataComparison, "value": netdataTrimNumber(fractionAbove * total)})''')
    source = replace(source, "trimmed := h.Copy().TrimBuckets(0.5, true)", "request := netdataTrimRequest(h, 0.5, true, true)\n\t\ttrimmed := h.Copy().TrimBuckets(0.5, true)")
    anchor = "require.Zero(t, trimmed.Count)"
    source = replace(source, anchor, anchor + '''
        netdataTrimRecord(output, "TestTrimBuckets_HistogramFractionCrossCheck:zero", "float-trim", request,
            netdataTrimActual(h, trimmed, trimmed, ""), map[string]any{"comparison": "exact", "value": netdataTrimNumber(0)})''')
    source = replace(source, "for _, tc := range divergences {", "for netdataIndex, tc := range divergences {")
    source = replace(source, "trimmed := tc.h.Copy().TrimBuckets(tc.x, tc.isUpperTrim)", "request := netdataTrimRequest(tc.h, tc.x, tc.isUpperTrim, true)\n\t\t\ttrimmed := tc.h.Copy().TrimBuckets(tc.x, tc.isUpperTrim)")
    anchor = 'require.Equal(t, tc.expectedFraction, fraction, "HistogramFraction handles the %s differently", tc.why)'
    return replace(source, anchor, anchor + '''
            netdataTrimRecord(output, fmt.Sprintf("TestTrimBuckets_HistogramFractionCrossCheck:divergence:%03d", netdataIndex),
                "float-trim", request, netdataTrimActual(tc.h, trimmed, trimmed, ""),
                map[string]any{"comparison": "exact", "value": netdataTrimNumber(tc.expectedCount)})''')


def generate_hook(package, definitions):
    template = (HERE / "capture/trim_test.go.in").read_text()
    imports = ('"github.com/prometheus/prometheus/model/histogram"\n"github.com/prometheus/prometheus/promql/parser/posrange"'
               if package == "promql" else "")
    alias = "histogram.FloatHistogram" if package == "promql" else "FloatHistogram"
    replacements = {"PACKAGE": package, "IMPORTS": imports, "ALIASES": f"type netdataTrimHistogram = {alias}",
                    "ORIGINAL_ASSERTIONS": "\n".join(f't.Run("{name}", {name})' for name in TESTS[package]),
                    "CAPTURE_FUNCTIONS": "\n\n".join(instrument(name, definitions[name]) for name in TESTS[package]),
                    "CAPTURE_CALLS": "\n".join(f"netdata{name}(t, &cases)" for name in TESTS[package])}
    if package == "histogram":
        replacements["CAPTURE_FUNCTIONS"] += "\n\n" + (HERE / "capture/trim_supplemental.go.in").read_text()
        replacements["CAPTURE_CALLS"] += "\nnetdataTrimSupplemental(&cases)"
    for marker, replacement in replacements.items():
        marker = "/* " + marker + " */"
        if marker not in template:
            raise ValueError("changed trim hook marker")
        template = template.replace(marker, replacement)
    if "/* " in template:
        raise ValueError("unfilled trim capture marker")
    return template.encode()


def capture(mod_cache, write_input_ledger=False):
    cache = directory(mod_cache)
    source = directory(cache / MODULE)
    inventory(source)
    parent = directory(HERE / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="trim-capture-", dir=parent))
    private_tmp = task / "tmp"
    private_tmp.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(GOMODCACHE=str(cache), GOCACHE=str(parent / "go-build"), GOPROXY="off", GOSUMDB="off",
               GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="-mod=readonly", GOAMD64="v1",
               TMPDIR=str(directory(private_tmp)), GOTMPDIR=str(private_tmp))
    profile = json.loads(subprocess.check_output(["go", "env", "-json", "GOVERSION", "GOOS", "GOARCH",
                                                "GOAMD64", "GOEXPERIMENT"], cwd=HERE / "oracle", env=env))
    if profile != {"GOVERSION": "go1.27.1", "GOOS": "linux", "GOARCH": "amd64", "GOAMD64": "v1", "GOEXPERIMENT": ""}:
        raise ValueError("unexpected trim reference compiler profile")
    run(["go", "mod", "verify"], HERE / "oracle", env)
    facade = task / "reference"
    shutil.copytree(source, facade)
    directory(facade)
    for path in [facade, *facade.rglob("*")]:
        if path.is_dir():
            path.chmod(path.stat().st_mode | 0o200)
    manifest = verify_facade(source, facade)
    if digest(manifest) != histogram_ledger()["source_manifest_sha256"]:
        raise ValueError("trim reference source pin changed")
    command = ["go", "run", HERE / "capture/trim_sources.go", facade]
    print(f"{HERE / 'oracle'} > {shlex.join(map(str, command))}", flush=True)
    definitions = json.loads(subprocess.check_output(command, cwd=HERE / "oracle", env=env))
    hooks = {package: generate_hook(package, definitions) for package in TESTS}
    for package, relative in HOOKS.items():
        destination = facade / relative
        if destination.exists():
            raise ValueError("trim hook would replace reference source")
        destination.write_bytes(hooks[package])
        env["NETDATA_TRIM_CAPTURE_" + package] = str(task / (package + ".json"))
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-run", "^TestNetdataCaptureTrim$",
         "./model/histogram", "./promql"], facade, env)
    if verify_facade(source, facade) != manifest:
        raise ValueError("trim reference source changed during capture")
    files = {path.relative_to(facade).as_posix() for path in facade.rglob("*") if path.is_file()}
    if files != manifest.keys() | set(HOOKS.values()) or any(path.is_symlink() for path in facade.rglob("*")):
        raise ValueError("unlisted trim facade source")
    if any((facade / HOOKS[key]).read_bytes() != value for key, value in hooks.items()):
        raise ValueError("trim capture hook changed during execution")
    documents = {key: json.loads((task / (key + ".json")).read_bytes()) for key in TESTS}
    from trim_reference import validate_cases
    ledger = {"schema_version": 1, "commit": COMMIT, "module": MODULE,
              "source_manifest_sha256": digest(manifest), "definitions": definitions,
              "cases": [{key: row[key] for key in ("id", "mode", "request", "declared")}
                        for data in documents.values() for row in data["cases"]]}
    if not write_input_ledger:
        previous = json.loads((HERE / "trim-original-inputs.json").read_bytes())
        if ledger != previous:
            raise ValueError("trim source input bindings changed")
        ledger = previous
    counts = validate_cases(documents, ledger)
    if write_input_ledger:
        (HERE / "trim-original-inputs.json").write_text(json.dumps(ledger, indent=2) + "\n")
    (task / "definitions.json").write_text(json.dumps(definitions, indent=2) + "\n")
    (task / "source-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    metadata = {"schema_version": 1, "commit": COMMIT, "counts": counts,
                "generator_sha256": hashlib.sha256((HERE / "capture/trim_sources.go").read_bytes()).hexdigest(),
                "template_sha256": hashlib.sha256((HERE / "capture/trim_test.go.in").read_bytes()).hexdigest(),
                "supplemental_sha256": hashlib.sha256((HERE / "capture/trim_supplemental.go.in").read_bytes()).hexdigest(),
                "definitions_sha256": digest(definitions), "source_files": len(manifest),
                "hooks": {key: hashlib.sha256(value).hexdigest() for key, value in hooks.items()},
                "outputs": {key: hashlib.sha256((task / (key + ".json")).read_bytes()).hexdigest() for key in TESTS}}
    (task / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(json.dumps({"capture": str(task), "counts": counts}), flush=True)
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mod-cache", required=True, help="existing pinned module cache; downloads disabled")
    parser.add_argument("--write-input-ledger", action="store_true", help="explicitly generate bindings from verified Go execution")
    args = parser.parse_args()
    capture(args.mod_cache, args.write_input_ledger)
