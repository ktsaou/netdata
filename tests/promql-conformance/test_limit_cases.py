"""Verify LIMIT declarations cover grouping, mixed heads, faults and sampler boundaries."""

import base64
import math
import re
import struct
import unittest
from copy import deepcopy
from itertools import pairwise
from unittest.mock import patch

import limit_calls
import ratio_sampler
from count_values_cases import FIRST, SECOND, native, native_series, span
from info_cases import bits
from limit_cases import cases


def decoder_sampler_rows(inputs):
    # Stand-ins qualify declaration transport only; query outcomes always come from Go.
    rows = []
    for request in inputs:
        hash_value = request["hash"] or "0000000000000000"
        offset = float(int(hash_value, 16)) / float((1 << 64) - 1)
        ratios = list(ratio_sampler.FIXED_RATIOS)
        for boundary in (offset, offset - 1):
            ratios += [
                bits(math.nextafter(boundary, -math.inf)),
                bits(boundary),
                bits(math.nextafter(boundary, math.inf)),
            ]
        thresholds = []
        for ratio in ratios:
            value = struct.unpack(">d", bytes.fromhex(ratio))[0]
            thresholds.append(
                {
                    "ratio": ratio,
                    "selected": (value >= 0 and offset < value)
                    or (value < 0 and offset >= 1 + value),
                }
            )
        rows.append(
            {
                "id": request["id"],
                "hash": hash_value,
                "offset": bits(offset),
                "thresholds": thresholds,
            }
        )
    return rows


class LimitDeclarations(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.sampler_inputs = ratio_sampler.cases()
        cls.sampler_outputs = decoder_sampler_rows(cls.sampler_inputs)
        cls.rows = cases(cls.sampler_inputs, cls.sampler_outputs)

    def test_roster_and_grouped_operator_replacements_reach_the_declared_paths(self):
        self.assertEqual(len(self.rows), 2240)
        self.assertEqual(
            len({row["request"]["id"] for row in self.rows}), len(self.rows)
        )
        self.assertEqual(
            {row["request"]["profile"] for row in self.rows}, {"upstream-tests"}
        )
        queries = {row["request"]["query"] for row in self.rows}
        self.assertTrue(
            {
                "limitk by(job)(2,m)",
                "limit_ratio by(job)(2,m)",
                "limitk without(z)(2,m)",
                "limit_ratio without(z)(2,m)",
                "limitk by(a,x)(1,m)",
                "limit_ratio by(a,x)(1,m)",
                "limit_ratio(-0,m)",
                "limit_ratio(NaN,m)",
                "limitk(time()+1,m @0)",
            }
            <= queries
        )
        self.assertFalse(
            any(re.search(r"\b(?:topk|bottomk)\b", query) for query in queries)
        )
        collision = [
            row for row in self.rows if ":group-hash-alias:" in row["request"]["id"]
        ]
        self.assertEqual(len(collision), 8)
        lengths = {
            len(row["dataset"]["series"])
            for row in self.rows
            if row["request"]["id"].startswith("limits:heap-")
        }
        self.assertEqual(lengths, {1, 2, 12, 13, 50, 51, 1024, 1025})

    def test_timelines_payload_shapes_and_fault_owners_are_independent(self):
        checked = set()
        for row in self.rows:
            ident = row["request"]["id"].rsplit(":", 2)[0]
            if ident not in checked:
                for record in row["dataset"]["series"]:
                    times = [point["time_ms"] for point in record["points"]]
                    self.assertTrue(
                        all(left < right for left, right in pairwise(times))
                    )
                    for point in record["points"]:
                        self.assertEqual(
                            point.keys(), {"time_ms", "kind", point["kind"]}
                        )
                checked.add(ident)
        siblings = [
            row
            for row in self.rows
            if "set-fault-topk-scalar(p)-m" in row["request"]["id"]
        ]
        self.assertGreaterEqual(len(siblings), 2)
        original = deepcopy(siblings[1]["faults"])
        siblings[0]["faults"]["m"]["set_error"] = "mutated"
        self.assertEqual(siblings[1]["faults"], original)
        siblings[0]["faults"]["m"]["set_error"] = original["m"]["set_error"]

    def test_sampler_inputs_bind_identity_raw_bytes_exact_ratios_and_native_fields(
        self,
    ):
        for request, outcome in zip(
            self.sampler_inputs, self.sampler_outputs, strict=True
        ):
            if not request["labels"]:
                continue
            prefix = "limits:exact-offset-" + request["id"].replace(":", "-") + ":"
            selected = [
                row for row in self.rows if row["request"]["id"].startswith(prefix)
            ]
            self.assertEqual(len(selected), 28)
            for qi, threshold in enumerate(outcome["thresholds"][9:]):
                row = next(
                    row for row in selected if row["request"]["id"].endswith(f":{qi}:0")
                )
                number = row["request"]["query"].split("(", 1)[1].split(",", 1)[0]
                self.assertEqual(bits(float(number)), threshold["ratio"])
                metric = row["dataset"]["series"][0]
                self.assertEqual(metric["labels"], request["labels"])
                self.assertEqual(metric["points"][1]["float_histogram"], native())
            beyond = next(
                row for row in selected if row["request"]["id"].endswith(":6:0")
            )
            ratio = float(beyond["request"]["query"].split("(", 1)[1].split(",", 1)[0])
            offset = struct.unpack(">d", bytes.fromhex(outcome["offset"]))[0]
            self.assertLess(ratio, 0)
            self.assertGreater(1 + ratio, offset)
        changed = deepcopy(self.sampler_outputs)
        changed[0]["id"] = changed[1]["id"]
        with self.assertRaises(ValueError):
            cases(self.sampler_inputs, changed)
        changed = deepcopy(self.sampler_inputs)
        changed[5]["labels"] = deepcopy(changed[6]["labels"])
        with self.assertRaises(ValueError):
            cases(changed, self.sampler_outputs)

    def test_mixed_alias_head_orders_and_incompatible_native_payloads_are_preserved(
        self,
    ):
        expected = {
            "mixed": (
                ("float", "float_histogram", "float"),
                ("float_histogram", "float", "float_histogram"),
            ),
            "float-before-mixed": (
                ("float", "float", "float_histogram"),
                ("float", "float_histogram", "float_histogram"),
            ),
            "mixed-before-float": (
                ("float", "float", "float_histogram"),
                ("float_histogram", "float", "float_histogram"),
            ),
            "histogram-before-mixed": (
                ("float_histogram", "float", "float_histogram"),
                ("float_histogram", "float_histogram", "float"),
            ),
            "mixed-before-histogram": (
                ("float_histogram", "float", "float_histogram"),
                ("float", "float_histogram", "float_histogram"),
            ),
        }
        for ident, timelines in expected.items():
            for operator in ("limitk", "limit_ratio"):
                prefix = f"limits:native-alias-{ident}-{operator}:"
                rows = [
                    row for row in self.rows if row["request"]["id"].startswith(prefix)
                ]
                self.assertEqual(len(rows), 32)
                records = rows[0]["dataset"]["series"]
                self.assertEqual(
                    [
                        base64.b64decode(row["labels"][0]["value"], validate=True)
                        for row in records
                    ],
                    [FIRST, SECOND],
                )
                self.assertEqual(
                    tuple(
                        tuple(point["kind"] for point in row["points"])
                        for row in records
                    ),
                    timelines,
                )
        row = next(
            row
            for row in self.rows
            if row["request"]["id"] == "limits:native-layouts-limitk:0:0"
        )
        self.assertEqual(
            row["dataset"]["series"],
            [
                native_series(
                    native(
                        schema=-53,
                        custom_values=[bits(-1), bits(1)],
                        positive_spans=[span(0, 3)],
                        positive_buckets=[bits(1), bits(2), bits(3)],
                    ),
                    labels={"__name__": "m", "z": "a"},
                ),
                native_series(
                    native(
                        schema=-53,
                        custom_values=[bits(0), bits(2)],
                        positive_spans=[span(0, 3)],
                        positive_buckets=[bits(4), bits(5), bits(6)],
                    ),
                    labels={"__name__": "m", "z": "b"},
                ),
            ],
        )


class LimitHarness(unittest.TestCase):
    def test_sampler_validation_precedes_capture_and_comparison_with_strict_flags(self):
        declared = [{"request": {"id": "declaration"}}]
        with (
            patch(
                "ratio_sampler.load_reference", return_value=("inputs", "outputs")
            ) as sampler,
            patch("limit_calls.cases", return_value=declared) as generator,
            patch("cursor_calls.capture", return_value="source") as capture,
            patch(
                "cursor_calls.qualify", return_value={"bitwise_pass": True}
            ) as qualify,
        ):
            self.assertEqual(limit_calls.capture("upstream", "sampler"), "source")
            capture.assert_called_once_with("upstream", declared, subject="limits")
            self.assertEqual(
                limit_calls.qualify("source", "binary", "sampler", 17),
                {"bitwise_pass": True},
            )
            qualify.assert_called_once_with(
                "source",
                "binary",
                17,
                declared,
                subject="limits",
                exact_numeric=True,
                ordered=True,
            )
            self.assertEqual(sampler.call_count, 2)
            generator.assert_called_with("inputs", "outputs")
        with (
            patch(
                "ratio_sampler.load_reference",
                side_effect=ValueError("invalid sampler"),
            ),
            patch("cursor_calls.capture") as capture,
        ):
            with self.assertRaises(ValueError):
                limit_calls.capture("upstream", "sampler")
            capture.assert_not_called()


if __name__ == "__main__":
    unittest.main()
