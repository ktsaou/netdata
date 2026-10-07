"""Falsify replay validation; stand-ins below are first-principles scalar outcomes."""
import copy
import unittest

from classic_replay import compare_outcomes, single_calls
from classic_replay_cases import bits


class ReplayReferenceTests(unittest.TestCase):
    def setUp(self):
        self.buckets = [{"upper_bound": bits(bound), "count": bits(count)}
                        for bound, count in ((1, 10), (2, 20), (float("inf"), 20))]
        result = {"value": bits(1), "reciprocal": bits(1), "negative_reciprocal_infinite": False,
                  "positive_reciprocal_infinite": False, "less_than_positive_infinity": True,
                  "quantile": {"forced_monotonicity": False, "fixed_precision": False,
                               "min_bound": bits(float("inf")), "max_bound": bits(-float("inf")),
                               "max_decrease": bits(0)}}
        self.cases = [{"id": "q", "request": {"buckets": self.buckets, "operation": {"kind": "quantile", "quantile": bits(0.5)}},
                       "original_after": self.buckets, "expected": {"panicked": False, "result": result}}]
        self.output = [{"id": "q", "actual": {"panicked": False, "result": result, "reads": 5, "polls": 10}}]

    def test_scalar_notes_compositions_and_id_changes_cannot_pass(self):
        self.assertTrue(compare_outcomes(self.cases, self.output)["bitwise_pass"])
        for key, value in (("value", bits(1.01)), ("reciprocal", bits(-1)),
                           ("quantile", None), ("less_than_positive_infinity", False)):
            changed = copy.deepcopy(self.output)
            changed[0]["actual"]["result"][key] = value
            self.assertFalse(compare_outcomes(self.cases, changed)["bitwise_pass"])
        changed[0]["id"] = "other"
        with self.assertRaises(ValueError): compare_outcomes(self.cases, changed)
        with self.assertRaises(ValueError): compare_outcomes(self.cases, self.output * 2)

    def test_read_bounds_and_nonboolean_types_are_rejected(self):
        for key, value in (("reads", 13), ("reads", True), ("reads", -1), ("polls", -1), ("panicked", 0)):
            changed = copy.deepcopy(self.output)
            changed[0]["actual"][key] = value
            self.assertFalse(compare_outcomes(self.cases, changed)["bitwise_pass"])

    def test_only_original_backing_state_supplies_subsequent_inputs(self):
        altered = copy.deepcopy(self.buckets)
        altered[1]["count"] = bits(10)
        operation = self.cases[0]["request"]["operation"]
        original = [{"id": "sequence", "mode": "classic-buckets",
                     "request": {"buckets": self.buckets, "operations": [operation, operation]},
                     "actual": {"steps": [dict(self.cases[0]["expected"], input_after=altered),
                                          dict(self.cases[0]["expected"], input_after=altered)]}}]
        calls = single_calls(original)
        self.assertEqual(calls[0]["request"]["buckets"], self.buckets)
        self.assertEqual(calls[1]["request"]["buckets"], altered)
        self.assertIsNot(calls[1]["request"]["buckets"], self.buckets)


if __name__ == "__main__":
    unittest.main()
