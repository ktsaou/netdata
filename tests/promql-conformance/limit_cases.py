"""Independent LIMIT declarations; the original engine supplies every outcome."""

import math
import re
import struct
from copy import deepcopy

import cursor_calls
import ratio_sampler
from count_values_cases import (
    FIRST,
    SECOND,
    STALE,
    WINDOWS,
    native,
    native_series,
    span,
)
from info_cases import series
from topk_cases import cases as selection_cases


def cases(sampler_inputs, sampler_outputs):
    if sampler_inputs != ratio_sampler.cases():
        raise ValueError("sampler declarations changed")
    ratio_sampler.checked_original(
        cursor_calls.PROFILE | {"cases": sampler_outputs}, sampler_inputs
    )
    inputs = []

    def add(ident, records, expressions, windows=WINDOWS, faults=None):
        for qi, expression in enumerate(expressions):
            for wi, (mode, time, start, end, step) in enumerate(windows):
                inputs.append(
                    {
                        "request": {
                            "id": f"limits:{ident}:{qi}:{wi}",
                            "query": expression,
                            "profile": "upstream-tests",
                            "mode": mode,
                            "time_ms": time,
                            "start_ms": start,
                            "end_ms": end,
                            "step_ms": step,
                            "lookback_ms": 1,
                            "subquery_step_ms": 1000,
                        },
                        "dataset": {"schema_version": 1, "series": deepcopy(records)},
                        "faults": deepcopy(faults or {}),
                    }
                )

    for source in selection_cases():
        ident = source["request"]["id"].split(":")[1]
        request = source["request"]
        if request["profile"] != "upstream-tests" or (
            "topk" not in ident
            and not (
                ident == "group-hash-alias" and request["query"].startswith("topk ")
            )
        ):
            continue
        # These body maps are qualified at singleton scope before multistep policy injection.
        if (
            ident.startswith("body-producers")
            and request["mode"] == "range"
            and request["start_ms"] != request["end_ms"]
        ):
            continue
        if (
            ident.startswith("parents")
            and request["id"].split(":")[-2] == "4"
            and request["mode"] == "range"
            and request["start_ms"] != request["end_ms"]
        ):
            continue
        if (
            ident.startswith("parameter-clock")
            and "scalar(topk(" in request["query"]
            and request["mode"] == "range"
            and request["start_ms"] != request["end_ms"]
        ):
            continue
        for operator in ("limitk", "limit_ratio"):
            row = deepcopy(source)
            row["request"]["id"] = "limits:" + operator + ":" + source["request"]["id"]
            row["request"]["query"] = re.sub(r"\btopk\b", operator, request["query"])
            inputs.append(row)
    ordinary = [
        series(
            {"__name__": "m", "job": str(index % 2), "z": str(index)},
            [
                (0, value, "float"),
                (1000, 6 - index, "float"),
                (2000, STALE if index == 0 else index % 3, "float"),
            ],
        )
        for index, value in enumerate((0.0, -0.0, 2, 2, -1, 6, 1))
    ]
    add(
        "ratio-parameters",
        ordinary,
        tuple(
            f"limit_ratio({ratio},m)"
            for ratio in (
                "-Inf",
                "-1.1",
                "-1",
                "-0.75",
                "-0.5",
                "-0.25",
                "-0",
                "0",
                "0.25",
                "0.5",
                "0.75",
                "1",
                "1.1",
                "Inf",
                "NaN",
            )
        ),
    )
    for ident, values in [
        ("sign", (1.1, -1.1, 0)),
        ("zero", (0, -0.0, 0)),
        ("tiny", (-5e-324, 5e-324, 0)),
        ("nan", (1, "7ff8000000000063", 0)),
    ]:
        records = ordinary + [
            series(
                {"__name__": "p"},
                [(t, v, "float") for t, v in zip((0, 1000, 2000), values, strict=True)],
            )
        ]
        add(
            "ratio-moving-" + ident,
            records,
            (
                "limit_ratio(scalar(p),m)",
                "limit_ratio(scalar(p),m @0)",
                "count(limit_ratio(scalar(p),m))",
            ),
        )
    specials = [
        "8000000000000000",
        "0000000000000000",
        "7ff8000000000063",
        "fff8000000000027",
        "7ff0000000000044",
        "7ff0000000000000",
        "fff0000000000000",
        "3ff0000000000000",
        "bff0000000000000",
    ]
    for length in (1, 2, 12, 13, 50, 51, 1024, 1025):
        for pattern in ("ascending", "descending", "special", "native"):
            records = []
            for i in range(length):
                kind = (
                    "float_histogram" if pattern == "native" and i % 3 == 0 else "float"
                )
                value = (
                    i
                    if pattern == "ascending"
                    else length - i
                    if pattern == "descending"
                    else specials[i % len(specials)]
                )
                records.append(
                    series({"__name__": "m", "z": f"{i:05d}"}, [(0, value, kind)])
                )
            ks = sorted({1, 2, max(1, length // 2), length, length + 1})
            add(
                f"heap-{length}-{pattern}",
                records,
                tuple(f"limitk({k},m)" for k in ks)
                + tuple(
                    f"limit_ratio({r},m)" for r in ("0.25", "0.5", "-0.5", "1", "-1")
                ),
                windows=WINDOWS[:1],
            )
    for request, outcome in zip(sampler_inputs, sampler_outputs, strict=True):
        if request["labels"] is None or not request["labels"]:
            continue
        ident = request["id"].replace(":", "-")
        records = [
            {
                "labels": deepcopy(request["labels"]),
                "points": [
                    {"time_ms": t, "kind": kind, kind: payload}
                    for t, kind, payload in (
                        (0, "float", "3ff0000000000000"),
                        (1000, "float_histogram", native()),
                        (2000, "float", "8000000000000000"),
                    )
                ],
            }
        ]
        names = {label["name"] for label in request["labels"]}
        selector = (
            '{"__name__"=~".+"}'
            if "X19uYW1lX18=" in names
            else '{a=~".+"}'
            if "YQ==" in names
            else '{z=~".+"}'
        )
        expressions = []
        for threshold in outcome["thresholds"][9:]:
            number = struct.unpack(">d", bytes.fromhex(threshold["ratio"]))[0]
            expressions.append(f"limit_ratio({number!r},{selector})")
        beyond = struct.unpack(">d", bytes.fromhex(outcome["offset"]))[0] - 1
        for _ in range(16):
            beyond = math.nextafter(beyond, math.inf)
        if not (
            beyond < 0
            and 1 + beyond > struct.unpack(">d", bytes.fromhex(outcome["offset"]))[0]
        ):
            raise ValueError(
                "negative source control does not pass the rounding plateau"
            )
        expressions.append(f"limit_ratio({beyond!r},{selector})")
        add("exact-offset-" + ident, records, tuple(expressions))
    timelines = {
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
    for ident, kinds in timelines.items():
        records = [
            series(
                {"a": label},
                [
                    (time, index + 1, kind)
                    for time, kind in zip((0, 1000, 2000), row_kinds, strict=True)
                ],
            )
            for index, (label, row_kinds) in enumerate(
                zip((FIRST, SECOND), kinds, strict=True)
            )
        ]
        for operator, param in (("limitk", "2"), ("limit_ratio", "1.1")):
            body = f'{operator}({param},{{a=~".+"}})'
            add(
                "native-alias-" + ident + "-" + operator,
                records,
                (
                    body,
                    f"(({body}))",
                    f"count({body})",
                    f"timestamp({body})",
                    f"histogram_count({body})",
                    f"abs({body})",
                    f"(-{body})",
                    f"limitk(1,{body})",
                ),
            )
    for operator in ("limitk", "limit_ratio"):
        param = "2" if operator == "limitk" else "1.1"
        for kind in ("histogram", "float_histogram"):
            records = [
                series(
                    {"__name__": name, "tag": "x"},
                    [(0, value, kind), (1000, value, "float"), (2000, value, kind)],
                )
                for name, value in (("a", 1), ("b", 2))
            ]
            body = f'{operator}({param},{{tag="x"}})'
            add(
                "native-cleanup-" + operator + "-" + kind,
                records,
                (
                    body,
                    f"(-{body})",
                    f"(({body}*1))",
                    f"histogram_count({body})",
                    f"timestamp({body})",
                    f"count({body})",
                ),
            )
        records = [
            native_series(
                native(
                    schema=-53,
                    custom_values=[struct.pack(">d", x).hex() for x in (-1, 1)],
                    positive_spans=[span(0, 3)],
                    positive_buckets=[struct.pack(">d", x).hex() for x in (1, 2, 3)],
                ),
                labels={"__name__": "m", "z": "a"},
            ),
            native_series(
                native(
                    schema=-53,
                    custom_values=[struct.pack(">d", x).hex() for x in (0, 2)],
                    positive_spans=[span(0, 3)],
                    positive_buckets=[struct.pack(">d", x).hex() for x in (4, 5, 6)],
                ),
                labels={"__name__": "m", "z": "b"},
            ),
        ]
        add(
            "native-layouts-" + operator,
            records,
            (f"{operator}({param},m)", f"count({operator}({param},m))"),
        )
    if len({r["request"]["id"] for r in inputs}) != len(inputs):
        raise ValueError("duplicate LIMIT declaration")
    return inputs
