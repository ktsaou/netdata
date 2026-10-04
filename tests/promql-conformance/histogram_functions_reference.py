"""Validate source-bound native helper inputs, diagnostics and original assertion scopes."""
import hashlib
import json
import math
import re

from build import directory
from compare import bits
from histogram_reference import compare_value, fields, histogram, input_ledger as histogram_ledger, integer, number
from histogram_functions_capture import COMMIT, HERE, HOOKS, MODULE, TEST, TEMPLATES, digest, generate_hook
from numerical_reference import PROFILE

SOURCE = "promql/promqltest/testdata/native_histograms.test"
PARSER_OPTIONS = {"EnableExperimentalFunctions": True, "ExperimentalDurationExpr": True,
                  "EnableExtendedRangeSelectors": True, "EnableBinopFillModifiers": True}
IDENTITIES = {"NativeHistogramQuantileNaNResultInfo", "NativeHistogramQuantileNaNSkewInfo",
              "NativeHistogramFractionNaNsInfo"}


def hex_bytes(value):
    if type(value) is not str or re.fullmatch(r"(?:[0-9a-f]{2})*", value) is None:
        raise ValueError("invalid native helper byte string")
    return bytes.fromhex(value)


def position(value):
    fields(value, ("start", "end"))
    for key in value:
        integer(value[key], -(1 << 63), (1 << 63) - 1)


def labels(value):
    if type(value) is not list:
        raise ValueError("invalid native helper labels")
    names = []
    for label in value:
        fields(label, ("name", "value"))
        names.append(hex_bytes(label["name"]))
        hex_bytes(label["value"])
    if names != sorted(set(names)):
        raise ValueError("unsorted or duplicate native helper label names")


def definitions(value):
    expected = {TEST: ("promql/quantile_test.go", 325, [439, 448, 462, 529]),
                "HistogramFraction": ("promql/quantile.go", 400, []),
                "HistogramQuantile": ("promql/quantile.go", 225, [])}
    fields(value, expected)
    for name, (source, line, calls) in expected.items():
        item = value[name]
        fields(item, ("source", "line", "body", "call_lines"))
        if (item["source"] != source or type(item["line"]) is not int or item["line"] != line
                or item["call_lines"] != calls or type(item["body"]) is not str
                or not item["body"].startswith(f"func {name}(") or not item["body"].endswith("\n}")):
            raise ValueError("native helper definition binding changed")


def input_ledger():
    return json.loads((HERE / "histogram-functions-original-inputs.json").read_bytes())


def roster(source_commands):
    expected = {}
    for index, count in enumerate((33, 15, 13, 14, 9)):
        for threshold in range(count):
            for direction in ("below", "above"):
                expected[f"{TEST}:{index:03d}:{threshold:03d}:{direction}"] = "histogram-fraction"
    expected[f"{TEST}:zero"] = "histogram-fraction"
    for index in range(2):
        expected[f"{TEST}:divergence:{index:03d}"] = "histogram-fraction"
    for command in source_commands:
        mode = "histogram-quantile" if command["function"] == "histogram_quantile" else "histogram-fraction"
        for index in range(len(command["row_lines"])):
            expected[f"dsl:native_histograms.test:{command['line']}:{index:03d}"] = mode
    for histogram_index in range(23):
        for quantile in range(18):
            expected[f"supplement:quantile:{histogram_index:03d}:{quantile:03d}"] = "histogram-quantile"
        for lower in range(17):
            for upper in range(17):
                expected[f"supplement:fraction:{histogram_index:03d}:{lower:03d}:{upper:03d}"] = "histogram-fraction"
    for metric in range(4):
        for context in range(16):
            for kind in range(3):
                expected[f"supplement:annotation:{metric}:{context}:{kind}"] = "histogram-fraction" if kind == 2 else "histogram-quantile"
    return expected


def validate_outcome(request, actual):
    fields(actual, ("value", "input_after", "annotations", "reciprocal", "negative_reciprocal_infinite", "positive_reciprocal_infinite"))
    for key in ("value", "reciprocal"):
        number(actual[key])
    histogram(actual["input_after"], False)
    if actual["input_after"] != request["input"]:
        raise ValueError("native helper changed its borrowed input")
    annos = actual["annotations"]
    if annos is not None:
        if type(annos) is not list or len(annos) > 1:
            raise ValueError("invalid native helper annotation container")
        for annotation in annos:
            fields(annotation, ("identity", "severity", "raw", "formatted"))
            if annotation["identity"] not in IDENTITIES or annotation["severity"] != "info":
                raise ValueError("unknown native helper annotation")
            quantile = "quantile" in request
            if (annotation["identity"] == "NativeHistogramFractionNaNsInfo") == quantile:
                raise ValueError("native helper annotation belongs to another function")
            hex_bytes(annotation["raw"])
            hex_bytes(annotation["formatted"])
    reciprocal = bits(actual["reciprocal"])[1]
    for key, expected in (("negative_reciprocal_infinite", reciprocal == -math.inf),
                          ("positive_reciprocal_infinite", reciprocal == math.inf)):
        if type(actual[key]) is not bool or actual[key] != expected:
            raise ValueError("invalid native helper reciprocal classification")


def validate_declaration(row):
    declared, actual = row["declared"], row["actual"]
    if declared is None:
        if row["source"] is not None or not row["id"].startswith("supplement:"):
            raise ValueError("missing original native helper declaration")
        return
    comparison = declared.get("comparison")
    if comparison == "trim_crosscheck":
        fields(declared, ("comparison", "trim_count", "total", "annotations_empty"))
        for key in ("trim_count", "total"):
            number(declared[key])
        value, total, count = (bits(text)[1] for text in (actual["value"], declared["total"], declared["trim_count"]))
        expected = value * total
        if (value == 0 and count != 0) or (value != 0 and (not math.isfinite(expected) or abs(expected-count)/abs(expected) > 1e-9)):
            raise ValueError("native fraction violates the original trim cross-check")
    elif comparison == "nan":
        fields(declared, ("comparison", "annotations_empty"))
        if not math.isnan(bits(actual["value"])[1]):
            raise ValueError("native fraction violates the original NaN assertion")
    elif comparison in {"exact", "almost"}:
        fields(declared, ("comparison", "value", "annotations_empty") if comparison == "exact" else ("comparison", "value"))
        number(declared["value"])
        if comparison == "exact":
            if bits(declared["value"])[1] != bits(actual["value"])[1]:
                raise ValueError("native fraction violates the original exact assertion")
        else:
            failures = []
            compare_value(declared["value"], actual["value"], row["id"], "declared", failures, [])
            if failures:
                raise ValueError("native helper violates its original almost.Equal assertion")
    else:
        raise ValueError("unknown native helper source assertion")
    if "annotations_empty" in declared and (declared["annotations_empty"] is not True or actual["annotations"] not in (None, [])):
        raise ValueError("native helper violates its original empty-annotation assertion")


def validate_commands(source_commands, commands):
    if type(source_commands) is not list or type(commands) is not list or len(source_commands) != 173 or len(commands) != 173:
        raise ValueError("incomplete native helper original command roster")
    source_by_line = {}
    totals = {"histogram_fraction": [0, 0], "histogram_quantile": [0, 0]}
    for source in source_commands:
        fields(source, ("line", "query", "function", "argument_ranges", "row_lines"))
        integer(source["line"], 1, (1 << 31)-1)
        if source["line"] in source_by_line or source["function"] not in totals:
            raise ValueError("duplicate or unknown original native helper source command")
        query = hex_bytes(source["query"])
        if type(source["row_lines"]) is not list or not source["row_lines"]:
            raise ValueError("missing native helper declared rows")
        for line in source["row_lines"]:
            integer(line, source["line"] + 1, (1 << 31)-1)
        if source["row_lines"] != sorted(set(source["row_lines"])):
            raise ValueError("duplicate native helper declared row line")
        ranges = source["argument_ranges"]
        if type(ranges) is not list or len(ranges) != (3 if source["function"] == "histogram_fraction" else 2):
            raise ValueError("incomplete native helper argument positions")
        for item in ranges:
            position(item)
            if not 0 <= item["start"] <= item["end"] <= len(query):
                raise ValueError("invalid original native helper argument position")
        source_by_line[source["line"]] = source
        totals[source["function"]][0] += 1
        totals[source["function"]][1] += len(source["row_lines"])
    if totals != {"histogram_fraction": [114, 118], "histogram_quantile": [59, 69]}:
        raise ValueError("native helper original source expansion changed")
    seen = set()
    for command in commands:
        fields(command, ("file", "line", "query", "function", "evaluation_time_ms", "argument_ranges", "argument_queries",
                         "parser_options", "declarations"))
        source = source_by_line.get(command["line"])
        if command["line"] in seen or source is None or any(command[key] != source[key] for key in ("query", "function", "argument_ranges")):
            raise ValueError("captured native helper command differs from original source")
        if command["file"] != SOURCE or command["parser_options"] != PARSER_OPTIONS:
            raise ValueError("native helper source file or parser profile changed")
        integer(command["evaluation_time_ms"], -(1 << 63), (1 << 63)-1)
        queries = command["argument_queries"]
        raw = hex_bytes(command["query"])
        if type(queries) is not list or len(queries) != len(source["argument_ranges"]):
            raise ValueError("native helper argument query omitted")
        for query, pos in zip(queries, source["argument_ranges"]):
            if type(query) is not str or query.encode() != raw[pos["start"]:pos["end"]]:
                raise ValueError("native helper argument differs from original expression")
        declaration = command["declarations"]
        fields(declaration, ("rows", "annotation_commands", "warn", "info", "ordered", "failure", "exclude_generated_range"))
        if any(type(declaration[key]) is not bool for key in ("warn", "info", "ordered", "failure", "exclude_generated_range")) or declaration["failure"]:
            raise ValueError("native helper original command assertion changed")
        rows = declaration["rows"]
        if type(rows) is not list or len(rows) != len(source["row_lines"]):
            raise ValueError("native helper source declaration expansion changed")
        for index, row in enumerate(rows):
            fields(row, ("position", "labels", "value"))
            if type(row["position"]) is not int or row["position"] != index + 1:
                raise ValueError("native helper original row order changed")
            labels(row["labels"])
            number(row["value"])
        if type(declaration["annotation_commands"]) is not list:
            raise ValueError("invalid native helper original annotation declarations")
        for annotation in declaration["annotation_commands"]:
            fields(annotation, ("kind", "type", "value"))
            integer(annotation["kind"], 0, 255)
            if annotation["type"] not in {"message", "pattern"}:
                raise ValueError("invalid native helper original annotation predicate")
            hex_bytes(annotation["value"])
        seen.add(command["line"])
    return {command["line"]: command for command in commands}


def validate_source_infos(rows, commands):
    observed = {line: set() for line in commands}
    for row in rows:
        if row["id"].startswith("dsl:"):
            observed[row["source"]["command"]["line"]].update(
                item["formatted"] for item in (row["actual"]["annotations"] or []))
    for line, command in commands.items():
        predicates = command["declarations"]["annotation_commands"]
        infos = [item for item in predicates if item["kind"] == 4]
        if infos:
            # The eight original info assertions use literal messages on bare native selectors.
            if any(item["type"] != "message" for item in infos):
                raise ValueError("unqualified original native helper info predicate")
            if observed[line] != {item["value"] for item in infos}:
                raise ValueError("native helper infos differ from the original source declarations")
        if any(item["kind"] == 5 for item in predicates) and observed[line]:
            raise ValueError("native helper violates the original no_info assertion")


def validate_cases(documents, ledger):
    fields(ledger, ("schema_version", "commit", "module", "source_manifest_sha256", "definitions", "source_commands", "commands", "cases"))
    if (type(ledger["schema_version"]) is not int or ledger["schema_version"] != 1 or ledger["commit"] != COMMIT
            or ledger["module"] != MODULE or ledger["source_manifest_sha256"] != histogram_ledger()["source_manifest_sha256"]):
        raise ValueError("native helper ledger attribution changed")
    definitions(ledger["definitions"])
    commands = validate_commands(ledger["source_commands"], ledger["commands"])
    expected = roster(ledger["source_commands"])
    if type(ledger["cases"]) is not list or len(ledger["cases"]) != len(expected):
        raise ValueError("incomplete native helper input ledger")
    bindings = {}
    for binding in ledger["cases"]:
        fields(binding, ("id", "mode", "request", "declared", "source"))
        if type(binding["id"]) is not str or binding["id"] in bindings or expected.get(binding["id"]) != binding["mode"]:
            raise ValueError("duplicate or unknown native helper input binding")
        bindings[binding["id"]] = binding
    fields(documents, HOOKS)
    seen = set()
    for package, document in documents.items():
        fields(document, ("schema_version", "commit", "go_version", "go_os", "go_arch", "original_assertions_passed", "cases", "commands"))
        if (type(document["schema_version"]) is not int or document["schema_version"] != 1 or document["commit"] != COMMIT
                or document["go_version"] != "go1.27.1" or document["go_os"] != "linux" or document["go_arch"] != "amd64"
                or document["original_assertions_passed"] is not True or type(document["cases"]) is not list
                or document["commands"] != (ledger["commands"] if package == "promqltest" else [])):
            raise ValueError("native helper reference profile changed")
        for row in document["cases"]:
            fields(row, ("id", "mode", "request", "actual", "declared", "source"))
            identity, mode, request, source = (row[key] for key in ("id", "mode", "request", "source"))
            if type(identity) is not str or identity in seen or expected.get(identity) != mode:
                raise ValueError("duplicate or unknown native helper outcome")
            if {key: row[key] for key in ("id", "mode", "request", "declared", "source")} != bindings.get(identity):
                raise ValueError("native helper request or declaration changed")
            if identity.startswith("dsl:") != (package == "promqltest"):
                raise ValueError("native helper case moved across source packages")
            fields(request, ("input", "metric_name", "query", "position", "quantile") if mode == "histogram-quantile"
                   else ("input", "metric_name", "query", "position", "lower", "upper"))
            histogram(request["input"], False)
            hex_bytes(request["metric_name"])
            hex_bytes(request["query"])
            position(request["position"])
            for key in (("quantile",) if mode == "histogram-quantile" else ("lower", "upper")):
                number(request[key])
            validate_outcome(request, row["actual"])
            validate_declaration(row)
            if identity.startswith("dsl:"):
                fields(source, ("command", "row", "argument_labels", "argument_sample_time_ms", "output_labels"))
                command = source["command"]
                if command != commands.get(command["line"]) or command["query"] != request["query"]:
                    raise ValueError("native helper outcome lost original command attribution")
                integer(source["row"], 1, len(command["declarations"]["rows"]))
                declared_row = command["declarations"]["rows"][source["row"]-1]
                if (source["output_labels"] != declared_row["labels"] or row["declared"] != {"comparison": "almost", "value": declared_row["value"]}
                        or identity != f"dsl:native_histograms.test:{command['line']}:{source['row']-1:03d}"):
                    raise ValueError("native helper outcome lost original row attribution")
                labels(source["argument_labels"])
                labels(source["output_labels"])
                integer(source["argument_sample_time_ms"], -(1 << 63), (1 << 63)-1)
                index = 1 if mode == "histogram-quantile" else 2
                if request["position"] != command["argument_ranges"][index]:
                    raise ValueError("native helper annotation uses the wrong argument position")
            elif identity.startswith(TEST):
                fields(source, ("test", "call_line", "case_index", "threshold_index", "direction"))
                line_by_direction = {"below": 439, "above": 448, "zero": 462, "divergence": 529}
                direction = source["direction"]
                if (type(direction) is not str or source["test"] != TEST
                        or source["call_line"] != line_by_direction.get(direction)):
                    raise ValueError("native helper lost original Go assertion attribution")
                index, threshold = source["case_index"], source["threshold_index"]
                if direction in {"below", "above"}:
                    integer(index, 0, 4)
                    integer(threshold, 0, (33, 15, 13, 14, 9)[index]-1)
                    expected_id = f"{TEST}:{index:03d}:{threshold:03d}:{direction}"
                elif direction == "zero":
                    integer(index, -1, -1)
                    integer(threshold, -1, -1)
                    expected_id = f"{TEST}:zero"
                elif direction == "divergence":
                    integer(index, 0, 1)
                    integer(threshold, -1, -1)
                    expected_id = f"{TEST}:divergence:{index:03d}"
                else:
                    raise ValueError("unknown original Go helper direction")
                if identity != expected_id:
                    raise ValueError("native helper source coordinates differ from its original assertion ID")
            seen.add(identity)
    if seen != expected.keys() or seen != bindings.keys():
        raise ValueError("native helper outcome inventory is incomplete")
    validate_source_infos([row for document in documents.values() for row in document["cases"]], commands)
    return {"original_cases": 358, "original_qt_fraction_calls": 171, "original_dsl_fraction_commands": 114,
            "original_dsl_quantile_commands": 59, "original_dsl_fraction_rows": 118, "original_dsl_quantile_rows": 69,
            "supplemental_cases": 7253, "total": len(expected)}


def load(path):
    path = directory(path)
    metadata = json.loads((path / "metadata.json").read_bytes())
    fields(metadata, ("schema_version", "commit", "profile", "counts", "generator_sha256", "templates",
                      "definitions_sha256", "source_commands_sha256", "source_files", "hooks", "outputs"))
    ledger = input_ledger()
    manifest = json.loads((path / "source-manifest.json").read_bytes())
    source_definitions = json.loads((path / "definitions.json").read_bytes())
    source_commands = json.loads((path / "source-commands.json").read_bytes())
    hooks = {key: generate_hook(key, source_definitions) for key in HOOKS}
    if (type(metadata["schema_version"]) is not int or metadata["schema_version"] != 1 or metadata["commit"] != COMMIT
            or metadata["profile"] != PROFILE or metadata["source_files"] != len(manifest)
            or digest(manifest) != ledger["source_manifest_sha256"] or source_definitions != ledger["definitions"]
            or source_commands != ledger["source_commands"] or metadata["definitions_sha256"] != digest(source_definitions)
            or metadata["source_commands_sha256"] != digest(source_commands)
            or metadata["templates"] != {name: hashlib.sha256((HERE / "capture" / name).read_bytes()).hexdigest() for name in TEMPLATES}
            or metadata["generator_sha256"] != hashlib.sha256((HERE / "capture/histogram_functions_sources.go").read_bytes()).hexdigest()
            or metadata["hooks"] != {key: hashlib.sha256(value).hexdigest() for key, value in hooks.items()}):
        raise ValueError("native helper source or capture attribution changed")
    facade = directory(path / "reference")
    paths = list(facade.rglob("*"))
    files = {item.relative_to(facade).as_posix() for item in paths if item.is_file()}
    if files != manifest.keys() | set(HOOKS.values()) or any(item.is_symlink() for item in paths):
        raise ValueError("unlisted or missing native helper source")
    for name, expected in manifest.items():
        if hashlib.sha256((facade / name).read_bytes()).hexdigest() != expected:
            raise ValueError("native helper reference source changed")
    for definition in source_definitions.values():
        lines = (facade / definition["source"]).read_text().splitlines(keepends=True)
        if not "".join(lines[definition["line"]-1:]).startswith(definition["body"]):
            raise ValueError("native helper definition differs from its original source position")
    lines = (facade / SOURCE).read_text().splitlines()
    for command in source_commands:
        query = hex_bytes(command["query"]).decode()
        if not lines[command["line"]-1].rstrip().endswith(query):
            raise ValueError("native helper command differs from its original source line")
        if any(not lines[line-1].lstrip().startswith("{") for line in command["row_lines"]):
            raise ValueError("native helper declared row differs from its original source line")
    documents = {}
    fields(metadata["outputs"], HOOKS)
    for package, relative in HOOKS.items():
        output = (path / (package + ".json")).read_bytes()
        if hashlib.sha256(output).hexdigest() != metadata["outputs"][package] or (facade / relative).read_bytes() != hooks[package]:
            raise ValueError("native helper output or additive hook changed")
        documents[package] = json.loads(output)
    counts = validate_cases(documents, ledger)
    if counts != metadata["counts"]:
        raise ValueError("native helper comparison inventory changed")
    return [row for document in documents.values() for row in document["cases"]], counts
