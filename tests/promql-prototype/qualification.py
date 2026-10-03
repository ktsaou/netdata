#!/usr/bin/env python3
"""Inventory the pinned language and expose the separate unsupported native-histogram risk slice."""
import json
import re
import subprocess

from compare import difference
from run import HERE, invoke

LANGUAGES = ["c", "cpp", "rust"]

def main():
    output = HERE / "build/qualification"
    output.mkdir(parents=True, exist_ok=True)
    oracle = HERE / "oracle/.cache/oracle"
    catalog = json.loads(invoke([oracle, "--language-catalog"]))
    typed = json.loads(invoke([oracle, "--native-histogram-probe"]))
    expected = {"histogram_count": "2", "histogram_sum": "1.5", "histogram_avg": "0.75"}
    assert {r["id"] for r in typed["results"]} == set(expected)
    for r in typed["results"]:
        assert r["kind"] == 2 and r["rows"] == [{"labels": {"case": "hist"},
            "points": [[600000, expected[r["id"]]]], "point_bits": {
                "histogram_count": ["4000000000000000"], "histogram_sum": ["3ff8000000000000"],
                "histogram_avg": ["3fe8000000000000"]}[r["id"]]}]
        assert not r.get("warnings") and not r.get("infos")
    cases = [{"id": fn, "query": f"{fn}(vector(1))", "time_ms": 600000} for fn in expected]
    cases.append({"id": "after", "query": "sum(system_cpu)", "time_ms": 600000})
    cases_path = output / "native-histogram-dispatch.json"
    cases_path.write_text(json.dumps(cases, indent=2) + "\n")
    reference = invoke([oracle, HERE / "dataset.json", cases_path], output / "dispatch-reference.json")
    for r in reference["results"][:3]:
        assert r["kind"] == 2 and not r["rows"]
    # An object-valued sample cannot be represented by the native float-only PPPoint.
    data = {"series": [{"labels": typed["labels"], "points": [[typed["time_ms"],
        {"kind": "native_histogram", "value": typed["fixture"]}]]}]}
    data_path = output / "typed-histogram.json"
    data_path.write_text(json.dumps(data, indent=2) + "\n")
    risk = {"reference": typed, "common_profile_effect": "none; native histograms are a separate unsupported risk slice",
            "native_operation_status": "unsupported; no typed native operation or ownership PASS is claimed",
            "candidates": {}, "completion_cost_drivers": [
                "Typed sample ABI and owned bucket/span/custom-bound data",
                "Native histogram schemas, reset hints, arithmetic and quantile kernels",
                "Mixed float/histogram selectors, ranges, aggregation and annotations",
                "Separate approved Netdata retained histogram projection; no mapping is selected here"]}
    for language in LANGUAGES:
        host = HERE / "build/release" / f"{language}-host"
        dispatch = invoke([host, HERE / "dataset.json", cases_path], output / f"{language}-dispatch.json")
        assert [r["id"] for r in dispatch["results"]] == [r["id"] for r in cases]
        for r in dispatch["results"][:3]:
            assert r["kind"] == 4 and "unsupported" in r.get("error", ""), r
        assert difference(reference["results"][-1], dispatch["results"][-1]) is None
        rejected = subprocess.run([str(host), str(data_path), str(cases_path)], capture_output=True, text=True)
        assert rejected.returncode == 2 and "number required" in rejected.stderr
        results = [{key: r[key] for key in ["id", "query", "kind", "error", "work", "rows"]} for r in dispatch["results"][:3]]
        risk["candidates"][language] = {"status": "unsupported", "function_dispatch": results,
            "following_query": "sum(system_cpu) = 20", "typed_input_rejection": {
                "layer": "shared C host before evaluator entry; PPPoint has no histogram type",
                "exit": rejected.returncode, "error": rejected.stderr.strip()}}
    (HERE / "native-histogram-risk.json").write_text(json.dumps(risk, indent=2) + "\n")

    common = json.loads((HERE / "cases.json").read_text())
    calls = {}
    for case in common:
        for name in re.findall(r"\b([a-zA-Z_][a-zA-Z_0-9]*)\s*\(", case["query"]):
            calls.setdefault(name, []).append(case["id"])
    matrix = {"reference": catalog["reference"],
        "method": "Pinned upstream catalog enumerates every function, aggregation, lexer operator and modifier. Tested means only the frozen float profile; unimplemented/unqualified entries are remaining work, not conformance percentages.",
        "functions": [], "aggregations": [], "operators": [], "modifiers": [], "other_surfaces": []}
    for group in ["functions", "aggregations", "operators", "modifiers"]:
        for feature in catalog[group]:
            name = feature["name"]
            examples = calls.get(name, []) if group in ["functions", "aggregations"] else []
            if group == "aggregations":
                pattern = rf"\b{re.escape(name)}(?:\s+(?:by|without)\s*\([^)]*\))?\s*\("
                examples = [case["id"] for case in common if re.search(pattern, case["query"])]
            tested = bool(examples)
            if group == "operators":
                tested = name in ["+", "-", "*", "/", "%", "^", "==", "!=", "<", "<=", ">", ">=", "and", "or", "unless", "=~", "!~"]
            if group == "modifiers":
                tested = name in ["bool", "by", "without", "on", "ignoring", "group_left", "group_right", "offset"]
            status = "tested_float_slice" if tested else "unimplemented_or_unqualified"
            notes = "Queries outside the frozen profile remain unqualified"
            if name == "histogram_quantile":
                notes = "Classic cumulative buckets tested; native histogram input unsupported"
            elif name in ["label_replace", "=~", "!~"]:
                notes = "Shared bounded PCRE2 facility; full RE2 compatibility unsupported"
            elif name == "offset":
                notes = "Positive literal offset tested; negative offsets unsupported"
            elif name == "@":
                status, notes = "bounded_probe", "Numeric @ tested by R068; start/end and full placement rules unqualified"
            entry = {**feature, "candidates": {lang: status for lang in LANGUAGES}, "notes": notes}
            if examples:
                entry["common_cases"] = sorted(set(examples))
            matrix[group].append(entry)
    surfaces = [
        ("scalar/vector/matrix floats and arithmetic", "tested_float_slice", "67 profile cases, 27 regressions; numerical extremes/compensated reduction unqualified"),
        ("strings", "partial", "Function string arguments tested; escaping/top-level string results/type validation incomplete"),
        ("label selection, vector matching and cardinality", "tested_float_slice", "Exact and underscore names, all four matchers, on/ignoring/group_left/right; regex and full label validation unqualified"),
        ("selectors, lookback, ranges and positive offsets", "tested_float_slice", "Both instant and actual range API comparison; stale-marker payloads and tier seams unqualified"),
        ("subqueries and numeric @", "bounded_probe", "R068/R069 and alignment regressions; full duration/placement grammar unqualified"),
        ("full identifier/string/numeric/duration grammar", "partial", "Colon metric identifiers, negative offsets and duration-expression grammar unsupported; full parser/type rules unqualified"),
        ("native histogram values/operations", "unsupported", "Typed upstream reference and native rejection panel in native-histogram-risk.json; no native operation PASS"),
        ("mixed histogram/float samples and annotations", "unsupported", "Float-only ABI; no annotation channel, no mixed-sample handling"),
        ("experimental syntax/functions", "unimplemented_or_unqualified", "Catalog preserves experimental flags; no opt-in feature qualification"),
        ("numerical bit compatibility", "reported_not_required", "Actual output bits recorded; numeric 1e-6/class comparison remains the prototype gate"),
        ("cancellation/limits/error ownership", "bounded_probe", "10 runtime probes and concurrent ownership; hard deadline/sample-memory/OOM/runtime compatibility unqualified"),
        ("platform and Netdata integration", "unqualified", "Linux x86_64 C callers + Tier0 retained fixture only; production APIs, rules, Cloud, replicas and other platforms later")]
    for name, status, notes in surfaces:
        matrix["other_surfaces"].append({"name": name, "candidates": {lang: status for lang in LANGUAGES}, "notes": notes})
    (HERE / "language-matrix.json").write_text(json.dumps(matrix, indent=2) + "\n")
    print(json.dumps({"catalog_rows": {group: len(matrix[group]) for group in catalog if group != "reference"},
                      "native_histograms": {lang: "unsupported" for lang in LANGUAGES}}, indent=2))

if __name__ == "__main__":
    main()
