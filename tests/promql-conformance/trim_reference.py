"""Validate exact source/input bindings and original fraction/trim assertion scope."""
import hashlib
import json
import math

from build import directory
from compare import bits
from histogram_reference import bucket, fields, histogram, input_ledger as histogram_ledger, integer, number
from trim_capture import COMMIT, HERE, HOOKS, MODULE, TESTS, digest, generate_hook

SOURCE_DEFINITIONS = {
    "TestBucketFractionBelow": ("model/histogram/generic_test.go", 115, [176]),
    "TestFloatHistogramTrimBucketsInPlace": ("model/histogram/float_histogram_test.go", 4693, [4707, 4716]),
    "TestTrimBuckets_HistogramFractionCrossCheck": ("promql/quantile_test.go", 325, [438, 447, 466, 522]),
}


def original_ids():
    ids = {f"TestBucketFractionBelow:{i:03d}": "bucket-fraction" for i in range(7)}
    ids.update({f"TestFloatHistogramTrimBucketsInPlace:{i:03d}": "float-trim" for i in range(2)})
    for i, thresholds in enumerate((33, 15, 13, 14, 9)):
        for j in range(thresholds):
            for direction in ("upper", "lower"):
                ids[f"TestTrimBuckets_HistogramFractionCrossCheck:{i:03d}:{j:03d}:{direction}"] = "float-trim"
    ids["TestTrimBuckets_HistogramFractionCrossCheck:zero"] = "float-trim"
    ids.update({f"TestTrimBuckets_HistogramFractionCrossCheck:divergence:{i:03d}": "float-trim" for i in range(2)})
    return ids


def supplemental_ids():
    ids = {f"supplement:fraction:{i:03d}:{j:03d}:{k}": "bucket-fraction"
           for i in range(16) for j in range(13) for k in range(2)}
    ids.update({f"supplement:trim:{i:03d}:{j:03d}:{k}:{l}": "float-trim"
                for i in range(14) for j in range(19) for k in range(2) for l in range(2)})
    ids["supplement:trim-equality"] = "float-trim-equality"
    return ids


def definitions(value):
    fields(value, SOURCE_DEFINITIONS)
    for name, (source, line, calls) in SOURCE_DEFINITIONS.items():
        definition = value[name]
        fields(definition, ("source", "line", "body", "call_lines"))
        if (definition["source"] != source or type(definition["line"]) is not int or definition["line"] != line
                or definition["call_lines"] != calls or type(definition["body"]) is not str
                or not definition["body"].startswith(f"func {name}(t *testing.T) {{\n")
                or not definition["body"].endswith("\n}")):
            raise ValueError("changed trim source definition binding")


def input_ledger():
    ledger = json.loads((HERE / "trim-original-inputs.json").read_bytes())
    fields(ledger, ("schema_version", "commit", "module", "source_manifest_sha256", "definitions", "cases"))
    if (type(ledger["schema_version"]) is not int or ledger["schema_version"] != 1
            or ledger["commit"] != COMMIT or ledger["module"] != MODULE
            or ledger["source_manifest_sha256"] != histogram_ledger()["source_manifest_sha256"]):
        raise ValueError("trim input ledger source pin changed")
    definitions(ledger["definitions"])
    return ledger


def validate_outcome(request, actual):
    fields(actual, ("receiver_after", "input_after", "returned_receiver", "panic"))
    for key in ("receiver_after", "input_after"):
        histogram(actual[key], False)
    if type(actual["returned_receiver"]) is not bool or type(actual["panic"]) is not str:
        raise ValueError("invalid trim lifecycle outcome")
    if actual["returned_receiver"] != (not actual["panic"]):
        raise ValueError("trim did not return its receiver")
    if request["copy"]:
        if actual["input_after"] != request["input"]:
            raise ValueError("trim of a copy mutated the original")
    elif actual["input_after"] != actual["receiver_after"]:
        raise ValueError("in-place trim lost its receiver mutation")
    for key in ("reset_hint", "schema", "zero_threshold", "custom_values"):
        if key == "custom_values" and request["copy"] and request["input"]["schema"] != -53:
            expected = None
        elif key == "zero_threshold" and request["copy"] and request["input"]["schema"] == -53:
            expected = "0000000000000000"
        else:
            expected = request["input"][key]
        if actual["receiver_after"][key] != expected:
            raise ValueError("trim changed immutable histogram metadata")


def validate_case(row):
    fields(row, ("id", "mode", "request", "actual", "declared"))
    if type(row["id"]) is not str or not row["id"]:
        raise ValueError("invalid trim case identity")
    request, actual, declared = (row[key] for key in ("request", "actual", "declared"))
    if row["mode"] == "bucket-fraction":
        fields(request, ("bucket", "value", "linear"))
        bucket(request["bucket"])
        number(request["value"])
        if type(request["linear"]) is not bool:
            raise ValueError("invalid bucket interpolation mode")
        number(actual)
        if declared is not None:
            fields(declared, ("comparison", "value"))
            number(declared["value"])
            if declared["comparison"] != "absolute":
                raise ValueError("changed original bucket-fraction assertion")
            difference = abs(bits(declared["value"])[1] - bits(actual)[1])
            if not math.isfinite(difference) or difference > 1e-12:
                raise ValueError("Go fraction outcome violates its original assertion")
    elif row["mode"] == "float-trim":
        fields(request, ("input", "threshold", "upper_trim", "copy"))
        histogram(request["input"], False)
        number(request["threshold"])
        if any(type(request[key]) is not bool for key in ("upper_trim", "copy")):
            raise ValueError("invalid trim direction or ownership mode")
        validate_outcome(request, actual)
        if declared is not None:
            if actual["panic"]:
                raise ValueError("original trim assertion recovered a panic")
            if declared.get("comparison") == "lifecycle":
                fields(declared, ("comparison", "same_receiver", "preserved_input", "changed"))
                for key in ("same_receiver", "preserved_input", "changed"):
                    if type(declared[key]) is not bool and declared[key] is not None:
                        raise ValueError("invalid original trim ownership assertion")
                observed = {"same_receiver": actual["returned_receiver"],
                            "preserved_input": actual["input_after"] == request["input"],
                            "changed": actual["receiver_after"] != request["input"]}
                if any(value is not None and observed[key] != value
                       for key, value in declared.items() if key != "comparison"):
                    raise ValueError("Go trim outcome violates its original ownership assertion")
            else:
                fields(declared, ("comparison", "value"))
                number(declared["value"])
                expected, observed = bits(declared["value"])[1], bits(actual["receiver_after"]["count"])[1]
                comparison = declared["comparison"]
                if comparison == "exact":
                    valid = expected == observed
                elif comparison == "relative":
                    valid = (expected != 0 and math.isfinite(expected) and math.isfinite(observed)
                             and abs(expected - observed) / abs(expected) <= 1e-9)
                else:
                    raise ValueError("changed original trim Count assertion")
                if not valid:
                    raise ValueError("Go trim Count violates its original assertion")
    elif row["mode"] == "float-trim-equality":
        fields(request, ("trim", "peer"))
        histogram(request["peer"], False)
        fields(actual, ("trim", "forward", "reverse", "borrowed_forward", "borrowed_reverse", "peer_after"))
        histogram(actual["peer_after"], False)
        if actual["peer_after"] != request["peer"]:
            raise ValueError("trim equality consumer mutated its peer")
        for key in ("forward", "reverse", "borrowed_forward", "borrowed_reverse"):
            if type(actual[key]) is not bool:
                raise ValueError("invalid trim equality consumer outcome")
        if declared is not None:
            raise ValueError("invented original trim equality assertion")
        validate_case({"id": row["id"], "mode": "float-trim", "request": request["trim"],
                       "actual": actual["trim"], "declared": None})
    else:
        raise ValueError("unknown fraction/trim mode")


def validate_cases(documents, ledger):
    definitions(ledger["definitions"])
    fields(documents, TESTS)
    bindings = {}
    for row in ledger["cases"]:
        fields(row, ("id", "mode", "request", "declared"))
        if type(row["id"]) is not str or not row["id"] or row["id"] in bindings:
            raise ValueError("invalid or duplicate trim input binding")
        bindings[row["id"]] = row
    originals = original_ids()
    expected = originals | supplemental_ids()
    seen = set()
    for package, data in documents.items():
        fields(data, ("schema_version", "commit", "go_version", "go_os", "go_arch", "original_assertions_passed", "cases"))
        if (type(data["schema_version"]) is not int or data["schema_version"] != 1 or data["commit"] != COMMIT
                or data["go_version"] != "go1.27.1" or data["go_os"] != "linux" or data["go_arch"] != "amd64"
                or data["original_assertions_passed"] is not True or type(data["cases"]) is not list):
            raise ValueError("trim reference assertion or build profile changed")
        for row in data["cases"]:
            validate_case(row)
            identity = row["id"]
            if identity in seen or expected.get(identity) != row["mode"]:
                raise ValueError("duplicate or changed fraction/trim case inventory")
            if identity in originals and not any(identity.startswith(test + ":") for test in TESTS[package]):
                raise ValueError("trim observation moved to the wrong source package")
            if identity not in originals and package != "histogram":
                raise ValueError("supplemental trim observation moved to the wrong package")
            if (row["declared"] is not None) != (identity in originals):
                raise ValueError("missing original or invented supplemental trim assertion")
            if {key: row[key] for key in ("id", "mode", "request", "declared")} != bindings.get(identity):
                raise ValueError("fraction/trim request or declaration changed")
            seen.add(identity)
    if seen != expected.keys() or bindings.keys() != expected.keys():
        raise ValueError("incomplete fraction/trim source or input inventory")
    return {"original_cases": len(originals), "supplemental_cases": len(expected) - len(originals), "total": len(expected)}


def load(path):
    path = directory(path)
    ledger = input_ledger()
    manifest = json.loads((path / "source-manifest.json").read_bytes())
    source_definitions = json.loads((path / "definitions.json").read_bytes())
    metadata = json.loads((path / "metadata.json").read_bytes())
    fields(metadata, ("schema_version", "commit", "counts", "generator_sha256", "template_sha256", "supplemental_sha256",
                      "definitions_sha256", "source_files", "hooks", "outputs"))
    fields(metadata["hooks"], TESTS)
    fields(metadata["outputs"], TESTS)
    integer(metadata["source_files"], 1, (1 << 31) - 1)
    if (type(metadata["schema_version"]) is not int or metadata["schema_version"] != 1 or metadata["commit"] != COMMIT
            or metadata["source_files"] != len(manifest) or digest(manifest) != ledger["source_manifest_sha256"]
            or source_definitions != ledger["definitions"] or digest(source_definitions) != metadata["definitions_sha256"]
            or metadata["generator_sha256"] != hashlib.sha256((HERE / "capture/trim_sources.go").read_bytes()).hexdigest()
            or metadata["template_sha256"] != hashlib.sha256((HERE / "capture/trim_test.go.in").read_bytes()).hexdigest()
            or metadata["supplemental_sha256"] != hashlib.sha256((HERE / "capture/trim_supplemental.go.in").read_bytes()).hexdigest()):
        raise ValueError("fraction/trim source attribution or generator changed")
    facade = directory(path / "reference")
    files = {item.relative_to(facade).as_posix() for item in facade.rglob("*") if item.is_file()}
    if files != manifest.keys() | set(HOOKS.values()) or any(item.is_symlink() for item in facade.rglob("*")):
        raise ValueError("unlisted or unsafe fraction/trim reference source")
    for relative, expected in manifest.items():
        if hashlib.sha256((facade / relative).read_bytes()).hexdigest() != expected:
            raise ValueError("fraction/trim reference source changed")
    for definition in source_definitions.values():
        raw = (facade / definition["source"]).read_text()
        tail = "".join(raw.splitlines(keepends=True)[definition["line"] - 1:])
        if not tail.startswith(definition["body"]):
            raise ValueError("fraction/trim body is not at its original source position")
    documents = {}
    for package in TESTS:
        hook = generate_hook(package, source_definitions)
        if (facade / HOOKS[package]).read_bytes() != hook or hashlib.sha256(hook).hexdigest() != metadata["hooks"][package]:
            raise ValueError("fraction/trim observation hook changed")
        raw = (path / (package + ".json")).read_bytes()
        if hashlib.sha256(raw).hexdigest() != metadata["outputs"][package]:
            raise ValueError("fraction/trim output digest changed")
        documents[package] = json.loads(raw)
    counts = validate_cases(documents, ledger)
    if metadata["counts"] != counts:
        raise ValueError("fraction/trim recorded coverage changed")
    return [row for data in documents.values() for row in data["cases"]], counts
