"""Independent aggregate QUANTILE declarations; original queries supply outcomes."""

from copy import deepcopy

from count_values_cases import FIRST, SECOND, STALE, WINDOWS, native, native_series
from info_cases import series


def cases():
    inputs = []

    def add(
        ident,
        records,
        queries,
        windows=WINDOWS,
        faults=None,
        profiles=("server-default", "upstream-tests"),
    ):
        for profile in profiles:
            for qi, query in enumerate(queries):
                for wi, (mode, time, start, end, step) in enumerate(windows):
                    inputs.append(
                        {
                            "request": {
                                "id": f"aggregate-quantile:{ident}:{profile}:{qi}:{wi}",
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
                            "faults": deepcopy(faults or {}),
                        }
                    )

    ordinary = [
        series(
            {"__name__": "m", "job": "one" if index % 2 else "two", "z": str(index)},
            [
                (0, value, "float"),
                (1000, 6 - index, "float"),
                (2000, STALE if index == 0 else index % 3, "float"),
            ],
        )
        for index, value in enumerate((0.0, -0.0, 2, 2, -1, 6, 1))
    ]
    parameters = (
        "-Inf",
        "-1",
        "-0.1",
        "-0",
        "0",
        "0.125",
        "0.5",
        "0.8",
        "1",
        "1.1",
        "Inf",
        "NaN",
    )
    add("parameters", ordinary, tuple(f"quantile({q},m)" for q in parameters))
    add("empty", [], tuple(f"quantile({q},m)" for q in parameters))
    add(
        "stale",
        [series({"__name__": "m"}, [(0, STALE, "float")])],
        ("quantile(0.5,m)", "quantile(NaN,m)", "quantile(Inf,m)"),
    )
    add(
        "groups",
        ordinary,
        (
            "quantile by(job)(0.5,m)",
            "quantile without(z)(0.5,m)",
            "quantile by(job,job)(0.5,m)",
            "quantile without()(0.5,m)",
            "quantile by(__name__)(0.5,m)",
            "quantile by()(0.5,m)",
        ),
    )
    add(
        "parents",
        ordinary,
        (
            "sum(quantile by(job)(0.5,m))",
            "count(quantile by(job)(0.5,m))",
            "abs(quantile by(job)(0.5,m))",
            "timestamp(quantile by(job)(0.5,m))",
            "quantile(0.5,quantile by(job)(0.8,m))",
            "topk(1,quantile by(job)(0.5,m))",
            'label_replace(quantile by(job)(0.5,m),"new","$1","job","(.*)")',
        ),
    )
    add(
        "body-producers",
        ordinary,
        (
            "quantile(0.5,rate(m[2s]))",
            "quantile(0.5,avg by(job)(m))",
            "quantile(0.5,sum by(z)(m))",
            'quantile(0.5,count_values("v",m))',
            "quantile(0.5,delta(m[2s]))",
            "quantile(0.5,abs(m))",
        ),
    )

    specials = (
        "8000000000000000",
        "0000000000000000",
        "7ff8000000000063",
        "fff8000000000027",
        "7ff0000000000044",
        "fff0000000000045",
        "7ff0000000000000",
        "fff0000000000000",
        "3ff0000000000000",
        "bff0000000000000",
        "0000000000000001",
        "7fefffffffffffff",
    )
    finite_parameters = ("-0", "0", "0.125", "0.5", "0.8", "1")
    for index, value in enumerate(specials):
        add(
            f"singleton-{index}",
            [series({"__name__": "m"}, [(0, value, "float")])],
            tuple(f"quantile({q},m)" for q in parameters),
            windows=WINDOWS[:1],
        )
    pairs = (
        (specials[0], specials[1]),
        (specials[1], specials[0]),
        (specials[2], specials[3]),
        (specials[4], specials[5]),
        (specials[6], specials[6]),
        (specials[6], specials[7]),
        (specials[7], specials[6]),
        (specials[7], specials[7]),
        (specials[8], specials[9]),
        (specials[10], specials[10]),
        (specials[11], specials[11]),
    )
    for index, values in enumerate(pairs):
        records = [
            series({"__name__": "m", "z": str(i)}, [(0, value, "float")])
            for i, value in enumerate(values)
        ]
        add(
            f"pair-{index}",
            records,
            tuple(f"quantile({q},m)" for q in finite_parameters)
            + tuple(f"1/quantile({q},m)" for q in finite_parameters)
            + tuple(f"quantile({q},m)>bool 0" for q in finite_parameters),
            windows=WINDOWS[:1],
        )
    for length in (1, 2, 3, 12, 13, 49, 50, 51, 127, 129, 1024, 1025):
        for pattern in ("ascending", "descending", "equal", "zeros", "nan", "special"):
            values = {
                "ascending": list(range(length)),
                "descending": list(range(length))[::-1],
                "equal": [1] * length,
                "zeros": [specials[i % 2] for i in range(length)],
                "nan": [specials[2 + i % 4] for i in range(length)],
                "special": [specials[i % len(specials)] for i in range(length)],
            }[pattern]
            records = [
                series({"__name__": "m", "z": f"{i:05d}"}, [(0, value, "float")])
                for i, value in enumerate(values)
            ]
            add(
                f"threshold-{length}-{pattern}",
                records,
                tuple(f"quantile({q},m)" for q in finite_parameters),
                windows=WINDOWS[:1],
            )

    for ident, values in (
        ("moving", (0, 0.5, 1)),
        ("outside", (-0.5, 2, 0.8)),
        ("nan", ("7ff8000000000063", 2, -0.5)),
        ("stale", (0.5, STALE, 0.8)),
    ):
        clock = series(
            {"__name__": "p"},
            [
                (time, value, "float")
                for time, value in zip((0, 1000, 2000), values, strict=True)
            ],
        )
        add(
            "sequence-" + ident,
            ordinary + [clock],
            (
                "quantile(scalar(p),m)",
                "quantile(scalar(p),m @0)",
                "count(quantile(scalar(p),m))",
            ),
        )
        add("sequence-empty-" + ident, [clock], ("quantile(scalar(p),m)",))

    clock = series(
        {"__name__": "p"},
        [
            (time, value, "float")
            for time, value in (
                (-2000, 0.1),
                (-1000, 0.25),
                (0, 0.5),
                (1000, 0.75),
                (2000, 1),
            )
        ],
    )
    add(
        "clocks",
        ordinary + [clock],
        (
            "quantile(time()/2,m @0)",
            "quantile(time()/2,m @end())",
            "quantile(0.5,m offset 1s)",
            "(quantile((2),m))",
            "quantile(scalar(p @end()),m)",
            "quantile(scalar(timestamp(p @0))/2,m)",
            "quantile(scalar(timestamp((p @0)))/2,m)",
            "quantile(scalar(delta(p[2s] @0))/2,m)",
            "quantile(scalar(rate(p[2s] @0))/2,m)",
            "quantile(scalar(quantile(time()/2,p @0)),m)",
            "quantile(scalar(quantile(time()/2,p @0)),m @0)",
        ),
        windows=WINDOWS + (("range", 0, 1000, 2000, 1000),),
    )
    add(
        "context-parameter",
        ordinary,
        tuple(
            f"quantile({function}()/2,m)"
            for function in ("start", "end", "range", "step")
        ),
        profiles=("upstream-tests",),
    )

    for kind in ("histogram", "float_histogram"):
        changing = series(
            {"__name__": "m", "job": "native", "z": "native"},
            [(0, 10, kind), (1000, 20, "float"), (2000, 30, kind)],
        )
        for ident, records in (("mixed", ordinary + [changing]), ("only", [changing])):
            add(
                "native-" + kind + "-" + ident,
                records,
                (
                    "quantile(0.5,m)",
                    "quantile by(job)(0.5,m)",
                    "quantile(0,m)",
                    "quantile(-Inf,m)",
                    "quantile(Inf,m)",
                    "quantile(NaN,m)",
                    "quantile(0.5,(m))",
                    "count(quantile(0.5,m))",
                ),
            )
        add(
            "native-raw-" + kind,
            [native_series(native(kind), kind)],
            (
                "quantile(0.5,m)",
                "quantile(NaN,m)",
                "quantile(Inf,m)",
                "(quantile((2),(m)))",
            ),
            windows=WINDOWS[:1],
        )

    for first_kind, second_kind in (
        ("float", "float_histogram"),
        ("float_histogram", "float"),
    ):
        records = [
            series(
                {"__name__": "m", "job": "group", "z": "a"},
                [(0, 1, first_kind), (1000, 2, second_kind), (2000, 3, first_kind)],
            ),
            series(
                {"__name__": "m", "job": "group", "z": "b"},
                [(0, 4, second_kind), (1000, 5, first_kind), (2000, 6, second_kind)],
            ),
            series(
                {"__name__": "n", "job": "other", "z": "a"},
                [(0, 7, "float"), (1000, 8, "float"), (2000, 9, "float")],
            ),
        ]
        for order, body in enumerate(
            ('m{z="a"} or (m{z="b"}*1)', '(m{z="b"}*1) or m{z="a"}')
        ):
            add(
                f"native-flags-{first_kind}-{second_kind}-{order}",
                records,
                (
                    f"quantile by(__name__)(0.5,{body})",
                    f"quantile without(z)(0.5,{body})",
                    f"quantile by(__name__)(0.5,({body}) or n)",
                    f"abs(quantile by(__name__)(0.5,{body}))",
                    f"timestamp(quantile by(__name__)(0.5,{body}))",
                ),
            )

    for q in ("0.5", "NaN", "Inf", "scalar(p)"):
        for name in ("p", "m"):
            add(
                f"selection-fault-{q}-{name}",
                ordinary + [clock],
                (f"quantile({q},m)",),
                faults={name: {"set_error": "selection fault"}},
            )
            add(
                f"iterator-fault-{q}-{name}",
                ordinary + [clock],
                (f"quantile({q},m)",),
                faults={name: {"next_times": [1000], "message": "iterator fault"}},
            )

    collision = [
        series({"__name__": "m", "a": b"b\xffx\xffy", "z": "a"}, [(0, 1, "float")]),
        series({"__name__": "m", "a": b"b", "x": "y", "z": "b"}, [(0, 2, "float")]),
    ]
    add(
        "group-hash-alias",
        collision,
        (
            "quantile by(a,x)(0.5,m)",
            "quantile without(z)(0.5,m)",
            "topk(1,quantile by(a,x)(0.5,m))",
        ),
    )
    empty_labels = [
        series({"__name__": "m", "a": "", "z": "a"}, [(0, 1, "float")]),
        series({"__name__": "m", "z": "b"}, [(0, 2, "float")]),
    ]
    add(
        "empty-group-label",
        empty_labels,
        (
            "quantile by(a)(0.5,m)",
            "quantile without(z)(0.5,m)",
            "quantile by(a,a)(0.5,m)",
        ),
    )

    timelines = (
        (
            ((0, 1, "float"), (1000, 2, "float_histogram"), (2000, 3, "float")),
            (
                (0, 4, "float_histogram"),
                (1000, 5, "float"),
                (2000, 6, "float_histogram"),
            ),
        ),
        (
            ((0, 1, "float"), (1000, 2, "float"), (2000, 3, "float_histogram")),
            ((0, 4, "float"), (1000, 5, "float_histogram"), (2000, 6, "float")),
        ),
        (
            ((0, 1, "float"), (1000, 2, "float"), (2000, 3, "float_histogram")),
            ((0, 4, "float_histogram"), (1000, 5, "float"), (2000, 6, "float")),
        ),
        (
            (
                (0, 1, "float_histogram"),
                (1000, 2, "float"),
                (2000, 3, "float_histogram"),
            ),
            (
                (0, 4, "float_histogram"),
                (1000, 5, "float_histogram"),
                (2000, 6, "float"),
            ),
        ),
        (
            (
                (0, 1, "float_histogram"),
                (1000, 2, "float_histogram"),
                (2000, 3, "float"),
            ),
            (
                (0, 4, "float"),
                (1000, 5, "float_histogram"),
                (2000, 6, "float_histogram"),
            ),
        ),
    )
    for ti, (first, second) in enumerate(timelines):
        records = [series({"a": FIRST}, first), series({"a": SECOND}, second)]
        for operator, parameter in (
            ("topk", "2"),
            ("limitk", "2"),
            ("limit_ratio", "-1"),
        ):
            body = f'{operator}({parameter},{{a=~".+"}})'
            inner = f"quantile(0.5,{body})"
            add(
                f"mixed-heads-{ti}-{operator}",
                records,
                (
                    inner,
                    f"({inner})",
                    f"count({inner})",
                    f"timestamp({inner})",
                    f"abs({inner})",
                    f"-{inner}",
                    f"topk(1,{inner})",
                ),
                profiles=("upstream-tests",),
            )

    if len(inputs) != len({row["request"]["id"] for row in inputs}):
        raise ValueError("duplicate independent QUANTILE query identity")
    return inputs
