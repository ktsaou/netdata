"""Capture and compare default-profile stable execution and optional rejections."""

import argparse

import cursor_calls
from build import directory
from default_profile_cases import cases
from native_matrix_calls import recorded_run


def capture(builtin_capture, upstream):
    return cursor_calls.capture(
        upstream,
        cases(builtin_capture, upstream),
        subject="default-profile",
        construction_errors=True,
        process_runner=recorded_run,
    )


def qualify(builtin_capture, upstream, reference, binary, page_size=1024):
    return cursor_calls.qualify(
        reference,
        binary,
        page_size,
        cases(builtin_capture, upstream),
        subject="default-profile",
        exact_numeric=True,
        ordered=True,
        construction_errors=True,
        process_runner=recorded_run,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--builtin-capture", required=True)
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    args = parser.parse_args()
    upstream = directory(args.upstream)
    builtin = directory(args.builtin_capture)
    if args.capture:
        capture(builtin, upstream)
    else:
        if not args.reference or not args.binary:
            parser.error("comparison requires reference and binary")
        raise SystemExit(
            not qualify(builtin, upstream, args.reference, args.binary, args.page_size)[
                "bitwise_pass"
            ]
        )
