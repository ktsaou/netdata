"""Complete scheduled-label outcomes, clocks, omission and ordinary fallback controls."""

from info_cases import series


def _case(ident, expression, records, profile, *, count=8, first=0, step=500):
    return {
        "request": {
            "id": ident,
            "query": expression,
            "profile": profile,
            "mode": "range",
            "time_ms": first,
            "start_ms": first,
            "end_ms": first + (count - 1) * step,
            "step_ms": step,
            "lookback_ms": 750,
            "subquery_step_ms": 1000,
        },
        "dataset": {"schema_version": 1, "series": records},
        "faults": {},
    }


def _labels(name, instance):
    return {
        "__name__": name,
        "instance": instance,
        "job": "worker",
        "payload": b"value\x00\xff",
    }


def cases():
    stale = "7ff0000000000002"
    dense = [
        series(
            _labels("m", "b"),
            [
                (0, 3, "float"),
                (1000, -2, "float"),
                (2000, 5, "float"),
                (3000, 1, "float"),
            ],
        ),
        series(
            _labels("m", "a"),
            [
                (0, "8000000000000000", "float"),
                (1000, "7ff8000000000031", "float"),
                (2000, 2, "float"),
                (3000, -4, "float"),
            ],
        ),
    ]
    mixed = [
        series(
            _labels("m", "d"),
            [(0, 4, "float"), (2000, -3, "float"), (3000, 7, "float")],
        ),
        series(
            _labels("m", "b"),
            [
                (0, 2, "float_histogram"),
                (1000, 3, "histogram"),
                (2000, 5, "float"),
                (3000, stale, "float"),
            ],
        ),
        series(
            _labels("m", "a"),
            [
                (0, 1, "float_histogram"),
                (1000, 2, "float_histogram"),
                (3000, 3, "float_histogram"),
            ],
        ),
        series(
            _labels("m", "c"),
            [
                (0, stale, "float"),
                (1000, -1, "float"),
                (2000, stale, "float"),
                (3000, 2, "float"),
            ],
        ),
    ]
    common = [
        "m",
        "m+0",
        "0+m",
        "2-m",
        "m*2",
        "m/2",
        "m>0",
        "m >= bool 0",
        "-(m+0)",
        "abs(m)",
        "round(abs(m),.5)+0",
        "clamp(m,-2,3)",
        "m+time()",
        "timestamp(m)",
        "timestamp(m+0)",
        "timestamp(m offset 1s)",
    ]
    inputs = []
    for profile in ["server-default", "upstream-tests"]:
        for shape, records in [("dense", dense), ("mixed", mixed)]:
            for index, expression in enumerate(common):
                inputs.append(
                    _case(
                        f"scheduled-label-{shape}-{profile}-{index}",
                        expression,
                        records,
                        profile,
                    )
                )
        for index, expression in enumerate(
            [
                "sum_over_time((m+0)[2s:1s])",
                "max_over_time(abs(m)[2s:1s])",
                "sum by(job)(m)",
                "count(m+0)",
                "sort(m+0)",
            ]
        ):
            inputs.append(
                _case(
                    f"scheduled-label-parent-{profile}-{index}",
                    expression,
                    dense,
                    profile,
                )
            )
        for index, expression in enumerate(
            ["histogram_count(m)", "histogram_sum(m)", "histogram_avg(m)"]
        ):
            inputs.append(
                _case(
                    f"scheduled-label-native-{profile}-{index}",
                    expression,
                    mixed,
                    profile,
                )
            )
        for index, expression in enumerate(
            [
                'm{missing="yes"}+0',
                'abs(m{missing="yes"})',
                'timestamp(m{missing="yes"})',
                'sum by(job)(m{missing="yes"})',
            ]
        ):
            inputs.append(
                _case(
                    f"scheduled-label-empty-{profile}-{index}",
                    expression,
                    dense,
                    profile,
                )
            )
        for overlapping in [False, True]:
            records = [
                series(
                    _labels("m", "same"),
                    [
                        (0, 1, "float"),
                        (1000, 2 if overlapping else stale, "float"),
                        (2000, 3 if overlapping else stale, "float"),
                    ],
                ),
                series(
                    _labels("n", "same"),
                    [
                        (0, 3 if overlapping else stale, "float"),
                        (1000, 4, "float"),
                        (2000, 5, "float"),
                    ],
                ),
            ]
            for index, expression in enumerate(
                [
                    '{__name__=~"m|n"}+0',
                    'abs({__name__=~"m|n"})',
                    'timestamp({__name__=~"m|n"})',
                ]
            ):
                inputs.append(
                    _case(
                        f"scheduled-label-collision-{overlapping}-{profile}-{index}",
                        expression,
                        records,
                        profile,
                    )
                )
    for size in [16, 32768]:
        records = [
            series(
                {
                    "__name__": "m",
                    "instance": "0",
                    "job": "worker",
                    "payload": "Q" * size,
                },
                [((index + 1) * 1000, index + 1, "float") for index in range(64)],
            )
        ]
        for shape, expression in [("raw", "m"), ("map", "m+0")]:
            case = _case(
                f"payload-{size}-ticks64-{shape}",
                expression,
                records,
                "server-default",
                count=64,
                first=1000,
                step=1000,
            )
            case["request"]["lookback_ms"] = 1000
            inputs.append(case)
    return inputs
