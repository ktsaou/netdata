"""Falsify the metadata gate; these wire stand-ins are not numerical sort oracles."""
import copy
import json
import unittest

from go_sort import compare_outcomes, outcomes


class SortReferenceTests(unittest.TestCase):
    def setUp(self):
        self.requests = [{"id": "group", "bounds": ["8000000000000000", "0000000000000000",
                                                    "7ff0000000000044"]}]
        self.reference = [{"id": "group", "actual": {
            "items": [{"bound": bound, "ordinal": ordinal}
                      for ordinal, bound in enumerate(self.requests[0]["bounds"])],
            "comparisons": 2, "trace": "0000000000000042"}}]

    def test_complete_unique_bound_ordinal_identity_is_required(self):
        self.assertEqual(outcomes(json.dumps(self.reference[0]).encode(), self.requests), self.reference)
        for change in ("missing", "duplicate", "bits", "ordinal", "id", "extra"):
            altered = copy.deepcopy(self.reference)
            items = altered[0]["actual"]["items"]
            if change == "missing": items.pop()
            elif change == "duplicate": items[1] = copy.deepcopy(items[0])
            elif change == "bits": items[2]["bound"] = "7ff8000000000044"
            elif change == "ordinal": items[2]["ordinal"] = -1
            elif change == "id": altered[0]["id"] = "unknown"
            else: altered.append(copy.deepcopy(altered[0]))
            raw = b"\n".join(json.dumps(case).encode() for case in altered)
            with self.assertRaises(ValueError): outcomes(raw, self.requests)

    def test_valid_changed_permutation_is_an_exact_failure(self):
        altered = copy.deepcopy(self.reference)
        altered[0]["actual"]["items"].reverse()
        actual = outcomes(json.dumps(altered[0]).encode(), self.requests)
        self.assertEqual(compare_outcomes(self.reference, actual), [{"id": "group", "path": "actual.items"}])

    def test_changed_comparator_observations_and_ids_cannot_pass(self):
        for key, value in (("comparisons", 3), ("trace", "0000000000000043")):
            altered = copy.deepcopy(self.reference)
            altered[0]["actual"][key] = value
            self.assertEqual(compare_outcomes(self.reference, altered), [{"id": "group", "path": "actual." + key}])
        altered[0]["id"] = "unknown"
        with self.assertRaises(ValueError): compare_outcomes(self.reference, altered)


if __name__ == "__main__":
    unittest.main()
