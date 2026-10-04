#!/usr/bin/env python3
"""Capture pinned public Go math behavior without changing installed sources."""
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
from histogram_reference import input_ledger as histogram_ledger, load as load_histograms

HERE = Path(__file__).resolve().parent
HOOK = "src/math/netdata_numerical_capture_test.go"


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def source_manifest(root):
    paths = [path for path in (root / "src/math").iterdir() if path.is_file()]
    paths += [root / name for name in ("VERSION", "src/cmd/compile/internal/ssagen/intrinsics.go",
              "src/cmd/compile/internal/ssa/_gen/AMD64.rules", "src/cmd/compile/internal/ssa/_gen/ARM64.rules")]
    if any(path.is_symlink() for path in paths) or (root / HOOK).exists():
        raise ValueError("unsafe or previously altered Go math source")
    return {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(paths)}


def capture(histogram_capture, write_input_ledger=False):
    parent = directory(HERE / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="numerical-capture-", dir=parent))
    private_tmp = task / "tmp"
    private_tmp.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(GOCACHE=str(parent / "go-build"), GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local",
               GOWORK="off", GOFLAGS="-mod=readonly", GOAMD64="v1",
               TMPDIR=str(directory(private_tmp)), GOTMPDIR=str(private_tmp))
    profile = json.loads(subprocess.check_output(["go", "env", "-json", "GOROOT", "GOVERSION", "GOOS", "GOARCH",
                                                "GOAMD64", "GOEXPERIMENT"], cwd=HERE / "oracle", env=env))
    root = directory(profile.pop("GOROOT"))
    if profile != {"GOVERSION": "go1.27.1", "GOOS": "linux", "GOARCH": "amd64", "GOAMD64": "v1", "GOEXPERIMENT": ""}:
        raise ValueError("unexpected Go numerical build profile")
    manifest = source_manifest(root)
    old_path = HERE / "numerical-original-inputs.json"
    if old_path.exists() and json.loads(old_path.read_bytes())["source_manifest_sha256"] != digest(manifest):
        raise ValueError("Go numerical source pin changed")
    command = ["go", "run", HERE / "capture/numerical_sources.go", root]
    print(f"{HERE / 'oracle'} > {shlex.join(map(str, command))}", flush=True)
    source_data = subprocess.check_output(command, cwd=HERE / "oracle", env=env)
    definitions = json.loads(source_data)
    histograms, _ = load_histograms(histogram_capture)
    bounds = next(case["actual"] for case in histograms["cases"] if case["mode"] == "exponential-bound-table")
    bounds = [value for row in bounds for value in row]
    if len(bounds) != 511:
        raise ValueError("changed histogram bound inventory")
    (task / "bounds.json").write_text(json.dumps(bounds) + "\n")
    hook = (HERE / "capture/numerical_test.go.in").read_bytes()
    (task / "hook_test.go").write_bytes(hook)
    overlay = {"Replace": {str(root / HOOK): str(task / "hook_test.go")}}
    (task / "overlay.json").write_text(json.dumps(overlay) + "\n")
    env.update(NETDATA_NUMERICAL_CAPTURE=str(task / "numerical.json"), NETDATA_NUMERICAL_BOUNDS=str(task / "bounds.json"))
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-overlay", task / "overlay.json",
         "-run", "^TestNetdataCaptureNumerical$", "math"], HERE / "oracle", env)
    if source_manifest(root) != manifest or (task / "hook_test.go").read_bytes() != hook:
        raise ValueError("Go numerical source or hook changed during capture")
    reference = task / "reference"
    reference.mkdir()
    directory(reference)
    for name, expected in manifest.items():
        destination = reference / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / name, destination)
        if hashlib.sha256(destination.read_bytes()).hexdigest() != expected:
            raise ValueError("Go numerical source copy changed")
    output = (task / "numerical.json").read_bytes()
    from numerical_reference import validate_cases
    data = json.loads(output)
    source = {"source_manifest_sha256": digest(manifest), "definitions": definitions,
              "histogram_source_manifest_sha256": histogram_ledger()["source_manifest_sha256"], "bounds": bounds}
    ledger = source | {"schema_version": 1, "go_version": "go1.27.1", "profile": profile,
                      "cases": [{key: row[key] for key in ("id", "mode", "request", "declared")} for row in data["cases"]]}
    if not write_input_ledger:
        ledger = json.loads(old_path.read_bytes())
        if any(ledger[key] != value for key, value in source.items()):
            raise ValueError("numerical source bindings changed")
    counts = validate_cases(data, ledger)
    if write_input_ledger:
        old_path.write_text(json.dumps(ledger, indent=2) + "\n")
    (task / "sources.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (task / "definitions.json").write_text(json.dumps(definitions, indent=2) + "\n")
    metadata = {"schema_version": 1, "go_version": "go1.27.1", "profile": profile, "counts": counts,
                "source_manifest_sha256": digest(manifest), "definitions_sha256": digest(definitions),
                "bounds_sha256": digest(bounds), "output_sha256": hashlib.sha256(output).hexdigest(),
                "hook_sha256": hashlib.sha256(hook).hexdigest(),
                "generator_sha256": hashlib.sha256((HERE / "capture/numerical_sources.go").read_bytes()).hexdigest()}
    (task / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(json.dumps({"capture": str(task), "counts": counts}), flush=True)
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--histogram-capture", required=True, help="verified Go histogram capture supplies compiled bound inputs")
    parser.add_argument("--write-input-ledger", action="store_true", help="explicitly regenerate bindings from verified Go execution")
    args = parser.parse_args()
    capture(args.histogram_capture, args.write_input_ledger)
