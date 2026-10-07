"""Independent MAD inputs retain IEEE provenance, timeline scopes and fault domains."""

import base64
import unittest
from collections import Counter

from mad_cases import arrays, cases


class MadCases(unittest.TestCase):
    def test_numeric_provenance_and_permutations_are_preserved(self):
        declared = arrays()
        self.assertEqual(len(declared), 176)
        self.assertEqual(len({row["id"] for row in declared}), 176)
        indexed = {row["id"]: row for row in declared}
        for name, values in (
            ("subnormal-two", ["8000000000000001", "0000000000000001"]),
            (
                "first-median-inf-neighbor",
                ["0000000000000000", "0000000000000000", "7ff0000000000000"],
            ),
            (
                "second-median-inf-neighbor",
                ["fff0000000000000", "0000000000000000", "0000000000000000"],
            ),
            (
                "nan-positive",
                ["7ff8000000000042", "3ff0000000000000", "4000000000000000"],
            ),
            (
                "nan-negative",
                ["fff8000000000043", "3ff0000000000000", "4000000000000000"],
            ),
            (
                "nan-signaling",
                ["7ff0000000000043", "3ff0000000000000", "4000000000000000"],
            ),
        ):
            self.assertEqual(indexed[name + "-input"]["bits"], values)
            for permutation in ("reverse", "rotate", "shuffle"):
                self.assertEqual(
                    Counter(indexed[name + "-" + permutation]["bits"]), Counter(values)
                )
        for length in (13, 50, 51, 127):
            self.assertEqual(
                len(indexed[f"pdq-zeros-repeat-{length}-input"]["bits"]), length
            )
        self.assertEqual(len(indexed["page-boundaries-input"]["bits"]), 1030)

    def test_profile_and_window_rosters_are_complete(self):
        inputs = cases()
        self.assertEqual(len(inputs), 2248)
        self.assertEqual(len({row["request"]["id"] for row in inputs}), 2248)
        self.assertEqual(
            Counter(row["request"]["profile"] for row in inputs),
            {"server-default": 1124, "upstream-tests": 1124},
        )
        array_cases = [
            row for row in inputs if row["request"]["id"].startswith("mad:array-")
        ]
        self.assertEqual(len(array_cases), 176 * 2 * 4)
        for row in array_cases:
            points = row["dataset"]["series"][0]["points"]
            self.assertLessEqual(points[0]["time_ms"], 0)
            self.assertGreater(points[0]["time_ms"], -3600000)
            self.assertEqual(points[-1]["time_ms"], 0)
            self.assertTrue(all(point["kind"] == "float" for point in points))

    def test_faults_remain_after_an_earlier_raw_nan(self):
        selected = [
            row for row in cases() if row["request"]["id"].startswith("mad:fault-")
        ]
        self.assertEqual(len(selected), 3 * 2 * 2 * 4)
        self.assertEqual(
            {row["request"]["id"].split(":")[1] for row in selected},
            {"fault-selection", "fault-iterator-after-nan", "fault-seek"},
        )
        for row in selected:
            self.assertEqual(set(row["faults"]), {"m"})
            points = row["dataset"]["series"][0]["points"]
            self.assertEqual(points[0]["time_ms"], -1000)
            self.assertEqual(points[0]["float"], "7ff8000000000042")
            self.assertEqual(points[-1]["time_ms"], 1000)
            if "next_times" in row["faults"]["m"]:
                self.assertGreater(
                    row["faults"]["m"]["next_times"][0], points[0]["time_ms"]
                )

    def test_metadata_collision_keeps_distinct_raw_names(self):
        selected = next(
            row
            for row in cases()
            if row["request"]["id"] == "mad:name-collision:upstream-tests:0:0"
        )
        labels = [
            {
                base64.b64decode(label["name"]): base64.b64decode(label["value"])
                for label in record["labels"]
            }
            for record in selected["dataset"]["series"]
        ]
        self.assertEqual({row[b"__name__"] for row in labels}, {b"m", b"n"})
        self.assertEqual(
            [
                {key: value for key, value in row.items() if key != b"__name__"}
                for row in labels
            ],
            [{b"job": b"same"}, {b"job": b"same"}],
        )


if __name__ == "__main__":
    unittest.main()
