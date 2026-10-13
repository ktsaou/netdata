#!/usr/bin/env python3
"""Qualify ordinary matrix-reader operations against complete source-bound original queries."""

import argparse

import cursor_calls
from matrix_reader_cases import cases


def capture(upstream, roster="ordinary"):
    return cursor_calls.capture(
        upstream, cases(roster), subject="matrix-reader", construction_errors=True
    )


def load_reference(reference, roster="ordinary"):
    return cursor_calls.load_reference(
        reference, cases(roster), construction_errors=True
    )


def qualify(reference, binary, page_size=1024, roster="ordinary"):
    return cursor_calls.qualify(
        reference,
        binary,
        page_size,
        cases(roster),
        subject="matrix-reader",
        exact_numeric=True,
        ordered=True,
        construction_errors=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument(
        "--roster",
        choices=("ordinary", "clipped", "timestamp", "zero"),
        default="ordinary",
    )
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    arguments = parser.parse_args()
    if arguments.capture:
        if not arguments.upstream:
            parser.error("capture requires --upstream")
        capture(arguments.upstream, arguments.roster)
    else:
        if not arguments.reference or not arguments.binary:
            parser.error("comparison requires reference and binary")
        raise SystemExit(
            not qualify(
                arguments.reference,
                arguments.binary,
                arguments.page_size,
                arguments.roster,
            )["bitwise_pass"]
        )
