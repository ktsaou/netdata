"""Falsify missing capture owners and misbound complete INFO result packets."""
import json
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from unittest.mock import patch

from build import directory
from info_calls import (
    COMMIT,
    HERE,
    MODULE,
    ORACLE_SHA256,
    artifact_names,
    checked_candidate,
    load_reference,
    manifest_digest,
    sha,
)


def empty_result(ident):
    return {"id": ident, "kind": "vector", "rows": [], "warnings": [], "infos": []}


class InfoCaptureTests(unittest.TestCase):
    def setUp(self):
        # Decoder fixtures never supply numerical expectations for an engine.
        self.groups = [{"id": f"group-{index}", "dataset": {"schema_version": 1, "series": []},
                        "requests": [{"id": f"case-{index}", "query": "info(m)", "profile": "upstream-tests",
                                      "mode": "instant", "time_ms": index}]} for index in range(2)]
        self.source = {"promql/info.go": "0" * 64}
        self.task = directory(tempfile.mkdtemp(prefix="info-capture-falsification-", dir=directory(HERE / ".tmp")))
        self.write("inputs.json", self.groups)
        self.write("source.json", self.source)
        combined = []
        for index, group in enumerate(self.groups):
            self.write(f"dataset-{index}.json", group["dataset"])
            self.write(f"requests-{index}.json", group["requests"])
            results = [empty_result(group["requests"][0]["id"])]
            combined.extend(deepcopy(results))
            self.write(f"original-{index}.json", {"schema_version": 1, "results": results,
                       "commit": COMMIT, "reference": MODULE, "go_version": "go1.27.1",
                       "go_os": "linux", "go_arch": "amd64", "unicode_version": "17.0.0"})
            (self.task / f"original-{index}.stderr").write_bytes(b"")
        self.expected = {"schema_version": 1, "results": combined}
        self.write("original.json", self.expected)
        self.binding = {"cases": 2, "groups": 2, "oracle_sha256": ORACLE_SHA256,
                        "source_manifest_sha256": manifest_digest(self.source),
                        "case_source_sha256": sha(HERE / "info_cases.py"),
                        "artifacts": {name: sha(self.task / name) for name in artifact_names(self.groups)}}
        self.flush()
        self.case_patch = patch("info_calls.cases", return_value=self.groups)
        self.ledger_patch = patch("info_calls.input_ledger", return_value={
            "source_manifest_sha256": manifest_digest(self.source)})
        self.case_patch.start()
        self.ledger_patch.start()
        self.addCleanup(self.case_patch.stop)
        self.addCleanup(self.ledger_patch.stop)

    def write(self, name, value):
        (self.task / name).write_text(json.dumps(value) + "\n")

    def flush(self):
        self.write("manifest.json", self.binding)

    def rebind(self, name, value):
        self.write(name, value)
        self.binding["artifacts"][name] = sha(self.task / name)
        self.flush()

    def test_complete_original_capture_is_bound(self):
        self.assertEqual(load_reference(self.task), (self.groups, self.expected))

    def test_missing_extra_unsafe_and_malformed_artifact_rosters_fail(self):
        original = deepcopy(self.binding)
        for key in ("inputs.json", "source.json", "original.json", "original-1.json", "dataset-0.json"):
            with self.subTest(missing=key):
                self.binding = deepcopy(original)
                self.binding["artifacts"].pop(key)
                self.flush()
                with self.assertRaises(ValueError):
                    load_reference(self.task)
        for extra in ("unexpected.json", "../original.json", str(Path("/") / "original.json")):
            with self.subTest(extra=extra):
                self.binding = deepcopy(original)
                self.binding["artifacts"][extra] = "0" * 64
                self.flush()
                with self.assertRaises(ValueError):
                    load_reference(self.task)
        self.binding = deepcopy(original)
        self.binding["artifacts"] = list(self.binding["artifacts"])
        self.flush()
        with self.assertRaises(ValueError):
            load_reference(self.task)

    def test_counts_and_original_source_bindings_cannot_change(self):
        original = deepcopy(self.binding)
        for key, value in (("cases", 2.0), ("cases", 1), ("groups", True), ("groups", 3),
                           ("oracle_sha256", "0" * 64), ("case_source_sha256", "0" * 64),
                           ("source_manifest_sha256", "0" * 64)):
            with self.subTest(key=key, value=value):
                self.binding = deepcopy(original)
                self.binding[key] = value
                self.flush()
                with self.assertRaises(ValueError):
                    load_reference(self.task)
        self.binding = original
        self.rebind("source.json", {"promql/info.go": "1" * 64})
        with self.assertRaises(ValueError):
            load_reference(self.task)

    def test_rebound_group_inputs_are_rejected(self):
        requests = deepcopy(self.groups[0]["requests"])
        requests[0]["time_ms"] += 1
        self.rebind("requests-0.json", requests)
        with self.assertRaises(ValueError):
            load_reference(self.task)
        self.rebind("requests-0.json", self.groups[0]["requests"])
        self.rebind("dataset-0.json", {"schema_version": 1, "series": [], "rebound": True})
        with self.assertRaises(ValueError):
            load_reference(self.task)

    def test_missing_duplicate_moved_and_rebound_original_results_fail(self):
        original = json.loads((self.task / "original-0.json").read_bytes())
        for results in ([], [empty_result("case-1")], [empty_result("case-0"), empty_result("case-0")]):
            with self.subTest(results=results):
                self.rebind("original-0.json", original | {"results": results})
                with self.assertRaises(ValueError):
                    load_reference(self.task)
        self.rebind("original-0.json", original)
        changed = deepcopy(self.expected)
        changed["results"][0]["warnings"] = ["bG9zdA=="]
        self.rebind("original.json", changed)
        with self.assertRaises(ValueError):
            load_reference(self.task)

    def test_original_environment_is_required_even_with_a_valid_artifact_digest(self):
        original = json.loads((self.task / "original-0.json").read_bytes())
        for key in ("commit", "reference", "go_version", "go_os", "go_arch", "unicode_version"):
            with self.subTest(key=key):
                self.rebind("original-0.json", original | {key: "changed"})
                with self.assertRaises(ValueError):
                    load_reference(self.task)


class InfoPacketTests(unittest.TestCase):
    def setUp(self):
        self.groups = [{"id": "a", "requests": [{"id": "one"}, {"id": "two"}]},
                       {"id": "b", "requests": [{"id": "three"}]}]
        self.responses = [{"dataset_id": "a"}, {"id": "one", "result": empty_result("one")},
                          {"id": "two", "result": empty_result("two")}, {"dataset_id": "b"},
                          {"id": "three", "result": empty_result("three"), "observations": {}}]

    def test_complete_interleaved_packets_pass(self):
        self.assertEqual([row["id"] for row in checked_candidate(self.responses, self.groups)["results"]],
                         ["one", "two", "three"])

    def test_packet_loss_duplication_and_delayed_acknowledgement_fail(self):
        for rows in (self.responses[:-1], [*self.responses, self.responses[0]],
                     [self.responses[0], self.responses[1], self.responses[3], self.responses[2], self.responses[4]],
                     [self.responses[0], self.responses[1], self.responses[1], self.responses[3], self.responses[4]]):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                checked_candidate(rows, self.groups)

    def test_unsupported_misidentified_and_malformed_results_fail(self):
        for row in ({"id": "one", "unsupported": "info"}, {"id": "one"},
                    {"id": "one", "result": empty_result("two")},
                    {"id": "one", "result": empty_result("one"), "unsupported": "info"}):
            with self.subTest(row=row):
                responses = deepcopy(self.responses)
                responses[1] = row
                with self.assertRaises(ValueError):
                    checked_candidate(responses, self.groups)
        responses = deepcopy(self.responses)
        responses[0]["extra"] = True
        with self.assertRaises(ValueError):
            checked_candidate(responses, self.groups)
