#!/usr/bin/env python3
"""The default caller reader must include every emitted instant-set arm."""
import copy
import json
import unittest

import c_embedding


def witnesses():
    rows = []
    for page in (1, 17, 1024):
        for kind in range(3):
            for arm in range(6):
                rows.append({
                    "instant_set_case": "pass", "page": page, "request_kind": kind,
                    "delayed": 0, "arm": arm, "status": (0, 5, 5, 3, 6, 5)[arm],
                    "chunks": (4, 0, 0, 1, 1, 1)[arm], "first_output_cursor": 91,
                    "failed_cursor": 1 if arm == 1 else 91 if arm == 2 else 0,
                    "fetched": 100, "work": 1000,
                })
    rows.append({"instant_set_replay": "pass", "cases": 54, "page_sizes": 3, "request_kinds": 3})
    return rows


class CallerRecords(unittest.TestCase):
    def test_complete_set_panel(self):
        rows = witnesses()
        self.assertEqual(c_embedding.instant_set_records("complete", rows), rows)

    def test_missing_duplicate_reordered_and_unknown_records(self):
        rows = witnesses()
        cases = {
            "missing": rows[:17] + rows[18:],
            "duplicate": rows[:17] + [rows[16]] + rows[18:],
            "reordered": [rows[1], rows[0], *rows[2:]],
            "extra": [*rows[:-1], rows[0], rows[-1]],
            "summary": [*rows[:-1], {"instant_set_replay": "pass"}],
        }
        for name, case in cases.items():
            with self.subTest(name=name), self.assertRaises(ValueError):
                c_embedding.instant_set_records(name, case)

    def test_invalid_fields_and_failure_phases(self):
        changes = {
            "status": (1, "status", 0), "chunks": (4, "chunks", 4),
            "delayed": (0, "delayed", 1), "first": (0, "first_output_cursor", 4),
            "later_first": (3, "first_output_cursor", 92),
            "failed": (2, "failed_cursor", 90), "success_failed": (0, "failed_cursor", 1),
            "negative": (0, "work", -1), "boolean": (0, "arm", False),
            "unknown": (0, "unexpected", 0), "marker": (0, "instant_set_case", "failed"),
        }
        for name, (index, field, value) in changes.items():
            rows = copy.deepcopy(witnesses())
            rows[index][field] = value
            with self.subTest(name=name), self.assertRaises(ValueError):
                c_embedding.instant_set_records(name, rows)

    def test_legacy_roster_without_set_panel_is_rejected(self):
        # Even otherwise valid historical records cannot substitute for the current panel.
        with self.assertRaises(ValueError):
            c_embedding.native_records("missing-set", b"\n".join(json.dumps({}).encode() for _ in range(48)))


if __name__ == "__main__":
    unittest.main()
