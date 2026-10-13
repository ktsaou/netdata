"""Independent input witnesses preserve grouping, IEEE provenance and fault domains."""

import base64
import unittest

from quantile_aggregate_cases import cases


def labels(record):
    return {
        base64.b64decode(label["name"]): base64.b64decode(label["value"])
        for label in record["labels"]
    }


class QuantileAggregateCases(unittest.TestCase):
    def test_parameter_extrema_controls_keep_infinities_nan_and_empty_bodies(self):
        inputs = cases()
        extra = [
            row
            for row in inputs
            if row["request"]["id"].startswith(
                "aggregate-quantile:parameter-infinity-nan-"
            )
        ]
        self.assertEqual(len(extra), 360)
        checked = set()
        for row in extra:
            family = row["request"]["id"].split(":")[1]
            if family in checked:
                continue
            checked.add(family)
            parameter = next(
                record
                for record in row["dataset"]["series"]
                if labels(record)[b"__name__"] == b"p"
            )
            self.assertEqual(
                {point["float"] for point in parameter["points"]},
                {"7ff0000000000000", "fff0000000000000", "7ff8000000000063"},
            )
            if family.endswith("empty"):
                self.assertEqual(len(row["dataset"]["series"]), 1)
        self.assertEqual(len(checked), 12)

    def test_grouping_alias_and_empty_value_have_distinct_raw_identities(self):
        inputs = cases()
        for family, collision in (
            ("group-hash-alias", True),
            ("empty-group-label", False),
        ):
            row = next(
                row
                for row in inputs
                if row["request"]["id"]
                == f"aggregate-quantile:{family}:server-default:0:0"
            )
            left, right = [labels(record) for record in row["dataset"]["series"]]
            self.assertNotEqual(left, right)
            grouping = lambda record: b"".join(
                name + b"\xff" + value + b"\xff"
                for name, value in sorted(record.items())
                if name in (b"a", b"x")
            )
            self.assertEqual(grouping(left) == grouping(right), collision)
            if not collision:
                self.assertIn(b"a", left)
                self.assertNotIn(b"a", right)
                self.assertEqual(left[b"a"], b"")

    def test_ieee_inputs_keep_signed_zero_signalling_and_quiet_nan_payloads(self):
        inputs = {row["request"]["id"]: row for row in cases()}
        for index, raw in enumerate(
            (
                "8000000000000000",
                "0000000000000000",
                "7ff8000000000063",
                "fff8000000000027",
                "7ff0000000000044",
                "fff0000000000045",
            )
        ):
            row = inputs[f"aggregate-quantile:singleton-{index}:server-default:6:0"]
            self.assertEqual(row["request"]["query"], "quantile(0.5,m)")
            self.assertEqual(row["dataset"]["series"][0]["points"][0]["float"], raw)
        row = inputs["aggregate-quantile:pair-3:server-default:2:0"]
        self.assertEqual(
            [record["points"][0]["float"] for record in row["dataset"]["series"]],
            ["7ff0000000000044", "fff0000000000045"],
        )

    def test_native_first_writer_and_hash_alias_controls_retain_changing_kinds(self):
        inputs = cases()
        for kind in ("histogram", "float_histogram"):
            row = next(
                row
                for row in inputs
                if row["request"]["id"]
                == f"aggregate-quantile:native-{kind}-only:server-default:0:3"
            )
            self.assertEqual(
                [point["kind"] for point in row["dataset"]["series"][0]["points"]],
                [kind, "float", kind],
            )
        row = next(
            row
            for row in inputs
            if row["request"]["id"]
            == "aggregate-quantile:mixed-heads-3-limitk:upstream-tests:0:3"
        )
        self.assertNotEqual(
            *[labels(record)[b"a"] for record in row["dataset"]["series"]]
        )
        self.assertEqual(
            [
                [point["kind"] for point in record["points"]]
                for record in row["dataset"]["series"]
            ],
            [
                ["float_histogram", "float", "float_histogram"],
                ["float_histogram", "float_histogram", "float"],
            ],
        )

    def test_parameter_and_body_faults_are_separate_and_not_shared_between_calls(self):
        inputs = cases()
        rows = [
            row
            for row in inputs
            if row["request"]["id"].startswith(
                "aggregate-quantile:selection-fault-scalar(p)-"
            )
        ]
        self.assertEqual({tuple(row["faults"]) for row in rows}, {("p",), ("m",)})
        selected = next(
            row
            for row in rows
            if row["request"]["id"].endswith(":0:0") and "p" in row["faults"]
        )
        other = next(
            row
            for row in rows
            if row["request"]["id"].endswith(":0:3") and "p" in row["faults"]
        )
        selected["faults"]["p"]["set_error"] = "changed"
        self.assertEqual(other["faults"]["p"]["set_error"], "selection fault")


if __name__ == "__main__":
    unittest.main()
