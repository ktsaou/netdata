"""Independent mutation checks for conformance verdicts, including alert membership."""
import base64
from copy import deepcopy
import math
import struct
import unittest

from compare import compare, same_number


def raw(number):
    return struct.pack(">d", number).hex()


def text(value):
    return base64.b64encode(value).decode()


def result(value=0.6):
    return {"schema_version": 1, "results": [{"id": "threshold", "kind": "vector", "warnings": [], "infos": [],
        "rows": [{"labels": [{"name": text(b"instance"), "value": text(b"test")}],
                  "points": [{"time_ms": 1000, "kind": "float", "float": raw(value)}]}]}]}


class ComparisonTests(unittest.TestCase):
    cases = [{"id": "threshold"}]

    def test_upstream_tolerance_and_bit_report_are_separate(self):
        reference, actual = result(), result(math.nextafter(0.6, 0))
        report = compare(reference, actual, self.cases)
        self.assertTrue(report["semantic_pass"])
        self.assertFalse(report["bitwise_pass"])
        self.assertEqual(len(report["bitwise_differences"]), 1)

    def test_threshold_membership_never_uses_numeric_tolerance(self):
        reference, actual = result(), result()
        actual["results"][0]["rows"] = []
        self.assertFalse(compare(reference, actual, self.cases)["semantic_pass"])

    def test_exact_boolean_values(self):
        self.assertFalse(compare(result(1), result(0), self.cases)["semantic_pass"])
        self.assertFalse(compare(result(1), result(math.nextafter(1, 2)),
                                 [{"id": "threshold", "exact_numeric": True}])["semantic_pass"])

    def test_exact_numeric_preserves_upstream_nan_equivalence(self):
        reference, actual = result(math.nan), result(math.nan)
        cases = [{"id": "threshold", "exact_numeric": True}]
        self.assertTrue(compare(reference, actual, cases)["semantic_pass"])
        actual["results"][0]["rows"][0]["points"][0]["float"] = "7ff8000000000001"
        self.assertTrue(compare(reference, actual, cases)["semantic_pass"])
        self.assertFalse(compare(result(math.nan), result(1), cases)["semantic_pass"])

    def test_near_zero_nan_stale_and_infinity_follow_upstream(self):
        self.assertFalse(same_number(raw(0), raw(1e-9)))
        self.assertTrue(same_number(raw(-0.0), raw(0.0)))
        self.assertTrue(same_number("7ff8000000000001", "7ff8000000000002"))
        self.assertFalse(same_number("7ff0000000000002", "7ff8000000000002"))
        self.assertTrue(same_number("7ff0000000000002", "7ff0000000000002"))
        self.assertFalse(same_number(raw(math.inf), raw(-math.inf)))

    def test_diagnostics_labels_timestamps_types_and_counts_are_required(self):
        reference = result()
        mutations = [
            lambda r: r.update(warnings=[text(b"warning")]),
            lambda r: r.update(infos=[text(b"info")]),
            lambda r: r["rows"][0]["labels"][0].update(value=text(b"other")),
            lambda r: r["rows"][0]["points"][0].update(time_ms=1001),
            lambda r: r.update(kind="matrix"),
            lambda r: r["rows"][0]["points"].append({"time_ms": 2000, "kind": "float", "float": raw(1)}),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                actual = deepcopy(reference)
                mutation(actual["results"][0])
                try:
                    self.assertFalse(compare(reference, actual, self.cases)["semantic_pass"])
                except ValueError:
                    pass  # Invalid wire data is rejected before comparison.

    def test_missing_extra_and_duplicate_ids_cannot_pass(self):
        reference, actual = result(), result()
        actual["results"] = []
        self.assertFalse(compare(reference, actual, self.cases)["semantic_pass"])
        actual = result()
        extra = deepcopy(actual["results"][0]); extra["id"] = "extra"
        actual["results"].append(extra)
        self.assertFalse(compare(reference, actual, self.cases)["semantic_pass"])
        actual["results"].append(actual["results"][0])
        with self.assertRaises(ValueError):
            compare(reference, actual, self.cases)

    def test_byte_strings_are_exact(self):
        reference = result()
        r = reference["results"][0]
        r.update(kind="string", rows=[{"labels": [], "points": [{"time_ms": 0, "kind": "string", "string": text(b"\xff\x00")}]}])
        actual = deepcopy(reference)
        actual["results"][0]["rows"][0]["points"][0]["string"] = text(b"\xef\xbf\xbd\x00")
        self.assertFalse(compare(reference, actual, self.cases)["semantic_pass"])

    def test_order_is_enforced_when_required(self):
        reference = result()
        row = deepcopy(reference["results"][0]["rows"][0])
        row["labels"][0]["value"] = text(b"second")
        reference["results"][0]["rows"].append(row)
        actual = deepcopy(reference)
        actual["results"][0]["rows"].reverse()
        self.assertTrue(compare(reference, actual, self.cases)["semantic_pass"])
        self.assertFalse(compare(reference, actual, [{"id": "threshold", "ordered": True}])["semantic_pass"])

    def test_complete_histogram_and_exact_boundaries(self):
        reference = result()
        point = reference["results"][0]["rows"][0]["points"][0]
        point.pop("float")
        histogram = {"reset_hint": 3, "schema": 0, "zero_threshold": raw(0.001),
                     "zero_count": raw(0), "count": raw(2), "sum": raw(1.5),
                     "positive_spans": [{"Offset": 0, "Length": 1}], "negative_spans": None,
                     "positive_buckets": [raw(2)], "negative_buckets": None, "custom_values": None}
        point.update(kind="float_histogram", float_histogram=histogram)
        self.assertTrue(compare(reference, deepcopy(reference), self.cases)["bitwise_pass"])
        for key, value in {"schema": 1, "reset_hint": 0, "count": raw(3), "sum": raw(1.6),
                           "zero_threshold": raw(math.nextafter(0.001, 1)), "custom_values": [raw(1)]}.items():
            with self.subTest(field=key):
                actual = deepcopy(reference)
                actual["results"][0]["rows"][0]["points"][0]["float_histogram"][key] = value
                self.assertFalse(compare(reference, actual, self.cases)["semantic_pass"])
        actual = deepcopy(reference)
        histogram = actual["results"][0]["rows"][0]["points"][0]["float_histogram"]
        histogram["positive_spans"] = [{"Offset": -1, "Length": 2}]
        histogram["positive_buckets"] = [raw(0), raw(2)]
        self.assertTrue(compare(reference, actual, self.cases)["semantic_pass"], "upstream compacts zero buckets")
        histogram["positive_buckets"][1] = raw(math.nextafter(2, 3))
        report = compare(reference, actual, self.cases)
        self.assertTrue(report["semantic_pass"])
        self.assertEqual(len(report["bitwise_differences"]), 1)
        for key, value in {"negative_spans": {}, "negative_buckets": "", "custom_values": False}.items():
            with self.subTest(invalid_collection=key):
                actual = deepcopy(reference)
                actual["results"][0]["rows"][0]["points"][0]["float_histogram"][key] = value
                with self.assertRaises(ValueError):
                    compare(reference, actual, self.cases)

    def test_errors_retain_attribution_and_parser_positions(self):
        reference = result()
        reference["results"][0].update(kind="error", rows=[], error={"phase": "construction", "type": "parser.ParseErrors",
            "message": text(b"unexpected expression"), "parse_errors": [{"start": 2, "end": 3, "line_offset": 0,
                                                                       "message": text(b"invalid argument")}]})
        self.assertTrue(compare(reference, deepcopy(reference), self.cases)["semantic_pass"])
        actual = deepcopy(reference)
        actual["results"][0]["error"]["parse_errors"][0]["start"] = 3
        self.assertFalse(compare(reference, actual, self.cases)["semantic_pass"])

    def test_malformed_payloads_and_empty_required_list_are_rejected(self):
        mutations = [
            lambda r: r["rows"][0].update(points=[]),
            lambda r: r["rows"][0]["points"][0].update(string=text(b"unexpected")),
            lambda r: r["rows"][0]["points"][0].update(time_ms=True),
            lambda r: r["rows"][0]["points"][0].update(kind="string", string=text(b"string")),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                actual = result()
                mutation(actual["results"][0])
                with self.assertRaises(ValueError):
                    compare(result(), actual, self.cases)
        with self.assertRaises(ValueError):
            compare({"schema_version": 1, "results": []}, {"schema_version": 1, "results": []}, [])

    def test_wrong_empty_container_types_cannot_pass(self):
        reference = result()
        reference["results"][0]["rows"] = []
        for field, value in {"rows": {}, "warnings": "", "infos": {}}.items():
            with self.subTest(field=field):
                actual = deepcopy(reference)
                actual["results"][0][field] = value
                with self.assertRaises(ValueError):
                    compare(reference, actual, self.cases)
        reference = result()
        reference["results"][0]["rows"][0]["labels"] = []
        actual = deepcopy(reference)
        actual["results"][0]["rows"][0]["labels"] = {}
        with self.assertRaises(ValueError):
            compare(reference, actual, self.cases)

    def test_missing_matrix_point_does_not_hide_same_time_bit_difference(self):
        reference, actual = result(), result(math.nextafter(0.6, 0))
        reference["results"][0]["kind"] = actual["results"][0]["kind"] = "matrix"
        reference["results"][0]["rows"][0]["points"].insert(0, {"time_ms": 0, "kind": "float", "float": raw(0)})
        report = compare(reference, actual, self.cases)
        self.assertFalse(report["semantic_pass"])
        self.assertIn({"id": "threshold", "path": "row[0].point[1].float", "reference": raw(0.6),
                       "candidate": raw(math.nextafter(0.6, 0))}, report["bitwise_differences"])

    def test_bit_differences_survive_missing_or_reordered_series(self):
        reference, actual = result(), result(math.nextafter(0.6, 0))
        other = deepcopy(reference["results"][0]["rows"][0])
        other["labels"][0]["value"] = text(b"other")
        reference["results"][0]["rows"].append(other)
        report = compare(reference, actual, self.cases)
        self.assertFalse(report["semantic_pass"])
        self.assertEqual(len(report["bitwise_differences"]), 1)


if __name__ == "__main__":
    unittest.main()
