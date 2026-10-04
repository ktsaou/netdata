#!/usr/bin/env python3
"""Capture original classic assertions and independently executed Go helper sequences."""
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
HOOK = "promql/netdata_classic_capture_test.go"
TEMPLATES = ("classic_common.go.in", "classic_supplemental.go.in")
GO_FILES = ("src/slices/sort.go", "src/slices/zsortanyfunc.go", "src/sort/search.go", "src/math/bits/bits.go")


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError("classic original observation insertion changed")
    return text.replace(old, new)


def generate_hook(definitions):
    quantile = definitions["TestBucketQuantile_ForcedMonotonicity"]["body"]
    quantile = replace_once(quantile, "func TestBucketQuantile_ForcedMonotonicity(", "func TestNetdataCopiedBucketQuantile(")
    quantile = replace_once(quantile, "quantile, forced, fixed, _, _, _ := BucketQuantile(q, tc.getInput())", """input := tc.getInput()
                request := netdataClassicFreeze(map[string]any{"buckets": netdataClassicBuckets(input), "operations": []map[string]any{netdataClassicQuantile(q)}})
                quantile, forced, fixed, min, max, decrease := BucketQuantile(q, input)""")
    quantile = replace_once(quantile, "require.InEpsilon(t, v, quantile, eps)", """require.InEpsilon(t, v, quantile, eps)
                netdataClassicOriginal(name, q, v, eps, tc.expectedForced, tc.expectedFixed, request, input, quantile, forced, fixed, min, max, decrease)""")
    almost = definitions["TestEqual"]["body"]
    almost = replace_once(almost, "func TestEqual(", "func TestNetdataCopiedAlmost(")
    almost = replace_once(almost, "for _, tt := range tests", "for index, tt := range tests")
    almost = replace_once(almost, "if got := Equal(tt.a, tt.b, tt.epsilon)", "if got := almost.Equal(tt.a, tt.b, tt.epsilon)")
    almost = replace_once(almost, "\t\t\t}\n\t\t})", """\t\t\t}
            netdataClassicEqual(fmt.Sprintf("original-almost:%02d", index), tt.a, tt.b, tt.epsilon, tt.want,
                map[string]any{"file": "util/almost/almost_test.go", "line": 45, "row": index})
\t\t})""")
    hook = (HERE / "capture" / TEMPLATES[0]).read_text()
    for name, body in (("ORIGINAL_QUANTILE", quantile), ("ORIGINAL_ALMOST", almost),
                       ("SUPPLEMENTAL", (HERE / "capture" / TEMPLATES[1]).read_text())):
        hook = replace_once(hook, "/* " + name + " */", body)
    if "/* " in hook:
        raise ValueError("unfilled classic capture marker")
    return hook.encode()


def capture(mod_cache, write_input_ledger=False):
    cache = directory(mod_cache)
    source = directory(cache / MODULE)
    inventory(source)
    parent = directory(HERE / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="classic-capture-", dir=parent))
    private_tmp = task / "tmp"
    private_tmp.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(GOMODCACHE=str(cache), GOCACHE=str(parent / "go-build"), GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local",
               GOWORK="off", GOFLAGS="-mod=readonly", GOAMD64="v1", TMPDIR=str(directory(private_tmp)), GOTMPDIR=str(private_tmp))
    profile = json.loads(subprocess.check_output(["go", "env", "-json", *PROFILE, "GOROOT"], cwd=HERE / "oracle", env=env))
    goroot = directory(profile.pop("GOROOT"))
    if profile != PROFILE:
        raise ValueError("unexpected classic compiler profile")
    run(["go", "mod", "verify"], HERE / "oracle", env)
    facade = task / "reference"
    shutil.copytree(source, facade)
    directory(facade)
    for path in [facade, *facade.rglob("*")]:
        if path.is_dir():
            path.chmod(path.stat().st_mode | 0o200)
    manifest = verify_facade(source, facade)
    if digest(manifest) != histogram_ledger()["source_manifest_sha256"]:
        raise ValueError("classic Prometheus reference pin changed")
    go_manifest = {name: hashlib.sha256((goroot / name).read_bytes()).hexdigest() for name in GO_FILES}
    go_facade = task / "go-reference"
    go_facade.mkdir(mode=0o700)
    for name in GO_FILES:
        destination = go_facade / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(goroot / name, destination)
    generator = HERE / "capture/classic_sources.go"
    command = ["go", "run", generator, facade, goroot]
    print(f"{HERE / 'oracle'} > {shlex.join(map(str, command))}", flush=True)
    definitions = json.loads(subprocess.check_output(command, cwd=HERE / "oracle", env=env))
    hook = generate_hook(definitions)
    destination = facade / HOOK
    if destination.exists():
        raise ValueError("classic hook would replace original source")
    destination.write_bytes(hook)
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-run", "^TestEqual$", "./util/almost"], facade, env)
    env["NETDATA_CLASSIC_CAPTURE"] = str(task / "observations.json")
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-run", "^TestNetdataCaptureClassic$", "./promql"], facade, env)
    if (verify_facade(source, facade) != manifest or destination.read_bytes() != hook
            or go_manifest != {name: hashlib.sha256((goroot / name).read_bytes()).hexdigest() for name in GO_FILES}):
        raise ValueError("classic reference source changed during capture")
    paths = list(facade.rglob("*"))
    if ({path.relative_to(facade).as_posix() for path in paths if path.is_file()} != manifest.keys() | {HOOK}
            or any(path.is_symlink() for path in paths)):
        raise ValueError("unlisted classic reference source")
    document = json.loads((task / "observations.json").read_bytes())
    ledger = {"schema_version": 1, "commit": COMMIT, "module": MODULE, "source_manifest_sha256": digest(manifest),
              "go_manifest": go_manifest, "definitions": definitions,
              "cases": [{key: row[key] for key in ("id", "mode", "request", "declared", "source")} for row in document["cases"]]}
    ledger_path = HERE / "classic-original-inputs.json"
    if not write_input_ledger and ledger != json.loads(ledger_path.read_bytes()):
        raise ValueError("classic original source/input bindings changed")
    from classic_reference import validate_cases
    counts = validate_cases(document, ledger)
    if write_input_ledger:
        if ledger_path.exists():
            previous = json.loads(ledger_path.read_bytes())
            bindings = {row["id"]: row for row in ledger["cases"]}
            if any(bindings.get(row["id"]) != row for row in previous["cases"]):
                raise ValueError("earlier classic bindings changed")
        ledger_path.write_text(json.dumps(ledger, indent=2) + "\n")
    for name, obj in (("definitions.json", definitions), ("source-manifest.json", manifest), ("go-manifest.json", go_manifest)):
        (task / name).write_text(json.dumps(obj, indent=2) + "\n")
    metadata = {"schema_version": 1, "commit": COMMIT, "profile": profile, "counts": counts,
                "generator_sha256": hashlib.sha256(generator.read_bytes()).hexdigest(), "definitions_sha256": digest(definitions),
                "templates": {name: hashlib.sha256((HERE / "capture" / name).read_bytes()).hexdigest() for name in TEMPLATES},
                "source_files": len(manifest), "go_manifest_sha256": digest(go_manifest), "hook_sha256": hashlib.sha256(hook).hexdigest(),
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
