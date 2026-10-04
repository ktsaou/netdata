"""A candidate cannot qualify by losing cases, diagnostics or binary64 differences."""
from copy import deepcopy
import unittest

from query import dataset_hex, qualify_outputs, request_hex, result_base64


def fixture(ident="constant", value="3ff0000000000000"):
    reference = {"id": ident, "kind": "scalar", "rows": [{"labels": [], "points": [
        {"time_ms": 1000, "kind": "float", "float": value}]}], "warnings": [], "infos": []}
    entry = {"request": {"id": ident, "mode": "instant"},
             "source_command": {"ordered": False}, "result": reference, "exact_numeric": False}
    return entry, {"id": ident, "result": deepcopy(reference)}


class QueryDispositionTests(unittest.TestCase):
    def test_unsupported_is_open_in_every_full_gate(self):
        one, completed = fixture()
        two, _ = fixture("pending")
        report, _ = qualify_outputs([one, two], [completed, {
            "id": "pending", "unsupported": b"pending reducer".hex(), "observations": {}}])
        self.assertEqual((report["required"], report["completed"], report["passed"], report["failed"]),
                         (2, 1, 1, 0))
        self.assertEqual([row["status"] for row in report["dispositions"]], ["pass", "open"])
        self.assertTrue(report["completed_semantic_pass"])
        self.assertFalse(report["semantic_pass"])
        self.assertFalse(report["bitwise_pass"])

    def test_roster_loss_duplicates_and_ambiguous_outcomes_fail(self):
        entry, output = fixture()
        for rows in [[], [output, output], [dict(output, id="unknown")],
                     [dict(output, unsupported="", observations={})]]:
            with self.assertRaises(ValueError):
                qualify_outputs([entry], rows)

    def test_changed_boolean_or_dropped_warning_remains_a_failure(self):
        entry, output = fixture()
        output["result"]["rows"][0]["points"][0]["float"] = "0000000000000000"
        report, _ = qualify_outputs([entry], [output])
        self.assertEqual(report["failed"], 1)
        self.assertEqual(len(report["bitwise_differences"]), 1)
        self.assertFalse(report["semantic_pass"])
        entry, output = fixture()
        entry["result"]["warnings"] = ["d2FybmluZw=="]
        report, _ = qualify_outputs([entry], [output])
        self.assertEqual(report["semantic_failures"]["constant"], ["warnings"])

    def test_near_boolean_cannot_use_numeric_tolerance_or_missing_classification(self):
        entry, output = fixture()
        entry["exact_numeric"] = True
        output["result"]["rows"][0]["points"][0]["float"] = "3ff000001ad7f29b"
        report, _ = qualify_outputs([entry], [output])
        self.assertEqual(report["failed"], 1)
        self.assertFalse(report["semantic_pass"])
        del entry["exact_numeric"]
        with self.assertRaises(ValueError):
            qualify_outputs([entry], [output])

    def test_nan_payload_difference_is_reported_even_when_semantically_equal(self):
        entry, output = fixture(value="7ff8000000000001")
        output["result"]["rows"][0]["points"][0]["float"] = "fff8000000000002"
        report, _ = qualify_outputs([entry], [output])
        self.assertTrue(report["semantic_pass"])
        self.assertFalse(report["bitwise_pass"])
        self.assertEqual(len(report["bitwise_differences"]), 1)

    def test_byte_projection_preserves_non_utf8_and_source_order(self):
        data = {"series": [
            {"labels": [{"name": "AP8=", "value": "gAo="}]},
            {"labels": [{"name": "eA==", "value": "eQ=="}]}]}
        converted = dataset_hex(data)
        self.assertEqual(converted["series"][0]["labels"], [{"name": "00ff", "value": "800a"}])
        self.assertEqual(data["series"][0]["labels"][0]["name"], "AP8=")
        request = request_hex({"id": "bytes", "query_bytes": "AP8="}, "dataset")
        self.assertEqual(request["expression"], "00ff")
        result = {"id": "bytes", "rows": [{"labels": converted["series"][0]["labels"],
                  "points": [{"kind": "string", "string": "00ff"}]}], "warnings": ["800a"], "infos": []}
        projected = result_base64(result)
        self.assertEqual(projected["rows"][0]["labels"], data["series"][0]["labels"])
        self.assertEqual(projected["rows"][0]["points"][0]["string"], "AP8=")
        self.assertEqual(projected["warnings"], ["gAo="])
        result["warnings"] = ["00 ff"]
        with self.assertRaises(ValueError):
            result_base64(result)


if __name__ == "__main__":
    unittest.main()
