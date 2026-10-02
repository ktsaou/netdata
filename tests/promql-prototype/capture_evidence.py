#!/usr/bin/env python3
"""Preserve concise, path-free experiment evidence after completing the runners."""
import hashlib
import json
from pathlib import Path
import subprocess

from run import HERE

def read(path):
    return json.loads(path.read_text())

def main():
    repo = HERE.parents[1]
    fixture_name = (HERE / "build/latest-fixture.txt").read_text().strip()
    if Path(fixture_name).name != fixture_name or not fixture_name.startswith("fixture-"):
        raise RuntimeError("invalid fixture reference")
    fixture = HERE / "build" / fixture_name
    evidence = {"netdata_base": "f8f53c75c4d4ce1a0b43928c737278d447ee776d",
                "prometheus_reference": "v3.15.0 / 5241a27fe3c6983549fccc32f6e65917408c63cd / module v0.315.0",
                "common_profile": read(HERE / "build/release/results/summary.json"),
                "sanitizers": read(HERE / "build/sanitize/results/summary.json"),
                "dbengine": {"manifest": read(fixture / "manifest.json"), "replay": read(HERE / "build/release/replay/summary.json")},
                "measurements": read(HERE / "build/release/benchmark/summary.json"),
                "candidate_revisions": {}, "source_sha256": {}}
    for lang, suffix in [("c", "c"), ("cpp", "cpp"), ("rust", "rs")]:
        checkout = repo.parent / f"netdata-{lang}"
        evidence["candidate_revisions"][lang] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
        path = checkout / f"tests/promql-prototype/engine.{suffix}"
        evidence["source_sha256"][f"{lang}/engine.{suffix}"] = hashlib.sha256(path.read_bytes()).hexdigest()
    for name in ["abi.h", "host.c", "regex.c", "ownership.c", "fixtures.py", "cases.json", "dataset.json", "regression.json", "risk.json", "oracle/main.go", "oracle/go.mod", "oracle/go.sum"]:
        evidence["source_sha256"][name] = hashlib.sha256((HERE / name).read_bytes()).hexdigest()
    for name in ["src/daemon/main.c", "src/database/engine/dbengine-unittest.c"]:
        evidence["source_sha256"][name] = hashlib.sha256((repo / name).read_bytes()).hexdigest()
    evidence["qualification_limits"] = [
        "67 common query cases are a scope proxy, not statistical coverage or complete PromQL conformance",
        "Native histogram samples, annotations, full grammar/functions, stale-marker payloads, numerical extremes and RE2 equivalence remain unqualified",
        "C/C++ cores and common C caller/regex were ASan/UBSan instrumented; Rust core used debug assertions and the same instrumented C caller, not Rust sanitizer instrumentation",
        "Rust allocator OOM can abort; injected allocation failure is a controlled error, not an allocator-OOM recovery test",
        "Standalone work counters do not implement upstream MaxSamples or a hard elapsed-time deadline",
        "Native snapshot uses Tier 0 and fixed fixture identities; restart, tier seams, live NIDL catalog and replicas remain unqualified",
        "Only Linux x86_64 tested; no production API/meta-health lifecycle/Cloud split implementation or language selection",
        "Peak RSS includes JSON input and process/launcher overhead; no engine-only memory ranking",
    ]
    (HERE / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    (HERE / "reference.json").write_text((HERE / "build/release/results/reference.json").read_text())

if __name__ == "__main__":
    main()
