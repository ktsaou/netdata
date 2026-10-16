"""Reject false coverage caused by changed source, requests or partial outputs."""
import base64
from copy import deepcopy
import re
import unittest

import selector_prefix_cases as prefix


def original():
    _, _, declared = prefix.cases()
    return {"schema_version": 1, "reference": "github.com/prometheus/prometheus@v0.315.0",
            "commit": prefix.PROMETHEUS_COMMIT, "go_version": "go1.27.1", "go_arch": "amd64",
            "go_os": "linux", "unicode_version": "17.0.0", "results": declared["results"]}


def packets():
    result = [{"dataset_id": prefix.DATASET}]
    for row in original()["results"]:
        for series in row["rows"]:
            for label in series["labels"]:
                for field in ("name", "value"):
                    label[field] = base64.b64decode(label[field], validate=True).hex()
        result.append({"id": row["id"], "result": row})
    return result


class PrefixCoverageTests(unittest.TestCase):
    def test_complete_complements_preserve_source_bytes_and_unique_values(self):
        data = prefix.load_inputs()
        dataset, requests, expected = prefix.cases()
        self.assertEqual(len(dataset["series"]), 52)
        self.assertEqual(len(requests), 104)
        self.assertEqual(sum(bool(row["rows"]) for row in expected["results"]), 52)
        for index in range(0, 104, 2):
            first, second = expected["results"][index:index + 2]
            self.assertNotEqual(bool(first["rows"]), bool(second["rows"]))
            row = (first["rows"] or second["rows"])[0]
            self.assertEqual(int(row["points"][0]["float"], 16),
                             int(dataset["series"][index // 2]["points"][0]["float"], 16))
        self.assertEqual(len({row["points"][0]["float"] for row in dataset["series"]}), 52)
        self.assertEqual(sum(b"\n" in base64.b64decode(row["labels"][2]["value"])
                             for row in dataset["series"]), 2)
        for request in requests:
            pattern = re.search(r'subject(?:=~|!~)"((?:\\x[0-9a-f]{2})+)"}', request["query"])
            self.assertIsNotNone(pattern)
            decoded = bytes.fromhex(pattern.group(1).replace("\\x", ""))
            source = data["cases"][int(request["id"].split(":")[1])]
            self.assertEqual(decoded, base64.b64decode(source["pattern"]))

    def test_source_mutations_are_rejected(self):
        for mutation in ("remove", "duplicate", "reorder", "computed", "membership", "count", "newline"):
            with self.subTest(mutation=mutation):
                data = deepcopy(prefix.load_inputs())
                if mutation == "remove":
                    data["cases"].pop()
                elif mutation == "duplicate":
                    data["cases"][1] = deepcopy(data["cases"][0])
                elif mutation == "reorder":
                    data["cases"].reverse()
                elif mutation == "computed":
                    data["cases"][0]["pattern"] = {"expression": "pattern"}
                elif mutation == "membership":
                    data["cases"][0]["expectedMatches"][0], data["cases"][0]["expectedNotMatches"][0] = (
                        data["cases"][0]["expectedNotMatches"][0], data["cases"][0]["expectedMatches"][0])
                elif mutation == "count":
                    data["cases"][0]["expectedLiteralPrefixMatchers"] += 1
                else:
                    data["cases"][0]["expectedMatches"][-1] = base64.b64encode(b"xyz-016a-ixb-dp\\n").decode()
                with self.assertRaises(ValueError):
                    prefix.validate_extraction(data)

    def test_generated_dataset_and_requests_are_bound(self):
        for mutation in ("operator", "case", "subject", "value", "profile"):
            with self.subTest(mutation=mutation):
                dataset, requests, _ = prefix.cases()
                if mutation == "operator":
                    requests[0]["query"] = requests[0]["query"].replace("=~", "!~")
                elif mutation == "case":
                    requests[0]["id"] = "another-case"
                elif mutation == "subject":
                    dataset["series"][0]["labels"][2]["value"] = "Y2hhbmdlZA=="
                elif mutation == "value":
                    dataset["series"][0]["points"][0]["float"] = "0000000000000000"
                else:
                    requests[0]["profile"] = "upstream-tests"
                with self.assertRaises(ValueError):
                    prefix.validate_generated(dataset, requests)

    def test_runtime_attribution_is_not_inferred_from_old_probe(self):
        for field, value in (("commit", "changed"), ("go_version", "go1.27.0"),
                             ("go_arch", "arm64"), ("schema_version", True)):
            with self.subTest(field=field):
                data = original()
                data[field] = value
                with self.assertRaises(ValueError):
                    prefix.validate_original(data)

    def test_complete_original_and_candidate_outcomes_are_required(self):
        for mutation in ("labels", "time", "value", "membership", "diagnostic", "kind"):
            with self.subTest(mutation=mutation):
                data = original()
                row = data["results"][0]
                if mutation == "labels":
                    row["rows"][0]["labels"].pop()
                elif mutation == "time":
                    row["rows"][0]["points"][0]["time_ms"] += 1
                elif mutation == "value":
                    row["rows"][0]["points"][0]["float"] = "3ff0000000000001"
                elif mutation == "membership":
                    row["rows"] = []
                elif mutation == "diagnostic":
                    row["warnings"] = ["ZXh0cmE="]
                else:
                    row["kind"] = "matrix"
                with self.assertRaises(ValueError):
                    prefix.validate_original(data)
        candidate = packets()
        candidate[1]["result"]["rows"] = []
        with self.assertRaises(ValueError):
            prefix.compare_candidate(original(), candidate)

    def test_missing_extra_reordered_and_unsupported_packets_fail(self):
        for mutation in ("missing", "extra", "reordered", "unsupported", "ack"):
            with self.subTest(mutation=mutation):
                candidate = packets()
                if mutation == "missing":
                    candidate.pop()
                elif mutation == "extra":
                    candidate.append(deepcopy(candidate[-1]))
                elif mutation == "reordered":
                    candidate[1], candidate[2] = candidate[2], candidate[1]
                elif mutation == "unsupported":
                    candidate[1] = {"id": candidate[1]["id"], "unsupported": "skipped"}
                else:
                    candidate[0] = {"dataset_id": "another-dataset"}
                with self.assertRaises(ValueError):
                    prefix.compare_candidate(original(), candidate)

    def test_unmodified_complete_documents_pass(self):
        prefix.validate_extraction(prefix.load_inputs())
        dataset, requests, _ = prefix.cases()
        prefix.validate_generated(dataset, requests)
        self.assertTrue(prefix.validate_original(original())["bitwise_pass"])
        self.assertTrue(prefix.compare_candidate(original(), packets())["bitwise_pass"])


if __name__ == "__main__":
    unittest.main()
