"""Construction-error capture preserves existing helpers and successful source phases."""

import json
import unittest
from unittest.mock import patch

import cursor_calls


class CursorErrorCapture(unittest.TestCase):
    def test_optional_mode_preserves_default_helpers_and_successful_phases(self):
        baseline = cursor_calls.helper_files()
        enabled = cursor_calls.helper_files(construction_errors=True)
        self.assertEqual(set(enabled), set(baseline) | {"helper/cursor_errors.go"})
        for name, data in baseline.items():
            if name != "helper/cursor_phase.go":
                self.assertEqual(enabled[name], data)
        original = baseline["helper/cursor_phase.go"]
        changed = enabled["helper/cursor_phase.go"]
        self.assertEqual(original.count(b"constructionRecord("), 0)
        self.assertEqual(changed.count(b"constructionRecord("), 2)
        success = original.split(b"\t\tstmt := q.Statement().(*parser.EvalStmt)", 1)[1]
        self.assertEqual(
            changed.split(b"\t\tstmt := q.Statement().(*parser.EvalStmt)", 1)[1],
            success,
        )
        self.assertEqual(cursor_calls.helper_files(), baseline)

    def test_capture_mode_is_part_of_the_artifact_roster(self):
        baseline = cursor_calls.artifact_names()
        enabled = cursor_calls.artifact_names(construction_errors=True)
        self.assertEqual(enabled - baseline, {"helper/cursor_errors.go"})
        self.assertFalse(baseline - enabled)

    def test_unknown_or_repeated_failure_patch_sites_are_rejected(self):
        original = (cursor_calls.HERE / "capture/cursor_phase.go").read_bytes()
        target = cursor_calls.HERE / "capture/cursor_phase.go"
        patch_site = b"\t\tast, err := parser.NewParser(po).ParseExpr(text)"
        read_bytes = type(target).read_bytes
        for corrupted in (
            original.replace(patch_site, b"changed", 1),
            original + original,
        ):

            def read(path, data=corrupted):
                return data if path == target else read_bytes(path)

            with (
                patch.object(type(target), "read_bytes", read),
                self.assertRaisesRegex(ValueError, "construction-error patch"),
            ):
                cursor_calls.helper_files(construction_errors=True)

    def test_wrong_capture_mode_cannot_load_an_existing_capsule(self):
        binding = {
            "commit": cursor_calls.COMMIT,
            "cases": 0,
            "source_manifest_sha256": cursor_calls.input_ledger()[
                "source_manifest_sha256"
            ],
            "files": {name: "0" * 64 for name in cursor_calls.artifact_names()},
        }
        read_bytes = type(cursor_calls.HERE).read_bytes

        def read(path):
            return (
                json.dumps(binding).encode()
                if path == cursor_calls.HERE / "binding.json"
                else read_bytes(path)
            )

        with (
            patch.object(type(cursor_calls.HERE), "read_bytes", read),
            self.assertRaisesRegex(ValueError, "artifact roster"),
        ):
            cursor_calls.load_reference(cursor_calls.HERE, [], construction_errors=True)


if __name__ == "__main__":
    unittest.main()
