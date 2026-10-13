"""Falsify exact native source payloads and the narrow validator seam."""

import base64
import json
import unittest
from copy import deepcopy
from pathlib import Path

from compare import checked_results
from cursor_calls import PROFILE, checked_original, wire_request
from histogram_reference import COMMIT, input_ledger
from native_matrix_calls import exact_trim_results, qualify_outcomes, result_validator
from native_matrix_cases import PANELS, cases


def hex_result(result):
    result = deepcopy(result)
    for row in result["rows"]:
        for label in row["labels"]:
            for key in ("name", "value"):
                label[key] = base64.b64decode(label[key], validate=True).hex()
    for key in ("warnings", "infos"):
        result[key] = [
            base64.b64decode(value, validate=True).hex() for value in result[key]
        ]
    return result


class NativeSourceContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        source = json.loads(
            Path(__file__).with_name("native-trim-source-results.json").read_bytes()
        )
        if (
            source["commit"] != COMMIT
            or source["source_manifest_sha256"]
            != input_ledger()["source_manifest_sha256"]
            or source["original_sha256"]
            != "c50af651dce94fb66e6c8bedc6ab6fbda61cbdb03ab73159fd9dbf31707ae807"
            or len(source["results"]) != 4
        ):
            raise ValueError("retained original trim source binding changed")
        cls.trim = {result["id"]: result for result in source["results"]}

    def setUp(self):
        self.inputs = cases("native-checked")
        # Empty ordinary rows are decoder falsifiers, never query-engine expectations.
        self.document = {
            "schema_version": 1,
            "results": [
                deepcopy(
                    self.trim.get(
                        row["request"]["id"],
                        {
                            "id": row["request"]["id"],
                            "kind": "vector",
                            "rows": [],
                            "warnings": [],
                            "infos": [],
                        },
                    )
                )
                for row in self.inputs
            ],
        }
        self.validate = result_validator("native-checked")

    def test_all_independent_input_rosters_are_preserved(self):
        counts = {
            "unvisited-primary": 1512,
            "unvisited-supplement": 1044,
            "unvisited-phases": 336,
            "native-valid": 24,
            "native-checked": 123,
            "native-notes": 17,
            "matrix-copy": 39,
            "instant-copy": 8,
            "native-recovery": 126,
        }
        self.assertEqual(set(PANELS), set(counts))
        for panel, count in counts.items():
            with self.subTest(panel=panel):
                self.assertEqual(len(cases(panel)), count)
        self.assertEqual(sum(counts.values()), 3229)
        self.assertEqual(sum(counts.values()) - counts["native-recovery"], 3103)

    def test_exact_source_contract_preserves_all_four_malformed_payloads(self):
        self.assertEqual(exact_trim_results(), self.trim)
        self.validate(self.document)
        for result in self.trim.values():
            with self.subTest(ident=result["id"]), self.assertRaises(ValueError):
                checked_results({"schema_version": 1, "results": [result]})

    def test_complete_payload_changes_are_not_hidden_by_the_source_seam(self):
        for index in (34, 35, 40, 41):
            for mutation in (
                "count",
                "buckets",
                "span",
                "label",
                "timestamp",
                "notes",
                "extra",
                "missing",
            ):
                changed = deepcopy(self.document)
                result = changed["results"][index]
                point = result["rows"][0]["points"][0]
                histogram = point["float_histogram"]
                if mutation == "count":
                    histogram["count"] = "8000000000000000"
                elif mutation == "buckets":
                    histogram["positive_buckets"] = (
                        [] if histogram["positive_buckets"] else ["0000000000000000"]
                    )
                elif mutation == "span":
                    histogram["positive_spans"] = (
                        [] if histogram["positive_spans"] is None else None
                    )
                elif mutation == "label":
                    result["rows"][0]["labels"][0]["value"] = "Y2hhbmdlZA=="
                elif mutation == "timestamp":
                    point["time_ms"] = 2000.0
                elif mutation == "notes":
                    result["warnings"] = ["bm90ZQ=="]
                elif mutation == "extra":
                    result["unexpected"] = True
                else:
                    histogram.pop("negative_buckets")
                with (
                    self.subTest(index=index, mutation=mutation),
                    self.assertRaises(ValueError),
                ):
                    self.validate(changed)

    def test_other_malformed_results_and_roster_changes_still_fail(self):
        changed = deepcopy(self.document)
        changed["results"][0] = deepcopy(changed["results"][34])
        changed["results"][0]["id"] = self.inputs[0]["request"]["id"]
        with self.assertRaises(ValueError):
            self.validate(changed)
        for results in (self.document["results"][:-1], self.document["results"][::-1]):
            with self.subTest(results=len(results)), self.assertRaises(ValueError):
                self.validate(self.document | {"results": results})
        for schema in (True, 1.0, 2):
            with self.subTest(schema=schema), self.assertRaises(ValueError):
                self.validate(self.document | {"schema_version": schema})

    def test_optional_validator_keeps_original_phase_and_request_checks(self):
        original = PROFILE | {
            "cases": [
                {
                    "request": wire_request(source["request"]),
                    "result": result,
                    "events": [],
                    "header": {},
                    "raw_ast": [],
                    "prepared_ast": [],
                    "executed_ast": [],
                }
                for source, result in zip(
                    self.inputs, self.document["results"], strict=True
                )
            ]
        }
        with self.assertRaises(ValueError):
            checked_original(original, self.inputs)
        checked_original(original, self.inputs, result_validator=self.validate)
        for mutation in ("request", "phase", "result", "profile"):
            changed = deepcopy(original)
            if mutation == "request":
                changed["cases"][34]["request"]["time_ms"] += 1
            elif mutation == "phase":
                changed["cases"][34]["events"] = {}
            elif mutation == "result":
                changed["cases"][34]["result"]["infos"] = ["bm90ZQ=="]
            else:
                changed["go_arch"] = "arm64"
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                checked_original(changed, self.inputs, result_validator=self.validate)

    def test_candidate_exact_payload_comparison_and_required_open_are_distinct(self):
        for index in (34, 35, 40, 41):
            source = self.inputs[index]
            expected = {
                "schema_version": 1,
                "results": [self.document["results"][index]],
            }
            outcome = {
                "id": source["request"]["id"],
                "result": hex_result(expected["results"][0]),
            }
            report = qualify_outcomes([source], expected, [outcome], "native-checked")
            self.assertEqual(
                (report["passed"], report["failed"], report["open"]), (1, 0, {})
            )
            altered = deepcopy(outcome)
            altered["result"]["rows"][0]["points"][0]["float_histogram"][
                "positive_buckets"
            ] = ["3ff0000000000001"]
            report = qualify_outcomes([source], expected, [altered], "native-checked")
            self.assertEqual((report["passed"], report["failed"]), (0, 1))
            self.assertFalse(report["bitwise_pass"])
            opened = {
                "id": source["request"]["id"],
                "unsupported": "7265717569726564",
                "observations": {},
            }
            report = qualify_outcomes([source], expected, [opened], "native-checked")
            self.assertEqual(
                (report["passed"], report["failed"], len(report["open"])), (0, 0, 1)
            )
            self.assertFalse(report["semantic_pass"])
            self.assertFalse(report["bitwise_pass"])

    def test_missing_candidate_or_rewritten_source_expectation_is_rejected(self):
        source = self.inputs[34]
        expected = {"schema_version": 1, "results": [self.document["results"][34]]}
        outcome = {
            "id": source["request"]["id"],
            "result": hex_result(expected["results"][0]),
        }
        with self.assertRaises(ValueError):
            qualify_outcomes([source], expected, [], "native-checked")
        changed = deepcopy(expected)
        changed["results"][0]["warnings"] = ["bm90ZQ=="]
        with self.assertRaises(ValueError):
            qualify_outcomes([source], changed, [outcome], "native-checked")


if __name__ == "__main__":
    unittest.main()
