#!/usr/bin/env python3
"""Capture and compare pinned AMD64 arithmetic without importing Go parent results."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile

from build import directory
from capture import verify_facade
from compare import checked_results, compare
from float_literals import SOURCES as FLOAT_SOURCES
from histogram_capture import COMMIT
from histogram_reference import compare_value, histogram, input_ledger
from numerical_capture import source_manifest as math_sources
from numerical_reference import PROFILE, input_ledger as math_ledger
from portable_arithmetic_cases import PANELS
from portable_arithmetic_native import qualify_native
from query import dataset_hex, result_base64

HERE = Path(__file__).resolve().parent
PROBES = {"native": HERE / "capture/portable_native.go",
          "histogram": HERE / "capture/portable_histogram.go",
          "queries": HERE / "capture/portable_queries.go"}
COUNTS = {"v1": 3630, "v2": 2664, "histogram": 8710, "engine": 3146, "range": 3684, "extra": 407,
          "raddeg": 22, "pointwise": 396, "literal": 25, "coupled": 9, "decimal": 54, "route": 1}
QUERY_PANELS = ("engine", "range", "extra", "raddeg", "pointwise", "literal", "coupled", "route")
NATIVE_PANELS = ("v1", "v2", "decimal")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def owners():
    paths = [Path(__file__), HERE / "portable_arithmetic_cases.py", HERE / "portable_arithmetic_native.py", *PROBES.values()]
    return {path.relative_to(HERE).as_posix(): sha(path) for path in paths}


def runtime_sources(root):
    manifest = math_sources(root)
    if digest(manifest) != math_ledger()["source_manifest_sha256"]:
        raise ValueError("original Go math/compiler source pin changed")
    if any(sha(root / name) != expected for name, expected in FLOAT_SOURCES.items()):
        raise ValueError("original Go decimal source pin changed")
    return manifest | FLOAT_SOURCES


def task_directory(prefix):
    return directory(tempfile.mkdtemp(prefix=prefix, dir=directory(HERE / ".tmp")))


def run(command, task, name, env=None):
    print(f"{HERE / 'oracle'} > {shlex.join(map(str, command))}", flush=True)
    with (task / (name + ".stdout")).open("xb") as out, (task / (name + ".stderr")).open("xb") as err:
        result = subprocess.run(list(map(str, command)), cwd=HERE / "oracle", env=env,
                                stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode:
        raise ValueError(f"portable command failed ({result.returncode}); evidence: {task}")


def capture(upstream):
    source = directory(upstream)
    manifest = verify_facade(source, source)
    if digest(manifest) != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original portable source pin changed")
    task = task_directory("portable-arithmetic-capture-")
    private = task / "build-tmp"
    private.mkdir(mode=0o700)
    directory(private)
    before = owners()
    env = os.environ.copy() | {"GOMODCACHE": str(directory(source.parents[2])),
                              "GOCACHE": str(directory(HERE / ".tmp/go-build")),
                              "GOPROXY": "off", "GOSUMDB": "off", "GOTOOLCHAIN": "local", "GOWORK": "off",
                              "GOFLAGS": "-mod=readonly", "GOAMD64": "v1", "GODEBUG": "", "GOMAXPROCS": "2",
                              "TMPDIR": str(private), "GOTMPDIR": str(private)}
    profile = json.loads(subprocess.check_output(["go", "env", "-json", "GOROOT", *PROFILE], cwd=HERE / "oracle", env=env))
    runtime = directory(profile.pop("GOROOT"))
    if profile != PROFILE:
        raise ValueError("original portable runtime profile changed")
    runtime_manifest = runtime_sources(runtime)
    files = {}
    for kind, probe in PROBES.items():
        copied = task / (kind + ".go")
        shutil.copy2(probe, copied)
        binary = task / (kind + "-original")
        command = ["go", "build", "-p", "4", "-o", binary, copied]
        (task / (kind + "-build-command.json")).write_text(json.dumps(["go", "build", "-p", "4", "-o", binary.name, copied.name]) + "\n")
        run(command, task, kind + "-build", env)
        for path in (copied, binary, task / (kind + "-build-command.json")):
            files[path.name] = sha(path)
    for panel, generate in PANELS.items():
        path = task / (panel + "-inputs.json")
        path.write_text(json.dumps(generate(), separators=(",", ":")) + "\n")
        kind = "native" if panel in NATIVE_PANELS else "histogram" if panel == "histogram" else "queries"
        command = [task / (kind + "-original"), path]
        if kind == "queries":
            command.append("false" if panel == "engine" else "true")
        (task / (panel + "-command.json")).write_text(json.dumps([Path(command[0]).name, path.name, *command[2:]]) + "\n")
        run(command, task, panel, env)
        for path in (path, task / (panel + ".stdout"), task / (panel + "-command.json")):
            files[path.name] = sha(path)
    if verify_facade(source, source) != manifest or runtime_sources(runtime) != runtime_manifest or owners() != before:
        raise ValueError("portable capture source or owner changed")
    (task / "source.json").write_text(json.dumps(manifest, indent=2) + "\n")
    files["source.json"] = sha(task / "source.json")
    (task / "runtime-source.json").write_text(json.dumps(runtime_manifest, indent=2) + "\n")
    files["runtime-source.json"] = sha(task / "runtime-source.json")
    binding = {"schema_version": 1, "commit": COMMIT, "profile": profile, "source_manifest_sha256": digest(manifest),
               "runtime_source_manifest_sha256": digest(runtime_manifest), "owners": before,
               "files": files, "counts": COUNTS, "godebug": "", "goamd64": "v1"}
    (task / "binding.json").write_text(json.dumps(binding, indent=2) + "\n")
    load(task)
    print(json.dumps({"capture": str(task), "cases": COUNTS}), flush=True)
    return task


def canonical_histogram(value):
    value = copy.deepcopy(value)
    if value is not None:
        for key in ("positive_spans", "negative_spans"):
            if value[key] is not None:
                value[key] = [{"Offset": span.get("Offset", span.get("offset")),
                               "Length": span.get("Length", span.get("length"))} for span in value[key]]
    return value


def canonical_case(panel, value):
    value = copy.deepcopy(value)
    if panel == "histogram":
        value = {"Lower": "", "Upper": "", "Linear": False, "UpperTrim": False, "Histogram": None} | value
        value["Histogram"] = canonical_histogram(value["Histogram"])
    else:
        for series in value["data"]["series"]:
            for point in series["points"]:
                if point.get("start_time_ms") == 0:
                    del point["start_time_ms"]
                if "float_histogram" in point:
                    point["float_histogram"] = canonical_histogram(point["float_histogram"])
    return value


def native_cases(value):
    names = {"id": "ID", "shape": "Shape", "op": "Op", "a": "A", "b": "B", "function": "Function",
             "scope": "Scope", "mode": "Mode", "text": "Text", "origin": "Origin",
             "exponent": "Exponent", "offset": "Offset"}
    fields = {"native": ("ID", "Shape", "Op", "A", "B"), "reduction": ("ID", "Op", "A", "B"),
              "functions": ("ID", "Function", "A", "B", "Scope"),
              "widths": ("ID", "Mode", "A", "Op", "Text", "Origin", "Exponent", "Offset")}
    result = []
    for group, keys in fields.items():
        for case in value.get(group, []):
            encoded = {key: 0 if key in ("Exponent", "Offset") else "" for key in keys}
            encoded.update({names[key]: item for key, item in case.items()})
            result.append(encoded)
    return result


def load(reference):
    reference = directory(reference)
    binding = json.loads((reference / "binding.json").read_bytes())
    if (binding["schema_version"] != 1 or binding["commit"] != COMMIT or binding["profile"] != PROFILE
            or binding["source_manifest_sha256"] != input_ledger()["source_manifest_sha256"]
            or binding["owners"] != owners() or binding["counts"] != COUNTS
            or binding["godebug"] != "" or binding["goamd64"] != "v1"):
        raise ValueError("portable source/profile/owner binding changed")
    for name, expected in binding["files"].items():
        if Path(name).name != name or sha(reference / name) != expected:
            raise ValueError("portable original artifact changed")
    runtime_manifest = json.loads((reference / "runtime-source.json").read_bytes())
    if (digest(runtime_manifest) != binding["runtime_source_manifest_sha256"]
            or any(runtime_manifest.get(name) != expected for name, expected in FLOAT_SOURCES.items())
            or digest({name: value for name, value in runtime_manifest.items()
                       if name not in FLOAT_SOURCES or name == "VERSION"}) != math_ledger()["source_manifest_sha256"]):
        raise ValueError("portable original runtime source manifest changed")
    if digest(json.loads((reference / "source.json").read_bytes())) != binding["source_manifest_sha256"]:
        raise ValueError("portable original source manifest changed")
    panels = {}
    for panel, generate in PANELS.items():
        inputs = json.loads((reference / (panel + "-inputs.json")).read_bytes())
        if inputs != generate():
            raise ValueError("portable independent input roster changed")
        rows = [json.loads(line) for line in (reference / (panel + ".stdout")).read_bytes().splitlines()]
        if len(rows) != COUNTS[panel] or len({row["id"] for row in rows}) != len(rows) or any("panic" in row for row in rows):
            raise ValueError("portable original result roster changed or panicked")
        if panel in NATIVE_PANELS:
            actual_inputs = [row["input"] for row in rows]
            expected_inputs = native_cases(inputs)
            ids = [row["ID"] for row in expected_inputs]
        else:
            actual_inputs = [row["request"] for row in rows]
            expected_inputs = [canonical_case(panel, row) for row in inputs]
            ids = [row["ID" if panel == "histogram" else "id"] for row in expected_inputs]
        if actual_inputs != expected_inputs or [row["id"] for row in rows] != ids:
            raise ValueError("portable original input/result identity changed")
        if panel == "route":
            expected = {"id": "route-fma", "kind": "scalar", "rows": [{"labels": [], "points": [
                {"time_ms": 0, "kind": "float", "float": "3fd6a09e667f3bcd"}]}], "warnings": [], "infos": []}
            if rows[0]["actual"] != expected:
                raise ValueError("original portable capture requires the retained FMA-on route")
        panels[panel] = rows
    return panels


def execute(binary, rows, task, name):
    binary = Path(binary).resolve(strict=True)
    before = sha(binary)
    (task / (name + "-requests.jsonl")).write_text("".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows))
    with (task / (name + "-requests.jsonl")).open("rb") as inputs, (task / (name + "-candidate.jsonl")).open("xb") as out, \
            (task / (name + "-candidate.stderr")).open("xb") as err:
        result = subprocess.run([str(binary)], stdin=inputs, stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode or sha(binary) != before:
        raise ValueError(f"portable candidate failed or changed; evidence: {task}")
    return [json.loads(line) for line in (task / (name + "-candidate.jsonl")).read_bytes().splitlines()]


def histogram_wire(value):
    return {key: value[key] for key in ("count", "sum", "schema", "zero_count", "zero_threshold",
                                       "positive_spans", "negative_spans")} | {
        "reset_hint": value["hint"], "positive_buckets": value["positive"],
        "negative_buckets": value["negative"], "custom_values": value["custom"]}


def qualify(reference, kernel, query, frontend, page_size):
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be positive")
    panels = load(reference)
    kernel, query, frontend = (Path(path).resolve(strict=True) for path in (kernel, query, frontend))
    task = task_directory("portable-arithmetic-compare-")
    reports = {"native": qualify_native(panels, kernel, task)}
    requests, expected = [], {}
    for row in panels["histogram"]:
        case = row["request"]
        if case["Mode"] == "fraction":
            mode = "bucket-fraction"
            request = {"bucket": {"lower": case["Lower"], "upper": case["Upper"], "count": "3ff0000000000000",
                                   "index": 0, "lower_inclusive": False, "upper_inclusive": False},
                       "value": case["Value"], "linear": case["Linear"]}
            expected[row["id"]] = row["result"]
        else:
            if case["Mode"] != "trim" or histogram_wire(row["before"]) != case["Histogram"]:
                raise ValueError("portable trim original snapshot changed")
            histogram(case["Histogram"], False)
            after = histogram_wire(row["after"])
            histogram(after, False)
            mode = "float-trim"
            request = {"input": case["Histogram"], "threshold": case["Value"], "upper_trim": case["UpperTrim"], "copy": False}
            expected[row["id"]] = {"receiver_after": after, "input_after": after, "returned_receiver": True, "panic": ""}
        requests.append({"id": row["id"], "mode": mode, "request": request, "borrowed": False})
    responses = execute(kernel, requests, task, "histogram")
    if [row["id"] for row in responses] != list(expected):
        raise ValueError("portable histogram candidate roster changed")
    failures, bits = [], []
    for row in responses:
        if set(row) != {"id", "actual"}:
            raise ValueError("portable histogram candidate packet changed")
        compare_value(expected[row["id"]], row["actual"], row["id"], "actual", failures, bits,
                      exact_numeric=True, exact_bits=True)
    reports["histogram"] = {"cases": len(responses), "failures": failures, "bitwise_differences": bits,
                            "passed": not failures and not bits}
    for family in QUERY_PANELS:
        rows = panels[family]
        requests = []
        for row in rows:
            request = row["request"]
            requests += [{"op": "dataset", "dataset_id": row["id"], "page_size": page_size,
                          "dataset": dataset_hex(request["data"])},
                         {"id": row["id"], "dataset_id": row["id"], "expression": request["query"].encode().hex(),
                          "profile": "upstream-tests", "mode": "instant", "time_ms": 0}]
        responses = execute(query, requests, task, family)
        if (len(responses) != 2 * len(rows)
                or responses[::2] != [{"dataset_id": row["id"]} for row in rows]
                or [row.get("id") for row in responses[1::2]] != [row["id"] for row in rows]
                or any(set(row) != {"id", "result", "observations"} for row in responses[1::2])):
            raise ValueError("portable query candidate roster/packet changed")
        expected = {"schema_version": 1, "results": [row["actual"] for row in rows]}
        actual = {"schema_version": 1, "results": [result_base64(row["result"]) for row in responses[1::2]]}
        checked_results(expected)
        checked_results(actual)
        report = compare(expected, actual, [{"id": row["id"], "ordered": False, "exact_numeric": True} for row in rows])
        report.update(cases=len(rows), passed=report["semantic_pass"] and report["bitwise_pass"])
        reports[family] = report
    decimals = [row for row in panels["v2"] if row["input"].get("Mode") == "decimal"] + panels["decimal"]
    payload = "".join(f"{row['id']}\tf\t0\t{row['input']['Text'].encode().hex()}\n" for row in decimals)
    before = sha(frontend)
    (task / "decimal-requests.tsv").write_text(payload)
    with (task / "decimal-candidate.jsonl").open("xb") as out, (task / "decimal-candidate.stderr").open("xb") as err:
        result = subprocess.run([str(frontend)], input=payload.encode(), stdout=out, stderr=err, timeout=600, check=False)
    if result.returncode or sha(frontend) != before:
        raise ValueError("portable decimal candidate failed or changed")
    actual = [json.loads(line) for line in (task / "decimal-candidate.jsonl").read_bytes().splitlines()]
    expected = []
    for row in decimals:
        error = row.get("error", "")
        if error and error != f'strconv.ParseFloat: parsing "{row["input"]["Text"]}": value out of range':
            raise ValueError("portable original decimal input returned an unclassified error")
        expected.append({"id": row["id"], "value_bits": row["result"], "error": "range" if error else ""})
    reports["decimal"] = {"cases": len(decimals), "passed": actual == expected, "expected": expected, "actual": actual}
    load(reference)
    report = {"passed": all(item["passed"] for item in reports.values()), "families": reports, "page_size": page_size,
              "binary_sha256": {"kernel": sha(kernel), "query": sha(query), "frontend": sha(frontend)},
              "reference_binding_sha256": sha(Path(reference) / "binding.json")}
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "page_size": page_size, "evidence": str(task),
                      "families": {name: {"passed": result["passed"], "cases": result.get("cases", result.get("kernel_cases"))}
                                   for name, result in reports.items()}}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--kernel")
    parser.add_argument("--query")
    parser.add_argument("--frontend")
    parser.add_argument("--page-size", type=int, default=17)
    args = parser.parse_args()
    if args.capture:
        if not args.upstream:
            parser.error("capture requires --upstream")
        capture(args.upstream)
    else:
        if not all((args.reference, args.kernel, args.query, args.frontend)):
            parser.error("comparison requires --reference, --kernel, --query and --frontend")
        raise SystemExit(not qualify(args.reference, args.kernel, args.query, args.frontend, args.page_size)["passed"])
