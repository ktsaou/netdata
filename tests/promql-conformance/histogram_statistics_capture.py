#!/usr/bin/env python3
"""Capture original native statistic assertions and independent registered Go helper results."""
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
from histogram_functions_capture import digest
from histogram_reference import input_ledger as histogram_ledger
from numerical_reference import PROFILE
from upstream_inventory import inventory

HERE = Path(__file__).resolve().parent
HOOK = "promql/promqltest/netdata_histogram_statistics_capture_test.go"
TEMPLATES = ("histogram_functions_common.go.in", "histogram_functions_supplemental.go.in",
             "histogram_statistics_dsl.go.in", "histogram_statistics_supplemental.go.in")
TEST_FILES = ("extended_vectors.test", "histograms.test", "native_histograms.test")


def generate_hook():
    previous = (HERE / "capture" / TEMPLATES[1]).read_text()
    start, end = previous.index("\tnan :="), previous.index("\tquantiles :=")
    supplemental = (HERE / "capture" / TEMPLATES[3]).read_text().replace("/* PRIOR_HISTOGRAM_FIXTURES */", previous[start:end])
    template = (HERE / "capture" / TEMPLATES[0]).read_text()
    replacements = {
        "PACKAGE": "promqltest",
        "IMPORTS": '''"io/fs"
            "time"
            "github.com/prometheus/prometheus/model/labels"
            "github.com/prometheus/prometheus/schema"
            "github.com/prometheus/prometheus/promql"
            "github.com/prometheus/prometheus/promql/parser"''',
        "HELPER_ALIASES": "var netdataHelperFraction = promql.HistogramFraction\nvar netdataHelperQuantile = promql.HistogramQuantile",
        "CAPTURE_FUNCTIONS": (HERE / "capture" / TEMPLATES[2]).read_text() + "\n" + supplemental,
        "CAPTURE_BODY": '''engine := NewTestEngine(t, false, 5*time.Minute, DefaultMaxSamplesPerQuery)
            cases := []map[string]any{}
            commands := []any{}
            for _, name := range []string{"extended_vectors.test", "histograms.test", "native_histograms.test"} {
                raw, err := fs.ReadFile(testsFs, "testdata/" + name); require.NoError(t, err)
                t.Run("original_" + name, func(t *testing.T) { RunTest(t, string(raw), engine) })
                if t.Failed() { t.Fatal("original histogram statistic assertions failed") }
                commands = append(commands, netdataStatisticCaptureDSL(t, name, string(raw), engine, &cases)...)
            }
            netdataStatisticSupplemental(t, &cases)
            netdataHelperWrite(t, cases, commands)''',
    }
    for name, value in replacements.items():
        marker = "/* " + name + " */"
        if marker not in template:
            raise ValueError("missing native statistic capture marker")
        template = template.replace(marker, value)
    template = template.replace("TestNetdataCaptureHistogramFunctions", "TestNetdataCaptureHistogramStatistics")
    if "/* " in template:
        raise ValueError("unfilled native statistic capture marker")
    return template.encode()


def capture(mod_cache, write_input_ledger=False):
    cache = directory(mod_cache)
    source = directory(cache / MODULE)
    inventory(source)
    parent = directory(HERE / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="histogram-statistics-capture-", dir=parent))
    private_tmp = task / "tmp"
    private_tmp.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(GOMODCACHE=str(cache), GOCACHE=str(parent / "go-build"), GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local",
               GOWORK="off", GOFLAGS="-mod=readonly", GOAMD64="v1", TMPDIR=str(directory(private_tmp)), GOTMPDIR=str(private_tmp))
    profile = json.loads(subprocess.check_output(["go", "env", "-json", "GOVERSION", "GOOS", "GOARCH", "GOAMD64", "GOEXPERIMENT"],
                                               cwd=HERE / "oracle", env=env))
    if profile != PROFILE:
        raise ValueError("unexpected native statistic compiler profile")
    run(["go", "mod", "verify"], HERE / "oracle", env)
    facade = task / "reference"
    shutil.copytree(source, facade)
    directory(facade)
    for path in [facade, *facade.rglob("*")]:
        if path.is_dir():
            path.chmod(path.stat().st_mode | 0o200)
    manifest = verify_facade(source, facade)
    if digest(manifest) != histogram_ledger()["source_manifest_sha256"]:
        raise ValueError("native statistic reference pin changed")
    generator = HERE / "capture/histogram_statistics_sources.go"
    command = ["go", "run", generator, facade]
    print(f"{HERE / 'oracle'} > {shlex.join(map(str, command))}", flush=True)
    sources = json.loads(subprocess.check_output(command, cwd=HERE / "oracle", env=env))
    hook = generate_hook()
    destination = facade / HOOK
    if destination.exists():
        raise ValueError("native statistic hook would replace reference source")
    destination.write_bytes(hook)
    env["NETDATA_HISTOGRAM_FUNCTIONS_promqltest"] = str(task / "observations.json")
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-run", "^TestNetdataCaptureHistogramStatistics$",
         "./promql/promqltest"], facade, env)
    if verify_facade(source, facade) != manifest or destination.read_bytes() != hook:
        raise ValueError("native statistic source changed during capture")
    paths = list(facade.rglob("*"))
    if ({path.relative_to(facade).as_posix() for path in paths if path.is_file()} != manifest.keys() | {HOOK}
            or any(path.is_symlink() for path in paths)):
        raise ValueError("unlisted native statistic reference source")
    document = json.loads((task / "observations.json").read_bytes())
    ledger = {"schema_version": 1, "commit": COMMIT, "module": MODULE, "source_manifest_sha256": digest(manifest),
              "definitions": sources["definitions"], "source_commands": sources["commands"], "commands": document["commands"],
              "cases": [{key: row[key] for key in ("id", "mode", "request", "declared", "source")} for row in document["cases"]]}
    ledger_path = HERE / "histogram-statistics-original-inputs.json"
    if not write_input_ledger and ledger != json.loads(ledger_path.read_bytes()):
        raise ValueError("native statistic source/input bindings changed")
    from histogram_statistics_reference import validate_cases
    counts = validate_cases(document, ledger)
    if write_input_ledger:
        if ledger_path.exists():
            previous = json.loads(ledger_path.read_bytes())
            bindings = {row["id"]: row for row in ledger["cases"]}
            if any(bindings.get(row["id"]) != row for row in previous["cases"]):
                raise ValueError("earlier native statistic bindings changed")
        ledger_path.write_text(json.dumps(ledger, indent=2) + "\n")
    for name, value in (("sources.json", sources), ("source-manifest.json", manifest)):
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    metadata = {"schema_version": 1, "commit": COMMIT, "profile": profile, "counts": counts,
                "generator_sha256": hashlib.sha256(generator.read_bytes()).hexdigest(), "sources_sha256": digest(sources),
                "templates": {name: hashlib.sha256((HERE / "capture" / name).read_bytes()).hexdigest() for name in TEMPLATES},
                "source_files": len(manifest), "hook_sha256": hashlib.sha256(hook).hexdigest(),
                "output_sha256": hashlib.sha256((task / "observations.json").read_bytes()).hexdigest()}
    (task / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(json.dumps({"capture": str(task), "counts": counts}), flush=True)
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mod-cache", required=True, help="existing pinned Go module cache; downloads disabled")
    parser.add_argument("--write-input-ledger", action="store_true", help="explicitly extend independent Go source/input bindings")
    args = parser.parse_args()
    capture(args.mod_cache, args.write_input_ledger)
