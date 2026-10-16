"""Source-defined prefix combinations through complete PromQL selectors."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import re

from build import directory
from compare import checked_results, compare
from info_cases import series
from query import checked_candidate_packets, dataset_hex, request_hex
from regex_reference import PROMETHEUS_COMMIT, byte_string, integer, keys
from utf8_work_cases import _quote

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


def cases():
    records, requests, results = [], [], []
    for pi, row in enumerate(load_inputs()["cases"]):
        pattern = _quote(byte_string(row["pattern"]))
        for field, matches in (("expectedMatches", True), ("expectedNotMatches", False)):
            for si, subject in enumerate(row[field]):
                ordinal = len(records)
                case_id = f"{ordinal:03d}"
                record = series({"__name__": "selector_prefix_case", "case_id": case_id,
                                 "subject": byte_string(subject)}, [(1000, ordinal + 1, "float")])
                records.append(record)
                for operator in ("=~", "!~"):
                    ident = f"selector-prefix:{pi}:{field}:{si}:{operator}"
                    query = f"selector_prefix_case{{case_id={_quote(case_id.encode())},subject{operator}{pattern}}}"
                    requests.append({"id": ident, "query": query, "profile": "server-default",
                                     "mode": "instant", "time_ms": 1000, "start_ms": 0,
                                     "end_ms": 0, "step_ms": 1000, "lookback_ms": 300000,
                                     "subquery_step_ms": 60000})
                    accepted = matches if operator == "=~" else not matches
                    results.append({"id": ident, "kind": "vector", "warnings": [], "infos": [],
                                    "rows": [deepcopy(record)] if accepted else []})
    return {"schema_version": 1, "series": records}, requests, {"schema_version": 1, "results": results}


def validate_generated(dataset, requests):
    expected_dataset, expected_requests, _ = cases()
    if dataset != expected_dataset or requests != expected_requests:
        raise ValueError("generated prefix dataset or request identity changed")


def _comparison(expected, actual):
    _, requests, _ = cases()
    checked_results(actual)
    if [row["id"] for row in actual["results"]] != [row["id"] for row in requests]:
        raise ValueError("prefix result IDs are missing, extra or reordered")
    report = compare(expected, actual, [{"id": row["id"], "ordered": True, "exact_numeric": True}
                                        for row in requests])
    if not report["semantic_pass"] or not report["bitwise_pass"]:
        raise ValueError("complete prefix results differ: " + json.dumps(report))
    return report


def validate_original(data):
    expected_header = {"schema_version": 1, "reference": "github.com/prometheus/prometheus@v0.315.0",
                       "commit": PROMETHEUS_COMMIT, "go_version": "go1.27.1", "go_arch": "amd64",
                       "go_os": "linux", "unicode_version": "17.0.0"}
    keys(data, (*expected_header, "results"))
    if ({key: data[key] for key in expected_header} != expected_header
            or type(data["schema_version"]) is not int):
        raise ValueError("prefix original runtime attribution changed")
    _, _, expected = cases()
    return _comparison(expected, {"schema_version": 1, "results": data["results"]})


def compare_candidate(original, packets):
    validate_original(original)
    _, requests, _ = cases()
    if (len(packets) != len(requests) + 1 or packets[0] != {"dataset_id": DATASET}
            or [row.get("id") for row in packets[1:]] != [row["id"] for row in requests]):
        raise ValueError("prefix candidate packet roster changed")
    actual = checked_candidate_packets(packets, [{"id": DATASET, "requests": requests}], "selector prefix")
    return _comparison({"schema_version": 1, "results": original["results"]}, actual)


def write_fixtures(target, page_size=17):
    target = directory(target)
    integer(page_size, 1)
    dataset, requests, _ = cases()
    for name, value in (("dataset.json", dataset), ("requests.json", requests)):
        with (target / name).open("x") as stream:
            json.dump(value, stream, indent=2)
            stream.write("\n")
    packets = [{"op": "dataset", "dataset_id": DATASET, "page_size": page_size,
                "dataset": dataset_hex(dataset)}]
    packets.extend(request_hex(request, DATASET) for request in requests)
    with (target / "inputs.jsonl").open("x") as stream:
        for packet in packets:
            stream.write(json.dumps(packet) + "\n")


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    action = cli.add_mutually_exclusive_group(required=True)
    action.add_argument("--write-dir", help="existing private directory for fresh fixture files")
    action.add_argument("--check-source", help="standalone Go extractor JSON output")
    action.add_argument("--check-original", help="fresh complete original probe JSON output")
    action.add_argument("--compare", nargs=2, metavar=("ORIGINAL", "CANDIDATE"))
    cli.add_argument("--page-size", type=int, default=17)
    args = cli.parse_args()
    if args.write_dir:
        write_fixtures(args.write_dir, args.page_size)
    elif args.check_source:
        validate_extraction(json.loads(Path(args.check_source).read_bytes()))
    elif args.check_original:
        validate_original(json.loads(Path(args.check_original).read_bytes()))
    else:
        original, candidate = map(Path, args.compare)
        compare_candidate(json.loads(original.read_bytes()),
                          [json.loads(line) for line in candidate.read_bytes().splitlines()])
    print(json.dumps({"status": "pass", "source_pairs": 52, "expressions": 104}))
