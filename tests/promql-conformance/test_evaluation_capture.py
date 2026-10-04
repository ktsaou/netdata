"""Evaluation capture must retain every original command, variant, input and option."""
from copy import deepcopy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from capture import BUILTIN_OPTIONS, HERE, source_commands, validate_evaluation


class MemoryPath:
    def __init__(self, files, path=""):
        self.files, self.path = files, path

    def __truediv__(self, part):
        return MemoryPath(self.files, self.path + ("/" if self.path else "") + part)

    def open(self):
        return io.StringIO(self.files[self.path].decode())

    def read_bytes(self):
        return self.files[self.path]


def fixtures():
    raw = b'{"schema_version":1,"series":[]}'
    digest = hashlib.sha256(raw).hexdigest()
    commands, entries = [], []
    for index in range(2243):
        command = {"id": f"testdata/synthetic.test:{index+1}", "query": "MQ==", "mode": "instant"}
        ident = command["id"] + ":variant:0000"
        commands.append(command)
        entries.append({"source_command": command, "variant_ordinal": 0, "dataset_id": digest,
            "fixture_replay_exact": True,
            "request": {"id": ident, "query_bytes": "MQ==", "profile": "upstream-tests",
                "mode": "instant", "time_ms": 0, "lookback_ms": 300000, "subquery_step_ms": 60000},
            "query_options": {"nil": True, "per_step": False, "lookback_ns": 0},
            "effective_context": {"start_ms": 0, "end_ms": 0, "interval_ns": 0, "lookback_ns": 300_000_000_000},
            "result": {"id": ident, "kind": "scalar", "warnings": [], "infos": [],
                "rows": [{"labels": [], "points": [{"time_ms": 0, "kind": "float", "float": "3ff0000000000000"}]}]}})
    metadata = {"schema_version": 1, "commit": "5241a27fe3c6983549fccc32f6e65917408c63cd",
        "original_assertions_passed": True, "fixture_replays_exact": True,
        "engine_options": deepcopy(BUILTIN_OPTIONS), "commands": deepcopy(commands), "entries": len(entries), "datasets": 1}
    files = {f"datasets/{digest}.json": raw}
    expected = {c["id"]: c["mode"] for c in commands}
    return metadata, entries, files, expected


def validate(metadata, entries, files, expected):
    files = dict(files, **{"evaluation.jsonl": ("\n".join(json.dumps(e) for e in entries) + "\n").encode()})
    return validate_evaluation(metadata, MemoryPath(files), expected)


class EvaluationCaptureMutationTests(unittest.TestCase):
    def test_command_and_variant_loss_or_duplication_fail(self):
        metadata, entries, files, expected = fixtures()
        self.assertEqual(validate(metadata, entries, files, expected)["source_commands"], 2243)
        for changed in [entries[:-1], entries + [entries[-1]]]:
            with self.assertRaises(ValueError):
                validate(metadata, changed, files, expected)
        entries[0]["variant_ordinal"] = 1
        with self.assertRaises(ValueError):
            validate(metadata, entries, files, expected)

    def test_source_query_and_replay_failure_cannot_be_rewritten(self):
        metadata, entries, files, expected = fixtures()
        entries[0]["request"]["query_bytes"] = "Mg=="
        with self.assertRaises(ValueError):
            validate(metadata, entries, files, expected)
        entries[0]["request"]["query_bytes"] = "MQ=="
        entries[0]["fixture_replay_exact"] = False
        with self.assertRaises(ValueError):
            validate(metadata, entries, files, expected)

    def test_effective_options_and_time_cannot_be_dropped(self):
        metadata, entries, files, expected = fixtures()
        for field, value in [("query_options", {"nil": False, "per_step": False, "lookback_ns": 0}),
                             ("effective_context", {"start_ms": 1, "end_ms": 1, "interval_ns": 0, "lookback_ns": 300_000_000_000})]:
            changed = deepcopy(entries)
            changed[0][field] = value
            with self.assertRaises(ValueError):
                validate(metadata, changed, files, expected)
        metadata["engine_options"]["use_start_timestamps"] = False
        with self.assertRaises(ValueError):
            validate(metadata, entries, files, expected)

    def test_dataset_digest_and_complete_result_are_checked(self):
        metadata, entries, files, expected = fixtures()
        mutated = {name: raw + b" " for name, raw in files.items()}
        with self.assertRaises(ValueError):
            validate(metadata, entries, mutated, expected)
        entries[0]["result"]["rows"] = []
        with self.assertRaises(ValueError):
            validate(metadata, entries, files, expected)

    def test_indented_original_command_is_included(self):
        output = HERE / ".tmp"
        output.mkdir(exist_ok=True)
        root = Path(tempfile.mkdtemp(prefix="inventory-", dir=output))
        target = root / "promql/promqltest/testdata"
        target.mkdir(parents=True)
        (target / "synthetic.test").write_text(' eval instant at 5m increase(series[5m])\n\teval range from 0m to 1m step 1m vector(1)\n# eval instant at 0m 2\n')
        self.assertEqual(source_commands(root), {"testdata/synthetic.test:1": "instant", "testdata/synthetic.test:2": "range"})


if __name__ == "__main__":
    unittest.main()
