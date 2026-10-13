"""Verify independent collision witnesses and COUNT-specific capture/qualification contracts."""
import base64
import unittest
from copy import deepcopy
from unittest.mock import patch

import count_values_calls
from count_values_cases import FIRST, SECOND, cases
from cursor_calls import PROFILE, checked_original, wire_request


class CountValuesCaseTests(unittest.TestCase):
    def test_grouping_collision_is_identical_bytes_with_distinct_full_identity(self):
        # HashForLabels uses separators; full stringlabels hashing includes field lengths.
        row = next(row for row in cases() if row["request"]["id"] ==
                   "count-values:grouping-hash-collision:server-default:2:0")
        left, right = [[(base64.b64decode(label["name"]), base64.b64decode(label["value"]))
                        for label in series["labels"] if base64.b64decode(label["name"]) in (b"a", b"x")]
                       + [(b"z", b"1")] for series in row["dataset"]["series"]]
        grouping = lambda labels: b"".join(name + b"\xff" + value + b"\xff" for name, value in labels)
        self.assertEqual(grouping(left), grouping(right))
        full = lambda labels: b"".join(bytes([len(name)]) + name + bytes([len(value)]) + value
                                       for name, value in labels)
        self.assertNotEqual(full(left), full(right))

    def test_full_hash_witnesses_keep_unequal_source_label_bytes(self):
        # The retained typed hash test independently pins their shared 0x9c710b2e3747e3bd source hash.
        full = lambda value: bytes([1]) + b"a" + bytes([len(value)]) + value
        self.assertNotEqual(FIRST, SECOND)
        self.assertNotEqual(full(FIRST), full(SECOND))
        row = next(row for row in cases() if row["request"]["id"] ==
                   "count-values:full-hash-False:server-default:0:0")
        values = [base64.b64decode(next(label["value"] for label in series["labels"]
                                       if base64.b64decode(label["name"]) == b"a"))
                  for series in row["dataset"]["series"]]
        self.assertEqual(values, [FIRST, SECOND])

    def test_wrapper_requires_exact_values_and_only_relaxes_instant_map_order(self):
        inputs = [{"request": {"id": "one", "query": 'count_values("v",m)'},
                   "dataset": {"schema_version": 1, "series": []}}]
        with patch("count_values_calls.cases", return_value=inputs):
            with patch("cursor_calls.qualify", return_value={"bitwise_pass": True}) as qualify:
                self.assertTrue(count_values_calls.qualify("capture", "binary", 17)["bitwise_pass"])
                qualify.assert_called_once_with("capture", "binary", 17, inputs,
                    subject="count-values", exact_numeric=True, ordered=False)
            with patch("cursor_calls.load_reference", return_value=(inputs, {})) as load:
                count_values_calls.load_reference("capture")
                load.assert_called_once_with("capture", inputs)

    def test_error_class_and_message_remain_required_in_complete_outcomes(self):
        inputs = cases()[:1]
        ident = inputs[0]["request"]["id"]
        original = PROFILE | {"cases": [{"request": wire_request(inputs[0]["request"]),
            "result": {"id": ident, "kind": "error", "rows": [], "warnings": [], "infos": [],
                       "error": {"phase": "execution", "type": "*fmt.wrapError",
                                 "message": base64.b64encode(b"source failure").decode()}},
            "events": [], "header": {}, "raw_ast": [], "prepared_ast": [], "executed_ast": []}]}
        self.assertEqual(checked_original(original, inputs)["results"][0], original["cases"][0]["result"])
        for field in ("phase", "type", "message"):
            changed = deepcopy(original)
            changed["cases"][0]["result"]["error"].pop(field)
            with self.subTest(field=field), self.assertRaises(ValueError):
                checked_original(changed, inputs)


if __name__ == "__main__":
    unittest.main()
