"""Original suffix and optional-character tables through complete selectors."""
import json
from pathlib import Path
import re

from regex_reference import PROMETHEUS_COMMIT, byte_string, integer, keys
from selector_source_cases import Pair, Panel

HERE = Path(__file__).resolve().parent
DATASET = "selector-suffix"
FUNCTIONS = (("TestStringMatcherFromRegexp_LiteralSuffix", "expectedLiteralSuffixMatchers", 4),
             ("TestStringMatcherFromRegexp_Quest", "expectedZeroOrOneMatchers", 6))
SOURCE = "model/labels/regexp_test.go"


def _unique_pairs(data):
    seen, result = {}, []
    for ti, table in enumerate(data["tables"]):
        for pi, row in enumerate(table["cases"]):
            pattern = byte_string(row["pattern"])
            for field, matches in (("expectedMatches", True), ("expectedNotMatches", False)):
                for si, subject in enumerate(row[field]):
                    subject = byte_string(subject)
                    identity = pattern, subject
                    if identity in seen:
                        if seen[identity] != matches:
                            raise ValueError("contradictory original selector declaration")
                        continue
                    seen[identity] = matches
                    result.append(Pair(f"{ti}:{pi}:{field}:{si}", pattern, subject, matches))
    return result


def _digest(value):
    if type(value) is not str or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError("invalid original selector digest")


def validate_inputs(data):
    keys(data, ("schema_version", "commit", "source", "source_sha256", "tables"))
    if (type(data["schema_version"]) is not int or data["schema_version"] != 1
            or data["commit"] != PROMETHEUS_COMMIT or data["source"] != SOURCE):
        raise ValueError("literal selector source identity changed")
    _digest(data["source_sha256"])
    if type(data["tables"]) is not list or len(data["tables"]) != len(FUNCTIONS):
        raise ValueError("original selector table roster changed")
    patterns, previous_function, previous_offset, totals = set(), 0, 0, [0, 0]
    for table, (function, count_field, count) in zip(data["tables"], FUNCTIONS, strict=True):
        keys(table, ("schema_version", "commit", "source", "source_sha256", "function",
                     "function_line", "function_sha256", "cases"))
        if (any(table[key] != data[key] for key in ("schema_version", "commit", "source", "source_sha256"))
                or type(table["schema_version"]) is not int or table["function"] != function):
            raise ValueError("original selector function identity changed")
        integer(table["function_line"], previous_function + 1)
        previous_function = previous_line = table["function_line"]
        _digest(table["function_sha256"])
        if type(table["cases"]) is not list or len(table["cases"]) != count:
            raise ValueError("original selector row roster changed")
        for row in table["cases"]:
            keys(row, ("pattern", "line", count_field, "expectedMatches", "expectedNotMatches",
                       "declaration_positions"))
            pattern = byte_string(row["pattern"])
            integer(row["line"], previous_line + 1)
            previous_line = row["line"]
            integer(row[count_field], 1)
            if pattern in patterns:
                raise ValueError("duplicate original selector pattern")
            patterns.add(pattern)
            positions = row["declaration_positions"]
            keys(positions, ("pattern", "expectedMatches", "expectedNotMatches"))
            for index, field in enumerate(("pattern", "expectedMatches", "expectedNotMatches")):
                values = [row[field]] if field == "pattern" else row[field]
                if type(values) is not list or not values:
                    raise ValueError("missing original selector declarations")
                for value in values:
                    byte_string(value)
                if index:
                    totals[index - 1] += len(values)
                if type(positions[field]) is not list or len(positions[field]) != len(values):
                    raise ValueError("original selector literal positions missing")
                for position in positions[field]:
                    keys(position, ("line", "column", "offset", "end_offset"))
                    integer(position["line"], row["line"])
                    integer(position["column"], 1)
                    integer(position["offset"], previous_offset)
                    integer(position["end_offset"], position["offset"] + 1)
                    previous_offset = position["end_offset"]
    pairs = _unique_pairs(data)
    if totals != [60, 41] or len(pairs) != 100 or sum(pair.matches for pair in pairs) != 59:
        raise ValueError("original selector declaration coverage changed")
    return data


def load_inputs():
    return validate_inputs(json.loads((HERE / "selector-suffix-inputs.json").read_bytes()))


def validate_extraction(data):
    validate_inputs(data)
    if data != load_inputs():
        raise ValueError("original selector extraction differs from committed ledger")


def _pairs():
    yield from _unique_pairs(load_inputs())


_panel = Panel(DATASET, "selector_suffix_case", _pairs)
cases = _panel.cases
validate_generated = _panel.validate_generated
validate_original = _panel.validate_original
compare_candidate = _panel.compare_candidate
write_fixtures = _panel.write_fixtures


if __name__ == "__main__":
    _panel.cli(__doc__, validate_extraction)
