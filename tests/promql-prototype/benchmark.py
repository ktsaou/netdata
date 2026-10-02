#!/usr/bin/env python3
"""Bounded sequential measurements; this compares implementations, not languages in general."""
import hashlib
import json
from pathlib import Path
import platform
import statistics
import subprocess

from compare import compare
from run import HERE, invoke

def dataset(nodes):
    rows = []
    def add(name, labels, value):
        rows.append({"labels": {"__name__": name, **labels}, "points": [[t*1000, value(t)] for t in range(0, 601, 10)]})
    for i in range(nodes):
        ls = {"instance": f"node-{i:04}", "job": f"job-{i%8}", "zone": f"zone-{i%4}"}
        for mode, value in [("user", 2+i%7), ("system", 4+i%5)]:
            add("system_cpu", {**ls, "mode": mode}, lambda t, value=value: value)
        add("counter_total", ls, lambda t, i=i: 1000+t*(1+i%3))
        add("limits", {**ls, "tier": "one"}, lambda t: 20)
        for le, rate in [("0.1", 2), ("0.5", 6), ("1", 8), ("+Inf", 10)]:
            add("request_bucket", {**ls, "le": le}, lambda t, rate=rate: t*rate)
    return {"series": rows}

def main():
    output = HERE / "build/release/benchmark"
    output.mkdir(parents=True, exist_ok=True)
    queries = ["sum(system_cpu)", "sum by(instance)(system_cpu)",
               "sum by(job)(rate(counter_total[5m]))",
               "system_cpu / on(instance) group_left(tier) limits",
               'label_join(system_cpu,"key","/","instance","mode")',
               "histogram_quantile(0.95,sum by(le)(rate(request_bucket[5m])))"]
    report = {"machine": {"architecture": platform.machine(), "system": platform.system()},
              "method": "3 sequential fresh-process trials; parse/evaluate/release repeated; JSON load and serialization excluded from query timing; RSS includes the C JSON host and input dataset; no Go timing comparison",
              "scales": {}, "toolchains": {}}
    for tool in ["gcc", "g++", "rustc", "go"]:
        report["toolchains"][tool] = subprocess.check_output([tool, "version"] if tool=="go" else [tool, "--version"], text=True).splitlines()[0]
    cpu = Path("/proc/cpuinfo")
    if cpu.exists():
        report["machine"]["cpu"] = next((line.split(":",1)[1].strip() for line in cpu.read_text().splitlines() if line.startswith("model name")), "unknown")
    for nodes, repetitions in [(32, 100), (512, 20)]:
        scale = output / str(nodes)
        scale.mkdir(exist_ok=True)
        data = dataset(nodes)
        data_path = scale / "dataset.json"
        data_path.write_text(json.dumps(data, separators=(",", ":")) + "\n")
        cases = [{"id": f"P{i:02}", "query": q, "time_ms": 600000, "repetitions": repetitions} for i,q in enumerate(queries,1)]
        cases_path = scale / "cases.json"
        cases_path.write_text(json.dumps(cases, indent=2) + "\n")
        reference = invoke([HERE / "oracle/.cache/oracle", data_path, cases_path], scale / "reference.json")
        metrics = {"nodes": nodes, "series": len(data["series"]), "samples": sum(len(r["points"]) for r in data["series"]), "repetitions": repetitions,
                   "dataset_sha256": hashlib.sha256(data_path.read_bytes()).hexdigest(), "candidates": {}}
        for language in ["c", "cpp", "rust"]:
            times, rss = {}, []
            for trial in range(3):
                result = invoke([HERE / "build/release" / f"{language}-host", data_path, cases_path], scale / f"{language}-{trial}.json")
                failures = compare(reference, result, cases)
                if failures:
                    raise RuntimeError(f"benchmark correctness failed: {language}: {failures}")
                rss.append(result["maxrss_kb"])
                for r in result["results"]:
                    times.setdefault(r["id"], []).append(r["seconds"]*1000/repetitions)
            metrics["candidates"][language] = {"queries": [{"id": c["id"], "query": c["query"], "median_ms": statistics.median(times[c["id"]]), "min_ms": min(times[c["id"]]), "max_ms": max(times[c["id"]])} for c in cases],
                                                "process_peak_rss_kib": rss}
        report["scales"][str(nodes)] = metrics
    report["build"] = json.loads((HERE / "build/release/build-metrics.json").read_text())
    (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))

if __name__ == "__main__":
    main()
