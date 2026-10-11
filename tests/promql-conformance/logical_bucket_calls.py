#!/usr/bin/env python3
"""Compare tiny native histograms with canonical 64-bit logical bucket addresses."""

import argparse
import base64
import copy
import json

import cursor_calls
from native_matrix_calls import recorded_run


def label(name, value):
    return {
        "name": base64.b64encode(name.encode()).decode(),
        "value": base64.b64encode(value.encode()).decode(),
    }


def cases():
    inputs = []
    for shape, spans, buckets in [
        ("read-high", [(0, 2147483647), (-2, 3)], ["3ff0000000000000"]),
        (
            "alias-high",
            [(0, 2147483647), (-2147483647, 2147483647), (-3, 4)],
            ["3ff0000000000000"] * 2,
        ),
        ("insert-high", [(0, 2147483647), (-2, 2)], ["3ff0000000000000"]),
    ]:
        for side in ["positive", "negative"]:
            base = {
                "reset_hint": 3,
                "schema": 0,
                "zero_threshold": "0000000000000000",
                "zero_count": "0000000000000000",
                "count": "3ff0000000000000",
                "sum": "3ff0000000000000",
                "positive_spans": None,
                "negative_spans": None,
                "positive_buckets": None,
                "negative_buckets": None,
                "custom_values": None,
            }
            left = copy.deepcopy(base)
            left[side + "_spans"] = [
                {"Offset": offset, "Length": length} for offset, length in spans
            ]
            left[side + "_buckets"] = buckets
            right = copy.deepcopy(base)
            right[side + "_spans"] = [{"Offset": 2147483647, "Length": 1}]
            right[side + "_buckets"] = ["4000000000000000"]
            dataset = {
                "schema_version": 1,
                "series": [
                    {
                        "labels": [label("__name__", name)],
                        "points": [
                            {
                                "time_ms": 2000,
                                "kind": "float_histogram",
                                "float_histogram": value,
                            }
                        ],
                    }
                    for name, value in [("a", left), ("b", right)]
                ],
            }
            for operation, expression in [
                ("add", "a + b"),
                ("sub", "a - b"),
                ("kahan", "sum_over_time(h[2s])"),
                ("average", "avg_over_time(h[2s])"),
            ]:
                case_dataset = (
                    dataset
                    if operation in ["add", "sub"]
                    else {
                        "schema_version": 1,
                        "series": [
                            {
                                "labels": [label("__name__", "h")],
                                "points": [
                                    {
                                        "time_ms": time,
                                        "kind": "float_histogram",
                                        "float_histogram": value,
                                    }
                                    for time, value in [(1000, left), (2000, right)]
                                ],
                            }
                        ],
                    }
                )
                for profile in ["server-default", "upstream-tests"]:
                    for mode in ["instant", "range"]:
                        request = {
                            "id": f"logical-bucket-width:{shape}:{side}:{operation}:{profile}:{mode}",
                            "query": expression,
                            "profile": profile,
                            "mode": mode,
                        }
                        request.update(
                            {"time_ms": 2000}
                            if mode == "instant"
                            else {"start_ms": 2000, "end_ms": 3000, "step_ms": 1000}
                        )
                        inputs.append(
                            {"request": request, "dataset": case_dataset, "faults": {}}
                        )
    if len(inputs) != 96:
        raise ValueError("logical bucket request roster changed")
    return inputs


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument(
        "--upstream", help="pinned Prometheus source for original Go capture"
    )
    source.add_argument("--reference", help="immutable original Go capture")
    parser.add_argument("--binary", help="compiled query fixture")
    parser.add_argument("--page-size", type=int, choices=[1, 17, 1024], default=1024)
    args = parser.parse_args()
    inputs = cases()
    reference = args.reference
    if args.upstream:
        reference = cursor_calls.capture(
            args.upstream,
            inputs,
            subject="logical-bucket-width",
            process_runner=recorded_run,
        )
    if args.binary:
        report = cursor_calls.qualify(
            reference,
            args.binary,
            args.page_size,
            inputs,
            subject="logical-bucket-width",
            exact_numeric=True,
            ordered=True,
            process_runner=recorded_run,
        )
        if not report["semantic_pass"] or not report["bitwise_pass"]:
            raise SystemExit(1)
    elif not args.upstream:
        parser.error("--reference requires --binary")
    print(
        json.dumps({"reference": str(reference), "required": len(inputs)}), flush=True
    )
