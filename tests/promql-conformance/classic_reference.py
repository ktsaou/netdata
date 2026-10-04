"""Validate pinned classic source/input bindings, original assertions and mutable snapshots."""
import hashlib
import json
import math
import struct

from build import directory
from classic_capture import COMMIT, GO_FILES, HERE, HOOK, MODULE, PROFILE, TEMPLATES, digest, generate_hook
from compare import bits, same_number
from histogram_reference import fields, input_ledger as histogram_ledger, integer, number

OWNERS = {name: source for source, names in {
    "promql/quantile.go": ("BucketQuantile", "BucketFraction", "coalesceBuckets", "ensureMonotonicAndIgnoreSmallDeltas", "smallDeltaTolerance"),
    "promql/quantile_test.go": ("TestBucketQuantile_ForcedMonotonicity",),
    "util/almost/almost.go": ("Equal", "minNormal"), "util/almost/almost_test.go": ("TestEqual",),
    "src/slices/sort.go": ("SortFunc", "sortedHint", "unknownHint", "xorshift", "Next", "nextPowerOfTwo"),
    "src/slices/zsortanyfunc.go": ("insertionSortCmpFunc", "siftDownCmpFunc", "heapSortCmpFunc", "pdqsortCmpFunc", "partitionCmpFunc",
                                 "partitionEqualCmpFunc", "partialInsertionSortCmpFunc", "breakPatternsCmpFunc", "choosePivotCmpFunc",
                                 "order2CmpFunc", "medianCmpFunc", "medianAdjacentCmpFunc", "reverseRangeCmpFunc"),
    "src/sort/search.go": ("Search",),
}.items() for name in names}
QUANTILE_FIXTURES = {"simple - monotonic": False, "simple - non-monotonic middle": True,
                     "real example - monotonic": False, "real example - non-monotonic": True,
                     "real example 2 - monotonic": False, "real example 2 - non-monotonic": True}
QUANTILES = (1.0, .99, .9, .5)


def encoded(value):
    return struct.pack(">d", value).hex()


def input_ledger():
    return json.loads((HERE / "classic-original-inputs.json").read_bytes())


def buckets(value):
    if value is None:
        return
    if type(value) is not list:
        raise ValueError("invalid classic bucket array")
    for bucket in value:
        fields(bucket, ("upper_bound", "count"))
        number(bucket["upper_bound"])
        number(bucket["count"])


def validate_request(mode, request):
    if mode == "almost-equal":
        fields(request, ("left", "right", "epsilon"))
        for value in request.values():
            number(value)
        return
    if mode != "classic-buckets":
        raise ValueError("unknown classic kernel")
    fields(request, ("buckets", "operations"))
    buckets(request["buckets"])
    if type(request["operations"]) is not list or not request["operations"]:
        raise ValueError("missing classic operations")
    for operation in request["operations"]:
        if type(operation) is not dict or operation.get("kind") not in {"quantile", "fraction"}:
            raise ValueError("unknown classic operation")
        fields(operation, ("kind", "quantile") if operation["kind"] == "quantile" else ("kind", "lower", "upper"))
        for key, value in operation.items():
            if key != "kind":
                number(value)


def validate_outcome(mode, request, actual):
    validate_request(mode, request)
    if mode == "almost-equal":
        if type(actual) is not bool:
            raise ValueError("almost.Equal did not return a boolean")
        return
    fields(actual, ("steps",))
    if type(actual["steps"]) is not list or len(actual["steps"]) != len(request["operations"]):
        raise ValueError("incomplete classic operation sequence")
    length = len(request["buckets"] or [])
    bounds = {bucket["upper_bound"] for bucket in request["buckets"] or []}
    previous = request["buckets"]
    for operation, step in zip(request["operations"], actual["steps"]):
        fields(step, ("panicked", "result", "input_after"))
        buckets(step["input_after"])
        if ((step["input_after"] is None) != (request["buckets"] is None) or len(step["input_after"] or []) != length
                or any(bucket["upper_bound"] not in bounds for bucket in step["input_after"] or [])):
            raise ValueError("classic helper changed backing length/nil state or invented a bound")
        quantile = operation["kind"] == "quantile"
        q = bits(operation["quantile"])[1] if quantile else 0.0
        early = quantile and (math.isnan(q) or q < 0 or q > 1)
        panicked = length == 0 and not early
        if type(step["panicked"]) is not bool or step["panicked"] != panicked:
            raise ValueError("classic helper panic precondition changed")
        if panicked:
            if step["result"] is not None or step["input_after"] != previous:
                raise ValueError("panicking empty classic helper produced a result or mutation")
            continue
        result = step["result"]
        fields(result, ("value", "reciprocal", "negative_reciprocal_infinite", "positive_reciprocal_infinite",
                        "less_than_positive_infinity", "quantile"))
        number(result["value"])
        number(result["reciprocal"])
        value, reciprocal = bits(result["value"])[1], bits(result["reciprocal"])[1]
        for key, expected in (("negative_reciprocal_infinite", reciprocal == -math.inf),
                              ("positive_reciprocal_infinite", reciprocal == math.inf),
                              ("less_than_positive_infinity", value < math.inf)):
            if type(result[key]) is not bool or result[key] != expected:
                raise ValueError("classic composed comparison changed")
        expected_reciprocal = math.copysign(math.inf, value) if value == 0 else 1 / value
        if not same_number(encoded(expected_reciprocal), result["reciprocal"]):
            raise ValueError("classic reciprocal differs from its scalar")
        if quantile:
            details = result["quantile"]
            fields(details, ("forced_monotonicity", "fixed_precision", "min_bound", "max_bound", "max_decrease"))
            for flag in ("forced_monotonicity", "fixed_precision"):
                if type(details[flag]) is not bool:
                    raise ValueError("classic quantile classification is not boolean")
            for key in ("min_bound", "max_bound", "max_decrease"):
                number(details[key])
            corrected = not early and step["input_after"][-1]["upper_bound"] == encoded(math.inf)
            if not corrected:
                if details != {"forced_monotonicity": False, "fixed_precision": False, "min_bound": encoded(0.0),
                               "max_bound": encoded(0.0), "max_decrease": encoded(0.0)}:
                    raise ValueError("classic early return produced monotonicity metadata")
                expected = "7ff8000000000001" if math.isnan(q) or not early else encoded(-math.inf if q < 0 else math.inf)
                if result["value"] != expected:
                    raise ValueError("classic early return changed its source scalar")
            elif not details["forced_monotonicity"]:
                if (details["min_bound"], details["max_bound"], details["max_decrease"]) != (encoded(math.inf), encoded(-math.inf), encoded(0.0)):
                    raise ValueError("uncorrected classic quantile changed diagnostic sentinels")
            elif not bits(details["max_decrease"])[1] > 0:
                raise ValueError("forced classic quantile lacks a positive decrease")
            if early and step["input_after"] != previous:
                raise ValueError("special quantile mutated classic input")
        elif result["quantile"] is not None:
            raise ValueError("classic fraction invented quantile diagnostics")
        previous = step["input_after"]


def validate_declaration(row):
    declaration = row["declared"]
    if declaration is None:
        return
    if row["mode"] == "almost-equal":
        if type(declaration) is not bool or row["actual"] != declaration:
            raise ValueError("original almost.Equal assertion failed")
        return
    fields(declaration, ("comparison", "value", "epsilon", "forced_monotonicity", "fixed_precision"))
    if (declaration["comparison"] != "relative_epsilon" or declaration["epsilon"] != encoded(1e-12)
            or type(declaration["forced_monotonicity"]) is not bool or type(declaration["fixed_precision"]) is not bool):
        raise ValueError("original classic assertion scope changed")
    number(declaration["value"])
    step = row["actual"]["steps"][0]
    if step["panicked"]:
        raise ValueError("original classic assertion panicked")
    expected, actual = bits(declaration["value"])[1], bits(step["result"]["value"])[1]
    if expected == 0 or not abs(expected-actual)/abs(expected) <= 1e-12:
        raise ValueError("original classic relative-epsilon assertion failed")
    for flag in ("forced_monotonicity", "fixed_precision"):
        if step["result"]["quantile"][flag] != declaration[flag]:
            raise ValueError("original classic boolean assertion failed")


def roster():
    result = {f"original-quantile:{name.encode().hex()}:{encoded(q)}": "classic-buckets"
              for name in QUANTILE_FIXTURES for q in QUANTILES}
    result.update({f"original-almost:{index:02d}": "almost-equal" for index in range(10)})
    for index in range(34):
        result.update({f"supplement:bucket:{index:03d}:q:{q:02d}": "classic-buckets" for q in range(16)})
        result.update({f"supplement:bucket:{index:03d}:f:{bound:02d}:{side}": "classic-buckets"
                       for bound in range(13) for side in ("below", "above")})
        result.update({f"supplement:bucket:{index:03d}:{kind}": "classic-buckets" for kind in ("sequence", "reversed")})
    result.update({f"supplement:sort:{length:03d}:{pattern}": "classic-buckets"
                   for length in (12, 13, 49, 50, 51, 64, 127, 128, 257) for pattern in range(6)})
    result.update({f"supplement:almost:{left:02d}:{right:02d}:{epsilon}": "almost-equal"
                   for left in range(18) for right in range(18) for epsilon in range(8)})
    return result


def validate_cases(document, ledger):
    fields(ledger, ("schema_version", "commit", "module", "source_manifest_sha256", "go_manifest", "definitions", "cases"))
    if (type(ledger["schema_version"]) is not int or ledger["schema_version"] != 1 or ledger["commit"] != COMMIT
            or ledger["module"] != MODULE or ledger["source_manifest_sha256"] != histogram_ledger()["source_manifest_sha256"]):
        raise ValueError("classic input attribution changed")
    fields(ledger["go_manifest"], GO_FILES)
    fields(ledger["definitions"], OWNERS)
    for name, source in OWNERS.items():
        definition = ledger["definitions"][name]
        fields(definition, ("source", "line", "body", "call_lines"))
        integer(definition["line"], 1, 20000)
        if definition["source"] != source or type(definition["body"]) is not str or not definition["body"]:
            raise ValueError("classic source owner changed")
        if type(definition["call_lines"]) is not list:
            raise ValueError("classic source call positions changed")
        for line in definition["call_lines"]:
            integer(line, definition["line"], 20000)
    for name, lines in (("TestBucketQuantile_ForcedMonotonicity", [315]), ("TestEqual", [45])):
        if ledger["definitions"][name]["call_lines"] != lines:
            raise ValueError("classic original helper call position changed")
    fields(document, ("schema_version", "commit", "go_version", "go_os", "go_arch", "original_assertions_passed", "cases"))
    if (type(document["schema_version"]) is not int or document["schema_version"] != 1 or document["commit"] != COMMIT
            or document["go_version"] != "go1.27.1" or document["go_os"] != "linux" or document["go_arch"] != "amd64"
            or document["original_assertions_passed"] is not True):
        raise ValueError("classic original assertions or compiler profile changed")
    expected, bindings = roster(), {}
    if type(ledger["cases"]) is not list or len(ledger["cases"]) != len(expected):
        raise ValueError("incomplete classic input ledger")
    for binding in ledger["cases"]:
        fields(binding, ("id", "mode", "request", "declared", "source"))
        identity = binding["id"]
        if type(identity) is not str or identity in bindings or expected.get(identity) != binding["mode"]:
            raise ValueError("unknown or duplicate classic input binding")
        bindings[identity] = binding
    if type(document["cases"]) is not list or len(document["cases"]) != len(expected):
        raise ValueError("incomplete classic output inventory")
    seen = set()
    for row in document["cases"]:
        fields(row, ("id", "mode", "request", "declared", "source", "actual"))
        identity = row["id"]
        if (type(identity) is not str or identity in seen or expected.get(identity) != row["mode"]
                or {key: row[key] for key in ("id", "mode", "request", "declared", "source")} != bindings.get(identity)):
            raise ValueError("classic result/input binding changed")
        validate_outcome(row["mode"], row["request"], row["actual"])
        validate_declaration(row)
        if identity.startswith("original-quantile:"):
            fields(row["source"], ("file", "line", "fixture"))
            source = row["source"]
            integer(source["line"], 315, 315)
            parts = identity.split(":")
            fixture = bytes.fromhex(parts[1]).decode()
            if (source != {"file": "promql/quantile_test.go", "line": 315, "fixture": parts[1]}
                    or row["request"]["operations"] != [{"kind": "quantile", "quantile": parts[2]}]
                    or row["declared"] is None or row["declared"]["forced_monotonicity"] is not False
                    or row["declared"]["fixed_precision"] != QUANTILE_FIXTURES[fixture]):
                raise ValueError("classic original quantile scope changed")
        elif identity.startswith("original-almost:"):
            index = int(identity.split(":")[1])
            fields(row["source"], ("file", "line", "row"))
            integer(row["source"]["line"], 45, 45)
            integer(row["source"]["row"], index, index)
            if row["source"] != {"file": "util/almost/almost_test.go", "line": 45, "row": index} or type(row["declared"]) is not bool:
                raise ValueError("classic original almost.Equal scope changed")
        elif row["source"] is not None or row["declared"] is not None:
            raise ValueError("supplemental classic case was attributed as an original assertion")
        seen.add(identity)
    if seen != expected.keys() or seen != bindings.keys():
        raise ValueError("missing original or supplemental classic output")
    if sum(len(row["request"]["operations"]) for row in document["cases"] if row["mode"] == "classic-buckets") != 1676:
        raise ValueError("classic operation step expansion changed")
    return {"original_quantiles": 24, "original_almost": 10, "supplemental_bucket_cases": 1550,
            "supplemental_almost": 2592, "classic_steps": 1676, "total": len(expected)}


def load(path):
    path = directory(path)
    metadata = json.loads((path / "metadata.json").read_bytes())
    fields(metadata, ("schema_version", "commit", "profile", "counts", "generator_sha256", "definitions_sha256", "templates",
                      "source_files", "go_manifest_sha256", "hook_sha256", "output_sha256"))
    ledger = input_ledger()
    definitions = json.loads((path / "definitions.json").read_bytes())
    manifest = json.loads((path / "source-manifest.json").read_bytes())
    go_manifest = json.loads((path / "go-manifest.json").read_bytes())
    hook, raw = generate_hook(definitions), (path / "observations.json").read_bytes()
    if (type(metadata["schema_version"]) is not int or metadata["schema_version"] != 1 or metadata["commit"] != COMMIT
            or metadata["profile"] != PROFILE or metadata["source_files"] != len(manifest)
            or digest(manifest) != ledger["source_manifest_sha256"] or definitions != ledger["definitions"]
            or go_manifest != ledger["go_manifest"] or metadata["go_manifest_sha256"] != digest(go_manifest)
            or metadata["definitions_sha256"] != digest(definitions) or metadata["hook_sha256"] != hashlib.sha256(hook).hexdigest()
            or metadata["output_sha256"] != hashlib.sha256(raw).hexdigest()
            or metadata["generator_sha256"] != hashlib.sha256((HERE / "capture/classic_sources.go").read_bytes()).hexdigest()
            or metadata["templates"] != {name: hashlib.sha256((HERE / "capture" / name).read_bytes()).hexdigest() for name in TEMPLATES}):
        raise ValueError("classic reference source/artifact attribution changed")
    facade, go_facade = directory(path / "reference"), directory(path / "go-reference")
    for base, expected in ((facade, manifest | {HOOK: hashlib.sha256(hook).hexdigest()}), (go_facade, go_manifest)):
        paths = list(base.rglob("*"))
        if ({item.relative_to(base).as_posix() for item in paths if item.is_file()} != expected.keys()
                or any(item.is_symlink() for item in paths)):
            raise ValueError("unlisted classic reference source/hook")
        for name, sha256 in expected.items():
            if hashlib.sha256((base / name).read_bytes()).hexdigest() != sha256:
                raise ValueError("classic reference source changed")
    for definition in definitions.values():
        base = go_facade if definition["source"].startswith("src/") else facade
        lines = (base / definition["source"]).read_text().splitlines(keepends=True)
        if not "".join(lines[definition["line"]-1:]).startswith(definition["body"]):
            raise ValueError("classic definition differs from its source position")
    document = json.loads(raw)
    counts = validate_cases(document, ledger)
    if counts != metadata["counts"]:
        raise ValueError("classic source expansion counts changed")
    return document["cases"], counts
