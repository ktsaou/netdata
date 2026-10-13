#!/usr/bin/env python3
"""Qualify source duplicate projected rows and their operation-specific cleanup."""

import argparse
from copy import deepcopy

import cursor_calls
from count_values_cases import WINDOWS
from info_cases import series


def cases():
    inputs = []

    def add(ident, records, queries):
        for profile in ("server-default", "upstream-tests"):
            for qi, query in enumerate(queries):
                for wi, (mode, time, start, end, step) in enumerate(WINDOWS):
                    inputs.append(
                        {
                            "request": {
                                "id": f"{ident}:{profile}:{qi}:{wi}",
                                "query": query,
                                "profile": profile,
                                "mode": mode,
                                "time_ms": time,
                                "start_ms": start,
                                "end_ms": end,
                                "step_ms": step,
                                "lookback_ms": 1,
                                "subquery_step_ms": 1000,
                            },
                            "dataset": {
                                "schema_version": 1,
                                "series": deepcopy(records),
                            },
                            "faults": {},
                        }
                    )

    # engine.go1630-1668 hashes raw groups; stringlabels.go471-480 drops empty values.
    add(
        "duplicate-row",
        [
            series({"__name__": "m", "a": "", "z": "a"}, [(0, 1, "float")]),
            series({"__name__": "m", "z": "b"}, [(0, 2, "float")]),
        ],
        (
            "sum by(a)(m)",
            "min by(a)(m)",
            "count by(a)(m)",
            "group by(a)(m)",
            "sum without(z)(m)",
            "count without(z)(m)",
            "abs(sum by(a)(m))",
        ),
    )
    for length in (2, 12, 13, 50, 51, 129):
        names = [f"a{bit}" for bit in range((length - 1).bit_length())]
        records = [
            series(
                {"__name__": "m", "z": str(index)}
                | {name: "" for bit, name in enumerate(names) if index & (1 << bit)},
                [(0, index + 1, "float")],
            )
            for index in range(length)
        ]
        add(
            f"duplicate-row-{length}",
            records,
            tuple(
                f"{operator} by({','.join(names)})(m)"
                for operator in ("sum", "min", "count")
            ),
        )
    return inputs


def capture(upstream):
    return cursor_calls.capture(upstream, cases(), subject="output-label")


def load_reference(reference):
    return cursor_calls.load_reference(reference, cases())


def qualify(reference, binary, page_size=1024):
    return cursor_calls.qualify(
        reference,
        binary,
        page_size,
        cases(),
        subject="output-label",
        exact_numeric=True,
        ordered=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    arguments = parser.parse_args()
    if arguments.capture:
        if not arguments.upstream:
            parser.error("capture requires --upstream")
        capture(arguments.upstream)
    else:
        if not arguments.reference or not arguments.binary:
            parser.error("comparison requires --reference and --binary")
        raise SystemExit(
            not qualify(arguments.reference, arguments.binary, arguments.page_size)[
                "bitwise_pass"
            ]
        )
