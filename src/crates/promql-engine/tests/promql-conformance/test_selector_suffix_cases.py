"""Preserve raw original occurrences while rejecting false complete-query evidence."""
import base64
from copy import deepcopy
import unittest

import selector_suffix_cases as suffix


def original():
    return {"schema_version": 1, "reference": "github.com/prometheus/prometheus@v0.315.0",
            "commit": suffix.PROMETHEUS_COMMIT, "go_version": "go1.27.1", "go_arch": "amd64",
            "go_os": "linux", "unicode_version": "17.0.0", "results": suffix.cases()[2]["results"]}


class SuffixCoverageTests(unittest.TestCase):
    def test_raw_duplicate_has_distinct_literal_positions_and_one_generated_pair(self):
        data = suffix.load_inputs()
        row = data["tables"][0]["cases"][2]
        self.assertEqual(row["expectedMatches"][5], row["expectedMatches"][6])
        first, second = row["declaration_positions"]["expectedMatches"][5:7]
        self.assertEqual(first["line"], second["line"])
        self.assertLess(first["end_offset"], second["offset"])
        self.assertLess(first["column"], second["column"])
        pairs = suffix._unique_pairs(data)
        self.assertEqual((len(pairs), sum(pair.matches for pair in pairs)), (100, 59))
        self.assertEqual(sum(pair.pattern == suffix.byte_string(row["pattern"])
                             and pair.subject == b"bbbddd" for pair in pairs), 1)
        row["expectedNotMatches"][0] = row["expectedMatches"][0]
        with self.assertRaisesRegex(ValueError, "contradictory"):
            suffix.validate_inputs(data)

    def test_actual_newlines_and_scoped_flags_follow_declarations(self):
        pairs = suffix._unique_pairs(suffix.load_inputs())
        wanted = {(b"test.?", b"test\n"): True,
                  (b"(.*aaa|.*bbb((?i)(.*ccc|.*ddd)))", b"bbbXXXCCC"): True,
                  (b"(.*aaa|.*bbb((?i)(.*ccc|.*ddd)))", b"BBBccc"): False}
        observed = {(pair.pattern, pair.subject): pair.matches for pair in pairs}
        for key, value in wanted.items():
            self.assertIs(observed[key], value)
        dataset, requests, results = suffix.cases()
        self.assertEqual((len(dataset["series"]), len(requests)), (100, 200))
        self.assertEqual(len({row["points"][0]["float"] for row in dataset["series"]}), 100)
        self.assertEqual(sum(bool(row["rows"]) for row in results["results"]), 100)
        for index in range(0, 200, 2):
            self.assertNotEqual(bool(results["results"][index]["rows"]),
                                bool(results["results"][index + 1]["rows"]))

    def test_changed_source_rosters_counts_positions_and_bytes_are_rejected(self):
        for change in ("function", "row", "declaration", "count", "position", "bytes", "digest", "schema"):
            with self.subTest(change=change):
                data = deepcopy(suffix.load_inputs())
                row = data["tables"][0]["cases"][0]
                if change == "function":
                    data["tables"].reverse()
                elif change == "row":
                    data["tables"][0]["cases"].pop()
                elif change == "declaration":
                    row["expectedMatches"].pop()
                elif change == "count":
                    row["expectedLiteralSuffixMatchers"] += 1
                elif change == "position":
                    row["declaration_positions"]["expectedMatches"][1] = deepcopy(
                        row["declaration_positions"]["expectedMatches"][0])
                elif change == "bytes":
                    row["expectedMatches"][0] = base64.b64encode(b"changed").decode()
                elif change == "digest":
                    data["source_sha256"] = "0" * 64
                else:
                    data["schema_version"] = True
                with self.assertRaises(ValueError):
                    suffix.validate_extraction(data)

    def test_complete_outputs_and_original_attribution_cannot_be_inferred(self):
        self.assertTrue(suffix.validate_original(original())["bitwise_pass"])
        for change in ("runtime", "missing", "extra", "membership", "label", "time", "bits", "diagnostic"):
            with self.subTest(change=change):
                data = original()
                row = next(row for row in data["results"] if row["rows"])
                if change == "runtime":
                    data["go_arch"] = "arm64"
                elif change == "missing":
                    data["results"].pop()
                elif change == "extra":
                    data["results"].append(deepcopy(data["results"][-1]))
                elif change == "membership":
                    row["rows"] = []
                elif change == "label":
                    row["rows"][0]["labels"].pop()
                elif change == "time":
                    row["rows"][0]["points"][0]["time_ms"] += 1
                elif change == "bits":
                    row["rows"][0]["points"][0]["float"] = "3ff0000000000001"
                else:
                    row["warnings"] = ["ZXh0cmE="]
                with self.assertRaises(ValueError):
                    suffix.validate_original(data)


if __name__ == "__main__":
    unittest.main()
