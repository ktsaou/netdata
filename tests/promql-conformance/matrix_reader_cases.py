"""Source-derived ordinary buffered-reader controls; pinned Go alone supplies outcomes."""

from copy import deepcopy

from info_cases import series

STALE = "7ff0000000000002"
FUNCTIONS = {
    **{
        name: name + "(m[RANGE])"
        for name in (
            "sum_over_time",
            "avg_over_time",
            "count_over_time",
            "min_over_time",
            "max_over_time",
            "stddev_over_time",
            "stdvar_over_time",
            "present_over_time",
            "absent_over_time",
            "first_over_time",
            "last_over_time",
            "changes",
            "resets",
            "rate",
            "increase",
            "delta",
            "irate",
            "idelta",
            "deriv",
            "mad_over_time",
        )
    },
    "predict_linear": "predict_linear(m[RANGE],2)",
    "double_exponential_smoothing": "double_exponential_smoothing(m[RANGE],0.4,0.6)",
}


def ordinary_cases():
    inputs = []
    labels = {"__name__": "m", "z": "one"}
    anchor = series(
        {"__name__": "anchor"},
        [(0, 0, "float"), (1000, 0, "float"), (2000, 0, "float"), (3000, 0, "float")],
    )

    def add(
        ident,
        expression,
        points,
        fault=None,
        *,
        mode="instant",
        time=1000,
        start=1000,
        end=2000,
        step=500,
        anchors=False,
        more=(),
    ):
        for profile in ("server-default", "upstream-tests"):
            inputs.append(
                {
                    "request": {
                        "id": f"matrix-reader:{ident}:{profile}",
                        "query": expression,
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
                        "series": [
                            series(labels, points),
                            *([deepcopy(anchor)] if anchors else []),
                            *deepcopy(more),
                        ],
                    },
                    "faults": {} if fault is None else {"m": fault},
                }
            )

    def points(times, kind="float"):
        return [(t, float(i + 1), kind) for i, t in enumerate(times)]

    def fault(message, **fields):
        return {"message": message, **fields}

    for name, template in FUNCTIONS.items():
        expr = template.replace("RANGE", "500ms")
        add(name + ":endpoint", expr, points([600, 1000]))
        add(
            name + ":endpoint-eof",
            expr,
            points([600, 1000]),
            fault("unexpected-endpoint-eof", next_eof=True),
        )
        add(
            name + ":no-endpoint-eof",
            expr,
            points([600, 900]),
            fault("required-nonendpoint-eof", next_eof=True),
        )
        add(name + ":empty-eof", expr, [], fault("required-empty-eof", next_eof=True))
        add(
            name + ":current-seek",
            expr,
            points([600, 1000]),
            fault("required-current-seek", seek_targets=[500]),
        )
        add(
            name + ":initial-prefix",
            expr + " + scalar(anchor @0)",
            points([100, 600, 1000]),
            fault("required-initial-prefix", next_times=[100]),
            anchors=True,
        )
        add(
            name + ":skip-middle",
            expr + " + scalar(anchor @0)",
            points([100, 200, 600, 1000]),
            fault("unexpected-skipped-middle", next_times=[200]),
            anchors=True,
        )
        add(
            name + ":future-next",
            expr + " + scalar(anchor @3)",
            points([600, 1200]),
            fault("required-future-next", next_times=[1200]),
            anchors=True,
        )
        for ranged in ("500ms", "1s"):
            rexpr = template.replace("RANGE", ranged)
            add(
                name + ":omitted-seek-" + ranged,
                rexpr,
                points([600, 1000, 2000]),
                fault("unexpected-omitted-seek", seek_targets=[1000, 1500]),
                mode="range",
            )
            add(
                name + ":range-endpoint-eof-" + ranged,
                rexpr,
                points([600, 1000, 1500, 2000]),
                fault("unexpected-range-endpoint-eof", next_eof=True),
                mode="range",
            )
            add(
                name + ":range-missing-end-eof-" + ranged,
                rexpr,
                points([600, 1000, 1400, 1900]),
                fault("required-range-missing-end-eof", next_eof=True),
                mode="range",
            )
            add(
                name + ":empty-step-" + ranged,
                rexpr,
                [
                    (600, 1, "float"),
                    (1000, STALE, "float"),
                    (1200, STALE, "float"),
                    (2000, 5, "float"),
                ],
                mode="range",
            )
        for tiny in ("0.0005", "1ms"):
            add(
                name + ":physical-" + tiny,
                template.replace("RANGE", tiny),
                points([1000]),
                fault("initial-physical-window", next_times=[1000]),
            )
        fixed = template.replace("RANGE", "500ms").replace("m[500ms]", "m[500ms] @1")
        add(
            name + ":fixed-at",
            fixed,
            points([600, 1000, 2000]),
            fault(
                "unexpected-fixed-refetch", seek_targets=[1000, 1500], next_times=[2000]
            ),
            mode="range",
        )
        add(
            name + ":offset",
            template.replace("RANGE", "500ms").replace(
                "m[500ms]", "m[500ms] offset 500ms"
            ),
            points([100, 500, 1000, 1500]),
            mode="range",
        )
        add(
            name + ":wide-source-control",
            expr + " + scalar(anchor @0)",
            points([100, 200, 500, 600, 1000, 1100]),
            anchors=True,
        )
        # The first series' helper must finish before a later series' acquisition failure.
        if name == "double_exponential_smoothing":
            add(
                name + ":earlier-helper-error",
                "double_exponential_smoothing(m[500ms],0,0.6)",
                points([600, 700, 800, 1000]),
                more=[series({"__name__": "m", "z": "two"}, points([600, 1000]))],
            )
    for kind in ("histogram", "float_histogram"):
        for name in (
            "sum_over_time",
            "avg_over_time",
            "count_over_time",
            "first_over_time",
            "last_over_time",
            "changes",
            "resets",
            "rate",
            "increase",
            "delta",
            "irate",
            "idelta",
        ):
            template = FUNCTIONS[name].replace("RANGE", "1s")
            native = [
                (0, 8, kind),
                (500, 5, kind),
                (1000, 4, kind),
                (1500, 9, kind),
                (2000, 1, kind),
            ]
            for projection in ("plain", "count", "sum"):
                expression = (
                    template
                    if projection == "plain"
                    else "histogram_" + projection + "(" + template + ")"
                )
                add(
                    kind + ":" + name + ":" + projection + ":epochs",
                    expression,
                    native,
                    mode="range",
                )
                add(
                    kind + ":" + name + ":" + projection + ":current-seek",
                    expression,
                    native,
                    fault("required-native-current-seek", seek_targets=[0]),
                )
                add(
                    kind + ":" + name + ":" + projection + ":endpoint-eof",
                    expression,
                    native,
                    fault("unexpected-native-endpoint-eof", next_eof=True),
                    mode="range",
                )
                add(
                    kind + ":" + name + ":" + projection + ":omitted-seek",
                    expression,
                    native,
                    fault("unexpected-native-omitted-seek", seek_targets=[1000, 1500]),
                    mode="range",
                )
                gap = [
                    (0, 8, kind),
                    (500, 3, "float"),
                    (1000, 4, kind),
                    (1500, STALE, kind),
                    (2000, 1, kind),
                ]
                add(
                    kind + ":" + name + ":" + projection + ":stale-epochs",
                    expression,
                    gap,
                    mode="range",
                )
                add(
                    kind + ":" + name + ":" + projection + ":empty-before-native",
                    expression,
                    [
                        (0, STALE, kind),
                        (500, STALE, "float"),
                        (1500, 9, kind),
                        (2000, 1, kind),
                    ],
                    mode="range",
                )
    # Only ordinary matrix roots; range-mode roots fail source construction by design.
    for kind in ("float", "histogram", "float_histogram"):
        add(
            "raw:" + kind + ":endpoint-eof",
            "m[500ms]",
            points([600, 1000], kind),
            fault("unexpected-raw-endpoint-eof", next_eof=True),
        )
        add(
            "raw:" + kind + ":missing-endpoint",
            "m[500ms]",
            points([600, 900], kind),
            fault("required-raw-nonendpoint-eof", next_eof=True),
        )
        add(
            "raw:" + kind + ":current-seek",
            "m[500ms]",
            points([600, 1000], kind),
            fault("required-raw-current-seek", seek_targets=[500]),
        )
        add(
            "raw:" + kind + ":physical-zero",
            "m[0.0005]",
            points([1000], kind),
            fault("ignored-raw-zero-error", next_times=[1000]),
        )
        add(
            "raw:" + kind + ":range-construction",
            "m[500ms]",
            points([600, 1000], kind),
            mode="range",
        )
    for expression in (
        "predict_linear(m[500ms] @1,time())",
        "double_exponential_smoothing(m[500ms] @1,time()/10,0.6)",
        "double_exponential_smoothing(m[500ms] @1,time()-1,0.6)",
    ):
        add(
            "fixed-varying:" + expression,
            expression,
            points([600, 700, 800, 1000, 2000]),
            fault(
                "unexpected-fixed-varying-refetch",
                seek_targets=[1000, 1500],
                next_times=[2000],
            ),
            mode="range",
        )
    assert len({r["request"]["id"] for r in inputs}) == len(inputs)
    return inputs


def clipped_cases(ordinary):
    supplement = []
    for name in list(FUNCTIONS) + ["raw-matrix"]:
        for profile in ("server-default", "upstream-tests"):
            prefix = f"matrix-reader:{name}:endpoint:{profile}"
            row = (
                deepcopy(next(r for r in ordinary if r["request"]["id"] == prefix))
                if name != "raw-matrix"
                else deepcopy(
                    next(
                        r
                        for r in ordinary
                        if r["request"]["id"]
                        == f"matrix-reader:raw:float:endpoint-eof:{profile}"
                    )
                )
            )
            for spelling in ("0.5ms", "0.0005"):
                for variant in (
                    "healthy",
                    "initial-next",
                    "current-seek",
                    "unrequested-seek",
                ):
                    case = deepcopy(row)
                    case["request"].update(
                        id=f"matrix-reader:clipped:{name}:{spelling}:{variant}:{profile}",
                        query=case["request"]["query"].replace(
                            "m[500ms]", f"m[1ms] @1 offset {spelling}"
                        ),
                        mode="instant",
                        time_ms=0,
                        start_ms=0,
                        end_ms=0,
                    )
                    case["dataset"]["series"][0]["points"] = case["dataset"]["series"][
                        0
                    ]["points"][-1:]
                    case["faults"] = {}
                    if variant != "healthy":
                        field = (
                            "next_times"
                            if variant == "initial-next"
                            else "seek_targets"
                        )
                        target = 998 if variant == "current-seek" else 1000
                        case["faults"] = {
                            "m": {field: [target], "message": "clipped-" + variant}
                        }
                    supplement.append(case)
    for profile in ("server-default", "upstream-tests"):
        row = deepcopy(
            next(
                r
                for r in ordinary
                if r["request"]["id"]
                == f"matrix-reader:double_exponential_smoothing:earlier-helper-error:{profile}"
            )
        )
        row["request"]["id"] = "matrix-reader:helper-before-later-series:" + profile
        row["faults"] = {
            "m/two": {
                "next_times": [600],
                "message": "unexpected-later-series-before-helper",
            }
        }
        supplement.append(row)
    assert len({r["request"]["id"] for r in supplement}) == len(supplement)
    return supplement


def timestamp_cases(ordinary, clipped):
    rows = []
    renames = {
        "first_over_time": "ts_of_first_over_time",
        "last_over_time": "ts_of_last_over_time",
        "min_over_time": "ts_of_min_over_time",
        "max_over_time": "ts_of_max_over_time",
    }
    for group in (ordinary, clipped):
        for source in group:
            for old, new in renames.items():
                if old + "(" in source["request"]["query"]:
                    row = deepcopy(source)
                    row["request"]["query"] = row["request"]["query"].replace(
                        old + "(", new + "("
                    )
                    row["request"]["id"] = "matrix-timestamp-fold:" + row["request"][
                        "id"
                    ].replace(old, new)
                    rows.append(row)
    assert len({r["request"]["id"] for r in rows}) == len(rows)
    return rows


def zero_range_cases(ordinary):
    controls = []
    for name, template in FUNCTIONS.items():
        for profile in ("server-default", "upstream-tests"):
            source = next(
                row
                for row in ordinary
                if row["request"]["id"] == f"matrix-reader:{name}:endpoint:{profile}"
            )
            for variant in (
                "healthy",
                "initial-next",
                "unrequested-seek",
                "nonempty-eof",
                "empty-eof",
            ):
                row = deepcopy(source)
                row["request"].update(
                    id=f"matrix-reader:zero-widened:{name}:{variant}:{profile}",
                    query=template.replace("RANGE", "0.0005") + " + scalar(anchor @0)",
                )
                row["dataset"]["series"][0]["points"] = (
                    []
                    if variant == "empty-eof"
                    else row["dataset"]["series"][0]["points"][-1:]
                )
                row["dataset"]["series"].append(
                    series({"__name__": "anchor"}, [(0, 0, "float")])
                )
                row["faults"] = {}
                if variant != "healthy":
                    field, value = {
                        "initial-next": ("next_times", [1000]),
                        "unrequested-seek": ("seek_targets", [1000]),
                        "nonempty-eof": ("next_eof", True),
                        "empty-eof": ("next_eof", True),
                    }[variant]
                    row["faults"] = {
                        "m": {field: value, "message": "zero-widened-" + variant}
                    }
                controls.append(row)
    return controls


def cases(roster="ordinary"):
    ordinary = ordinary_cases()
    if roster == "ordinary":
        return ordinary
    if roster == "zero":
        return zero_range_cases(ordinary)
    clipped = clipped_cases(ordinary)
    if roster == "clipped":
        return clipped
    if roster == "timestamp":
        return timestamp_cases(ordinary, clipped)
    raise ValueError("unknown matrix reader roster")
