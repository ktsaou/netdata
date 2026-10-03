#!/usr/bin/env python3
"""Bounded, independent timing and heap experiments for the native prototypes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import statistics
import subprocess
import time

from benchmark import dataset
from build import candidate_source, flags
from compare import compare, numeric_difference_report

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
OUT = HERE / "build/performance"
ENV = {**os.environ, "CCACHE_DISABLE": "1"}
ENV["PATH"] = os.pathsep.join([str(Path.home() / ".local/bin"), str(Path.home() / ".cargo/bin"),
                                ENV.get("PATH", os.defpath)])
PINS = {
    "c": "21121f3cfb5c7e079e8bc7d8953e71352cb753b1",
    "cpp": "d584f36ab4194a95fc54fe84f91a16e034efa2ce",
    "rust": "43a63afcd8b15a90ed126b170e581856359c4c91",
}
PROFILES = {
    "baseline": ["opt-level=2", "debug-assertions=yes", "overflow-checks=yes"],
    "assertions-off": ["opt-level=2", "debug-assertions=no", "overflow-checks=yes"],
    "o3": ["opt-level=3", "debug-assertions=no", "overflow-checks=yes"],
    "one-unit": ["opt-level=3", "debug-assertions=no", "overflow-checks=yes", "codegen-units=1"],
    "thin-lto": ["opt-level=3", "debug-assertions=no", "overflow-checks=yes", "codegen-units=1", "lto=thin"],
}
QUERIES = [
    "sum(system_cpu)", "sum by(instance)(system_cpu)",
    "sum by(job)(rate(counter_total[5m]))",
    "system_cpu / on(instance) group_left(tier) limits",
    'label_join(system_cpu,"key","/","instance","mode")',
    "histogram_quantile(0.95,sum by(le)(rate(request_bucket[5m])))",
]


def relative(path):
    return os.path.relpath(path, HERE)


def command(args, capture=False):
    args = list(map(str, args))
    print(f"tests/promql-prototype > {shlex.join(args)}", flush=True)
    result = subprocess.run(args, cwd=HERE, text=True, capture_output=capture,
                            env=ENV, timeout=600)
    if result.returncode:
        raise RuntimeError(f"exit {result.returncode}: {shlex.join(args)}\n{result.stderr or ''}")
    return result.stdout if capture else None


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(phase, selected=None):
    variants = OUT / "variants"
    variants.mkdir(exist_ok=True)
    common = ["-std=c11", "-O2", "-g", "-Wall", "-Wextra", "-Werror",
              "-Wno-misleading-indentation", "-I", "."]
    for name in ["host", "regex", "runtime", "heap_profile"]:
        extra = ["-DPP_HEAP_PROFILE"] if name == "host" else []
        command(["gcc", *common, *extra, *flags("--cflags", "json-c", "libpcre2-8"),
                 "-c", f"{name}.c", "-o", relative(variants / f"{name}-heap.o")])
        if name == "host":
            command(["gcc", *common, *flags("--cflags", "json-c", "libpcre2-8"),
                     "-c", "host.c", "-o", relative(variants / "host.o")])
    # The counter check calls the real interposed allocation functions, including failed realloc.
    command(["gcc", *common, "-fno-builtin", "test_heap_profile.c",
             relative(variants / "heap_profile-heap.o"), "-o", relative(variants / "heap-check")])
    command([relative(variants / "heap-check")])
    specifications = []
    if phase == "baseline":
        specifications += [("c", "c", None, False), ("cpp", "cpp", None, False)]
        specifications += [(f"rust-{name}", "rust", name, False) for name in PROFILES]
    else:
        for name in dict.fromkeys(["baseline", selected]):
            specifications.append((f"rust-optimized-{name}", "rust", name, True))
    report = {}
    for name, language, profile, current in specifications:
        engine = candidate_source(language)
        checkout = engine.parents[2]
        suffix = {"c": "c", "cpp": "cpp", "rust": "rs"}[language]
        path = f"tests/promql-prototype/engine.{suffix}"
        source = engine.read_text() if current else subprocess.check_output(
            ["git", "-C", str(checkout), "show", f"{PINS[language]}:{path}"], text=True)
        snapshot = variants / f"{name}.{suffix}"
        snapshot.write_text(source)
        began = time.perf_counter()
        library = variants / f"{name}.a"
        if language == "rust":
            options = ["debuginfo=1", "panic=unwind", *PROFILES[profile]]
            args = ["rustc", "--edition", "2021", "--crate-name", "promql_rust", "--crate-type", "staticlib"]
            for option in options:
                args += ["-C", option]
            args += [relative(snapshot), "-o", relative(library)]
            command(args)
        else:
            compiler, standard = ("gcc", "c11") if language == "c" else ("g++", "c++17")
            options = [f"-std={standard}", "-O2", "-g", "-I", ".", "-Wall", "-Wextra", "-Werror",
                       "-Wno-misleading-indentation"]
            command([compiler, *options, "-c", relative(snapshot), "-o", relative(variants / f"{name}.o")])
            command(["ar", "rcs", relative(library), relative(variants / f"{name}.o")])
        compile_seconds = time.perf_counter() - began
        for heap in [False, True]:
            objects = [variants / ("host-heap.o" if heap else "host.o"),
                       variants / "regex-heap.o", variants / "runtime-heap.o"]
            if heap:
                objects += [variants / "heap_profile-heap.o"]
            host = variants / f"{name}{'-heap' if heap else ''}-host"
            command(["gcc", *map(relative, objects), relative(library),
                     *(["-lstdc++"] if language == "cpp" else []),
                     *flags("--libs", "json-c", "libpcre2-8"), "-lm", "-ldl", "-pthread", "-o", relative(host)])
        binary = variants / f"{name}-host"
        command(["objcopy", "--strip-debug", relative(binary), relative(variants / f"{name}-stripped")])
        report[name] = {"language": language, "profile": profile, "compiler_options": options,
                        "source_sha256": digest(snapshot), "baseline_commit": PINS[language],
                        "current_source": current, "compile_seconds": compile_seconds,
                        "archive_bytes": library.stat().st_size, "binary_sha256": digest(binary),
                        "stripped_binary_bytes": (variants / f"{name}-stripped").stat().st_size}
    save(OUT / f"{phase}-build.json", report)
    return report


def workloads(nodes, extended):
    repetitions = 200 if nodes == 32 else 40
    cases = [{"id": f"P{i:02}", "query": q, "time_ms": 600000, "repetitions": repetitions}
             for i, q in enumerate(QUERIES, 1)]
    if extended:
        for ident, query, start, step in [
            ("A01", "sum by(instance)(system_cpu) > 8", 400000, 10000),
            ("A02", "sum by(job)(rate(counter_total[5m])) > 10", 400000, 10000),
            ("R01", "sum by(instance)(rate(counter_total[5m]))", 480000, 12000),
            ("R02", QUERIES[3], 480000, 12000),
        ]:
            cases.append({"id": ident, "query": query, "time_ms": 600000, "start_ms": start,
                          "end_ms": 600000, "step_ms": step, "repetitions": 5 if nodes == 32 else 2})
    return cases


def invoke(host, data, request, cpu):
    return json.loads(command(["taskset", "-c", cpu, relative(host), relative(data), relative(request)], True))


def measure(names, phase, trials, cpu, extended):
    report = {}
    for nodes in [32, 512]:
        scale = OUT / f"{phase}-{nodes}"
        scale.mkdir(exist_ok=True)
        data = dataset(nodes)
        data_path = scale / "dataset.json"
        save(data_path, data)
        cases = workloads(nodes, extended)
        requests = scale / "requests.json"
        save(requests, cases)
        reference = invoke(HERE / "oracle/.cache/oracle", data_path, requests, cpu)
        save(scale / "reference.json", reference)
        metrics = {"nodes": nodes, "series": len(data["series"]),
                   "samples": sum(len(s["points"]) for s in data["series"]),
                   "dataset_sha256": digest(data_path), "cases_sha256": digest(requests), "candidates": {}}
        for case in cases:
            request = scale / f"{case['id']}.json"
            save(request, [case])
            baseline_result = (invoke(OUT / "variants/rust-baseline-host", data_path, request, cpu)
                               if phase == "final" else None)
            results = {name: [] for name in names}
            numeric = {}
            for trial in range(trials):
                # Rotate and reverse order so a variant is not always measured first or last.
                order = names[trial % len(names):] + names[:trial % len(names)]
                if trial % 2:
                    order = list(reversed(order))
                for name in order:
                    got = invoke(OUT / "variants" / f"{name}-host", data_path, request, cpu)
                    failure = compare(reference, got, [case])
                    if failure:
                        raise RuntimeError(f"correctness failure {name}/{nodes}/{case['id']}: {failure}")
                    result = got["results"][0]
                    calls = result["evaluation_calls"]
                    expected_calls = case["repetitions"] * (
                        (case["end_ms"] - case["start_ms"]) // case["step_ms"] + 1 if "step_ms" in case else 1)
                    if calls != expected_calls or result["seconds"] <= 0 or result["cpu_seconds"] <= 0:
                        raise RuntimeError("invalid timing/call counts")
                    bits = numeric_difference_report(reference, got, [case])
                    numeric[name] = {"finite_values": bits["finite_values"], "nonfinite_values": bits["nonfinite_values"],
                                     "go_bitwise_difference_count": len(bits["bitwise_differences"]),
                                     "go_bitwise_difference_examples": bits["bitwise_differences"][:3]}
                    if baseline_result is not None and name.startswith("rust-optimized-"):
                        baseline_bits = numeric_difference_report(baseline_result, got, [case])
                        if baseline_bits["bitwise_differences"] or compare(baseline_result, got, [{**case, "ordered": True}]) or \
                                baseline_result["results"][0]["work"] != result["work"]:
                            raise RuntimeError(f"optimized Rust changed baseline results: {name}/{case['id']}")
                        numeric[name]["rust_baseline_bitwise_difference_count"] = 0
                    save(scale / f"{name}-{case['id']}-{trial}.json", got)
                    fields = ["seconds", "cpu_seconds", "parse_seconds", "evaluation_seconds", "first_seconds",
                              "range_host_seconds", "range_host_cpu_seconds"]
                    results[name].append({**{key: result[key] for key in fields if key in result},
                                          "evaluation_calls": calls, "process_peak_rss_kib": got["maxrss_kb"]})
                print(f"completed {phase}: {nodes} nodes {case['id']} trial {trial + 1}/{trials}", flush=True)
            for name in names:
                trial_results = results[name]
                repetitions = case["repetitions"]
                wall = [r["seconds"] * 1000 / repetitions for r in trial_results]
                cpu_values = [r["cpu_seconds"] * 1000 / repetitions for r in trial_results]
                result = {"id": case["id"], "query": case["query"], "case": case,
                          "median_ms": statistics.median(wall), "min_ms": min(wall), "max_ms": max(wall),
                          "median_cpu_ms": statistics.median(cpu_values),
                          "parse_median_ms": statistics.median(r["parse_seconds"] * 1000 / repetitions for r in trial_results),
                          "evaluation_median_ms": statistics.median(r["evaluation_seconds"] * 1000 / repetitions for r in trial_results),
                          "first_median_ms": statistics.median(r["first_seconds"] * 1000 for r in trial_results),
                          "numeric": numeric[name], "trials": trial_results}
                if "step_ms" in case:
                    result["range_host_median_ms"] = statistics.median(r["range_host_seconds"] * 1000 for r in trial_results)
                    result["range_host_median_cpu_ms"] = statistics.median(r["range_host_cpu_seconds"] * 1000 for r in trial_results)
                metrics["candidates"].setdefault(name, {"queries": []})["queries"].append(result)
        report[str(nodes)] = metrics
    save(OUT / f"{phase}-timings.json", report)
    return report


def heap_measure(names, phase, cpu, extended):
    report = {}
    for nodes in [32, 512]:
        scale = OUT / f"{phase}-{nodes}"
        cases = workloads(nodes, extended)
        data = scale / "dataset.json"
        reference = json.loads((scale / "reference.json").read_text())
        report[str(nodes)] = {}
        for name in names:
            query_results = []
            for original in cases:
                case = {**original, "repetitions": 1}
                request = scale / "heap-request.json"
                save(request, [case])
                got = invoke(OUT / "variants" / f"{name}-heap-host", data, request, cpu)
                if compare(reference, got, [case]):
                    raise RuntimeError(f"instrumented correctness failure: {name}/{case['id']}")
                result = got["results"][0]
                for key in ["engine_heap", "range_host_heap"]:
                    if key in result and result[key]["tracking_overflows"]:
                        raise RuntimeError(f"heap accounting overflow: {name}/{case['id']}")
                query_results.append({"id": case["id"], "evaluation_calls": result["evaluation_calls"],
                                      **{key: result[key] for key in ["engine_heap", "range_host_heap"] if key in result}})
            report[str(nodes)][name] = query_results
    save(OUT / f"{phase}-heap.json", report)
    return report


def profile(name, cpu):
    """Instruction attribution is evidence of executed work, not elapsed-time percentages."""
    output = OUT / "profiles"
    output.mkdir(exist_ok=True)
    version = command(["valgrind", "--version"], True).strip()
    annotator = shutil.which("callgrind_annotate", path=ENV["PATH"])
    if not annotator:
        candidate = Path.home() / ".local/share/toolchains/versions" / version / "bin/callgrind_annotate"
        if candidate.is_file():
            annotator = relative(candidate)
    if not annotator:
        raise RuntimeError("locate the installed callgrind_annotate before profiling")
    report = {"variant": name, "tool": version,
              "method": "Callgrind instruction events within pp_eval and pp_free; 3 timed+1 validation invocation per shape; no profiled latency claims",
              "queries": {}}
    data = OUT / "baseline-512/dataset.json"
    host = OUT / "variants" / f"{name}-host"
    if not data.is_file() or not host.is_file():
        raise RuntimeError("run the baseline build/measurement before profiling")
    for ident in ["P01", "P02", "P04"]:
        case = {"id": ident, "query": QUERIES[int(ident[1:]) - 1], "time_ms": 600000, "repetitions": 3}
        request = output / f"{name}-{ident}.json"
        save(request, [case])
        raw = output / f"{name}-{ident}.callgrind"
        got = json.loads(command(["taskset", "-c", cpu, "valgrind", "--tool=callgrind",
                                 "--collect-atstart=no", "--toggle-collect=pp_eval", "--toggle-collect=pp_free",
                                 f"--callgrind-out-file={relative(raw)}", relative(host), relative(data), relative(request)], True))
        reference = invoke(HERE / "oracle/.cache/oracle", data, request, cpu)
        if compare(reference, got, [case]):
            raise RuntimeError("profiled correctness failed")
        annotation = command([annotator, "--inclusive=no", "--threshold=95", "--auto=no", relative(raw)], True)
        annotation = annotation.replace(str(REPO.parent), "[WORKSPACE]").replace(str(Path.home()), "[HOME]")
        (output / f"{name}-{ident}-exclusive.txt").write_text(annotation)
        inclusive = command([annotator, "--inclusive=yes", "--threshold=95", "--auto=no", relative(raw)], True)
        inclusive = inclusive.replace(str(REPO.parent), "[WORKSPACE]").replace(str(Path.home()), "[HOME]")
        (output / f"{name}-{ident}-inclusive.txt").write_text(inclusive)
        summary = re.search(r"^summary: (\d+)", raw.read_text(), re.MULTILINE)
        if not summary or int(summary[1]) == 0:
            raise RuntimeError("empty instruction profile")
        report["queries"][ident] = {"query": case["query"], "instructions": int(summary[1]),
                                    "exclusive_annotation": annotation, "inclusive_annotation": inclusive}
    save(output / f"{name}-summary.json", report)
    print(f"complete: tests/promql-prototype/build/performance/profiles/{name}-summary.json", flush=True)


def verify(names, cpu):
    output = OUT / "validation"
    output.mkdir(exist_ok=True)
    common = ["-std=c11", "-O2", "-g", "-I", ".", "-Wall", "-Wextra", "-Werror"]
    command(["gcc", *common, "-c", "ownership.c", "-o", relative(output / "ownership.o")])
    report = {}
    suites = [("common", HERE / "dataset.json", HERE / "cases.json"),
              ("regression", HERE / "dataset.json", HERE / "regression.json"),
              ("performance-regression", HERE / "dataset.json", HERE / "performance-regression.json")]
    fixture_name = (HERE / "build/latest-fixture.txt").read_text().strip()
    fixture = (HERE / "build" / fixture_name).resolve(strict=True)
    if fixture.parent != (HERE / "build").resolve() or not (fixture / ".promql-prototype-fixture").is_file():
        raise RuntimeError("invalid retained replay fixture")
    suites += [("retained-replay", fixture / "snapshot.json", fixture / "cases.json")]
    for title, data, requests in suites:
        cases = json.loads(requests.read_text())
        reference = invoke(HERE / "oracle/.cache/oracle", data, requests, cpu)
        frozen = invoke(OUT / "variants/rust-baseline-host", data, requests, cpu)
        report[title] = {"total": len(cases), "dataset_sha256": digest(data), "cases_sha256": digest(requests),
                         "candidates": {}}
        for name in names:
            got = invoke(OUT / "variants" / f"{name}-host", data, requests, cpu)
            if compare(reference, got, cases):
                raise RuntimeError(f"differential verification failed: {name}/{title}: {compare(reference, got, cases)}")
            entry = {"passed": len(cases), "go_numeric_bits": numeric_difference_report(reference, got, cases)}
            if name.startswith("rust-"):
                bits = numeric_difference_report(frozen, got, cases)
                if bits["bitwise_differences"] or compare(frozen, got, [{**c, "ordered": True} for c in cases]) or \
                        [r["work"] for r in frozen["results"]] != [r["work"] for r in got["results"]]:
                    raise RuntimeError(f"Rust baseline bits/order/work changed: {name}/{title}")
                entry["rust_baseline_bits_order_work"] = "identical"
            report[title]["candidates"][name] = entry
    for name in names:
        binary = output / f"{name}-ownership"
        command(["gcc", relative(output / "ownership.o"), relative(OUT / "variants/regex-heap.o"),
                 relative(OUT / "variants/runtime-heap.o"), relative(OUT / "variants" / f"{name}.a"),
                 *(["-lstdc++"] if name == "cpp" else []), *flags("--libs", "libpcre2-8"),
                 "-lm", "-ldl", "-pthread", "-o", relative(binary)])
        report.setdefault("ownership", {})[name] = command([relative(binary)], True).strip()
    risk_cases = json.loads((HERE / "risk.json").read_text())
    frozen_risk = invoke(OUT / "variants/rust-baseline-host", HERE / "dataset.json", HERE / "risk.json", cpu)
    report["controlled-probes"] = {"total": len(risk_cases), "candidates": {}}
    for name in names:
        got = invoke(OUT / "variants" / f"{name}-host", HERE / "dataset.json", HERE / "risk.json", cpu)
        if compare(frozen_risk, got, risk_cases):
            raise RuntimeError(f"controlled-probe result changed: {name}")
        if name.startswith("rust-") and [(r["kind"], r.get("error"), r["work"]) for r in got["results"]] != \
                [(r["kind"], r.get("error"), r["work"]) for r in frozen_risk["results"]]:
            raise RuntimeError(f"Rust controlled failure/message/work changed: {name}")
        report["controlled-probes"]["candidates"][name] = {"passed": len(risk_cases),
            "reference": "frozen Rust paths qualified by the existing run.py Go/explicit-error gates"}
    save(output / "summary.json", report)
    return report


def memcheck(cpu):
    output = OUT / "memcheck"
    output.mkdir(exist_ok=True)
    report = {}
    final = json.loads((OUT / "final-summary.json").read_text())
    for name in final["builds"]:
        for suite in ["ownership", "performance-regression", "controlled-probes"]:
            log = output / f"{name}-{suite}.log"
            if suite == "ownership":
                args = [relative(OUT / "validation" / f"{name}-ownership")]
            else:
                requests = "performance-regression.json" if suite == "performance-regression" else "risk.json"
                args = [relative(OUT / "variants" / f"{name}-host"), "dataset.json", requests]
            command(["taskset", "-c", cpu, "valgrind", "--tool=memcheck", "--error-exitcode=99",
                     "--leak-check=full", "--errors-for-leak-kinds=definite", f"--log-file={relative(log)}", *args], True)
            text = log.read_text().replace(str(REPO.parent), "[WORKSPACE]").replace(str(Path.home()), "[HOME]")
            log.write_text(text)
            summary = re.search(r"ERROR SUMMARY: (.+)", text)
            if not summary or not summary[1].startswith("0 errors"):
                raise RuntimeError(f"memory validation failed: {name}/{suite}")
            report.setdefault(name, {})[suite] = {"error_summary": summary[1], "log": relative(log)}
    save(output / "summary.json", report)
    print("complete: tests/promql-prototype/build/performance/memcheck/summary.json", flush=True)


def capture():
    """Preserve new evidence separately from the completed three-language experiment."""
    load = lambda path: json.loads(path.read_text())
    baseline = load(OUT / "baseline-summary.json")
    final = load(OUT / "final-summary.json")
    final["verification"] = load(OUT / "validation/summary.json")
    harness = ["performance.py", "host.c", "abi.h", "regex.c", "runtime.c", "heap_profile.c",
               "heap_profile.h", "test_heap_profile.c", "ownership.c", "performance-regression.json"]
    sizes = {}
    for language in ["c", "cpp", "rust"]:
        source = candidate_source(language)
        sizes[language] = {"lines": len(source.read_text().splitlines()), "bytes": source.stat().st_size,
                           "sha256": digest(source), "commit": subprocess.check_output(
                               ["git", "-C", str(source.parents[2]), "rev-parse", "HEAD"], text=True).strip()}
        builds = final["builds"] if language == "rust" else {language: baseline["builds"][language]}
        if any(entry["source_sha256"] != sizes[language]["sha256"] for entry in builds.values()):
            raise RuntimeError(f"current {language} source differs from the measured source")
    report = {"scope": "bounded Rust performance experiment over the frozen common-query prototype; no production selection",
              "baseline": baseline, "final": final,
              "source_sizes": sizes, "harness_sha256": {name: digest(HERE / name) for name in harness},
              "instruction_profiles": {name: load(OUT / "profiles" / f"{name}-summary.json")
                                       for name in ["rust-baseline", "rust-optimized-baseline"]},
              "memory_checks": load(OUT / "memcheck/summary.json"),
              "legacy_gates": {mode: load(HERE / "build" / mode / "results/summary.json")
                               for mode in ["release", "sanitize"]},
              "limitations": [
                  "C/C++ remain the unchanged O2 prototypes; this is not an equally optimized language-ceiling comparison.",
                  f"Build-only phase used CPU {baseline['method']['cpu_affinity']}; final phase remeasured every compared baseline/optimized implementation together on CPU {final['method']['cpu_affinity']}.",
                  "Five trials on two synthetic scales do not establish production latency, concurrency capacity or workload frequency.",
                  "Requested live heap excludes input, stacks, fragmentation, executable pages and transient internals of libc realloc; it is not engine RSS.",
                  "Allocation counts include successful reallocations; profiled timings are not used as speed measurements.",
                  "Flat output backing arrays increase label_join peak versus frozen Rust, while remaining below C/C++ on these fixtures.",
                  "Full range JSON assembly is shared host work with linear row lookup; core streams immediately release each step.",
                  "Rust core is debug-assertion/Valgrind checked; C callers and C/C++ cores have ASan/UBSan, not a Rust-core sanitizer claim.",
                  "Existing Go few-bit rate and NaN-payload differences are reported; optimized Rust preserves its frozen bits/order/work.",
                  "Full PromQL/native histograms, production storage/API/rule integration, platforms and real OOM qualification remain outside this experiment.",
              ]}
    save(HERE / "rust-performance.json", report)
    print("captured: tests/promql-prototype/rust-performance.json", flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("phase", choices=["baseline", "final", "profile", "verify", "memcheck", "capture"])
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--selected-profile", choices=list(PROFILES), default="thin-lto")
    parser.add_argument("--reuse-builds", action="store_true")
    parser.add_argument("--variant", default="rust-baseline")
    args = parser.parse_args()
    if not 3 <= args.trials <= 9:
        parser.error("trials must be between 3 and 9")
    OUT.mkdir(parents=True, exist_ok=True)
    cpu = str(min(os.sched_getaffinity(0)))
    if args.phase == "capture":
        capture()
        return
    if args.phase == "profile":
        if not re.fullmatch(r"(?:rust-(?:optimized-)?(?:baseline|assertions-off|o3|one-unit|thin-lto)|c|cpp)", args.variant):
            parser.error("invalid profiling variant")
        profile(args.variant, cpu)
        return
    if args.phase == "verify":
        final = json.loads((OUT / "final-summary.json").read_text())
        names = list(final["scales"]["32"]["candidates"])
        verify(names, cpu)
        print("complete: tests/promql-prototype/build/performance/validation/summary.json", flush=True)
        return
    if args.phase == "memcheck":
        memcheck(cpu)
        return
    if args.reuse_builds:
        builds = json.loads((OUT / f"{args.phase}-build.json").read_text())
        for name, entry in builds.items():
            suffix = {"c": "c", "cpp": "cpp", "rust": "rs"}[entry["language"]]
            if digest(OUT / "variants" / f"{name}.{suffix}") != entry["source_sha256"] or \
                    digest(OUT / "variants" / f"{name}-host") != entry["binary_sha256"]:
                raise RuntimeError("reused source/binary digest mismatch")
    else:
        builds = build(args.phase, args.selected_profile)
    names = list(builds)
    if args.phase == "final":
        names = ["c", "cpp", "rust-baseline", f"rust-{args.selected_profile}", *names]
        names = list(dict.fromkeys(names))
        for name in names:
            if not (OUT / "variants" / f"{name}-host").is_file():
                raise RuntimeError("run the baseline phase first")
    verification = verify(names, cpu) if args.phase == "final" else None
    timings = measure(names, args.phase, args.trials, cpu, args.phase == "final")
    heaps = heap_measure(names, args.phase, cpu, args.phase == "final")
    report = {"phase": args.phase, "method": {
        "trials": args.trials, "cpu_affinity": int(cpu), "sequential": True,
        "wall_and_cpu": "uninstrumented pp_eval through pp_free; per-request medians; input/JSON excluded",
        "first": "first request in a fresh loaded single-workload process; loader is outside timing",
        "alert_sequence": "advancing timestamps over immutable input; no scheduler or prepared-query reuse",
        "range_core": "all step evaluations released immediately; reparsing per step; not retained-matrix memory",
        "range_host": "separate one-request evaluation plus JSON matrix assembly; no final serialization/release",
        "heap": "separate glibc interposed single-threaded runs; requested allocation bytes; excludes input, stacks, fragmentation and executable pages; core peak is maximum per individual step",
        "builds": "portable CPU; checked integer arithmetic and panic unwinding retained; no production flag selection",
    }, "machine": {"architecture": platform.machine(), "system": platform.system()},
        "toolchains": {tool: subprocess.check_output([tool, "--version"], text=True, env=ENV).splitlines()[0]
                       for tool in ["gcc", "g++", "rustc"]},
        "baseline_pins": PINS, "builds": builds, "scales": timings, "heaps": heaps,
        "verification": verification}
    save(OUT / f"{args.phase}-summary.json", report)
    print(f"complete: tests/promql-prototype/build/performance/{args.phase}-summary.json", flush=True)


if __name__ == "__main__":
    main()
