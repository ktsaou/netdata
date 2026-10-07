"""Falsify LIMIT policy controls; retain source diagnostics and non-boundary outcomes."""

import base64
import unittest
from copy import deepcopy

from limit_policy import declarations, expected_policy, inject


def row(value):
    return {
        "labels": [
            {"name": "dg==", "value": base64.b64encode(value.encode()).decode()}
        ],
        "points": [{"time_ms": 1000, "kind": "float", "float": "8000000000000000"}],
    }


def result(ident, rows, warnings=None):
    return {
        "id": ident,
        "kind": "vector",
        "rows": rows,
        "warnings": warnings or [],
        "infos": [],
    }


def inputs():
    return [
        {
            "request": {
                "id": f"limits:{operator}:topk:body-producers-topk:upstream-tests:3:{window}",
                "query": f'{operator}(2,count_values("v",m))',
                "mode": "instant",
                "profile": "upstream-tests",
                "time_ms": 1000,
                "lookback_ms": 1,
            },
            "dataset": {"schema_version": 1, "id": "source", "series": [row("z")]},
            "faults": {},
        }
        for operator in ("limitk", "limit_ratio")
        for window in (0, 1, 2)
    ]


class LimitPolicyTests(unittest.TestCase):
    def test_controls_preserve_parent_scope_and_validate_the_actual_boundary(self):
        original = inputs()
        retained = deepcopy(original)
        declared, children = declarations(original)
        self.assertEqual(len(declared), 6)
        self.assertEqual(original, retained)
        for spec, child in zip(declared, children, strict=True):
            parent = next(item for item in original if item["request"]["id"] == spec[0])
            self.assertEqual(
                child["request"],
                parent["request"] | {"id": "inner:" + spec[0], "query": spec[1]},
            )
            self.assertEqual(child["dataset"], parent["dataset"])
        original[0]["request"]["query"] = "limitk(2,m)"
        with self.assertRaises(ValueError):
            declarations(original)
        original = inputs()
        original[0]["faults"] = {"m": {"set_error": "fault"}}
        with self.assertRaises(ValueError):
            declarations(original)

    def test_injection_keeps_raw_v_labels_points_and_source_metadata_only(self):
        declared, children = declarations(inputs())
        inner = {
            "results": [
                result(child["request"]["id"], [row("z"), row("a")])
                for child in children
            ]
        }
        original = deepcopy(inner)
        output = inject(declared, children, inner)
        self.assertEqual(inner, original)
        self.assertEqual(
            output[0]["dataset"],
            {"schema_version": 1, "id": "source", "series": [row("a"), row("z")]},
        )
        for mutation in ("name", "reserved", "native", "repeat", "notes", "error"):
            changed = deepcopy(inner)
            if mutation == "name":
                changed["results"][0]["rows"][0]["labels"][0]["name"] = "eA=="
            elif mutation == "reserved":
                changed["results"][0]["rows"][0]["labels"].append(
                    {"name": "X19uYW1lX18=", "value": "bQ=="}
                )
            elif mutation == "native":
                changed["results"][0]["rows"][0]["points"][0]["kind"] = (
                    "float_histogram"
                )
            elif mutation == "repeat":
                changed["results"][0]["rows"][0]["points"] *= 2
            elif mutation == "notes":
                changed["results"][0]["warnings"] = ["warning"]
            else:
                changed["results"][0]["kind"] = "error"
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                inject(declared, children, changed)

    def test_only_ten_boundaries_change_and_warning_defects_remain_failures(self):
        declared, _ = declarations(inputs())
        counts = [
            f"limits:{operator}:topk:parents-topk:upstream-tests:5:{window}"
            for operator in ("limitk", "limit_ratio")
            for window in (0, 1)
        ]
        raw = {
            "schema_version": 1,
            "results": [
                result(spec[0], [row("z"), row("a")], ["source-warning"])
                for spec in declared
            ]
            + [result(ident, [row("z"), row("a")]) for ident in counts]
            + [result("span-defect", [row("z")], ["source-span"])],
        }
        outer = {
            "results": [
                result("outer:" + spec[0], [row("a")], ["source-warning"])
                for spec in declared
            ]
        }
        original = deepcopy(raw)
        expected, boundaries = expected_policy(raw, declared, outer)
        self.assertEqual(raw, original)
        self.assertEqual(len(boundaries), 10)
        self.assertEqual(expected["results"][-1], raw["results"][-1])
        self.assertEqual(expected["results"][0]["warnings"], ["source-warning"])
        self.assertEqual(expected["results"][6]["rows"], [row("a"), row("z")])
        outer["results"][0]["warnings"] = ["changed-span"]
        with self.assertRaises(ValueError):
            expected_policy(raw, declared, outer)
        with self.assertRaises(ValueError):
            expected_policy({"results": raw["results"][:-2]}, declared, outer)


if __name__ == "__main__":
    unittest.main()
