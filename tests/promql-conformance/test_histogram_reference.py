"""Falsify kernel input/provenance coverage and complete-result comparison."""
import copy
import unittest

from histogram import compare_results
from histogram_reference import COMMIT, input_ledger, validate_cases


class HistogramReferenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ledger = input_ledger()
        # These are decoder fixtures; numerical correctness uses executed Go results.
        empty_float = next(case["declared"] for case in cls.ledger["cases"]
                           if case["id"] == "TestFloatHistogramCopy:000")
        cls.document = {"schema_version": 1, "commit": COMMIT, "go_version": "go1.27.1",
                        "go_os": "linux", "go_arch": "amd64", "original_assertions_passed": True, "cases": []}
        for binding in cls.ledger["cases"]:
            case = copy.deepcopy(binding)
            if case["declared"] is not None:
                case["actual"] = copy.deepcopy(case["declared"])
            elif case["mode"] in {"compensated-add", "compensated-sub"}:
                case["actual"] = {"sum": "0000000000000000", "compensation": "0000000000000000"}
            elif case["mode"] == "float-copy-to-schema":
                case["actual"] = {"result": copy.deepcopy(empty_float), "input_after": copy.deepcopy(case["request"]["input"])}
            elif case["mode"] == "float-reduce-resolution":
                case["actual"] = {"result": copy.deepcopy(empty_float), "error": "", "histogram_error": False}
            elif case["mode"] == "exponential-bound":
                case["actual"] = "0000000000000000"
            elif case["mode"] == "exponential-bound-table":
                case["actual"] = [["3fe0000000000000"] * (1 << schema) for schema in range(9)]
            else:
                case["actual"] = copy.deepcopy(empty_float)
            if case["mode"] == "float-reduce-resolution" and case["actual"]["result"] is None:
                case["actual"]["result"] = copy.deepcopy(empty_float)
            cls.document["cases"].append(case)

    def test_complete_decoder_roster(self):
        self.assertEqual(validate_cases(self.document, self.ledger["tables"]),
                         {"original_table_cases": 85, "supplemental_cases": 5549, "total": 5634})

    def test_valid_but_rebound_inputs_are_rejected(self):
        mutations = [
            ("TestFloatHistogramMul:000", lambda case: case["request"].update(scalar="0000000000000000")),
            ("TestFloatHistogramCopy:000", lambda case: case["request"]["input"].update(positive_spans=[])),
            ("TestHistogramToFloat:001", lambda case: case["request"].update(destination=None)),
            ("supplement:arithmetic:float-mul:0:0:0", lambda case: case["request"]["input"].update(reset_hint=255)),
            ("supplement:arithmetic:float-mul:0:0:0", lambda case: case["request"].update(scalar="8000000000000000")),
            ("supplement:compensated-add:0:0", lambda case: case["request"].update(compensation="0000000000000000")),
            ("TestFloatHistogramCompact:000", lambda case: case["request"].update(max_empty=1)),
            ("TestFloatHistogramCopyToSchema:001", lambda case: case["request"].update(target_schema=-1)),
            ("TestFloatHistogramReduceResolution:negative offset", lambda case: case["request"]["input"]["positive_spans"][1].update(Offset=0)),
            ("TestGetBoundExponential:000", lambda case: case["request"].update(index=0)),
            ("supplement:bound:8:255:-1075", lambda case: case["request"].update(schema=7)),
        ]
        for identity, mutate in mutations:
            with self.subTest(identity=identity):
                document = copy.deepcopy(self.document)
                mutate(next(case for case in document["cases"] if case["id"] == identity))
                with self.assertRaises(ValueError):
                    validate_cases(document, self.ledger["tables"])

    def test_same_mode_payload_cannot_replace_another_case(self):
        document = copy.deepcopy(self.document)
        for key in ("request", "actual", "declared"):
            document["cases"][0][key] = copy.deepcopy(document["cases"][1][key])
        with self.assertRaises(ValueError):
            validate_cases(document, self.ledger["tables"])

    def test_declarations_and_source_positions_are_bound(self):
        for mutation in ("prefix", "line", "case_line", "source_sha256", "variable"):
            with self.subTest(mutation=mutation):
                tables = copy.deepcopy(self.ledger["tables"])
                if mutation == "prefix":
                    tables[0]["prefix"] += "\n// replaced declaration\n"
                elif mutation == "line":
                    tables[0]["line"] += 1
                elif mutation == "case_line":
                    tables[0]["case_lines"][0] = tables[0]["case_lines"][1]
                elif mutation == "source_sha256":
                    tables[0]["source_sha256"] = "0" * 64
                else:
                    tables[-1]["variable"] = "cases"
                with self.assertRaises(ValueError):
                    validate_cases(self.document, tables)

    def test_missing_duplicate_and_extra_cases_are_rejected(self):
        for mutation in ("missing", "duplicate", "extra"):
            with self.subTest(mutation=mutation):
                document = copy.deepcopy(self.document)
                if mutation == "missing":
                    document["cases"].pop()
                elif mutation == "duplicate":
                    document["cases"][-1] = copy.deepcopy(document["cases"][0])
                else:
                    document["cases"].append(copy.deepcopy(document["cases"][0]))
                with self.assertRaises(ValueError):
                    validate_cases(document, self.ledger["tables"])

    def test_numeric_bit_differences_are_reported_and_discrete_fields_remain_exact(self):
        expected = {"zero": {"sum": "0000000000000000"}, "nan": {"sum": "7ff8000000000001"}}
        output = [{"id": "zero", "actual": {"sum": "8000000000000000"}},
                  {"id": "nan", "actual": {"sum": "fff8000000000001"}}]
        report = compare_results(expected, output)
        self.assertTrue(report["semantic_pass"])
        self.assertFalse(report["bitwise_pass"])
        self.assertEqual(len(report["bit_differences"]), 2)
        for actual in ({"schema": 1}, {"schema": False}, {"schema": 0, "extra": 0}):
            with self.subTest(actual=actual):
                self.assertFalse(compare_results({"case": {"schema": 0}},
                                                 [{"id": "case", "actual": actual}])["semantic_pass"])

    def test_bound_underflow_is_compared_exactly(self):
        expected = {"bound": "0000000000000001"}
        output = [{"id": "bound", "actual": "0000000000000000"}]
        self.assertTrue(compare_results(expected, output)["semantic_pass"])
        self.assertFalse(compare_results(expected, output, {"bound"})["semantic_pass"])

    def test_complete_candidate_output_is_required(self):
        expected = {"case": {"positive_spans": None}}
        for output in ([], [{"id": "unknown", "actual": {"positive_spans": None}}],
                       [{"id": "case", "actual": {"positive_spans": None}}] * 2):
            with self.subTest(output=output), self.assertRaises(ValueError):
                compare_results(expected, output)
        report = compare_results(expected, [{"id": "case", "actual": {"positive_spans": []}}])
        self.assertFalse(report["semantic_pass"])


if __name__ == "__main__":
    unittest.main()
