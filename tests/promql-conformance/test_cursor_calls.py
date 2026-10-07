"""Falsify cursor capture owners, fault context and complete result packets."""
import json
import tempfile
import unittest
from copy import deepcopy
from unittest.mock import patch

from build import directory
from cursor_calls import (
    COMMIT,
    HERE,
    PROFILE,
    artifact_names,
    candidate_packets,
    checked_original,
    helper_files,
    load_reference,
    manifest_digest,
    sha,
    wire_request,
)
from query import checked_candidate_packets


def empty_result(ident):
    return {"id": ident, "kind": "vector", "rows": [], "warnings": [], "infos": []}


class CursorCaptureTests(unittest.TestCase):
    def setUp(self):
        # These are decoder falsifiers, never numerical expectations for an engine.
        self.inputs = [{"request": {"id": f"case-{index}", "query": "m", "profile": "server-default",
                                   "mode": "instant", "time_ms": 1000},
                        "dataset": {"schema_version": 1, "series": []},
                        "faults": {"m": {"message": f"fault-{index}", "next_eof": bool(index)}}}
                       for index in range(2)]
        self.source = {"promql/engine.go": "0" * 64}
        self.task = directory(tempfile.mkdtemp(prefix="cursor-capture-falsification-", dir=directory(HERE / ".tmp")))
        for name, raw in helper_files().items():
            path = self.task / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
        self.original = PROFILE | {"cases": [{"request": wire_request(row["request"]),
            "result": empty_result(row["request"]["id"]), "events": [], "header": {},
            "raw_ast": [], "prepared_ast": [], "executed_ast": []} for row in self.inputs]}
        for name, value in (("inputs.json", self.inputs), ("source.json", self.source),
                            ("original.json", self.original), ("build-command.json", [
                                "go", "build", "-p", "4", "-o", "../cursor-oracle", "."])):
            self.write(name, value)
        for name in ("original.stderr", "build.stdout", "build.stderr"):
            (self.task / name).write_bytes(b"")
        (self.task / "cursor-oracle").write_bytes(b"decoder-only binary fixture")
        (self.task / "toolchain.txt").write_text("go version go1.27.1 linux/amd64\n")
        self.binding = {"commit": COMMIT, "cases": 2, "source_manifest_sha256": manifest_digest(self.source),
                        "files": {name: sha(self.task / name) for name in artifact_names()}}
        self.flush()
        for mock in (patch("cursor_calls.cases", return_value=self.inputs),
                     patch("cursor_calls.input_ledger", return_value={
                         "source_manifest_sha256": manifest_digest(self.source)})):
            mock.start()
            self.addCleanup(mock.stop)

    def write(self, name, value):
        (self.task / name).write_text(json.dumps(value) + "\n")

    def flush(self):
        self.write("binding.json", self.binding)

    def rebind(self, name, value):
        self.write(name, value)
        self.binding["files"][name] = sha(self.task / name)
        self.flush()

    def test_complete_capture_and_explicit_wire_defaults_pass(self):
        inputs, results = load_reference(self.task)
        self.assertEqual(inputs, self.inputs)
        self.assertEqual(results, {"schema_version": 1, "results": [empty_result("case-0"), empty_result("case-1")]})
        changed = deepcopy(self.original)
        changed["cases"][0]["request"].pop("subquery_step_ms")
        with self.assertRaises(ValueError):
            checked_original(changed, self.inputs)

    def test_explicit_expression_cases_bind_inputs_without_loading_the_legacy_roster(self):
        with patch("cursor_calls.cases", side_effect=AssertionError("legacy cases loaded")):
            inputs, results = load_reference(self.task, self.inputs)
            self.assertEqual(inputs, self.inputs)
            self.assertEqual(len(results["results"]), 2)
            changed = deepcopy(self.inputs)
            changed[0]["request"]["query"] = 'count_values("v",m)'
            with self.assertRaises(ValueError):
                load_reference(self.task, changed)

    def test_missing_extra_and_unsafe_capture_rosters_fail(self):
        original = deepcopy(self.binding)
        for name in ("inputs.json", "source.json", "original.json", "cursor-oracle", "helper/main.go"):
            with self.subTest(missing=name):
                self.binding = deepcopy(original)
                self.binding["files"].pop(name)
                self.flush()
                with self.assertRaises(ValueError):
                    load_reference(self.task)
        for name in ("unexpected.json", "../original.json", "/original.json"):
            with self.subTest(extra=name):
                self.binding = deepcopy(original)
                self.binding["files"][name] = "0" * 64
                self.flush()
                with self.assertRaises(ValueError):
                    load_reference(self.task)

    def test_swapped_or_stripped_faults_cannot_alias_equal_datasets(self):
        for change in ("swap", "strip", "request"):
            altered = deepcopy(self.inputs)
            if change == "swap":
                altered[0]["faults"], altered[1]["faults"] = altered[1]["faults"], altered[0]["faults"]
            elif change == "strip":
                altered[1]["faults"]["m"].pop("next_eof")
            else:
                altered[0]["request"]["time_ms"] += 1
            with self.subTest(change=change):
                self.rebind("inputs.json", altered)
                with self.assertRaises(ValueError):
                    load_reference(self.task)

    def test_changed_helper_and_unbound_compilation_input_fail(self):
        original = (self.task / "helper/cursor_phase.go").read_bytes()
        (self.task / "helper/cursor_phase.go").write_bytes(original + b"\n// altered\n")
        self.binding["files"]["helper/cursor_phase.go"] = sha(self.task / "helper/cursor_phase.go")
        self.flush()
        with self.assertRaises(ValueError):
            load_reference(self.task)
        (self.task / "helper/cursor_phase.go").write_bytes(original)
        self.binding["files"]["helper/cursor_phase.go"] = sha(self.task / "helper/cursor_phase.go")
        self.flush()
        (self.task / "helper/extra.go").write_text("package main\n")
        with self.assertRaises(ValueError):
            load_reference(self.task)

    def test_rebound_source_profile_counts_and_request_identity_fail(self):
        original = deepcopy(self.binding)
        for key, value in (("commit", "changed"), ("cases", True), ("cases", 2.0), ("cases", 1),
                           ("source_manifest_sha256", "0" * 64)):
            with self.subTest(key=key):
                self.binding = original | {key: value}
                self.flush()
                with self.assertRaises(ValueError):
                    load_reference(self.task)
        self.binding = original
        self.rebind("source.json", {"promql/engine.go": "1" * 64})
        with self.assertRaises(ValueError):
            load_reference(self.task)
        for key in PROFILE:
            changed = deepcopy(self.original)
            changed[key] = "changed"
            with self.subTest(profile=key), self.assertRaises(ValueError):
                checked_original(changed, self.inputs)
        for field in ("id", "query", "time_ms", "subquery_step_ms"):
            changed = deepcopy(self.original)
            changed["cases"][0]["request"][field] = "changed"
            with self.subTest(request=field), self.assertRaises(ValueError):
                checked_original(changed, self.inputs)

    def test_lost_moved_and_incomplete_original_outcomes_fail(self):
        for rows in (self.original["cases"][:-1], self.original["cases"][::-1],
                     [self.original["cases"][0]] * 2):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                checked_original(self.original | {"cases": rows}, self.inputs)
        for field in ("rows", "warnings", "infos"):
            changed = deepcopy(self.original)
            changed["cases"][0]["result"].pop(field)
            with self.subTest(field=field), self.assertRaises(ValueError):
                checked_original(changed, self.inputs)
        changed = deepcopy(self.original)
        changed["cases"][0]["result"].update(kind="error", error={"phase": "execution", "message": "ZQ=="})
        with self.assertRaises(ValueError):
            checked_original(changed, self.inputs)


class CursorPacketTests(unittest.TestCase):
    def setUp(self):
        self.groups = [{"id": f"dataset-{i}", "requests": [{"id": f"case-{i}"}]} for i in range(2)]
        self.responses = [{"dataset_id": "dataset-0"}, {"id": "case-0", "result": empty_result("case-0")},
                          {"dataset_id": "dataset-1"}, {"id": "case-1", "result": empty_result("case-1")}]

    def test_complete_interleaved_packets_pass(self):
        self.assertEqual(checked_candidate_packets(self.responses, self.groups, "cursor"),
                         {"schema_version": 1, "results": [empty_result("case-0"), empty_result("case-1")]})

    def test_fault_strings_and_operations_keep_their_independent_context(self):
        dataset = {"schema_version": 1, "series": []}
        inputs = [{"request": {"id": "next", "query": "m"}, "dataset": dataset,
                   "faults": {"m": {"message": "Ω next\n", "next_times": [100], "next_eof": True}}},
                  {"request": {"id": "seek", "query": "m"}, "dataset": dataset,
                   "faults": {"m": {"message": "seek \"failure\"", "seek_targets": [900], "seek_eof": True}}}]
        before = deepcopy(inputs)
        groups, rows = candidate_packets(inputs, 17)
        self.assertEqual(groups, [{"id": "next", "requests": [inputs[0]["request"]]},
                                  {"id": "seek", "requests": [inputs[1]["request"]]}])
        self.assertEqual(rows[0], {"op": "dataset", "dataset_id": "next", "page_size": 17,
                                 "dataset": dataset, "faults": inputs[0]["faults"]})
        self.assertEqual(rows[2], {"op": "dataset", "dataset_id": "seek", "page_size": 17,
                                 "dataset": dataset, "faults": inputs[1]["faults"]})
        self.assertEqual(rows[1], {"id": "next", "expression": "6d", "dataset_id": "next"})
        self.assertEqual(rows[3], {"id": "seek", "expression": "6d", "dataset_id": "seek"})
        self.assertEqual(inputs, before)

    def test_lost_reordered_misidentified_and_unsupported_packets_fail(self):
        for rows in (self.responses[:-1], self.responses[::-1],
                     [self.responses[0], self.responses[2], self.responses[1], self.responses[3]],
                     [self.responses[0], {"id": "case-0", "unsupported": "selector"}, *self.responses[2:]],
                     [self.responses[0], {"id": "case-0", "result": empty_result("case-1")}, *self.responses[2:]]):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                checked_candidate_packets(rows, self.groups, "cursor")
