"""Falsify classic transport/provenance and source-derived composed outcomes."""
import copy
import math
import unittest

from classic import compare_outcomes
from classic_reference import COMMIT, encoded, input_ledger, roster, validate_cases, validate_outcome
from compare import bits


def scalar(value, quantile):
    floating = bits(value)[1]
    reciprocal = math.copysign(math.inf, floating) if floating == 0 else 1 / floating
    return {"value": value, "reciprocal": encoded(reciprocal), "negative_reciprocal_infinite": reciprocal == -math.inf,
            "positive_reciprocal_infinite": reciprocal == math.inf, "less_than_positive_infinity": floating < math.inf,
            "quantile": quantile}


def details(fixed=False, early=False):
    return {"forced_monotonicity": False, "fixed_precision": fixed, "min_bound": encoded(0.0 if early else math.inf),
            "max_bound": encoded(0.0 if early else -math.inf), "max_decrease": encoded(0.0)}


def protocol_document(ledger):
    # These are wire-validation stand-ins; numeric qualification executes Go independently.
    rows = []
    for binding in ledger["cases"]:
        row = copy.deepcopy(binding)
        if row["mode"] == "almost-equal":
            row["actual"] = row["declared"] if row["declared"] is not None else False
        else:
            request, declaration = row["request"], row["declared"]
            steps = []
            for operation in request["operations"]:
                quantile = operation["kind"] == "quantile"
                q = bits(operation["quantile"])[1] if quantile else 0.0
                early = quantile and (math.isnan(q) or q < 0 or q > 1)
                panicked = not request["buckets"] and not early
                result = None
                if not panicked:
                    corrected = not early and quantile and request["buckets"][-1]["upper_bound"] == encoded(math.inf)
                    value = declaration["value"] if declaration else encoded(1.0)
                    if quantile and not corrected:
                        value = "7ff8000000000001" if not early or math.isnan(q) else encoded(-math.inf if q < 0 else math.inf)
                    result = scalar(value, details(bool(declaration and declaration["fixed_precision"]), not corrected) if quantile else None)
                steps.append({"panicked": panicked, "result": result, "input_after": copy.deepcopy(request["buckets"])})
            row["actual"] = {"steps": steps}
        rows.append(row)
    return {"schema_version": 1, "commit": COMMIT, "go_version": "go1.27.1", "go_os": "linux", "go_arch": "amd64",
            "original_assertions_passed": True, "cases": rows}


class ClassicReferenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ledger = input_ledger()
        cls.document = protocol_document(cls.ledger)
        cls.rows = {row["id"]: row for row in cls.document["cases"]}

    def test_complete_inventory_and_missing_duplicate_extra_are_checked(self):
        self.assertEqual(validate_cases(self.document, self.ledger)["total"], 4176)
        self.assertEqual(len(roster()), 4176)
        for action in ("missing", "duplicate", "extra"):
            document = copy.deepcopy(self.document)
            if action == "missing": document["cases"].pop()
            elif action == "duplicate": document["cases"][-1] = copy.deepcopy(document["cases"][0])
            else: document["cases"][0]["id"] = "unexpected"
            with self.assertRaises(ValueError): validate_cases(document, self.ledger)

    def test_resealed_original_source_and_scope_drift_are_rejected(self):
        mutations = (
            lambda row: row["source"].update(line=316),
            lambda row: row["declared"].update(epsilon=encoded(1e-6)),
            lambda row: row["request"]["operations"].append({"kind": "quantile", "quantile": encoded(.5)}),
        )
        identity = next(identity for identity in self.rows if identity.startswith("original-quantile:"))
        for mutate in mutations:
            ledger, document = copy.deepcopy(self.ledger), copy.deepcopy(self.document)
            for rows in (ledger["cases"], document["cases"]):
                mutate(next(row for row in rows if row["id"] == identity))
            with self.assertRaises(ValueError): validate_cases(document, ledger)

    def test_numeric_boolean_and_original_almost_contracts_are_exact(self):
        row = copy.deepcopy(self.rows["original-almost:09"])
        output = [{"id": row["id"], "actual": False}]
        self.assertFalse(compare_outcomes([row], output)["semantic_pass"])
        for actual in (0, 1, None, "true"):
            with self.assertRaises(ValueError): validate_outcome(row["mode"], row["request"], actual)

    def test_special_quantile_preserves_input_and_precedes_empty_panic(self):
        row = copy.deepcopy(self.rows["supplement:bucket:006:q:00"])
        row["actual"]["steps"][0]["input_after"].reverse()
        with self.assertRaises(ValueError): validate_outcome(row["mode"], row["request"], row["actual"])
        empty = copy.deepcopy(self.rows["supplement:bucket:000:q:00"])
        empty["actual"]["steps"][0]["panicked"] = True
        with self.assertRaises(ValueError): validate_outcome(empty["mode"], empty["request"], empty["actual"])

    def test_backing_tail_length_and_nil_state_are_preserved(self):
        row = copy.deepcopy(self.rows["supplement:bucket:006:sequence"])
        row["actual"]["steps"][1]["input_after"].pop()
        with self.assertRaises(ValueError): validate_outcome(row["mode"], row["request"], row["actual"])
        empty = copy.deepcopy(self.rows["supplement:bucket:000:q:00"])
        empty["actual"]["steps"][0]["input_after"] = []
        with self.assertRaises(ValueError): validate_outcome(empty["mode"], empty["request"], empty["actual"])

    def test_signed_zero_reciprocal_change_rejects_numerical_tolerance(self):
        row = copy.deepcopy(self.rows["supplement:bucket:007:q:04"])
        after = [{"upper_bound": encoded(0.0), "count": encoded(3.0)},
                 {"upper_bound": encoded(math.inf), "count": encoded(3.0)},
                 {"upper_bound": encoded(math.inf), "count": encoded(3.0)}]
        row["actual"] = {"steps": [{"panicked": False, "input_after": after, "result": scalar(encoded(0.0), details())}]}
        candidate = copy.deepcopy(row["actual"])
        candidate["steps"][0]["result"] = scalar(encoded(-0.0), details())
        self.assertFalse(compare_outcomes([row], [{"id": row["id"], "actual": candidate}])["semantic_pass"])

    def test_go_binary_search_nan_witness_rejects_first_hit_substitution(self):
        row = copy.deepcopy(self.rows["supplement:bucket:010:q:08"])
        row["actual"]["steps"][0]["result"] = scalar("7ff8000000000099", details())
        candidate = copy.deepcopy(row["actual"])
        candidate["steps"][0]["result"] = scalar(encoded(.625), details())
        self.assertFalse(compare_outcomes([row], [{"id": row["id"], "actual": candidate}])["semantic_pass"])

    def test_repeated_coalescing_go_witness_keeps_full_mutated_tail(self):
        row = copy.deepcopy(self.rows["supplement:bucket:006:sequence"])
        steps = []
        for index, (count, value) in enumerate(((4.0, .25), (8.0, 1.0), (12.0, 1/6), (16.0, 1.0))):
            after = [{"upper_bound": encoded(1.0), "count": encoded(2.0)},
                     {"upper_bound": encoded(math.inf), "count": encoded(count)},
                     {"upper_bound": encoded(math.inf), "count": encoded(4.0)}]
            steps.append({"panicked": False, "input_after": after, "result": scalar(encoded(value), None if index == 2 else details())})
        row["actual"] = {"steps": steps}
        candidate = copy.deepcopy(row["actual"])
        candidate["steps"][1]["result"] = scalar(encoded(.5), details())
        self.assertFalse(compare_outcomes([row], [{"id": row["id"], "actual": candidate}])["semantic_pass"])

    def test_monotonicity_metadata_and_composed_booleans_are_typed(self):
        row = self.rows["supplement:bucket:004:q:08"]
        for path in ("details", "flag", "sentinel"):
            candidate = copy.deepcopy(row["actual"])
            result = candidate["steps"][0]["result"]
            if path == "details": result["quantile"]["fixed_precision"] = 0
            elif path == "flag": result["less_than_positive_infinity"] = 1
            else: result["quantile"]["min_bound"] = encoded(0.0)
            with self.assertRaises(ValueError): validate_outcome(row["mode"], row["request"], candidate)


if __name__ == "__main__":
    unittest.main()
