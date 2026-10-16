"""Source-defined prefix combinations through complete PromQL selectors."""
import json
from pathlib import Path
import re

from regex_reference import PROMETHEUS_COMMIT, byte_string, integer, keys
from selector_source_cases import Pair, Panel

HERE = Path(__file__).resolve().parent
DATASET = "selector-prefix"
FUNCTION = "TestStringMatcherFromRegexp_LiteralPrefix"


def validate_inputs(data):
    keys(data, ("schema_version", "commit", "source", "source_sha256", "function",
                "function_line", "function_sha256", "cases"))
    if (type(data["schema_version"]) is not int or data["schema_version"] != 1
            or data["commit"] != PROMETHEUS_COMMIT
            or data["source"] != "model/labels/regexp_test.go" or data["function"] != FUNCTION):
        raise ValueError("prefix input source identity changed")
    for field in ("source_sha256", "function_sha256"):
        if type(data[field]) is not str or not re.fullmatch(r"[0-9a-f]{64}", data[field]):
            raise ValueError("invalid prefix source digest")
    integer(data["function_line"], 1)
    if type(data["cases"]) is not list or len(data["cases"]) != 5:
        raise ValueError("prefix source table roster changed")
    patterns, previous_line, positive, negative = set(), data["function_line"], 0, 0
    for row in data["cases"]:
        keys(row, ("pattern", "line", "expectedLiteralPrefixMatchers",
                   "expectedMatches", "expectedNotMatches"))
        pattern = byte_string(row["pattern"])
        integer(row["line"], previous_line + 1)
        integer(row["expectedLiteralPrefixMatchers"], 1)
        if pattern in patterns:
            raise ValueError("duplicate prefix pattern")
        patterns.add(pattern)
        previous_line = row["line"]
        subjects = []
        for field in ("expectedMatches", "expectedNotMatches"):
            if type(row[field]) is not list or not row[field]:
                raise ValueError("missing prefix declarations")
            subjects.append([byte_string(value) for value in row[field]])
        if len(set(subjects[0] + subjects[1])) != len(subjects[0]) + len(subjects[1]):
            raise ValueError("duplicate or contradictory prefix subject")
        positive += len(subjects[0])
        negative += len(subjects[1])
    if (positive, negative) != (32, 20):
        raise ValueError("prefix declaration coverage changed")
    return data


def load_inputs():
    return validate_inputs(json.loads((HERE / "selector-prefix-inputs.json").read_bytes()))


def validate_extraction(data):
    validate_inputs(data)
    if data != load_inputs():
        raise ValueError("original prefix extraction differs from the committed ledger")


def _pairs():
    for pi, row in enumerate(load_inputs()["cases"]):
        for field, matches in (("expectedMatches", True), ("expectedNotMatches", False)):
            for si, subject in enumerate(row[field]):
                yield Pair(f"{pi}:{field}:{si}", byte_string(row["pattern"]), byte_string(subject), matches)


_panel = Panel(DATASET, "selector_prefix_case", _pairs)
cases = _panel.cases
validate_generated = _panel.validate_generated
_comparison = _panel.comparison
validate_original = _panel.validate_original
compare_candidate = _panel.compare_candidate
write_fixtures = _panel.write_fixtures


if __name__ == "__main__":
    _panel.cli(__doc__, validate_extraction)
