"""Validate source-bound statistic inputs and original instant/range declarations."""
import hashlib
import json
import math
import struct

from build import directory
from compare import bits, same_number
from histogram_reference import compare_value, fields, histogram, input_ledger as histogram_ledger, integer, number
from histogram_functions_reference import hex_bytes, labels, position
from histogram_statistics_capture import COMMIT, HERE, HOOK, MODULE, PROFILE, TEMPLATES, digest, generate_hook

FUNCTIONS = {"histogram_" + name: "histogram-" + name for name in ("count", "sum", "avg", "stddev", "stdvar")}
OWNERS = {name: "promql/functions.go" for name in ("simpleHistogramFunc", "funcHistogramCount", "funcHistogramSum",
          "funcHistogramAvg", "histogramVariance", "funcHistogramStdDev", "funcHistogramStdVar")}
OWNERS["Inc"] = "util/kahansum/kahansum.go"
OPTIONS = {"EnableExperimentalFunctions": True, "ExperimentalDurationExpr": True,
           "EnableExtendedRangeSelectors": True, "EnableBinopFillModifiers": True}


def input_ledger():
    return json.loads((HERE / "histogram-statistics-original-inputs.json").read_bytes())


def validate_outcome(mode, request, actual):
    fields(request, ("input",))
    histogram(request["input"], False)
    fields(actual, ("value", "input_after", "annotations", "reciprocal", "negative_reciprocal_infinite",
                   "positive_reciprocal_infinite", "less_than_positive_infinity", "greater_than_compensation_threshold"))
    number(actual["value"])
    number(actual["reciprocal"])
    histogram(actual["input_after"], False)
    if actual["input_after"] != request["input"] or actual["annotations"] is not None:
        raise ValueError("statistic helper changed its input or empty diagnostics")
    if mode in {"histogram-count", "histogram-sum"} and actual["value"] != request["input"][mode[10:]]:
        raise ValueError("statistic helper changed an unchanged source field")
    value, reciprocal = bits(actual["value"])[1], bits(actual["reciprocal"])[1]
    for key, expected in (("negative_reciprocal_infinite", reciprocal == -math.inf),
                          ("positive_reciprocal_infinite", reciprocal == math.inf),
                          ("less_than_positive_infinity", value < math.inf),
                          ("greater_than_compensation_threshold", value > 1666666666666668.0)):
        if type(actual[key]) is not bool or actual[key] != expected:
            raise ValueError("statistic helper composition changed")
    expected_reciprocal = math.copysign(math.inf, value) if value == 0 else 1 / value
    if not same_number(struct.pack(">d", expected_reciprocal).hex(), actual["reciprocal"]):
        raise ValueError("statistic helper reciprocal differs from its scalar")


def validate_commands(ledger):
    definitions = ledger["definitions"]
    fields(definitions, OWNERS)
    for name, source in OWNERS.items():
        definition = definitions[name]
        fields(definition, ("source", "line", "body"))
        integer(definition["line"], 1, 20000)
        if (definition["source"] != source or type(definition["body"]) is not str
                or not definition["body"].startswith(f"func {name}(") or not definition["body"].endswith("\n}")):
            raise ValueError("statistic source owner changed")
    sources, commands = ledger["source_commands"], ledger["commands"]
    if type(sources) is not list or type(commands) is not list or len(sources) != 135 or len(commands) != 135:
        raise ValueError("incomplete original statistic command roster")
    source_by_key, command_by_key = {}, {}
    totals = {"extended_vectors.test": [0, 0], "histograms.test": [0, 0], "native_histograms.test": [0, 0]}
    for source in sources:
        fields(source, ("file", "line", "query", "function", "mode", "argument_range", "row_lines"))
        integer(source["line"], 1, 10000)
        name = source["file"].removeprefix("promql/promqltest/testdata/")
        key = source["file"], source["line"]
        if name not in totals or key in source_by_key or source["function"] not in FUNCTIONS or source["mode"] not in {"instant", "range"}:
            raise ValueError("unknown or duplicate statistic source command")
        query = hex_bytes(source["query"])
        position(source["argument_range"])
        start, end = (source["argument_range"][key] for key in ("start", "end"))
        if not 0 <= start <= end <= len(query):
            raise ValueError("invalid statistic argument source range")
        if type(source["row_lines"]) is not list or source["row_lines"] != sorted(set(source["row_lines"])):
            raise ValueError("invalid statistic declared row positions")
        for line in source["row_lines"]:
            integer(line, source["line"]+1, 10000)
        source_by_key[key] = source
        totals[name][source["mode"] == "range"] += 1
    if totals != {"extended_vectors.test": [33, 1], "histograms.test": [7, 0], "native_histograms.test": [94, 0]}:
        raise ValueError("original statistic source expansion changed")
    for command in commands:
        fields(command, ("file", "line", "query", "function", "mode", "evaluation_time_ms", "range_start_ms", "range_end_ms",
                         "range_step_ns", "argument_range", "argument_query", "parser_options", "declarations",
                         "caller_warnings", "caller_infos", "native_samples", "ignored_float_samples", "capture_boundary"))
        key = command["file"], command["line"]
        source = source_by_key.get(key)
        if source is None or key in command_by_key or any(command[field] != source[field] for field in
                                                        ("query", "function", "mode", "argument_range")):
            raise ValueError("captured statistic command lost original source attribution")
        if command["parser_options"] != OPTIONS or command["capture_boundary"] != "function_dispatch":
            raise ValueError("statistic parser profile changed")
        for field in ("evaluation_time_ms", "range_start_ms", "range_end_ms", "range_step_ns"):
            integer(command[field], -(1 << 63), (1 << 63)-1)
        if command["mode"] == "range" and (key != ("promql/promqltest/testdata/extended_vectors.test", 545)
                or (command["range_start_ms"], command["range_end_ms"], command["range_step_ns"]) != (0, 60000, 15000000000)):
            raise ValueError("original statistic range context changed")
        query, pos = hex_bytes(command["query"]), command["argument_range"]
        if type(command["argument_query"]) is not str or command["argument_query"].encode() != query[pos["start"]:pos["end"]]:
            raise ValueError("statistic argument differs from its source byte slice")
        declaration = command["declarations"]
        fields(declaration, ("rows", "annotation_commands", "warn", "info", "ordered", "failure", "exclude_generated_range"))
        for flag in ("warn", "info", "ordered", "failure", "exclude_generated_range"):
            if type(declaration[flag]) is not bool:
                raise ValueError("invalid statistic original assertion flags")
        if declaration["failure"] or type(declaration["rows"]) is not list or len(declaration["rows"]) != len(source["row_lines"]):
            raise ValueError("statistic declaration expansion changed")
        for index, row in enumerate(declaration["rows"]):
            fields(row, ("position", "labels", "values"))
            integer(row["position"], index+1, index+1)
            labels(row["labels"])
            if type(row["values"]) is not list or len(row["values"]) != (5 if command["mode"] == "range" else 1):
                raise ValueError("statistic declaration point expansion changed")
            for value in row["values"]:
                number(value)
        if type(declaration["annotation_commands"]) is not list:
            raise ValueError("invalid statistic caller annotation declarations")
        for annotation in declaration["annotation_commands"]:
            fields(annotation, ("kind", "type", "value"))
            integer(annotation["kind"], 0, 5)
            if annotation["type"] not in {"message", "pattern"}:
                raise ValueError("invalid statistic caller annotation predicate")
            hex_bytes(annotation["value"])
        for field in ("caller_warnings", "caller_infos"):
            if type(command[field]) is not list or command[field] != sorted(set(command[field])):
                raise ValueError("invalid inherited statistic argument diagnostics")
            for value in command[field]:
                hex_bytes(value)
        points = sum(len(row["values"]) for row in declaration["rows"])
        integer(command["native_samples"], points, points)
        ignored = int(command["file"].endswith("native_histograms.test") and command["line"] in {1702, 1706, 1710, 1714})
        integer(command["ignored_float_samples"], ignored, ignored)
        command_by_key[key] = command
    if sum(command["native_samples"] for command in commands) != 145 or sum(command["ignored_float_samples"] for command in commands) != 4:
        raise ValueError("original statistic native/ignored expansion changed")
    return command_by_key


def roster(commands):
    expected = {}
    for command in commands.values():
        name = command["file"].rsplit("/", 1)[1]
        for row in command["declarations"]["rows"]:
            for step in range(len(row["values"])):
                expected[f"dsl:{name}:{command['line']}:{row['position']-1:03d}:{step:03d}"] = FUNCTIONS[command["function"]]
    for index in range(225):
        for function, mode in FUNCTIONS.items():
            expected[f"supplement:{index:03d}:{function}"] = mode
    return expected


def validate_cases(document, ledger):
    fields(ledger, ("schema_version", "commit", "module", "source_manifest_sha256", "definitions", "source_commands", "commands", "cases"))
    if (type(ledger["schema_version"]) is not int or ledger["schema_version"] != 1 or ledger["commit"] != COMMIT
            or ledger["module"] != MODULE or ledger["source_manifest_sha256"] != histogram_ledger()["source_manifest_sha256"]):
        raise ValueError("statistic input ledger attribution changed")
    commands = validate_commands(ledger)
    expected = roster(commands)
    fields(document, ("schema_version", "commit", "go_version", "go_os", "go_arch", "original_assertions_passed", "cases", "commands"))
    if (type(document["schema_version"]) is not int or document["schema_version"] != 1 or document["commit"] != COMMIT
            or document["go_version"] != "go1.27.1" or document["go_os"] != "linux" or document["go_arch"] != "amd64"
            or document["original_assertions_passed"] is not True or document["commands"] != ledger["commands"]):
        raise ValueError("statistic capture profile or original command evidence changed")
    bindings = {}
    if type(ledger["cases"]) is not list or len(ledger["cases"]) != len(expected):
        raise ValueError("incomplete statistic input ledger")
    for binding in ledger["cases"]:
        fields(binding, ("id", "mode", "request", "declared", "source"))
        if type(binding["id"]) is not str or binding["id"] in bindings or expected.get(binding["id"]) != binding["mode"]:
            raise ValueError("unknown or duplicate statistic input binding")
        bindings[binding["id"]] = binding
    if type(document["cases"]) is not list or len(document["cases"]) != len(expected):
        raise ValueError("incomplete statistic output inventory")
    seen = set()
    for row in document["cases"]:
        fields(row, ("id", "mode", "request", "declared", "source", "actual"))
        identity = row["id"]
        if (type(identity) is not str or identity in seen or expected.get(identity) != row["mode"]
                or {key: row[key] for key in ("id", "mode", "request", "declared", "source")} != bindings.get(identity)):
            raise ValueError("statistic result/input binding changed")
        validate_outcome(row["mode"], row["request"], row["actual"])
        if identity.startswith("dsl:"):
            source = row["source"]
            fields(source, ("file", "line", "row", "step", "argument_labels", "sample_time_ms", "evaluation_time_ms", "output_labels"))
            command = commands.get((source["file"], source["line"]))
            if command is None:
                raise ValueError("statistic sample lost original command attribution")
            integer(source["row"], 1, len(command["declarations"]["rows"]))
            declaration = command["declarations"]["rows"][source["row"]-1]
            integer(source["step"], 0, len(declaration["values"])-1)
            labels(source["argument_labels"])
            labels(source["output_labels"])
            integer(source["sample_time_ms"], -(1 << 63), (1 << 63)-1)
            integer(source["evaluation_time_ms"], -(1 << 63), (1 << 63)-1)
            time = command["evaluation_time_ms"] if command["mode"] == "instant" else command["range_start_ms"] + source["step"]*15000
            name = command["file"].rsplit("/", 1)[1]
            if (source["evaluation_time_ms"] != time or source["output_labels"] != declaration["labels"]
                    or row["mode"] != FUNCTIONS[command["function"]]
                    or identity != f"dsl:{name}:{command['line']}:{source['row']-1:03d}:{source['step']:03d}"):
                raise ValueError("statistic sample differs from original row/time attribution")
            declared = {"comparison": "almost", "value": declaration["values"][source["step"]]}
            if row["declared"] != declared:
                raise ValueError("statistic sample lost its original numerical declaration")
            failures = []
            compare_value(declared["value"], row["actual"]["value"], identity, "declared", failures, [])
            if failures:
                raise ValueError("statistic sample violates original almost.Equal assertion")
        elif row["source"] is not None or row["declared"] is not None:
            raise ValueError("supplemental statistic was attributed as an original assertion")
        seen.add(identity)
    if seen != expected.keys() or seen != bindings.keys():
        raise ValueError("missing original or supplemental statistic output")
    return {"original_source_commands": 135, "original_instant_cases": 140, "original_range_cases": 5,
            "ignored_float_arguments": 4, "empty_argument_commands": 2, "supplemental_cases": 1125, "total": len(expected)}


def load(path):
    path = directory(path)
    metadata = json.loads((path / "metadata.json").read_bytes())
    fields(metadata, ("schema_version", "commit", "profile", "counts", "generator_sha256", "sources_sha256",
                      "templates", "source_files", "hook_sha256", "output_sha256"))
    ledger = input_ledger()
    sources = json.loads((path / "sources.json").read_bytes())
    manifest = json.loads((path / "source-manifest.json").read_bytes())
    hook, raw = generate_hook(), (path / "observations.json").read_bytes()
    if (type(metadata["schema_version"]) is not int or metadata["schema_version"] != 1 or metadata["commit"] != COMMIT
            or metadata["profile"] != PROFILE or metadata["source_files"] != len(manifest)
            or digest(manifest) != ledger["source_manifest_sha256"] or sources != {"definitions": ledger["definitions"], "commands": ledger["source_commands"]}
            or metadata["sources_sha256"] != digest(sources) or metadata["hook_sha256"] != hashlib.sha256(hook).hexdigest()
            or metadata["output_sha256"] != hashlib.sha256(raw).hexdigest()
            or metadata["generator_sha256"] != hashlib.sha256((HERE / "capture/histogram_statistics_sources.go").read_bytes()).hexdigest()
            or metadata["templates"] != {name: hashlib.sha256((HERE / "capture" / name).read_bytes()).hexdigest() for name in TEMPLATES}):
        raise ValueError("statistic reference source/artifact attribution changed")
    facade = directory(path / "reference")
    paths = list(facade.rglob("*"))
    if ({item.relative_to(facade).as_posix() for item in paths if item.is_file()} != manifest.keys() | {HOOK}
            or any(item.is_symlink() for item in paths) or (facade / HOOK).read_bytes() != hook):
        raise ValueError("unlisted statistic reference source/hook")
    for name, expected in manifest.items():
        if hashlib.sha256((facade / name).read_bytes()).hexdigest() != expected:
            raise ValueError("statistic reference source changed")
    for definition in sources["definitions"].values():
        lines = (facade / definition["source"]).read_text().splitlines(keepends=True)
        if not "".join(lines[definition["line"]-1:]).startswith(definition["body"]):
            raise ValueError("statistic definition differs from its original source position")
    for command in sources["commands"]:
        lines = (facade / command["file"]).read_text().splitlines()
        if not lines[command["line"]-1].rstrip().endswith(hex_bytes(command["query"]).decode()):
            raise ValueError("statistic query differs from its original source line")
        if any(not lines[line-1].lstrip().startswith("{") for line in command["row_lines"]):
            raise ValueError("statistic declaration row differs from its original source line")
    document = json.loads(raw)
    counts = validate_cases(document, ledger)
    if counts != metadata["counts"]:
        raise ValueError("statistic source expansion counts changed")
    return document["cases"], counts
