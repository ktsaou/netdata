"""Flag-aware policy controls stay confined and preserve strict numerical checks."""

import unittest
from copy import deepcopy

import cursor_calls
import quantile_aggregate_calls
import quantile_policy
from compare import compare


def result(ident, rows):
    return {"id": ident, "kind": "matrix", "rows": rows, "warnings": [], "infos": []}


class QuantilePolicy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inputs = quantile_aggregate_calls.cases()

    def control(self):
        selected = quantile_policy.declarations(self.inputs)[:1]
        ident = selected[0]["request"]["id"]
        child = result("child:" + ident, deepcopy(selected[0]["dataset"]["series"][:2]))
        answer = result(
            ident,
            [
                {
                    "labels": [],
                    "points": [
                        {"time_ms": 0, "kind": "float", "float": "3ff0000000000000"}
                    ],
                }
            ],
        )
        return selected, cursor_calls.PROFILE | {
            "cases": [
                {
                    "request": cursor_calls.wire_request(selected[0]["request"]),
                    "result": answer,
                    "child": child,
                    "child_drop_names": [False, True],
                }
            ]
        }

    def test_eight_boundaries_preserve_the_entire_original_request_dataset_and_faults(
        self,
    ):
        selected = quantile_policy.declarations(self.inputs)
        self.assertEqual(len(selected), 8)
        self.assertEqual(len({row["request"]["id"] for row in selected}), 8)
        original = {row["request"]["id"]: row for row in self.inputs}
        for row in selected:
            self.assertEqual(row, original[row["request"]["id"]])
            self.assertEqual(row["request"]["profile"], "upstream-tests")
            self.assertEqual(row["request"]["mode"], "range")
            self.assertEqual(row["faults"], {})
        corrupted = list(self.inputs)
        index = next(
            i
            for i, row in enumerate(corrupted)
            if row["request"]["id"] == selected[0]["request"]["id"]
        )
        corrupted[index] = deepcopy(corrupted[index])
        corrupted[index]["faults"] = {"m": {"message": "declared fault"}}
        with self.assertRaisesRegex(ValueError, "map boundary"):
            quantile_policy.declarations(corrupted)

    def test_source_decoder_retains_whole_matrix_flags_and_complete_scope(self):
        selected, document = self.control()
        expected = quantile_policy.checked_source(document, selected)
        self.assertEqual(expected["results"], [document["cases"][0]["result"]])
        for mutation in (
            "scope",
            "flag-type",
            "flag-value",
            "flag-count",
            "order",
            "identity",
        ):
            bad = deepcopy(document)
            row = bad["cases"][0]
            if mutation == "scope":
                row["request"]["end_ms"] += 1000
            elif mutation == "flag-type":
                row["child_drop_names"][0] = 0
            elif mutation == "flag-value":
                row["child_drop_names"][1] = False
            elif mutation == "flag-count":
                row["child_drop_names"].pop()
            elif mutation == "order":
                row["child"]["rows"].reverse()
            else:
                row["child"]["id"] = "another-child"
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                quantile_policy.checked_source(bad, selected)

    def test_policy_keeps_undeclared_results_and_rejects_changed_bits_and_labels(self):
        selected, document = self.control()
        controls = quantile_policy.checked_source(document, selected)
        raw = deepcopy(controls)
        raw["results"][0]["rows"][0]["labels"] = [
            {"name": "X19uYW1lX18=", "value": "bQ=="}
        ]
        untouched = deepcopy(controls["results"][0])
        untouched["id"] = "outside-policy"
        raw["results"].append(untouched)
        retained = deepcopy(raw)
        expected, boundaries = quantile_policy.expected_policy(raw, selected, controls)
        self.assertEqual(raw, retained)
        self.assertEqual(boundaries, [selected[0]["request"]["id"]])
        self.assertEqual(expected["results"][1], untouched)
        cases = [
            {"id": row["id"], "ordered": True, "exact_numeric": True}
            for row in raw["results"]
        ]
        self.assertTrue(compare(expected, deepcopy(expected), cases)["bitwise_pass"])
        for index, mutation in ((0, "bits"), (0, "labels"), (1, "bits")):
            actual = deepcopy(expected)
            row = actual["results"][index]["rows"][0]
            if mutation == "bits":
                row["points"][0]["float"] = "3ff0000000000001"
            else:
                row["labels"] = [{"name": "X19uYW1lX18=", "value": "bQ=="}]
            report = compare(expected, actual, cases)
            with self.subTest(index=index, mutation=mutation):
                self.assertEqual(
                    set(report["semantic_failures"]), {expected["results"][index]["id"]}
                )
                self.assertFalse(report["bitwise_pass"])
                if mutation == "bits":
                    self.assertEqual(len(report["bitwise_differences"]), 1)

    def test_missing_duplicate_and_undeclared_controls_cannot_expand_the_boundary_roster(
        self,
    ):
        selected, document = self.control()
        controls = quantile_policy.checked_source(document, selected)
        for mutation in ("missing", "duplicate", "undeclared"):
            bad = deepcopy(controls)
            if mutation == "missing":
                bad["results"].clear()
            elif mutation == "duplicate":
                bad["results"].append(deepcopy(bad["results"][0]))
            else:
                bad["results"][0]["id"] = "outside-policy"
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                quantile_policy.expected_policy(controls, selected, bad)
