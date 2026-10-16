"""Complete complementary selector queries for validated original declarations."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
from typing import NamedTuple

from build import directory
from compare import checked_results, compare
from info_cases import series
from query import checked_candidate_packets, dataset_hex, request_hex
from regex_reference import PROMETHEUS_COMMIT, integer, keys
from utf8_work_cases import _quote


class Pair(NamedTuple):
    identity: str
    pattern: bytes
    subject: bytes
    matches: bool


class Panel:
    def __init__(self, dataset_id, metric_name, pair_factory):
        self.dataset_id = dataset_id
        self.metric_name = metric_name
        self.pair_factory = pair_factory

    def cases(self):
        records, requests, results = [], [], []
        pairs = list(self.pair_factory())
        if len({pair.identity for pair in pairs}) != len(pairs):
            raise ValueError("duplicate source pair ID")
        for ordinal, pair in enumerate(pairs):
            case_id = f"{ordinal:03d}"
            record = series({"__name__": self.metric_name, "case_id": case_id,
                             "subject": pair.subject}, [(1000, ordinal + 1, "float")])
            records.append(record)
            for operator in ("=~", "!~"):
                ident = f"{self.dataset_id}:{pair.identity}:{operator}"
                query = (f"{self.metric_name}{{case_id={_quote(case_id.encode())},"
                         f"subject{operator}{_quote(pair.pattern)}}}")
                requests.append({"id": ident, "query": query, "profile": "server-default",
                                 "mode": "instant", "time_ms": 1000, "start_ms": 0,
                                 "end_ms": 0, "step_ms": 1000, "lookback_ms": 300000,
                                 "subquery_step_ms": 60000})
                accepted = pair.matches if operator == "=~" else not pair.matches
                results.append({"id": ident, "kind": "vector", "warnings": [], "infos": [],
                                "rows": [deepcopy(record)] if accepted else []})
        return {"schema_version": 1, "series": records}, requests, {"schema_version": 1, "results": results}

    def validate_generated(self, dataset, requests):
        expected_dataset, expected_requests, _ = self.cases()
        if dataset != expected_dataset or requests != expected_requests:
            raise ValueError("generated selector dataset or request identity changed")

    def comparison(self, expected, actual):
        _, requests, _ = self.cases()
        checked_results(actual)
        if [row["id"] for row in actual["results"]] != [row["id"] for row in requests]:
            raise ValueError("selector result IDs are missing, extra or reordered")
        report = compare(expected, actual, [{"id": row["id"], "ordered": True, "exact_numeric": True}
                                            for row in requests])
        if not report["semantic_pass"] or not report["bitwise_pass"]:
            raise ValueError("complete selector results differ: " + json.dumps(report))
        return report

    def validate_original(self, data):
        expected_header = {"schema_version": 1, "reference": "github.com/prometheus/prometheus@v0.315.0",
                           "commit": PROMETHEUS_COMMIT, "go_version": "go1.27.1", "go_arch": "amd64",
                           "go_os": "linux", "unicode_version": "17.0.0"}
        keys(data, (*expected_header, "results"))
        if ({key: data[key] for key in expected_header} != expected_header
                or type(data["schema_version"]) is not int):
            raise ValueError("selector original runtime attribution changed")
        _, _, expected = self.cases()
        return self.comparison(expected, {"schema_version": 1, "results": data["results"]})

    def compare_candidate(self, original, packets):
        self.validate_original(original)
        _, requests, _ = self.cases()
        if (len(packets) != len(requests) + 1 or packets[0] != {"dataset_id": self.dataset_id}
                or [row.get("id") for row in packets[1:]] != [row["id"] for row in requests]):
            raise ValueError("selector candidate packet roster changed")
        actual = checked_candidate_packets(packets, [{"id": self.dataset_id, "requests": requests}],
                                           "selector source examples")
        return self.comparison({"schema_version": 1, "results": original["results"]}, actual)

    def write_fixtures(self, target, page_size=17):
        target = directory(target)
        integer(page_size, 1)
        dataset, requests, _ = self.cases()
        for name, value in (("dataset.json", dataset), ("requests.json", requests)):
            with (target / name).open("x") as stream:
                json.dump(value, stream, indent=2)
                stream.write("\n")
        packets = [{"op": "dataset", "dataset_id": self.dataset_id, "page_size": page_size,
                    "dataset": dataset_hex(dataset)}]
        packets.extend(request_hex(request, self.dataset_id) for request in requests)
        with (target / "inputs.jsonl").open("x") as stream:
            for packet in packets:
                stream.write(json.dumps(packet) + "\n")

    def cli(self, description, validate_extraction):
        cli = argparse.ArgumentParser(description=description)
        action = cli.add_mutually_exclusive_group(required=True)
        action.add_argument("--write-dir", help="existing private directory for fresh fixture files")
        action.add_argument("--check-source", help="standalone Go extractor JSON output")
        action.add_argument("--check-original", help="fresh complete original probe JSON output")
        action.add_argument("--compare", nargs=2, metavar=("ORIGINAL", "CANDIDATE"))
        cli.add_argument("--page-size", type=int, default=17)
        args = cli.parse_args()
        if args.write_dir:
            self.write_fixtures(args.write_dir, args.page_size)
        elif args.check_source:
            validate_extraction(json.loads(Path(args.check_source).read_bytes()))
        elif args.check_original:
            self.validate_original(json.loads(Path(args.check_original).read_bytes()))
        else:
            original, candidate = map(Path, args.compare)
            self.compare_candidate(json.loads(original.read_bytes()),
                                   [json.loads(line) for line in candidate.read_bytes().splitlines()])
        _, requests, _ = self.cases()
        print(json.dumps({"status": "pass", "source_pairs": len(requests) // 2, "expressions": len(requests)}))
