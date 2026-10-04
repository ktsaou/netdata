"""Strict validation of independently captured, source-attributed histogram kernels."""
import hashlib
import json
from pathlib import Path
import re

from build import directory
from compare import bits, same_number
from histogram_capture import COMMIT, MODULE, HOOK, HERE, generate_hook

TABLES = [
    ("TestFloatHistogramMul", "float-mul", 9),
    ("TestFloatHistogramDiv", "float-div", 8),
    ("TestFloatHistogramCopy", "float-copy", 4),
    ("TestFloatHistogramCopyTo", "float-copy-to", 4),
    ("TestHistogramCopy", "integer-copy", 4),
    ("TestHistogramCopyTo", "integer-copy-to", 4),
    ("TestHistogramToFloat", "integer-to-float", 3),
    ("TestCustomBucketsHistogramToFloat", "integer-to-float", 3),
]


def fields(value, expected):
    if type(value) is not dict or value.keys() != set(expected):
        raise ValueError("missing or extra histogram kernel fields")


def compare_value(expected, actual, identity, path, failures, bit_differences, exact_numeric=False):
    if type(expected) is not type(actual):
        failures.append({"id": identity, "path": path, "reason": "changed field type or nil/empty distinction"})
        return
    if type(expected) is dict:
        if expected.keys() != actual.keys():
            failures.append({"id": identity, "path": path, "reason": "missing or extra fields"})
            return
        for key in expected:
            compare_value(expected[key], actual[key], identity, path + "." + key,
                          failures, bit_differences, exact_numeric)
    elif type(expected) is list:
        if len(expected) != len(actual):
            failures.append({"id": identity, "path": path, "reason": "changed bucket/span length"})
            return
        for i, (left, right) in enumerate(zip(expected, actual)):
            compare_value(left, right, identity, f"{path}[{i}]", failures, bit_differences, exact_numeric)
    elif type(expected) is str and re.fullmatch(r"[0-9a-f]{16}", expected):
        if re.fullmatch(r"[0-9a-f]{16}", actual) is None:
            raise ValueError("malformed candidate binary64 field")
        if expected != actual:
            bit_differences.append({"id": identity, "path": path, "expected": expected, "actual": actual})
        exact = exact_numeric or path.endswith(".zero_threshold") or ".custom_values[" in path
        if not (bits(expected)[1] == bits(actual)[1] if exact else same_number(expected, actual)):
            if expected != actual:
                failures.append({"id": identity, "path": path, "reason": "numeric mismatch"})
    elif expected != actual:
        failures.append({"id": identity, "path": path, "reason": "changed discrete field"})


def input_ledger():
    ledger = json.loads((HERE / "histogram-original-inputs.json").read_bytes())
    fields(ledger, ("schema_version", "commit", "module", "source_manifest_sha256", "tables", "cases"))
    if (type(ledger["schema_version"]) is not int or ledger["schema_version"] != 1
            or ledger["commit"] != COMMIT or ledger["module"] != MODULE
            or re.fullmatch(r"[0-9a-f]{64}", ledger["source_manifest_sha256"]) is None):
        raise ValueError("histogram input ledger attribution changed")
    return ledger


def integer(value, lower, upper):
    if type(value) is not int or not lower <= value <= upper:
        raise ValueError("invalid histogram kernel integer")


def number(value):
    if type(value) is not str or re.fullmatch(r"[0-9a-f]{16}", value) is None:
        raise ValueError("invalid histogram kernel binary64")


def spans(value):
    if value is None:
        return
    if type(value) is not list:
        raise ValueError("invalid histogram spans")
    for span in value:
        fields(span, ("Offset", "Length"))
        integer(span["Offset"], -(1 << 31), (1 << 31) - 1)
        integer(span["Length"], 0, (1 << 32) - 1)


def histogram(value, is_integer):
    fields(value, ("reset_hint", "schema", "zero_threshold", "zero_count", "count", "sum",
                   "positive_spans", "negative_spans", "positive_buckets", "negative_buckets", "custom_values"))
    integer(value["reset_hint"], 0, 255)
    integer(value["schema"], -(1 << 31), (1 << 31) - 1)
    for field in ("zero_threshold", "sum"):
        number(value[field])
    for field in ("zero_count", "count"):
        if is_integer:
            integer(value[field], 0, (1 << 64) - 1)
        else:
            number(value[field])
    for field in ("positive_spans", "negative_spans"):
        spans(value[field])
    for field in ("positive_buckets", "negative_buckets", "custom_values"):
        values = value[field]
        if values is None:
            continue
        if type(values) is not list:
            raise ValueError("invalid histogram bucket array")
        for item in values:
            if is_integer and field != "custom_values":
                integer(item, -(1 << 63), (1 << 63) - 1)
            else:
                number(item)


def validate_case(case):
    fields(case, ("id", "mode", "request", "actual", "declared"))
    if type(case["id"]) is not str or not case["id"]:
        raise ValueError("invalid histogram kernel identity")
    mode, request, actual = case["mode"], case["request"], case["actual"]
    if mode in {"compensated-add", "compensated-sub"}:
        fields(request, ("sum", "increment", "compensation"))
        fields(actual, ("sum", "compensation"))
        for value in [*request.values(), *actual.values()]:
            number(value)
    elif mode in {"float-mul", "float-div", "float-copy", "float-copy-to", "integer-copy",
                  "integer-copy-to", "integer-to-float"}:
        if mode in {"float-mul", "float-div"}:
            fields(request, ("input", "scalar"))
            number(request["scalar"])
        else:
            fields(request, ("input", "destination"))
            if request["destination"] is not None:
                histogram(request["destination"], mode.startswith("integer") and mode != "integer-to-float")
        histogram(request["input"], mode.startswith("integer"))
        output_integer = mode.startswith("integer") and mode != "integer-to-float"
        histogram(actual, output_integer)
        if case["declared"] is not None:
            histogram(case["declared"], output_integer)
            failures = []
            compare_value(case["declared"], actual, case["id"], "declared", failures, [], exact_numeric=True)
            if failures:
                raise ValueError("captured kernel differs from its declared source assertion")
    else:
        raise ValueError("unknown histogram kernel mode")


def supplemental_ids():
    result = {}
    for hint in range(4):
        for schema in (0, -53):
            for i in range(6):
                for mode in ("float-mul", "float-div"):
                    result[f"supplement:arithmetic:{mode}:{hint}:{schema}:{i}"] = mode
    for i in range(4):
        result[f"supplement:integer-to-float:{i}"] = "integer-to-float"
    for i in range(11):
        for j in range(11):
            for mode in ("compensated-add", "compensated-sub"):
                result[f"supplement:{mode}:{i}:{j}"] = mode
    return result


def validate_cases(data, tables):
    fields(data, ("schema_version", "commit", "go_version", "go_os", "go_arch",
                  "original_assertions_passed", "cases"))
    if (type(data["schema_version"]) is not int or data["schema_version"] != 1
            or data["commit"] != COMMIT or data["go_version"] != "go1.27.1"
            or data["go_os"] != "linux" or data["go_arch"] != "amd64"
            or data["original_assertions_passed"] is not True):
        raise ValueError("unexpected histogram kernel reference")
    if type(tables) is not list or len(tables) != len(TABLES):
        raise ValueError("incomplete histogram source table inventory")
    ledger = input_ledger()
    if tables != ledger["tables"]:
        raise ValueError("histogram source declaration or positions changed")
    bindings = {}
    for binding in ledger["cases"]:
        fields(binding, ("id", "mode", "request", "declared"))
        if type(binding["id"]) is not str or not binding["id"] or binding["id"] in bindings:
            raise ValueError("duplicate or invalid histogram input binding")
        bindings[binding["id"]] = binding
    expected = {}
    for table, (test, mode, count) in zip(tables, TABLES):
        fields(table, ("test", "file", "mode", "cases", "line", "case_lines", "source_sha256", "prefix"))
        file = "float_histogram_test.go" if test.startswith("TestFloat") else "histogram_test.go"
        if (table["test"] != test or table["file"] != file or table["mode"] != mode
                or type(table["cases"]) is not int or table["cases"] != count
                or type(table["prefix"]) is not str or not table["prefix"].strip()
                or type(table["case_lines"]) is not list or len(table["case_lines"]) != count
                or re.fullmatch(r"[0-9a-f]{64}", table["source_sha256"]) is None):
            raise ValueError("changed histogram source table binding")
        integer(table["line"], 1, (1 << 31) - 1)
        for line in table["case_lines"]:
            integer(line, table["line"] + 1, (1 << 31) - 1)
        for i in range(count):
            expected[f"{test}:{i:03d}"] = mode
    original_count = len(expected)
    expected.update(supplemental_ids())
    if type(data["cases"]) is not list or len(data["cases"]) != len(expected):
        raise ValueError("incomplete histogram kernel capture")
    seen = set()
    for case in data["cases"]:
        validate_case(case)
        if case["id"] in seen or expected.get(case["id"]) != case["mode"]:
            raise ValueError("duplicate or changed histogram kernel binding")
        if {key: case[key] for key in ("id", "mode", "request", "declared")} != bindings.get(case["id"]):
            raise ValueError("histogram kernel request or declared expectation changed")
        if case["id"].startswith("supplement:") and case["declared"] is not None:
            raise ValueError("supplemental histogram case misattributed as upstream assertion")
        if case["id"].startswith("Test"):
            needs_declared = case["mode"] != "integer-to-float"
            if (case["declared"] is not None) != needs_declared:
                raise ValueError("missing or invented histogram table expectation")
        seen.add(case["id"])
    if bindings.keys() != expected.keys():
        raise ValueError("histogram input ledger is incomplete")
    return {"original_table_cases": original_count, "supplemental_cases": len(expected) - original_count,
            "total": len(expected)}


def load(path):
    path = directory(path)
    metadata = json.loads((path / "metadata.json").read_bytes())
    fields(metadata, ("schema_version", "commit", "source_files", "hook_sha256", "generator_sha256",
                      "template_sha256", "tables_sha256", "output_sha256", "counts"))
    tables_raw = (path / "tables.json").read_bytes()
    output_raw = (path / "histograms.json").read_bytes()
    tables, data = json.loads(tables_raw), json.loads(output_raw)
    manifest = json.loads((path / "source-manifest.json").read_bytes())
    digest = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if digest != input_ledger()["source_manifest_sha256"]:
        raise ValueError("histogram reference manifest differs from pinned source")
    generator = (HERE / "capture/histogram_tables.go").read_bytes()
    template = (HERE / "capture/histogram_test.go.in").read_bytes()
    if (type(metadata["schema_version"]) is not int or metadata["schema_version"] != 1
            or metadata["commit"] != COMMIT
            or metadata["source_files"] != len(manifest)
            or metadata["tables_sha256"] != hashlib.sha256(tables_raw).hexdigest()
            or metadata["output_sha256"] != hashlib.sha256(output_raw).hexdigest()
            or metadata["generator_sha256"] != hashlib.sha256(generator).hexdigest()
            or metadata["template_sha256"] != hashlib.sha256(template).hexdigest()):
        raise ValueError("histogram capture digest or attribution changed")
    counts = validate_cases(data, tables)
    if metadata["counts"] != counts:
        raise ValueError("histogram kernel coverage changed")
    hook = generate_hook(tables)
    facade = path / "reference"
    if (facade / HOOK).read_bytes() != hook or hashlib.sha256(hook).hexdigest() != metadata["hook_sha256"]:
        raise ValueError("histogram capture hook changed")
    files = {item.relative_to(facade).as_posix() for item in facade.rglob("*") if item.is_file()}
    if files != manifest.keys() | {HOOK}:
        raise ValueError("unlisted or missing histogram reference file")
    for relative, digest in manifest.items():
        item = facade / relative
        if not item.is_file() or item.is_symlink() or hashlib.sha256(item.read_bytes()).hexdigest() != digest:
            raise ValueError("histogram reference source changed")
    for table in tables:
        if manifest["model/histogram/" + table["file"]] != table["source_sha256"]:
            raise ValueError("histogram table source differs")
    return data, counts
