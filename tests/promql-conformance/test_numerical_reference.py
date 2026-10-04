"""Falsify numerical input bindings, source assertions and exact-power outcomes."""
import copy
import unittest

from histogram import compare_results
from numerical_reference import assertion_matches, input_ledger, validate_cases


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
                              "log": "0000000000000000", "log2": "0000000000000000", "sqrt": "0000000000000000"}
                             if row["mode"] == "math-set" else "0000000000000000")
            cls.document["cases"].append(row)

    def test_complete_source_and_primitive_inventory(self):
        self.assertEqual(validate_cases(self.document, self.ledger),
                         {"original_cases": 4336, "supplemental_cases": 22110, "total": 26446,
                          "primitive_invocations": 49963})

    def test_computed_input_and_declaration_mutations_are_rejected(self):
        for identity, mutation in (
            ("TestLog2:power:-1074", lambda row: row["request"].update(input="0000000000000002")),
            ("TestSqrt:sc:002", lambda row: row["request"].update(input="0000000000000000")),
            ("supplement:bound:0:1", lambda row: row["request"].update(input="0000000000000000")),
            ("TestLog:vf:000", lambda row: row["declared"].update(comparison="veryclose")),
            ("TestExp2:power:-1074", lambda row: row["request"].update(input="c090c40000000000")),
            ("TestLdexp:sc:011", lambda row: row["request"].update(exponent=1)),
        ):
            with self.subTest(identity=identity):
                data = copy.deepcopy(self.document)
                mutation(next(row for row in data["cases"] if row["id"] == identity))
                with self.assertRaises(ValueError):
                    validate_cases(data, self.ledger)

    def test_original_assertions_reject_wrong_numbers_exponents_and_zero_sign(self):
        for identity, actual in (("TestLog:constant", "0000000000000000"),
                                 ("TestSqrt:sc:002", "0000000000000000"),
                                 ("TestFrexp:bc:000", {"fraction": "3fe0000000000000", "exponent": -1022}),
                                 ("TestExp2:power:0", "4000000000000000"),
                                 ("TestLdexp:bc:005", "0000000000000000")):
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
        for identity, expected, changed in (("TestLog2:power:-1074", "c090c80000000000", "c090c80000000001"),
                                            ("TestExp2:power:0", "3ff0000000000000", "3ff0000000000001")):
            with self.subTest(identity=identity):
                candidate = [{"id": identity, "actual": changed}]
                self.assertTrue(compare_results({identity: expected}, candidate)["semantic_pass"])
                self.assertFalse(compare_results({identity: expected}, candidate, {identity})["semantic_pass"])

    def test_ldexp_exponent_cannot_be_float_boolean_or_outside_machine_int(self):
        identity = "TestLdexp:vf:000"
        for exponent in (1.0, True, -(1 << 63) - 1, 1 << 63):
            with self.subTest(exponent=exponent):
                data, ledger = copy.deepcopy(self.document), copy.deepcopy(self.ledger)
                for rows in (data["cases"], ledger["cases"]):
                    next(row for row in rows if row["id"] == identity)["request"]["exponent"] = exponent
                with self.assertRaises(ValueError):
                    validate_cases(data, ledger)

    def test_close_keeps_its_original_tolerance_separate_from_veryclose(self):
        self.assertTrue(assertion_matches("3ff0000000000000", "3ff0000000000010", "close"))
        self.assertFalse(assertion_matches("3ff0000000000000", "3ff0000000000010", "veryclose"))


if __name__ == "__main__":
    unittest.main()
