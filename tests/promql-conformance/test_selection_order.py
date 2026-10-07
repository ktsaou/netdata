"""Adversarial checks for ordering identities, source helper returns and heap mutation scope."""
import json
import unittest
from copy import deepcopy

from selection_order import outcomes
from selection_order_cases import generate


class SelectionOrderTests(unittest.TestCase):
    def test_declared_roster_covers_boundaries_and_boolean_reverse_choices(self):
        cases = generate()
        self.assertEqual(len(cases), len({case["id"] for case in cases}))
        public = [case for case in cases if case["action"] == "sort"]
        self.assertTrue({0, 1, 2, 12, 13, 49, 50, 51, 1023, 1024, 1025} <=
                        {len(case["values"]) for case in public})
        self.assertEqual({(case["bottom"], case["reverse"]) for case in public},
                         {(False, False), (False, True), (True, False), (True, True)})
        self.assertTrue({"push", "fix", "heap", "partial", "partition", "equal", "pivot",
                         "reverse", "patterns", "insertion"} <= {case["action"] for case in cases})

    def test_identity_and_value_corruption_is_rejected(self):
        request = {"id": "one", "action": "sort", "values": ["0000000000000000", "8000000000000000"]}
        original = {"id": "one", "actual": {"items": [
            {"ordinal": 0, "bits": request["values"][0]}, {"ordinal": 1, "bits": request["values"][1]}],
            "comparisons": 0, "swaps": 0, "trace": "0000000000000000", "result": None}}
        self.assertEqual(outcomes(json.dumps(original).encode(), [request]), [original])
        mutations = (lambda row: row.update(id="other"),
                     lambda row: row["actual"]["items"][0].update(ordinal=True),
                     lambda row: row["actual"]["items"][0].update(ordinal=1),
                     lambda row: row["actual"]["items"][0].update(bits="8000000000000000"),
                     lambda row: row["actual"].update(comparisons=True),
                     lambda row: row["actual"].update(result=[0]),
                     lambda row: row["actual"]["items"].pop())
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index):
                changed = deepcopy(original)
                mutation(changed)
                with self.assertRaises(ValueError):
                    outcomes(json.dumps(changed).encode(), [request])

    def test_fix_can_replace_only_one_sample_value(self):
        request = {"id": "one", "action": "fix", "replacement": "3ff0000000000000",
                   "values": ["0000000000000000", "8000000000000000"]}
        original = {"id": "one", "actual": {"items": [
            {"ordinal": 1, "bits": "8000000000000000"}, {"ordinal": 0, "bits": "3ff0000000000000"}],
            "comparisons": 0, "swaps": 0, "trace": "0000000000000000", "result": None}}
        self.assertEqual(outcomes(json.dumps(original).encode(), [request]), [original])
        changed = deepcopy(original)
        changed["actual"]["items"][0]["bits"] = request["replacement"]
        with self.assertRaises(ValueError):
            outcomes(json.dumps(changed).encode(), [request])


if __name__ == "__main__":
    unittest.main()
