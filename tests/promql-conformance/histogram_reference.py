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
    ("TestFloatHistogramCompact", "float-compact", 24),
    ("TestFloatHistogramCopyToSchema", "float-copy-to-schema", 3),
    ("TestFloatHistogramReduceResolution", "float-reduce-resolution", 4),
    ("TestGetBoundExponential", "exponential-bound", 15),
    ("TestAllFloatBucketIterator", "float-iterator-set", 7),
    ("TestAllReverseFloatBucketIterator", "float-iterator-set", 9),
    ("TestFloatBucketIteratorTargetSchema", "float-iterator-set", 6),
    ("TestFloatCustomBucketsIterators", "float-iterator-set", 2),
    ("TestReverseFloatBucketIterator", "float-iterator-set", 1),
]


def fields(value, expected):
    if type(value) is not dict or value.keys() != set(expected):
        raise ValueError("missing or extra histogram kernel fields")


def compare_value(expected, actual, identity, path, failures, bit_differences, exact_numeric=False, exact_bits=False):
    if type(expected) is not type(actual):
        failures.append({"id": identity, "path": path, "reason": "changed field type or nil/empty distinction"})
        return
    if type(expected) is dict:
        if expected.keys() != actual.keys():
            failures.append({"id": identity, "path": path, "reason": "missing or extra fields"})
            return
        for key in expected:
            compare_value(expected[key], actual[key], identity, path + "." + key,
                          failures, bit_differences, exact_numeric, exact_bits)
    elif type(expected) is list:
        if len(expected) != len(actual):
            failures.append({"id": identity, "path": path, "reason": "changed bucket/span length"})
            return
        for i, (left, right) in enumerate(zip(expected, actual)):
            compare_value(left, right, identity, f"{path}[{i}]", failures, bit_differences, exact_numeric, exact_bits)
    elif type(expected) is str and re.fullmatch(r"[0-9a-f]{16}", expected):
        if re.fullmatch(r"[0-9a-f]{16}", actual) is None:
            raise ValueError("malformed candidate binary64 field")
        if expected != actual:
            bit_differences.append({"id": identity, "path": path, "expected": expected, "actual": actual})
        exact = exact_numeric or path.endswith(".zero_threshold") or ".custom_values[" in path
        if exact_bits or path.endswith((".lower", ".upper")):
            same = expected == actual
        else:
            same = bits(expected)[1] == bits(actual)[1] if exact else same_number(expected, actual)
        if not same:
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


def reduction_result(value, allow_nil=False):
    fields(value, ("result", "error", "histogram_error"))
    if type(value["error"]) is not str or type(value["histogram_error"]) is not bool:
        raise ValueError("invalid histogram reduction error")
    if value["result"] is not None or not allow_nil:
        histogram(value["result"], False)


ITERATOR_ROUTES = {"all", "all-reverse", "positive", "negative", "positive-reverse", "negative-reverse",
                   "low-positive", "low-negative"}


def iterator_result(value, routes):
    fields(value, ("buckets", "panic"))
    if type(value["panic"]) is not str:
        raise ValueError("invalid iterator panic")
    if value["panic"]:
        if value["buckets"] is not None:
            raise ValueError("iterator panic retained an incomplete result")
        return
    fields(value["buckets"], routes)
    for buckets in value["buckets"].values():
        if buckets is None:
            continue
        if type(buckets) is not list:
            raise ValueError("invalid iterator bucket list")
        for bucket in buckets:
            fields(bucket, ("lower", "upper", "count", "index", "lower_inclusive", "upper_inclusive"))
            for key in ("lower", "upper", "count"):
                number(bucket[key])
            integer(bucket["index"], -(1 << 31), (1 << 31) - 1)
            if any(type(bucket[key]) is not bool for key in ("lower_inclusive", "upper_inclusive")):
                raise ValueError("invalid bucket inclusivity")


def compare_declared(case, failures, bit_differences):
    declared, actual = case["declared"], case["actual"]
    if declared is None:
        return
    if case["mode"] == "float-reduce-resolution" and declared["result"] is None:
        # Original error assertions do not specify the post-failure receiver.
        declared = {key: declared[key] for key in ("error", "histogram_error")}
        actual = {key: actual[key] for key in declared}
    compare_value(declared, actual, case["id"], "declared", failures, bit_differences, exact_numeric=True)


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
    elif mode == "exponential-bound":
        fields(request, ("index", "schema"))
        for value in request.values():
            integer(value, -(1 << 31), (1 << 31) - 1)
        number(actual)
        if case["declared"] is not None:
            number(case["declared"])
    elif mode == "exponential-bound-table":
        fields(request, ())
        if type(actual) is not list or len(actual) != 9:
            raise ValueError("incomplete exponential bound table")
        for schema, values in enumerate(actual):
            if type(values) is not list or len(values) != 1 << schema:
                raise ValueError("changed exponential bound table layout")
            for value in values:
                number(value)
    elif mode == "float-iterator-set":
        fields(request, ("input", "routes", "start", "target_schema"))
        histogram(request["input"], False)
        number(request["start"])
        integer(request["target_schema"], -(1 << 31), (1 << 31) - 1)
        routes = request["routes"]
        if (type(routes) is not list or not routes or any(type(route) is not str for route in routes)
                or len(set(routes)) != len(routes) or not set(routes) <= ITERATOR_ROUTES):
            raise ValueError("invalid iterator route inventory")
        iterator_result(actual, routes)
        if case["declared"] is not None:
            iterator_result(case["declared"], routes)
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
    elif mode in {"float-compact", "float-copy-to-schema", "float-reduce-resolution"}:
        parameter = "max_empty" if mode == "float-compact" else "target_schema"
        fields(request, ("input", parameter))
        integer(request[parameter], -(1 << 31), (1 << 31) - 1)
        histogram(request["input"], False)
        if mode == "float-compact":
            histogram(actual, False)
            if case["declared"] is not None:
                histogram(case["declared"], False)
        elif mode == "float-copy-to-schema":
            for value in [actual] + ([] if case["declared"] is None else [case["declared"]]):
                fields(value, ("result", "input_after"))
                histogram(value["result"], False)
                histogram(value["input_after"], False)
        else:
            reduction_result(actual)
            if case["declared"] is not None:
                reduction_result(case["declared"], allow_nil=True)
    else:
        raise ValueError("unknown histogram kernel mode")
    failures = []
    compare_declared(case, failures, [])
    if failures:
        raise ValueError("captured kernel differs from its declared source assertion")


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
    for hint in range(4):
        for shape in range(4):
            result[f"supplement:copy-to-schema:{hint}:{shape}"] = "float-copy-to-schema"
    for shape in range(8):
        for max_empty in (-1, 0, 2):
            result[f"supplement:compact:{shape}:{max_empty}"] = "float-compact"
        result[f"supplement:reduce-resolution:{shape}"] = "float-reduce-resolution"
    for schema in range(9):
        for fraction in range(1 << schema):
            for exponent in (-1076, -1075, -1074, -1023, -1022, -1, 0, 1023, 1024, 1025):
                result[f"supplement:bound:{schema}:{fraction}:{exponent}"] = "exponential-bound"
    for schema in range(-4, 0):
        for i in range(12):
            result[f"supplement:bound-negative:{schema}:{i}"] = "exponential-bound"
    result["supplement:exponential-bound-table"] = "exponential-bound-table"
    for shape in range(12):
        for i in range(6):
            result[f"supplement:iterator:{shape}:{i}"] = "float-iterator-set"
    for shape in range(5):
        result[f"supplement:iterator-panic:{shape}"] = "float-iterator-set"
    return result


def validate_cases(data, tables, ledger=None):
    fields(data, ("schema_version", "commit", "go_version", "go_os", "go_arch",
                  "original_assertions_passed", "cases"))
    if (type(data["schema_version"]) is not int or data["schema_version"] != 1
            or data["commit"] != COMMIT or data["go_version"] != "go1.27.1"
            or data["go_os"] != "linux" or data["go_arch"] != "amd64"
            or data["original_assertions_passed"] is not True):
        raise ValueError("unexpected histogram kernel reference")
    if type(tables) is not list or len(tables) != len(TABLES):
        raise ValueError("incomplete histogram source table inventory")
    ledger = input_ledger() if ledger is None else ledger
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
        fields(table, ("test", "file", "mode", "cases", "line", "case_lines", "source_sha256", "prefix", "variable", "case_keys"))
        file = ("generic_test.go" if test == "TestGetBoundExponential" else
                "float_histogram_test.go" if test.startswith("TestFloat") or mode == "float-iterator-set" else "histogram_test.go")
        if (table["test"] != test or table["file"] != file or table["mode"] != mode
                or type(table["cases"]) is not int or table["cases"] != count
                or type(table["prefix"]) is not str or not table["prefix"].strip()
                or type(table["case_lines"]) is not list or len(table["case_lines"]) != count
                or re.fullmatch(r"[0-9a-f]{64}", table["source_sha256"]) is None):
            raise ValueError("changed histogram source table binding")
        integer(table["line"], 1, (1 << 31) - 1)
        for line in table["case_lines"]:
            integer(line, table["line"] + 1, (1 << 31) - 1)
        if test in {"TestFloatHistogramReduceResolution", "TestFloatBucketIteratorTargetSchema"}:
            variable = "tcs" if test == "TestFloatHistogramReduceResolution" else "cases"
            if (table["variable"] != variable or type(table["case_keys"]) is not list
                    or len(table["case_keys"]) != count or any(type(key) is not str or not key for key in table["case_keys"])
                    or len(set(table["case_keys"])) != count):
                raise ValueError("changed histogram source map keys")
            for key in table["case_keys"]:
                expected[f"{test}:{key}"] = mode
        else:
            variable = ("scenarios" if test == "TestGetBoundExponential" else
                        "h" if test == "TestReverseFloatBucketIterator" else "cases")
            if table["variable"] != variable or table["case_keys"] is not None:
                raise ValueError("changed histogram source table kind")
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
