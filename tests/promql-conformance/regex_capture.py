#!/usr/bin/env python3
"""Capture independent pinned Go regex corpora without modifying module caches."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile

from build import directory, run
from capture import verify_facade
from upstream_inventory import inventory
from regex_reference import (GRAFANA_COMMIT, PROMETHEUS_COMMIT, HOOKS, bindings, validate_cases)

HERE = Path(__file__).resolve().parent
GRAFANA_MODULE = "github.com/grafana/regexp@v0.0.0-20250905093917-f7b3be9d1853"
PROMETHEUS_MODULE = "github.com/prometheus/prometheus@v0.315.0"


def verify_extras(facade, manifest, extras):
    files = set()
    for path in facade.rglob("*"):
        if path.is_symlink():
            raise ValueError("regex facade contains a symlink")
        if path.is_file():
            files.add(path.relative_to(facade).as_posix())
    if files != manifest.keys() | set(extras):
        raise ValueError("unlisted or missing regex facade source")


def capture(mod_cache):
    cache = directory(mod_cache)
    parent = HERE / ".tmp"
    parent.mkdir(exist_ok=True)
    directory(parent)
    task = directory(tempfile.mkdtemp(prefix="regex-capture-", dir=parent))
    request_bytes = (HERE / "regex-requests.json").read_bytes()
    requests = json.loads(request_bytes)
    expected_bindings = bindings(requests)
    request_digest = hashlib.sha256(request_bytes).hexdigest()
    sources = {"grafana": directory(cache / GRAFANA_MODULE),
               "prometheus": directory(cache / PROMETHEUS_MODULE)}
    inventory(sources["prometheus"])
    manifests = {}
    hooks = {}
    for name, source in sources.items():
        facade = task / name
        shutil.copytree(source, facade)
        directory(facade)
        facade.chmod(facade.stat().st_mode | 0o200)
        for path in facade.rglob("*"):
            if path.is_dir():
                path.chmod(path.stat().st_mode | 0o200)
        manifests[name] = verify_facade(source, facade)
        verify_extras(facade, manifests[name], ())
    for name, relative, hook_name in (
            ("grafana", "syntax/netdata_capture_test.go", "regex_syntax_test.go"),
            ("grafana", "netdata_capture_test.go", "regex_test.go"),
            ("prometheus", "model/labels/netdata_capture_test.go", "regex_selector_test.go")):
        destination = task / name / relative
        if destination.exists():
            raise ValueError("regex hook would overwrite reference source")
        raw = (HERE / "capture" / hook_name).read_bytes()
        destination.write_bytes(raw)
        hooks[destination] = raw
    env = os.environ.copy()
    env.update(GOMODCACHE=str(cache), GOCACHE=str(parent / "go-build"), GOPROXY="off", GOSUMDB="off",
               GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="-mod=readonly",
               NETDATA_REGEX_SYNTAX_CAPTURE=str(task / "syntax.json"),
               NETDATA_REGEX_CAPTURE=str(task / "regexp.json"),
               NETDATA_REGEX_SELECTOR_CAPTURE=str(task / "selectors.json"))
    env["NETDATA_REGEX_REQUESTS"] = str(HERE / "regex-requests.json")
    run(["go", "mod", "verify"], HERE / "oracle", env)
    run(["go", "test", "-p", "4", "-parallel", "4", "-count=1", "-timeout", "10m", "-run",
         "^TestNetdataCaptureRegex(Syntax)?$", "./..."], task / "grafana", env)
    run(["go", "test", "-p", "4", "-parallel", "4", "-count=1", "-timeout", "10m", "-run",
         "^TestNetdataCaptureRegexSelectors$", "./model/labels"], task / "prometheus", env)
    for name, source in sources.items():
        if verify_facade(source, task / name) != manifests[name]:
            raise ValueError("regex reference source changed during capture")
        verify_extras(task / name, manifests[name], HOOKS[name])
    if any(path.read_bytes() != raw for path, raw in hooks.items()):
        raise ValueError("regex hook changed during capture")
    digests = {name: hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
               for name, manifest in manifests.items()}
    metadata = {"schema_version": 1, "source_file_counts": {name: len(rows) for name, rows in manifests.items()},
                "source_digests": digests, "hook_digests": {path.name + ":" + path.parent.name:
                    hashlib.sha256(raw).hexdigest() for path, raw in hooks.items()}, "requests_sha256": request_digest,
                    "outputs": {}}
    (task / "source-manifests.json").write_text(json.dumps(manifests, indent=2)+"\n")
    counts = {}
    expected_counts = {name: len(rows) for name, rows in expected_bindings.items()}
    for name, expected_commit in (("syntax", GRAFANA_COMMIT), ("regexp", GRAFANA_COMMIT), ("selectors", PROMETHEUS_COMMIT)):
        expected_count = expected_counts[name]
        raw = (task / f"{name}.json").read_bytes()
        data = json.loads(raw)
        validate_cases(name, data, requests)
        if (data["schema_version"] != 1 or data["commit"] != expected_commit
                or data["go_version"] != "go1.27.1" or data["unicode_version"] != "17.0.0"
                or data["original_assertions_passed"] is not True or len(data["cases"]) != expected_count):
            raise ValueError(f"incomplete or misattributed {name} capture")
        if len({case["id"] for case in data["cases"]}) != expected_count:
            raise ValueError("duplicate regex case identity")
        counts[name] = expected_count
        metadata["outputs"][name] = {"sha256": hashlib.sha256(raw).hexdigest(), "cases": expected_count}
    if (HERE / "regex-requests.json").read_bytes() != request_bytes:
        raise ValueError("targeted requests changed during regex capture")
    (task / "metadata.json").write_text(json.dumps(metadata, indent=2)+"\n")
    print(json.dumps({"capture": str(task), "cases": counts, "source_files": metadata["source_file_counts"]}))
    return task


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--mod-cache", required=True, help="existing pinned module cache; downloads disabled")
    capture(cli.parse_args().mod_cache)
