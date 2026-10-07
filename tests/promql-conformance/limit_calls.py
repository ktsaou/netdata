"""Capture and strictly compare LIMIT queries declared from source sampler controls."""

import argparse

import cursor_calls
import ratio_sampler
from limit_cases import cases


def inputs(sampler_reference):
    return cases(*ratio_sampler.load_reference(sampler_reference))


def capture(upstream, sampler_reference):
    return cursor_calls.capture(upstream, inputs(sampler_reference), subject="limits")


def load_reference(reference, sampler_reference):
    return cursor_calls.load_reference(reference, inputs(sampler_reference))


def qualify(reference, binary, sampler_reference, page_size=1024):
    return cursor_calls.qualify(
        reference,
        binary,
        page_size,
        inputs(sampler_reference),
        subject="limits",
        exact_numeric=True,
        ordered=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sampler-reference", required=True)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    arguments = parser.parse_args()
    if arguments.capture:
        if not arguments.upstream:
            parser.error("capture requires --upstream")
        capture(arguments.upstream, arguments.sampler_reference)
    else:
        if not arguments.reference or not arguments.binary:
            parser.error("comparison requires --reference and --binary")
        raise SystemExit(
            not qualify(
                arguments.reference,
                arguments.binary,
                arguments.sampler_reference,
                arguments.page_size,
            )["bitwise_pass"]
        )
