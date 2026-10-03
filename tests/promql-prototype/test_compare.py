"""Exercise the raw-bit report independently of evaluator outputs."""
import unittest

from compare import numeric_difference_report

def result(value, bits):
    return {"results": [{"id": "case", "kind": 1, "rows": [
        {"labels": {}, "points": [[10, value]], "point_bits": [bits]}]}]}

class NumericBitsTest(unittest.TestCase):
    def test_signed_zero_and_nan_bits_are_visible(self):
        for reference, actual in [(("0", "0000000000000000"), ("-0", "8000000000000000")),
                                  (("NaN", "7ff8000000000001"), ("NaN", "fff8000000000000"))]:
            with self.subTest(reference=reference):
                report = numeric_difference_report(result(*reference), result(*actual), [{"id": "case"}])
                self.assertEqual(len(report["bitwise_differences"]), 1)
                self.assertFalse(report["skipped_non_equivalent_cases"])

    def test_metadata_cannot_disagree_with_values(self):
        with self.assertRaises(ValueError):
            numeric_difference_report(result("1", "0000000000000000"), result("1", "3ff0000000000000"), [{"id": "case"}])

if __name__ == "__main__":
    unittest.main()
