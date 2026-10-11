#!/usr/bin/env python3
"""Qualify page crossing, derived backing and nested stores against unchanged Go."""

import argparse
import json
from pathlib import Path

import cursor_calls
from build import directory
from native_matrix_calls import recorded_run
from temporary_quantile_cases import cases


def capture(upstream):
    return cursor_calls.capture(
        upstream,
        cases(),
        subject="temporary-quantile",
        construction_errors=True,
        process_runner=recorded_run,
    )


def qualify(reference, binary, page_size=1024):
    binary = Path(binary).resolve(strict=True)
    before = cursor_calls.sha(binary)
    report = cursor_calls.qualify(
        reference,
        binary,
        page_size,
        cases(),
        subject="temporary-quantile",
        exact_numeric=True,
        construction_errors=True,
        process_runner=recorded_run,
    )
    if cursor_calls.sha(binary) != before:
        raise ValueError("temporary quantile candidate changed during execution")
    print(
        json.dumps(
            {key: report[key] for key in ("semantic_pass", "bitwise_pass")}
            | {"cases": report["cases"]}
        ),
        flush=True,
    )
    return report


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
        capture(directory(arguments.upstream))
    else:
        if not arguments.reference or not arguments.binary:
            parser.error("comparison requires reference and binary")
        raise SystemExit(
            not qualify(arguments.reference, arguments.binary, arguments.page_size)[
                "bitwise_pass"
            ]
        )
