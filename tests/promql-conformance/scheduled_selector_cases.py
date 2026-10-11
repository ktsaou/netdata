"""Original expressions for single-selector clocks, projections and retained replay."""

from copy import deepcopy

from info_cases import series


def cases():
    output = []

    def add(ident, records, bodies, lookback=1500):
        for profile in ("server-default", "upstream-tests"):
            for index, body in enumerate(bodies):
                for route, query, mode, start in (
                    ("instant", body, "instant", 4000),
                    ("range", body, "range", 1000),
                    ("matrix", f"({body})[4s:1s]", "instant", 4000),
                    ("fold", f"count_over_time(({body})[4s:1s])", "instant", 4000),
                ):
                    output.append(
                        {
                            "request": {
                                "id": f"scheduled:{ident}:{profile}:{index}:{route}",
                                "query": query,
                                "profile": profile,
                                "mode": mode,
                                "time_ms": 4000,
                                "start_ms": start,
                                "end_ms": 4000,
                                "step_ms": 1000,
                                "lookback_ms": lookback,
                                "subquery_step_ms": 1000,
                            },
                            "dataset": {
                                "schema_version": 1,
                                "series": deepcopy(records),
                            },
                            "faults": {},
                        }
                    )

    clock = series(
        {"__name__": "m", "instance": "one"},
        [(1000, 2.0, "float"), (3000, 2.0, "float")],
    )
    for point, start in zip(clock["points"], (100, 2500), strict=True):
        point["start_time_ms"] = start
    add(
        "clocks",
        [clock],
        (
            "timestamp(m)",
            "start_timestamp(m)",
            "start_timestamp(m offset 500ms)",
            "time()*(m@1)",
            "(m@1)*time()",
            "3>((m@1)*time())",
            "3>bool((m@1)*time())",
            "timestamp(abs(m))",
            "timestamp(start_timestamp(m@1))",
        ),
    )
    add(
        "projection",
        [
            series(
                {"__name__": "m", "instance": "1"},
                [(i * 1000, -3.0 * i, "float") for i in range(1, 5)],
            ),
            series(
                {"__name__": "m", "instance": "0"},
                [(i * 1000, -2.0 * i, "float") for i in range(1, 5)],
            ),
        ],
        (
            'label_join(round(abs(m),0.5),"row","-","instance")',
            'label_replace(round(abs(m),0.5),"row","$1","instance","(.+)")',
            'label_join(abs(m),"__name__","-","instance")*time()',
            "round(clamp(abs(m),2,time()*4),0.5)",
        ),
    )
    add(
        "mixed",
        [
            series(
                {"__name__": "m", "instance": "one"},
                [
                    (1000, 1.0, "float"),
                    (2000, 7.0, "float_histogram"),
                    (3000, 2.0, "float"),
                ],
            )
        ],
        (
            "m == bool 1",
            "abs(m)",
            "histogram_count(m*2)",
            "histogram_sum(m/2)",
            "histogram_avg(m)",
            "histogram_stdvar(m)",
            "histogram_stddev(m)",
        ),
        lookback=1,
    )
    add(
        "disjoint-projection",
        [
            series({"__name__": "a", "instance": "one"}, [(1000, -0.0, "float")]),
            series({"__name__": "b", "instance": "one"}, [(3000, 2.0, "float")]),
        ],
        (
            '-{__name__=~"a|b"}',
            'abs({__name__=~"a|b"})',
            'label_replace({__name__=~"a|b"},"__name__","same","__name__",".*")*1',
        ),
        lookback=1,
    )
    selector = '{__name__=~"a|b"}'
    duplicate = [
        series(
            {"__name__": name, "instance": "one"},
            [(1000, 1.0, "float"), (3000, 2.0, "float")],
        )
        for name in ("a", "b")
    ]
    add(
        "duplicates",
        duplicate,
        (f"abs({selector})>3", f"-({selector}*1)", f"round({selector},0.5)"),
    )
    return output
