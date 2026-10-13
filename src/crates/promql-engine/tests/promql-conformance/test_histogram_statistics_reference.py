"""Falsify statistic source bindings, range steps, borrowed inputs and threshold outcomes."""
import copy
import math
import struct
import unittest

from compare import bits
from histogram_statistics import compare_outcomes
from histogram_statistics_reference import COMMIT, input_ledger, validate_cases, validate_outcome


def outcome(value, h):
    value = value if type(value) is str else struct.pack(">d", value).hex()
    numeric = bits(value)[1]
    reciprocal = math.copysign(math.inf, numeric) if numeric == 0 else 1 / numeric
    return {"value": value, "input_after": copy.deepcopy(h), "annotations": None,
            "reciprocal": struct.pack(">d", reciprocal).hex(), "negative_reciprocal_infinite": reciprocal == -math.inf,
            "positive_reciprocal_infinite": reciprocal == math.inf, "less_than_positive_infinity": numeric < math.inf,
            "greater_than_compensation_threshold": numeric > 1666666666666668.0}


class StatisticReferenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ledger = input_ledger()
        cls.document = {"schema_version": 1, "commit": COMMIT, "go_version": "go1.27.1", "go_os": "linux", "go_arch": "amd64",
                        "original_assertions_passed": True, "commands": cls.ledger["commands"], "cases": []}
        # These decoder fixtures use declarations; qualification executes Go independently.
        for binding in cls.ledger["cases"]:
            row = copy.deepcopy(binding)
            value = row["request"]["input"][row["mode"][10:]] if row["mode"] in {"histogram-count", "histogram-sum"} else (
                row["declared"]["value"] if row["declared"] else "3ff0000000000000")
            row["actual"] = outcome(value, row["request"]["input"])
            cls.document["cases"].append(row)

    def test_original_and_supplemental_inventory(self):
        self.assertEqual(validate_cases(self.document, self.ledger)["total"], 1270)

    def test_missing_duplicate_extra_and_request_mutations_are_rejected(self):
        for kind in ("missing", "duplicate", "extra", "request"):
            with self.subTest(kind=kind):
                data = copy.deepcopy(self.document)
                if kind == "missing":
                    data["cases"].pop()
                elif kind == "duplicate":
                    data["cases"][-1] = copy.deepcopy(data["cases"][0])
                elif kind == "extra":
                    data["cases"].append(copy.deepcopy(data["cases"][0]))
                else:
                    data["cases"][0]["request"]["input"]["count"] = "0000000000000000"
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_resealed_source_row_step_time_and_dispatch_attribution_reject_drift(self):
        identity = "dsl:extended_vectors.test:545:000:002"
        for key, value in (("row", 2), ("step", 1), ("evaluation_time_ms", 0), ("line", 544)):
            with self.subTest(key=key):
                data, ledger = copy.deepcopy(self.document), copy.deepcopy(self.ledger)
                for rows in (data["cases"], ledger["cases"]):
                    next(row for row in rows if row["id"] == identity)["source"][key] = value
                with self.assertRaises(ValueError):
                    validate_cases(data, ledger)
        data, ledger = copy.deepcopy(self.document), copy.deepcopy(self.ledger)
        for commands in (data["commands"], ledger["commands"]):
            commands[0]["capture_boundary"] = "isolated_argument"
        with self.assertRaises(ValueError):
            validate_cases(data, ledger)

    def test_original_range_context_and_ignored_float_counts_are_bound(self):
        for line, key, value in ((545, "range_step_ns", 30000000000), (1702, "ignored_float_samples", 0)):
            with self.subTest(line=line):
                data, ledger = copy.deepcopy(self.document), copy.deepcopy(self.ledger)
                for commands in (data["commands"], ledger["commands"]):
                    next(command for command in commands if command["line"] == line)[key] = value
                with self.assertRaises(ValueError):
                    validate_cases(data, ledger)

    def test_source_assertions_and_copy_fields_reject_wrong_numbers(self):
        for mode in ("histogram-count", "histogram-sum", "histogram-stdvar"):
            with self.subTest(mode=mode):
                data = copy.deepcopy(self.document)
                row = next(row for row in data["cases"] if row["mode"] == mode and row["declared"]
                           and bits(row["actual"]["value"])[1] != 123.0)
                row["actual"] = outcome(123.0, row["request"]["input"])
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_input_snapshot_and_empty_diagnostics_are_exact(self):
        row = self.document["cases"][0]
        for kind in ("bit", "annotations"):
            with self.subTest(kind=kind):
                actual = copy.deepcopy(row["actual"])
                if kind == "bit":
                    value = actual["input_after"]["count"]
                    actual["input_after"]["count"] = f"{int(value,16) ^ 1:016x}"
                else:
                    actual["annotations"] = []
                with self.assertRaises(ValueError):
                    validate_outcome(row["mode"], row["request"], actual)

    def test_naive_compensation_and_regrouped_overflow_change_discrete_consumers(self):
        for expected, changed in (("4317af4c4a80aab1", "4317af4c4a80aab0"),
                                   ("7c9c847a6ebc8dfa", "7ff0000000000000")):
            with self.subTest(expected=expected):
                row = copy.deepcopy(self.document["cases"][-1])
                row["mode"] = "histogram-stdvar"
                row["actual"] = outcome(expected, row["request"]["input"])
                output = [{"id": row["id"], "actual": outcome(changed, row["request"]["input"])}]
                self.assertFalse(compare_outcomes([row], output)["semantic_pass"])

    def test_one_bit_numeric_difference_remains_visible_without_an_outcome_change(self):
        row = copy.deepcopy(self.document["cases"][-1])
        row["mode"] = "histogram-stdvar"
        row["actual"] = outcome(1.0, row["request"]["input"])
        output = [{"id": row["id"], "actual": outcome("3ff0000000000001", row["request"]["input"])}]
        report = compare_outcomes([row], output)
        self.assertTrue(report["semantic_pass"])
        self.assertFalse(report["bitwise_pass"])

    def test_signed_zero_reciprocal_and_nonboolean_flags_are_rejected(self):
        row = copy.deepcopy(self.document["cases"][-1])
        row["mode"] = "histogram-stdvar"
        row["actual"] = outcome(0.0, row["request"]["input"])
        output = [{"id": row["id"], "actual": outcome("8000000000000000", row["request"]["input"])}]
        self.assertFalse(compare_outcomes([row], output)["semantic_pass"])
        actual = copy.deepcopy(row["actual"])
        actual["less_than_positive_infinity"] = 1
        with self.assertRaises(ValueError):
            validate_outcome(row["mode"], row["request"], actual)


if __name__ == "__main__":
    unittest.main()
