"""Capture selection reducers through the unchanged source-bound query helper."""
import argparse

import cursor_calls
from topk_cases import cases


def capture(upstream):
    return cursor_calls.capture(upstream, cases(), subject="topk")


def load_reference(reference):
    return cursor_calls.load_reference(reference, cases())


def qualify(reference, binary, page_size=1024):
    return cursor_calls.qualify(reference, binary, page_size, cases(), subject="topk",
                                exact_numeric=True, ordered=True)


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
        raise SystemExit(not qualify(arguments.reference, arguments.binary,
                                     arguments.page_size)["bitwise_pass"])
