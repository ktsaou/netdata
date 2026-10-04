"""Validate immutable numerical source bindings and original assertion scope."""
import hashlib
import json
import math
from pathlib import Path
import re

from build import directory
from compare import bits
from histogram_reference import fields, integer, number
from numerical_capture import HERE, digest

PROFILE = {"GOVERSION": "go1.27.1", "GOOS": "linux", "GOARCH": "amd64", "GOAMD64": "v1", "GOEXPERIMENT": ""}
DEFINITIONS = {"vf", "frexp", "vffrexpSC", "frexpSC", "vffrexpBC", "frexpBC", "log", "log2", "vflogSC", "logSC",
               "sqrt", "vfsqrtSC", "sqrtSC", "TestFrexp", "TestLog", "TestLog2", "TestSqrt", "tolerance", "veryclose", "alike"}


def input_ledger():
    return json.loads((HERE / "numerical-original-inputs.json").read_bytes())


def result(value, mode):
    if mode == "math-frexp":
        fields(value, ("fraction", "exponent"))
        number(value["fraction"])
        integer(value["exponent"], -1073, 1024)
    elif mode == "math-set":
        fields(value, ("frexp", "log", "log2", "sqrt"))
        result(value["frexp"], "math-frexp")
        for key in ("log", "log2", "sqrt"):
            number(value[key])
    elif mode in {"math-log", "math-log2", "math-sqrt"}:
        number(value)
    else:
        raise ValueError("invalid numerical kernel mode")


def roster():
    expected = {}
    for test, mode in (("TestFrexp", "math-frexp"), ("TestLog", "math-log"),
                       ("TestLog2", "math-log2"), ("TestSqrt", "math-sqrt")):
        for index in range(10):
            expected[f"{test}:vf:{index:03d}"] = mode
        for index in range(5 if test == "TestFrexp" else 7):
            expected[f"{test}:sc:{index:03d}"] = mode
    for index in range(8):
        expected[f"TestFrexp:bc:{index:03d}"] = "math-frexp"
    expected["TestLog:constant"], expected["TestLog2:constant"] = "math-log", "math-log2"
    for exponent in range(-1074, 1024):
        expected[f"TestLog2:power:{exponent}"] = "math-log2"
        for direction in range(3):
            expected[f"supplement:power:{exponent}:{direction}"] = "math-set"
    for bound in range(511):
        for direction in range(3):
            expected[f"supplement:bound:{bound}:{direction}"] = "math-set"
    for index in range(12):
        expected[f"supplement:special:{index}"] = "math-set"
    return expected


def assertion_matches(declared, actual, comparison):
    if type(declared) is dict:
        return (declared["exponent"] == actual["exponent"]
                and assertion_matches(declared["fraction"], actual["fraction"], comparison))
    left, right = bits(declared)[1], bits(actual)[1]
    if comparison == "alike":
        return ((math.isnan(left) and math.isnan(right))
                or (left == right and (int(declared, 16) >> 63) == (int(actual, 16) >> 63)))
    if left == right:
        return True
    if comparison == "exact":
        return False
    # Original veryclose passes the computed result as tolerance's second argument.
    return abs(left - right) < abs(4e-16 * right) if right != 0 else abs(left - right) < 4e-16


def validate_cases(data, ledger):
    fields(ledger, ("schema_version", "go_version", "profile", "source_manifest_sha256", "definitions",
                    "histogram_source_manifest_sha256", "bounds", "cases"))
    if ledger["schema_version"] != 1 or type(ledger["schema_version"]) is not int or ledger["go_version"] != "go1.27.1" or ledger["profile"] != PROFILE:
        raise ValueError("numerical ledger profile changed")
    for key in ("source_manifest_sha256", "histogram_source_manifest_sha256"):
        if type(ledger[key]) is not str or re.fullmatch(r"[0-9a-f]{64}", ledger[key]) is None:
            raise ValueError("invalid numerical source manifest binding")
    fields(ledger["definitions"], DEFINITIONS)
    for definition in ledger["definitions"].values():
        fields(definition, ("source", "line", "body"))
        integer(definition["line"], 1, (1 << 31) - 1)
        if definition["source"] != "src/math/all_test.go" or type(definition["body"]) is not str or not definition["body"]:
            raise ValueError("invalid numerical source declaration")
    if type(ledger["bounds"]) is not list or len(ledger["bounds"]) != 511:
        raise ValueError("incomplete numerical bound inputs")
    for value in ledger["bounds"]:
        number(value)
    fields(data, ("schema_version", "go_version", "go_os", "go_arch", "original_assertions_passed", "cases"))
    if (type(data["schema_version"]) is not int or data["schema_version"] != 1 or data["go_version"] != "go1.27.1"
            or data["go_os"] != "linux" or data["go_arch"] != "amd64" or data["original_assertions_passed"] is not True):
        raise ValueError("numerical reference profile changed")
    expected = roster()
    if type(ledger["cases"]) is not list or type(data["cases"]) is not list or len(ledger["cases"]) != len(expected) or len(data["cases"]) != len(expected):
        raise ValueError("incomplete numerical capture")
    bindings = {}
    for row in ledger["cases"]:
        fields(row, ("id", "mode", "request", "declared"))
        if type(row["id"]) is not str or row["id"] in bindings or expected.get(row["id"]) != row["mode"]:
            raise ValueError("duplicate or changed numerical input binding")
        bindings[row["id"]] = row
    seen = set()
    for row in data["cases"]:
        fields(row, ("id", "mode", "request", "actual", "declared"))
        if type(row["id"]) is not str or row["id"] in seen or expected.get(row["id"]) != row["mode"]:
            raise ValueError("duplicate or changed numerical identity")
        if {key: row[key] for key in ("id", "mode", "request", "declared")} != bindings.get(row["id"]):
            raise ValueError("numerical request or declaration changed")
        fields(row["request"], ("input",))
        number(row["request"]["input"])
        result(row["actual"], row["mode"])
        supplement = row["id"].startswith("supplement:")
        if (row["declared"] is None) != supplement:
            raise ValueError("missing or invented numerical assertion")
        if not supplement:
            fields(row["declared"], ("comparison", "value"))
            if row["declared"]["comparison"] not in {"exact", "alike", "veryclose"}:
                raise ValueError("invalid numerical assertion kind")
            result(row["declared"]["value"], row["mode"])
            if not assertion_matches(row["declared"]["value"], row["actual"], row["declared"]["comparison"]):
                raise ValueError("numerical result differs from original assertion")
        seen.add(row["id"])
    if seen != bindings.keys() or seen != expected.keys():
        raise ValueError("numerical ledger is incomplete")
    return {"original_cases": 2174, "supplemental_cases": 7839, "total": len(expected),
            "primitive_invocations": 2174 + 7839 * 4}


def load(path):
    path = directory(path)
    metadata = json.loads((path / "metadata.json").read_bytes())
    fields(metadata, ("schema_version", "go_version", "profile", "counts", "source_manifest_sha256", "definitions_sha256",
                      "bounds_sha256", "output_sha256", "hook_sha256", "generator_sha256"))
    manifest = json.loads((path / "sources.json").read_bytes())
    definitions = json.loads((path / "definitions.json").read_bytes())
    bounds = json.loads((path / "bounds.json").read_bytes())
    ledger = input_ledger()
    output = (path / "numerical.json").read_bytes()
    hook = (HERE / "capture/numerical_test.go.in").read_bytes()
    if (type(metadata["schema_version"]) is not int or metadata["schema_version"] != 1 or metadata["go_version"] != "go1.27.1"
            or metadata["profile"] != PROFILE or metadata["source_manifest_sha256"] != digest(manifest)
            or ledger["source_manifest_sha256"] != digest(manifest) or definitions != ledger["definitions"] or bounds != ledger["bounds"]
            or metadata["definitions_sha256"] != digest(definitions) or metadata["bounds_sha256"] != digest(bounds)
            or metadata["output_sha256"] != hashlib.sha256(output).hexdigest() or metadata["hook_sha256"] != hashlib.sha256(hook).hexdigest()
            or (path / "hook_test.go").read_bytes() != hook
            or metadata["generator_sha256"] != hashlib.sha256((HERE / "capture/numerical_sources.go").read_bytes()).hexdigest()):
        raise ValueError("numerical source, input or output attribution changed")
    reference = directory(path / "reference")
    names = {name.relative_to(reference).as_posix() for name in reference.rglob("*") if name.is_file()}
    if names != manifest.keys():
        raise ValueError("unlisted or missing numerical source file")
    for name, expected in manifest.items():
        source = reference / name
        if source.is_symlink() or hashlib.sha256(source.read_bytes()).hexdigest() != expected:
            raise ValueError("numerical reference source changed")
    lines = (reference / "src/math/all_test.go").read_text().splitlines(keepends=True)
    for definition in definitions.values():
        if not "".join(lines[definition["line"] - 1:]).startswith(definition["body"]):
            raise ValueError("numerical declaration differs from its source position")
    data = json.loads(output)
    counts = validate_cases(data, ledger)
    if metadata["counts"] != counts:
        raise ValueError("numerical invocation inventory changed")
    return data, counts
