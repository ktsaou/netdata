"""Falsify complete sampler captures, unsigned rounding and strict threshold branches."""

import json
import tempfile
import unittest
from copy import deepcopy
from unittest.mock import patch

import cursor_calls
import ratio_sampler
from build import directory


class SamplerCaptureTests(unittest.TestCase):
    def setUp(self):
        # Literal decoder fixtures exercise the source inequalities, not a candidate engine.
        self.inputs = [
            row
            for row in ratio_sampler.cases()
            if row["id"] in {"uint64:0000000000000000", "uint64:ffffffffffffffff"}
        ]
        ratios = [
            list(ratio_sampler.FIXED_RATIOS)
            + [
                "8000000000000001",
                "0000000000000000",
                "0000000000000001",
                "bff0000000000001",
                "bff0000000000000",
                "bfefffffffffffff",
            ],
            list(ratio_sampler.FIXED_RATIOS)
            + [
                "3fefffffffffffff",
                "3ff0000000000000",
                "3ff0000000000001",
                "8000000000000001",
                "0000000000000000",
                "0000000000000001",
            ],
        ]
        selected = [
            [
                False,
                False,
                True,
                False,
                True,
                True,
                True,
                True,
                False,
                False,
                False,
                True,
                True,
                True,
                False,
            ],
            [
                False,
                False,
                False,
                True,
                False,
                True,
                True,
                True,
                False,
                False,
                False,
                True,
                True,
                False,
                False,
            ],
        ]
        self.original = cursor_calls.PROFILE | {
            "cases": [
                {
                    "id": request["id"],
                    "hash": request["hash"],
                    "offset": offset,
                    "thresholds": [
                        {"ratio": ratio, "selected": member}
                        for ratio, member in zip(
                            ratios[index], selected[index], strict=True
                        )
                    ],
                }
                for index, (request, offset) in enumerate(
                    zip(
                        self.inputs,
                        ("0000000000000000", "3ff0000000000000"),
                        strict=True,
                    )
                )
            ]
        }
        self.source = {"promql/engine.go": "0" * 64}
        self.task = directory(
            tempfile.mkdtemp(
                prefix="sampler-capture-falsification-",
                dir=directory(ratio_sampler.HERE / ".tmp"),
            )
        )
        helpers = ratio_sampler.helper_files()
        for name, raw in helpers.items():
            path = self.task / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
        (self.task / "ratio_sampler.go").write_bytes(helpers["helper/ratio_sampler.go"])
        for name, value in (
            ("inputs.json", self.inputs),
            ("original.json", self.original),
            ("source.json", self.source),
            ("build-command.json", ratio_sampler.COMMAND),
        ):
            self.write(name, value)
        for name in ("original.stderr", "build.stdout", "build.stderr"):
            (self.task / name).write_bytes(b"")
        (self.task / "sampler-oracle").write_bytes(b"decoder-only binary fixture")
        (self.task / "toolchain.txt").write_text("go version go1.27.1 linux/amd64\n")
        self.binding = {
            "commit": cursor_calls.COMMIT,
            "cases": 2,
            "source_manifest_sha256": cursor_calls.manifest_digest(self.source),
            "files": {
                name: cursor_calls.sha(self.task / name)
                for name in ratio_sampler.artifact_names()
            },
        }
        self.flush()
        for mock in (
            patch("ratio_sampler.cases", return_value=self.inputs),
            patch(
                "ratio_sampler.input_ledger",
                return_value={
                    "source_manifest_sha256": cursor_calls.manifest_digest(self.source)
                },
            ),
        ):
            mock.start()
            self.addCleanup(mock.stop)

    def write(self, name, value):
        (self.task / name).write_text(json.dumps(value) + "\n")

    def flush(self):
        self.write("binding.json", self.binding)

    def rebind(self, name, value):
        self.write(name, value)
        self.binding["files"][name] = cursor_calls.sha(self.task / name)
        self.flush()

    def test_complete_literal_capture_preserves_rounded_one_and_negative_branch(self):
        self.assertEqual(
            ratio_sampler.load_reference(self.task),
            (self.inputs, self.original["cases"]),
        )
        rounded = self.original["cases"][1]
        self.assertEqual(rounded["offset"], "3ff0000000000000")
        self.assertFalse(rounded["thresholds"][4]["selected"])
        self.assertTrue(rounded["thresholds"][12]["selected"])
        self.assertFalse(rounded["thresholds"][13]["selected"])

    def test_candidate_transport_rejects_loss_reordering_and_incomplete_values(self):
        original = self.original["cases"]
        packets = ratio_sampler.candidate_packets(original)
        self.assertEqual(len(packets), 30)
        self.assertEqual(
            packets[15],
            {
                "id": "uint64:ffffffffffffffff:0",
                "mode": "limit-ratio-sampler",
                "borrowed": False,
                "request": {"hash": "ffffffffffffffff", "ratio": "0000000000000000"},
            },
        )
        responses = [
            {
                "id": f"{row['id']}:{index}",
                "actual": {"offset": row["offset"], "selected": threshold["selected"]},
            }
            for row in original
            for index, threshold in enumerate(row["thresholds"])
        ]
        self.assertEqual(
            ratio_sampler.qualify_outputs(original, responses)["status"], "PASS"
        )
        for rows in (responses[:-1], responses[::-1], [responses[0]] * 30):
            with self.assertRaises(ValueError):
                ratio_sampler.qualify_outputs(original, rows)
        for field, value in (("offset", True), ("offset", "invalid"), ("selected", 0)):
            changed = deepcopy(responses)
            changed[0]["actual"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                ratio_sampler.qualify_outputs(original, changed)

    def test_valid_changed_offset_or_predicate_is_an_exact_failure(self):
        original = self.original["cases"]
        responses = [
            {
                "id": f"{row['id']}:{index}",
                "actual": {"offset": row["offset"], "selected": threshold["selected"]},
            }
            for row in original
            for index, threshold in enumerate(row["thresholds"])
        ]
        responses[15]["actual"]["offset"] = "3fefffffffffffff"
        responses[27]["actual"]["selected"] = False
        report = ratio_sampler.qualify_outputs(original, responses)
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["passed"], 28)
        self.assertEqual(
            {row["id"] for row in report["failures"]},
            {"uint64:ffffffffffffffff:0", "uint64:ffffffffffffffff:12"},
        )

    def test_resealed_offsets_thresholds_and_branch_booleans_fail(self):
        for mutation in (
            "offset",
            "hash",
            "adjacent",
            "equality",
            "negative-zero",
            "nan",
            "bool",
        ):
            changed = deepcopy(self.original)
            row = changed["cases"][1]
            if mutation == "offset":
                row["offset"] = "3fefffffffffffff"
            elif mutation == "hash":
                row["hash"] = "fffffffffffffffe"
            elif mutation == "adjacent":
                row["thresholds"][9]["ratio"] = "3feffffffffffffe"
            elif mutation == "equality":
                row["thresholds"][10]["selected"] = True
            elif mutation == "negative-zero":
                row["thresholds"][12]["selected"] = False
            elif mutation == "nan":
                row["thresholds"][8]["ratio"] = "7ff8000000000000"
            else:
                row["thresholds"][0]["selected"] = 0
            self.rebind("original.json", changed)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                ratio_sampler.load_reference(self.task)

    def test_reordered_missing_duplicate_and_incomplete_outcomes_fail(self):
        for rows in (
            self.original["cases"][::-1],
            self.original["cases"][:-1],
            [self.original["cases"][0]] * 2,
        ):
            with self.assertRaises(ValueError):
                ratio_sampler.checked_original(
                    self.original | {"cases": rows}, self.inputs
                )
        for field in ("hash", "offset", "thresholds"):
            changed = deepcopy(self.original)
            changed["cases"][0].pop(field)
            with self.subTest(field=field), self.assertRaises(ValueError):
                ratio_sampler.checked_original(changed, self.inputs)
        for value in (None, [], self.original["cases"][0]["thresholds"] + [{}]):
            changed = deepcopy(self.original)
            changed["cases"][0]["thresholds"] = value
            with self.assertRaises(ValueError):
                ratio_sampler.checked_original(changed, self.inputs)

    def test_resealed_input_compiler_helper_source_and_binding_changes_fail(self):
        mutations = [
            ("inputs.json", [self.inputs[0], self.inputs[0]]),
            ("build-command.json", ["go", "build", "."]),
            ("source.json", self.source | {"extra.go": "1" * 64}),
        ]
        for name, value in mutations:
            original = (self.task / name).read_bytes()
            self.rebind(name, value)
            with self.subTest(name=name), self.assertRaises(ValueError):
                ratio_sampler.load_reference(self.task)
            (self.task / name).write_bytes(original)
            self.binding["files"][name] = cursor_calls.sha(self.task / name)
            self.flush()
        for name in ("ratio_sampler.go", "helper/ratio_sampler.go", "toolchain.txt"):
            original = (self.task / name).read_bytes()
            (self.task / name).write_bytes(original + b"changed")
            self.binding["files"][name] = cursor_calls.sha(self.task / name)
            self.flush()
            with self.subTest(name=name), self.assertRaises(ValueError):
                ratio_sampler.load_reference(self.task)
            (self.task / name).write_bytes(original)
            self.binding["files"][name] = cursor_calls.sha(self.task / name)
            self.flush()
        for field, value in (("cases", True), ("commit", "0" * 40), ("files", {})):
            original = deepcopy(self.binding)
            self.binding[field] = value
            self.flush()
            with self.subTest(field=field), self.assertRaises(ValueError):
                ratio_sampler.load_reference(self.task)
            self.binding = original


class SamplerDeclarationTests(unittest.TestCase):
    def test_raw_labels_and_synthetic_unsigned_extremes_remain_distinct(self):
        rows = ratio_sampler.cases()
        self.assertEqual(rows, ratio_sampler.cases())
        self.assertEqual(len(rows), 30)
        self.assertEqual(len({row["id"] for row in rows}), 30)
        labels = [row for row in rows if row["hash"] is None]
        primitive = [row for row in rows if row["hash"] is not None]
        self.assertEqual(len(labels), 13)
        self.assertEqual(labels[0]["labels"], [])
        self.assertTrue(all(row["labels"] is None for row in primitive))
        self.assertIn({"name": "/wA=", "value": "/wA="}, labels[7]["labels"])
        self.assertTrue(
            {"fffffffffffffbff", "fffffffffffffc00", "ffffffffffffffff"}
            <= {row["hash"] for row in primitive}
        )


if __name__ == "__main__":
    unittest.main()
