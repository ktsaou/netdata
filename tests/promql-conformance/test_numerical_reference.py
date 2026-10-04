"""Falsify numerical input bindings, source assertions and exact-power outcomes."""
import copy
import unittest

from histogram import compare_results
from numerical_reference import input_ledger, validate_cases


class NumericalReferenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ledger = input_ledger()
        cls.document = {"schema_version": 1, "go_version": "go1.27.1", "go_os": "linux", "go_arch": "amd64",
                        "original_assertions_passed": True, "cases": []}
        # Decoder fixtures use declarations; numerical qualification executes Go independently.
        for binding in cls.ledger["cases"]:
            row = copy.deepcopy(binding)
            row["actual"] = (copy.deepcopy(row["declared"]["value"]) if row["declared"] is not None else
                             {"frexp": {"fraction": "0000000000000000", "exponent": 0},
                              "log": "0000000000000000", "log2": "0000000000000000", "sqrt": "0000000000000000"})
            cls.document["cases"].append(row)

    def test_complete_source_and_primitive_inventory(self):
        self.assertEqual(validate_cases(self.document, self.ledger),
                         {"original_cases": 2174, "supplemental_cases": 7839, "total": 10013,
                          "primitive_invocations": 33530})

    def test_computed_input_and_declaration_mutations_are_rejected(self):
        for identity, mutation in (
            ("TestLog2:power:-1074", lambda row: row["request"].update(input="0000000000000002")),
            ("TestSqrt:sc:002", lambda row: row["request"].update(input="0000000000000000")),
            ("supplement:bound:0:1", lambda row: row["request"].update(input="0000000000000000")),
            ("TestLog:vf:000", lambda row: row["declared"].update(comparison="veryclose")),
        ):
            with self.subTest(identity=identity):
                data = copy.deepcopy(self.document)
                mutation(next(row for row in data["cases"] if row["id"] == identity))
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_original_assertions_reject_wrong_numbers_exponents_and_zero_sign(self):
        for identity, actual in (("TestLog:constant", "0000000000000000"),
                                 ("TestSqrt:sc:002", "0000000000000000"),
                                 ("TestFrexp:bc:000", {"fraction": "3fe0000000000000", "exponent": -1022})):
            with self.subTest(identity=identity):
                data = copy.deepcopy(self.document)
                next(row for row in data["cases"] if row["id"] == identity)["actual"] = actual
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_missing_duplicate_and_extra_outputs_are_rejected(self):
        for mutation in ("missing", "duplicate", "extra"):
            with self.subTest(mutation=mutation):
                data = copy.deepcopy(self.document)
                if mutation == "missing":
                    data["cases"].pop()
                elif mutation == "duplicate":
                    data["cases"][-1] = copy.deepcopy(data["cases"][0])
                else:
                    data["cases"].append(copy.deepcopy(data["cases"][0]))
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_exact_power_result_cannot_pass_numeric_tolerance(self):
        identity = "TestLog2:power:-1074"
        expected = {identity: "c090c80000000000"}
        changed = [{"id": identity, "actual": "c090c80000000001"}]
        self.assertTrue(compare_results(expected, changed)["semantic_pass"])
        self.assertFalse(compare_results(expected, changed, {identity})["semantic_pass"])


if __name__ == "__main__":
    unittest.main()
