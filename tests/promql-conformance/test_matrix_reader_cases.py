"""Protect independent reader control coverage, negative controls and fault identity."""

import unittest
from collections import Counter
from copy import deepcopy

from matrix_reader_cases import cases


class MatrixReaderCasesTests(unittest.TestCase):
    def test_profiles_and_distinct_complete_rosters(self):
        ids = set()
        for roster, count in (("ordinary", 1826), ("clipped", 370), ("timestamp", 376)):
            rows = cases(roster)
            self.assertEqual(len(rows), count)
            self.assertEqual(len({row["request"]["id"] for row in rows}), count)
            self.assertEqual(
                Counter(row["request"]["profile"] for row in rows),
                {"server-default": count // 2, "upstream-tests": count // 2},
            )
            self.assertFalse(ids.intersection(row["request"]["id"] for row in rows))
            ids.update(row["request"]["id"] for row in rows)
        with self.assertRaises(ValueError):
            cases("missing")

    def test_required_and_unrequested_operations_remain_distinct_fault_controls(self):
        rows = {row["request"]["id"]: row for row in cases()}
        for profile in ("server-default", "upstream-tests"):
            prefix = "matrix-reader:sum_over_time:"
            initial = rows[prefix + "initial-prefix:" + profile]
            skipped = rows[prefix + "skip-middle:" + profile]
            current = rows[prefix + "current-seek:" + profile]
            future = rows[prefix + "future-next:" + profile]
            omitted = rows[prefix + "omitted-seek-500ms:" + profile]
            self.assertEqual(initial["faults"]["m"]["next_times"], [100])
            self.assertEqual(skipped["faults"]["m"]["next_times"], [200])
            self.assertEqual(current["faults"]["m"]["seek_targets"], [500])
            self.assertEqual(future["faults"]["m"]["next_times"], [1200])
            self.assertEqual(omitted["faults"]["m"]["seek_targets"], [1000, 1500])
            self.assertIn("scalar(anchor @0)", skipped["request"]["query"])
            self.assertEqual(
                [
                    point["time_ms"]
                    for point in skipped["dataset"]["series"][0]["points"]
                ],
                [100, 200, 600, 1000],
            )
            helper = next(
                row
                for row in cases("clipped")
                if row["request"]["id"]
                == "matrix-reader:helper-before-later-series:" + profile
            )
            self.assertEqual(
                helper["faults"],
                {
                    "m/two": {
                        "next_times": [600],
                        "message": "unexpected-later-series-before-helper",
                    }
                },
            )

    def test_clipped_construction_controls_keep_valid_numeric_offset_and_empty_membership(
        self,
    ):
        rows = {row["request"]["id"]: row for row in cases("clipped")}
        for profile in ("server-default", "upstream-tests"):
            prefix = "matrix-reader:clipped:sum_over_time:"
            invalid = rows[prefix + "0.5ms:healthy:" + profile]
            healthy = rows[prefix + "0.0005:healthy:" + profile]
            self.assertIn("offset 0.5ms", invalid["request"]["query"])
            self.assertEqual(
                healthy["request"]["query"], "sum_over_time(m[1ms] @1 offset 0.0005)"
            )
            self.assertEqual(healthy["request"]["time_ms"], 0)
            self.assertEqual(
                [
                    point["time_ms"]
                    for point in healthy["dataset"]["series"][0]["points"]
                ],
                [1000],
            )
            self.assertEqual(healthy["faults"], {})
            for variant, field, target in (
                ("initial-next", "next_times", 1000),
                ("current-seek", "seek_targets", 998),
                ("unrequested-seek", "seek_targets", 1000),
            ):
                row = rows[prefix + "0.0005:" + variant + ":" + profile]
                self.assertEqual(row["faults"]["m"][field], [target])
                self.assertEqual(row["dataset"], healthy["dataset"])

    def test_timestamp_migration_preserves_faults_datasets_and_request_scopes(self):
        originals = {row["request"]["id"]: row for row in cases() + cases("clipped")}
        names = {
            "ts_of_first_over_time": "first_over_time",
            "ts_of_last_over_time": "last_over_time",
            "ts_of_min_over_time": "min_over_time",
            "ts_of_max_over_time": "max_over_time",
        }
        for row in cases("timestamp"):
            original_id = row["request"]["id"].removeprefix("matrix-timestamp-fold:")
            expected = deepcopy(row)
            for new, old in names.items():
                original_id = original_id.replace(new, old)
                expected["request"]["query"] = expected["request"]["query"].replace(
                    new + "(", old + "("
                )
            expected["request"]["id"] = original_id
            self.assertEqual(expected, originals[original_id])

    def test_zero_range_has_an_available_sample_and_an_armed_error_contrast(self):
        rows = cases("zero")
        self.assertEqual(len(rows), 220)
        self.assertEqual(len({row["request"]["id"] for row in rows}), 220)
        for row in rows:
            self.assertIn("[0.0005]", row["request"]["query"])
            self.assertIn("scalar(anchor @0)", row["request"]["query"])
            self.assertEqual(row["dataset"]["series"][1]["points"][0]["time_ms"], 0)
            if ":empty-eof:" in row["request"]["id"]:
                self.assertEqual(row["dataset"]["series"][0]["points"], [])
                self.assertTrue(row["faults"]["m"]["next_eof"])
            else:
                self.assertEqual(
                    [
                        point["time_ms"]
                        for point in row["dataset"]["series"][0]["points"]
                    ],
                    [1000],
                )
                if ":initial-next:" in row["request"]["id"]:
                    self.assertEqual(row["faults"]["m"]["next_times"], [1000])


if __name__ == "__main__":
    unittest.main()
