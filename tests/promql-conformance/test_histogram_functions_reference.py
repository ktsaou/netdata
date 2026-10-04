"""Falsify native helper attribution, immutable inputs, annotations and composed outcomes."""
import copy
import math
import re
import struct
import unittest

from compare import bits
from histogram_functions import compare_outcomes
from histogram_functions_reference import COMMIT, TEST, input_ledger, validate_cases, validate_outcome


def encoded(value):
    return struct.pack(">d", value).hex()


def scalar_outcome(value, histogram, annotations=None):
    numeric = bits(value)[1]
    reciprocal = math.copysign(math.inf, numeric) if numeric == 0 else 1 / numeric
    return {"value": value, "input_after": copy.deepcopy(histogram), "annotations": annotations,
            "reciprocal": encoded(reciprocal), "negative_reciprocal_infinite": reciprocal == -math.inf,
            "positive_reciprocal_infinite": reciprocal == math.inf}


class HistogramFunctionReferenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ledger = input_ledger()
        cls.documents = {package: {"schema_version": 1, "commit": COMMIT, "go_version": "go1.27.1",
                                  "go_os": "linux", "go_arch": "amd64", "original_assertions_passed": True,
                                  "cases": [], "commands": cls.ledger["commands"] if package == "promqltest" else []}
                         for package in ("promql", "promqltest")}
        # These decoder fixtures emulate declarations; qualification executes Go independently.
        for binding in cls.ledger["cases"]:
            row = copy.deepcopy(binding)
            declaration, annotation = row["declared"], None
            value = encoded(1.0)
            if declaration is not None:
                comparison = declaration["comparison"]
                if comparison in {"exact", "almost"}:
                    value = declaration["value"]
                elif comparison == "nan":
                    value = "7ff8000000000001"
                else:
                    value = encoded(bits(declaration["trim_count"])[1] / bits(declaration["total"])[1])
            if row["id"].startswith("dsl:"):
                infos = [item for item in row["source"]["command"]["declarations"]["annotation_commands"]
                         if item["kind"] == 4]
                if infos:
                    message = bytes.fromhex(infos[0]["value"]).decode()
                    identity = ("NativeHistogramFractionNaNsInfo" if row["mode"] == "histogram-fraction" else
                                "NativeHistogramQuantileNaNResultInfo" if "result is NaN" in message else
                                "NativeHistogramQuantileNaNSkewInfo")
                    annotation = [{"identity": identity, "severity": "info", "formatted": infos[0]["value"],
                                   "raw": re.sub(r" \(\d+:\d+\)$", "", message).encode().hex()}]
            row["actual"] = scalar_outcome(value, row["request"]["input"], annotation)
            cls.documents["promqltest" if row["id"].startswith("dsl:") else "promql"]["cases"].append(row)

    def test_complete_original_and_supplemental_inventory(self):
        self.assertEqual(validate_cases(self.documents, self.ledger)["total"], 7611)

    def test_missing_duplicate_extra_and_wrong_package_rows_are_rejected(self):
        for kind in ("missing", "duplicate", "extra", "wrong_package"):
            with self.subTest(kind=kind):
                data = copy.deepcopy(self.documents)
                rows = data["promql"]["cases"]
                if kind == "missing":
                    rows.pop()
                elif kind == "duplicate":
                    rows[-1] = copy.deepcopy(rows[0])
                elif kind == "extra":
                    rows.append(copy.deepcopy(rows[0]))
                else:
                    data["promqltest"]["cases"].append(rows.pop())
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_requests_and_declarations_cannot_drift_from_ledger_bindings(self):
        for section, key, value in (("request", "upper", "3ff0000000000000"),
                                    ("declared", "trim_count", "3ff0000000000000"),
                                    ("source", "call_line", 438)):
            with self.subTest(section=section):
                data = copy.deepcopy(self.documents)
                data["promql"]["cases"][0][section][key] = value
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_resealed_qt_coordinates_must_match_exact_ids_and_fixture_bounds(self):
        for identity, changes in (
            (f"{TEST}:002:012:below", {"case_index": 0}),
            (f"{TEST}:002:012:below", {"threshold_index": 32}),
            (f"{TEST}:002:012:below", {"direction": "above", "call_line": 448}),
            (f"{TEST}:zero", {"case_index": 0}),
            (f"{TEST}:divergence:001", {"case_index": 0}),
            (f"{TEST}:divergence:001", {"threshold_index": 0}),
        ):
            with self.subTest(identity=identity, changes=changes):
                data, ledger = copy.deepcopy(self.documents), copy.deepcopy(self.ledger)
                for rows in (data["promql"]["cases"], ledger["cases"]):
                    next(row for row in rows if row["id"] == identity)["source"].update(changes)
                with self.assertRaises(ValueError):
                    validate_cases(data, ledger)

    def test_original_trim_nan_and_exact_assertions_reject_wrong_results(self):
        for identity in (f"{TEST}:000:032:below", f"{TEST}:zero", f"{TEST}:divergence:000"):
            with self.subTest(identity=identity):
                data = copy.deepcopy(self.documents)
                next(row for row in data["promql"]["cases"] if row["id"] == identity)["actual"]["value"] = encoded(0.5)
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_original_info_messages_and_no_info_commands_reject_changed_observations(self):
        for line, kind in ((1616, "missing"), (1616, "changed_position"), (53, "added")):
            with self.subTest(line=line, kind=kind):
                data = copy.deepcopy(self.documents)
                rows = [row for row in data["promqltest"]["cases"] if row["source"]["command"]["line"] == line]
                if kind == "added":
                    annotation = next(row["actual"]["annotations"] for row in data["promqltest"]["cases"]
                                      if row["actual"]["annotations"] and row["mode"] == rows[0]["mode"])
                    rows[0]["actual"]["annotations"] = copy.deepcopy(annotation)
                else:
                    for row in rows:
                        if kind == "missing":
                            row["actual"]["annotations"] = None
                        else:
                            row["actual"]["annotations"][0]["formatted"] += "20"
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_immutable_input_snapshot_rejects_even_one_bit_change(self):
        row = self.documents["promql"]["cases"][0]
        actual = copy.deepcopy(row["actual"])
        old = actual["input_after"]["count"]
        actual["input_after"]["count"] = f"{int(old, 16) ^ 1:016x}"
        with self.assertRaises(ValueError):
            validate_outcome(row["request"], actual)

    def test_positions_keep_machine_integer_precision_and_reject_bool_float_overflow(self):
        identity = "supplement:annotation:0:15:0"
        for value in (True, 1.0, -(1 << 63)-1, 1 << 63):
            with self.subTest(value=value):
                data, ledger = copy.deepcopy(self.documents), copy.deepcopy(self.ledger)
                for rows in (data["promql"]["cases"], ledger["cases"]):
                    next(row for row in rows if row["id"] == identity)["request"]["position"]["start"] = value
                with self.assertRaises(ValueError):
                    validate_cases(data, ledger)

    def test_source_argument_position_cannot_move_to_the_scalar_argument(self):
        identity = "dsl:native_histograms.test:1616:000"
        data, ledger = copy.deepcopy(self.documents), copy.deepcopy(self.ledger)
        for rows in (data["promqltest"]["cases"], ledger["cases"]):
            row = next(row for row in rows if row["id"] == identity)
            row["request"]["position"] = copy.deepcopy(row["source"]["command"]["argument_ranges"][0])
        with self.assertRaises(ValueError):
            validate_cases(data, ledger)

    def test_candidate_numeric_tolerance_keeps_bit_differences_visible(self):
        row = self.documents["promql"]["cases"][-1]
        output = [{"id": row["id"], "actual": copy.deepcopy(row["actual"])}]
        output[0]["actual"]["value"] = "3ff0000000000001"
        report = compare_outcomes([row], output)
        self.assertTrue(report["semantic_pass"])
        self.assertFalse(report["bitwise_pass"])
        self.assertEqual(len(report["bit_differences"]), 1)

    def test_signed_zero_composition_cannot_pass_numeric_tolerance(self):
        row = copy.deepcopy(self.documents["promql"]["cases"][-1])
        row["actual"] = scalar_outcome("0000000000000000", row["request"]["input"])
        output = [{"id": row["id"], "actual": scalar_outcome("8000000000000000", row["request"]["input"])}]
        self.assertFalse(compare_outcomes([row], output)["semantic_pass"])

    def test_candidate_annotation_identity_and_bytes_are_exact(self):
        row = next(row for row in self.documents["promqltest"]["cases"] if row["actual"]["annotations"])
        for key, value in (("identity", "NativeHistogramQuantileNaNSkewInfo"), ("raw", "00"),
                           ("formatted", "00"), ("severity", "warning")):
            with self.subTest(key=key):
                actual = copy.deepcopy(row["actual"])
                actual["annotations"][0][key] = value
                self.assertFalse(compare_outcomes([row], [{"id": row["id"], "actual": actual}])["semantic_pass"])

    def test_resealed_supplemental_annotations_must_match_source_bytes_and_positions(self):
        row = next(row for row in self.documents["promql"]["cases"]
                   if row["id"] == "supplement:annotation:0:15:0")
        message = b"PromQL info: input to histogram_quantile has NaN observations, result is skewed higher"
        actual = scalar_outcome(encoded(1.0), row["request"]["input"],
                                [{"identity": "NativeHistogramQuantileNaNSkewInfo", "severity": "info",
                                  "raw": message.hex(), "formatted": (message + b" (invalid position)").hex()}])
        validate_outcome(row["request"], actual)
        for key, value in (("raw", "00"), ("formatted", "00"), ("formatted", message.hex()),
                           ("identity", "NativeHistogramQuantileNaNResultInfo")):
            with self.subTest(key=key, value=value):
                changed = copy.deepcopy(actual)
                changed["annotations"][0][key] = value
                with self.assertRaises(ValueError):
                    validate_outcome(row["request"], changed)
        actual["annotations"] = []
        with self.assertRaises(ValueError):
            validate_outcome(row["request"], actual)


if __name__ == "__main__":
    unittest.main()
