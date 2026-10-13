"""Compare typed scalar trim execution errors and arithmetic/vector controls."""

import argparse

import cursor_calls
from build import directory
from native_matrix_calls import recorded_run


def cases():
    expressions = (
        "1 >/ 2",
        "1 </ 2",
        "(1+2) >/ (3+4)",
        "(1+2) </ (3+4)",
        "1+2",
        "1 > bool 2",
        "vector(1) >/ vector(2)",
        "vector(1) </ vector(2)",
    )
    return [
        {
            "request": {
                "id": f"scalar-trim:{profile}:{mode}:{index}",
                "query": expression,
                "profile": profile,
                "mode": mode,
                "time_ms": 0,
                "start_ms": 0,
                "end_ms": 2000,
                "step_ms": 1000,
            },
            "dataset": {"schema_version": 1, "series": []},
            "faults": {},
        }
        for profile in ("server-default", "upstream-tests")
        for mode in ("instant", "range")
        for index, expression in enumerate(expressions)
    ]


def capture(upstream):
    return cursor_calls.capture(
        upstream,
        cases(),
        subject="scalar-trim",
        construction_errors=True,
        process_runner=recorded_run,
    )


def qualify(reference, binary, page_size=1024):
    return cursor_calls.qualify(
        reference,
        binary,
        page_size,
        cases(),
        subject="scalar-trim",
        exact_numeric=True,
        ordered=True,
        construction_errors=True,
        process_runner=recorded_run,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    args = parser.parse_args()
    if args.capture:
        if not args.upstream:
            parser.error("capture requires --upstream")
        capture(directory(args.upstream))
    else:
        if not args.reference or not args.binary:
            parser.error("comparison requires reference and binary")
        raise SystemExit(
            not qualify(args.reference, args.binary, args.page_size)["bitwise_pass"]
        )
