"""Falsify fraction/trim source bindings, assertion scope and ownership snapshots."""
import copy
import struct
import unittest

from histogram import compare_results
from trim_reference import COMMIT, definitions, input_ledger, validate_case, validate_cases, validate_outcome


def encoded(value):
    return struct.pack(">d", value).hex()


class TrimReferenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ledger = input_ledger()
        cls.documents = {package: {"schema_version": 1, "commit": COMMIT, "go_version": "go1.27.1",
                         "go_os": "linux", "go_arch": "amd64", "original_assertions_passed": True, "cases": []}
                         for package in ("histogram", "promql")}
        # Decoder fixtures exercise assertions and ownership; Go execution qualifies numerical outcomes.
        for binding in cls.ledger["cases"]:
            row = copy.deepcopy(binding)
            declared = row["declared"]
            if row["mode"] == "float-trim-equality":
                request = row["request"]
                receiver = copy.deepcopy(request["peer"])
                row["actual"] = {"trim": {"receiver_after": receiver, "input_after": copy.deepcopy(receiver),
                                  "returned_receiver": True, "panic": ""},
                                 "forward": True, "reverse": True, "borrowed_forward": True,
                                 "borrowed_reverse": True, "peer_after": copy.deepcopy(request["peer"])}
            elif row["mode"] == "bucket-fraction":
                row["actual"] = declared["value"] if declared else encoded(0)
            else:
                request = row["request"]
                receiver = copy.deepcopy(request["input"])
                if request["copy"]:
                    if receiver["schema"] == -53:
                        receiver["zero_threshold"] = encoded(0)
                    else:
                        receiver["custom_values"] = None
                if declared:
                    receiver["count"] = (declared["value"] if declared["comparison"] != "lifecycle"
                                         else encoded(31))
                row["actual"] = {"receiver_after": receiver, "input_after": copy.deepcopy(
                    request["input"] if request["copy"] else receiver), "returned_receiver": True, "panic": ""}
            package = "promql" if row["id"].startswith("TestTrimBuckets_") else "histogram"
            cls.documents[package]["cases"].append(row)

    def test_complete_original_and_supplemental_inventory(self):
        self.assertEqual(validate_cases(self.documents, self.ledger),
                         {"original_cases": 180, "supplemental_cases": 1481, "total": 1661})

    def test_source_coordinates_calls_and_computed_inputs_are_bound(self):
        for key in ("line", "call_lines"):
            with self.subTest(key=key):
                changed = copy.deepcopy(self.ledger["definitions"])
                definition = changed["TestTrimBuckets_HistogramFractionCrossCheck"]
                definition[key] = definition[key] + 1 if key == "line" else definition[key][:-1]
                with self.assertRaises(ValueError):
                    definitions(changed)
        for key, value in (("threshold", encoded(99)), ("upper_trim", False), ("copy", False)):
            with self.subTest(key=key):
                changed = copy.deepcopy(self.documents)
                changed["promql"]["cases"][0]["request"][key] = value
                with self.assertRaises(ValueError):
                    validate_cases(changed, self.ledger)

    def test_original_absolute_relative_and_exact_predicates_are_preserved(self):
        rows = [next(row for row in self.documents["histogram"]["cases"] if row["mode"] == "bucket-fraction"),
                next(row for row in self.documents["promql"]["cases"] if row["declared"]["comparison"] == "relative"),
                next(row for row in self.documents["promql"]["cases"] if ":divergence:" in row["id"])]
        for row in rows:
            with self.subTest(identity=row["id"]):
                changed = copy.deepcopy(row)
                if row["mode"] == "bucket-fraction":
                    changed["actual"] = encoded(99)
                else:
                    changed["actual"]["receiver_after"]["count"] = encoded(99)
                with self.assertRaises(ValueError):
                    validate_case(changed)
        divergence = rows[-1]
        self.assertEqual(divergence["declared"]["value"], encoded(18))
        changed = copy.deepcopy(divergence)
        changed["actual"]["receiver_after"]["count"] = changed["request"]["input"]["count"]
        with self.assertRaises(ValueError):
            validate_case(changed)

    def test_read_only_copy_source_rejects_one_ulp_even_when_numeric_tolerance_passes(self):
        row = self.documents["promql"]["cases"][0]
        changed = copy.deepcopy(row["actual"])
        value = changed["input_after"]["count"]
        changed["input_after"]["count"] = f"{int(value, 16) + 1:016x}"
        report = compare_results({row["id"]: row["actual"]}, [{"id": row["id"], "actual": changed}])
        self.assertTrue(report["semantic_pass"])
        self.assertFalse(report["bitwise_pass"])
        with self.assertRaises(ValueError):
            validate_outcome(row["request"], changed)

    def test_original_lifecycle_and_panic_failures_cannot_be_hidden(self):
        source = next(row for row in self.documents["histogram"]["cases"] if row["mode"] == "float-trim")
        for kind in ("identity", "unchanged", "panic"):
            with self.subTest(kind=kind):
                row = copy.deepcopy(source)
                if kind == "identity":
                    row["actual"]["returned_receiver"] = False
                elif kind == "unchanged":
                    row["actual"]["receiver_after"] = copy.deepcopy(row["request"]["input"])
                    row["actual"]["input_after"] = copy.deepcopy(row["request"]["input"])
                else:
                    row["actual"].update(returned_receiver=False, panic="unexpected failure")
                with self.assertRaises(ValueError):
                    validate_case(row)

    def test_trim_equality_boolean_defects_fail_even_when_nan_values_pass_tolerance(self):
        source = next(row for row in self.documents["histogram"]["cases"] if row["mode"] == "float-trim-equality")
        for key in ("forward", "reverse", "borrowed_forward", "borrowed_reverse"):
            with self.subTest(key=key):
                changed = copy.deepcopy(source["actual"])
                changed[key] = False
                report = compare_results({source["id"]: source["actual"]}, [{"id": source["id"], "actual": changed}])
                self.assertFalse(report["semantic_pass"])

    def test_missing_duplicate_extra_and_moved_rows_are_rejected(self):
        for kind in ("missing", "duplicate", "extra", "moved"):
            with self.subTest(kind=kind):
                changed = copy.deepcopy(self.documents)
                rows = changed["promql"]["cases"]
                if kind == "missing":
                    rows.pop()
                elif kind == "duplicate":
                    rows[-1] = copy.deepcopy(rows[0])
                elif kind == "extra":
                    rows.append(copy.deepcopy(rows[0]))
                else:
                    changed["histogram"]["cases"].append(rows.pop())
                with self.assertRaises(ValueError):
                    validate_cases(changed, self.ledger)


if __name__ == "__main__":
    unittest.main()
