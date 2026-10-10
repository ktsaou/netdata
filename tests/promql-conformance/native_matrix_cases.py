"""Independent source controls for matrix phases and reached native histogram faults."""

import base64
import hashlib
import json
from copy import deepcopy

from info_cases import bits, series
from unvisited_matrix_cases import cases as _primary


def _unvisited_supplement():
    """Preserve the independently declared unvisited-supplement input roster."""
    PROFILES = ("server-default", "upstream-tests")
    MODES = ("instant", "range")
    STALE = "7ff0000000000002"
    rows = []
    obligations = []

    def floats(name, values=(1.0, 2.0, 3.0), labels=None):
        return series(
            {"__name__": name} | (labels or {}),
            list(zip((0, 1000, 2000), values, ("float",) * 3)),
        )

    def add(
        family,
        witness,
        query,
        records,
        faults=None,
        *,
        profiles=PROFILES,
        modes=MODES,
        context=None,
        source=None,
        claim=None,
    ):
        ids = []
        for profile in profiles:
            for mode in modes:
                ident = f"unvisited-supplement-{len(rows):04d}-{family}-{witness}-{profile}-{mode}"
                request = {
                    "id": ident,
                    "query": query,
                    "profile": profile,
                    "mode": mode,
                    "time_ms": 2000,
                    "start_ms": 0,
                    "end_ms": 2000,
                    "step_ms": 1000,
                    "lookback_ms": 1000,
                    "subquery_step_ms": 500,
                } | (context or {})
                rows.append(
                    {
                        "request": request,
                        "dataset": {"schema_version": 1, "series": deepcopy(records)},
                        "faults": deepcopy(faults or {}),
                    }
                )
                ids.append(ident)
        obligations.append(
            {
                "family": family,
                "witness": witness,
                "query": query,
                "ids": ids,
                "source": source or [],
                "static_obligation": claim,
            }
        )

    base = [
        floats("m", labels={"job": "a"}),
        floats("q", (0.5, 0.5, 0.5)),
        floats("r", (0.5, 0.5, 0.5)),
        floats("n", (11.0, 12.0, 13.0)),
    ]
    selection = lambda metric: {"set_error": metric + " selection fault"}
    iteration = lambda metric: {
        "next_times": [2000],
        "message": metric + " iterator fault",
    }
    calls = (
        ("quantile-generic-raw", "quantile_over_time(scalar(q),(m[2s]))"),
        ("quantile-ordinary-raw", "quantile_over_time(scalar(q),m[2s])"),
        ("predict-generic-raw", "predict_linear((m[2s]),scalar(q))"),
        ("predict-ordinary-raw", "predict_linear(m[2s],scalar(q))"),
        (
            "smoothing-generic-raw",
            "double_exponential_smoothing((m[2s]),scalar(q),scalar(r))",
        ),
        (
            "smoothing-ordinary-raw",
            "double_exponential_smoothing(m[2s],scalar(q),scalar(r))",
        ),
        ("quantile-generic-derived", "quantile_over_time(scalar(q),((m+1)[2s:500ms]))"),
        ("quantile-ordinary-derived", "quantile_over_time(scalar(q),(m+1)[2s:500ms])"),
    )
    faults = (
        ("both-selections", {"m": selection("m"), "q": selection("q")}),
        ("matrix-iterator-q-selection", {"m": iteration("m"), "q": selection("q")}),
        ("matrix-selection", {"m": selection("m")}),
        ("q-selection", {"q": selection("q")}),
    )
    for name, call in calls:
        for fault_name, injected in faults:
            add(
                "lexical",
                name + "-" + fault_name,
                f"quantile(scalar({call}),n)",
                base,
                injected,
                source=[
                    "promql/engine.go:1458",
                    "promql/engine.go:2184",
                    "promql/engine.go:2229",
                    "promql/engine.go:2237",
                ],
                claim="Generic arguments run in lexical order; actual subqueries run before scalars, actual raw matrices after scalars.",
            )
    for matrix_name, matrix in (("generic", "(m[2s])"), ("ordinary", "m[2s]")):
        for fault_name, injected in (
            (
                "all-selections",
                {"m": selection("m"), "q": selection("q"), "r": selection("r")},
            ),
            ("third-selection", {"r": selection("r")}),
            ("second-third-selections", {"q": selection("q"), "r": selection("r")}),
        ):
            add(
                "lexical",
                "smoothing-" + matrix_name + "-" + fault_name,
                f"quantile(scalar(double_exponential_smoothing({matrix},scalar(q),scalar(r))),n)",
                base,
                injected,
                source=["promql/engine.go:1458", "promql/functions.go:982"],
                claim="All scalar arguments are prepared despite the nil-matrix function returning before factors are inspected.",
            )
    for matrix_name, matrix in (("raw", "(m[2s])"), ("derived", "((m+1)[2s:500ms])")):
        invalid = (
            ("quantile-low", f"quantile_over_time(-1,{matrix})"),
            ("quantile-high", f"quantile_over_time(2,{matrix})"),
            ("quantile-nan", f"quantile_over_time(NaN,{matrix})"),
            ("predict-nan", f"predict_linear({matrix},NaN)"),
            ("smoothing-zero-one", f"double_exponential_smoothing({matrix},0,1)"),
            ("smoothing-high", f"double_exponential_smoothing({matrix},.5,2)"),
            ("smoothing-nan", f"double_exponential_smoothing({matrix},NaN,NaN)"),
        )
        for name, call in invalid:
            for populated in (False, True):
                records = base if populated else [base[-1]]
                add(
                    "nil-parameter",
                    matrix_name + "-" + name + ("-full" if populated else "-empty"),
                    f"quantile(scalar({call}),n)",
                    records,
                    source=[
                        "promql/functions.go:982",
                        "promql/functions.go:1653",
                        "promql/functions.go:2021",
                    ],
                    claim="The generic nil-matrix callback emits no own invalid-parameter diagnostic; outer quantile receives scalar(empty)=NaN.",
                )
    bad = floats("bad", (1.0, 1.0, 1.0), {"le": "broken"})
    bad2 = floats("bad2", (1.0, 1.0, 1.0), {"le": "also-broken"})
    warning_scalar = "scalar(histogram_quantile(.5,bad))"
    warning_scalar2 = "scalar(histogram_quantile(.5,bad2))"
    warning_calls = (
        ("quantile-first", f"quantile_over_time({warning_scalar},(m[2s]))"),
        ("predict-second", f"predict_linear((m[2s]),{warning_scalar})"),
        (
            "smoothing-both",
            f"double_exponential_smoothing((m[2s]),{warning_scalar},{warning_scalar2})",
        ),
        (
            "smoothing-second-empty-third-fault",
            f"double_exponential_smoothing((m[2s]),{warning_scalar},scalar(r))",
        ),
    )
    for name, call in warning_calls:
        fault_list = (
            ("success", {}),
            ("matrix-selection", {"m": selection("m")}),
            ("matrix-iterator", {"m": iteration("m")}),
        )
        if name.endswith("third-fault"):
            fault_list = (("third-selection", {"r": selection("r")}), ("success", {}))
        for fault_name, injected in fault_list:
            add(
                "warning-transport",
                name + "-" + fault_name,
                f"quantile(scalar({call}),n)",
                base + [bad, bad2],
                injected,
                source=[
                    "promql/engine.go:1222",
                    "promql/engine.go:1468",
                    "promql/engine.go:2915",
                ],
                claim="Successful generic evaluation merges child warnings; later plain/runtime errors discard prior child notes; expansion errors carry only their own warning packet.",
            )
    for matrix_name, matrix in (
        ("paren", "((rate(q[1s]))[2s:500ms])"),
        ("actual", "(rate(q[1s]))[2s:500ms]"),
    ):
        add(
            "warning-transport",
            "derived-rate-child-" + matrix_name,
            f"quantile(scalar(sum_over_time({matrix})),n)",
            base,
            source=["promql/engine.go:2612", "promql/functions.go:264"],
            claim="A non-identity derived body is evaluated on the full child grid and child helper notes survive successful parent preparation.",
        )
    for body_name, body in (
        ("variable", "n"),
        ("fixed", "n @ 2"),
        ("start", "n @ start()"),
        ("end", "n @ end()"),
    ):
        for fn in ("sum_over_time", "absent_over_time"):
            for state, records in (
                ("full", base),
                ("missing", [base[-1]]),
                (
                    "stale",
                    [
                        series(
                            {"__name__": "m", "job": "a"},
                            [(0, STALE, "float"), (2000, STALE, "float")],
                        ),
                        base[-1],
                    ],
                ),
            ):
                add(
                    "invariant-clock",
                    body_name + "-" + fn + "-" + state,
                    f'quantile(scalar({fn}((m{{job="a"}}[2s]))),{body})',
                    records,
                    source=[
                        "promql/engine.go:2606",
                        "promql/engine.go:2638",
                        "promql/functions.go:1733",
                        "promql/functions.go:2848",
                    ],
                    claim="Invariant aggregate bodies clamp the unvisited parameter to one local clock; generic absent_over_time returns one with empty labels independent of raw membership.",
                )
    for modifier in ("2", "start()", "end()"):
        add(
            "invariant-clock",
            "parameter-fixed-only-" + modifier.replace("()", ""),
            f"quantile(scalar(absent_over_time((m[2s] @ {modifier}))),n)",
            base,
            source=["promql/engine.go:2606", "promql/engine.go:2638"],
            claim="A fixed selector inside the unvisited parameter alone does not make the variable aggregate body invariant.",
        )

    def native_records(kind, state="valid"):
        times = (500, 1000, 2000)
        values = (2.0, 3.0, 4.0)
        if state == "stale":
            values = (STALE, STALE, STALE)
        record = series(
            {"__name__": "h", "job": "a"}, list(zip(times, values, (kind,) * 3))
        )
        integer = kind == "histogram"
        for index, point in enumerate(record["points"]):
            payload = point[kind]
            payload["zero_count"] = 1 if integer else bits(1.0)
            payload["count"] = index + 3 if integer else bits(index + 3.0)
            payload["positive_spans"] = [{"offset": 0, "length": 1}]
            payload["positive_buckets"] = (
                [index + 2] if integer else [bits(index + 2.0)]
            )
        if state in ("malformed-unknown", "malformed-known", "malformed-stale"):
            record["points"][0][kind]["positive_spans"][0]["length"] = 2
            if state == "malformed-known":
                for point in record["points"][1:]:
                    point[kind]["reset_hint"] = 2
            if state == "malformed-stale":
                record["points"][0][kind]["sum"] = STALE
        if state == "custom":
            for index, point in enumerate(record["points"]):
                payload = point[kind]
                payload["schema"] = -53
                payload["zero_threshold"] = bits(0.0)
                payload["zero_count"] = 0 if integer else bits(0.0)
                payload["count"] = index + 2 if integer else bits(index + 2.0)
                payload["custom_values"] = [bits(1.0)]
        return [record, base[-1]]

    native_states = (
        ("float-native", native_records("float_histogram")),
        ("integer-native", native_records("histogram")),
        ("float-stale-h", native_records("float_histogram", "stale")),
        ("integer-stale-h", native_records("histogram", "stale")),
        ("float-only", [floats("h", labels={"job": "a"}), base[-1]]),
        ("empty", [base[-1]]),
        (
            "mixed",
            [
                series(
                    {"__name__": "h", "job": "a"},
                    [
                        (500, 1.0, "float"),
                        (1000, 2.0, "float_histogram"),
                        (2000, 3.0, "float"),
                    ],
                ),
                base[-1],
            ],
        ),
        (
            "stale-after-end",
            [
                series(
                    {"__name__": "h", "job": "a"}, [(2500, STALE, "float_histogram")]
                ),
                base[-1],
            ],
        ),
        (
            "stale-beyond-end-lookback",
            [
                series(
                    {"__name__": "h", "job": "a"}, [(4000, STALE, "float_histogram")]
                ),
                base[-1],
            ],
        ),
    )
    for modifier in ("anchored", "smoothed"):
        for fn in ("sum_over_time", "absent_over_time", "rate"):
            for state, records in native_states:
                add(
                    "extended-acquisition",
                    modifier + "-" + fn + "-" + state,
                    f"quantile(scalar({fn}((h[2s] {modifier}))),n @ 2)",
                    records,
                    source=[
                        "promql/engine.go:2891",
                        "promql/engine.go:2934",
                        "promql/engine.go:2251",
                    ],
                    claim="The raw internal visitor widens acquisition and rejects any allocated histogram lane, including stale-only allocation, before the generic nil callback; actual-call safe-map guards do not apply to generic parentheses.",
                )
        add(
            "extended-acquisition",
            modifier + "-ordinary-unsafe-float",
            f"quantile(scalar(sum_over_time(h[2s] {modifier})),n @ 2)",
            [floats("h"), base[-1]],
            source=["promql/engine.go:2251"],
            claim="Actual raw matrix calls keep their unsafe-function modifier guard before acquisition.",
        )
        for injected_name, injected in (
            ("selection", {"h": selection("h")}),
            ("iterator", {"h": iteration("h")}),
        ):
            add(
                "extended-acquisition",
                modifier + "-native-" + injected_name,
                f"quantile(scalar(absent_over_time((h[2s] {modifier}))),n @ 2)",
                native_records("float_histogram"),
                injected,
                source=["promql/engine.go:2915", "promql/engine.go:2934"],
                claim="Acquisition failures precede the extended-histogram helper rejection.",
            )
    stats_calls = (
        ("no-stats-rate", "rate((h[2s]))"),
        ("count-rate", "histogram_count(rate((h[2s])))"),
        ("sum-rate", "histogram_sum(rate((h[2s])))"),
        ("avg-rate", "histogram_avg(rate((h[2s])))"),
        ("count-nil-sum", "histogram_count(sum_over_time((h[2s])))"),
        ("sum-nil-sum", "histogram_sum(sum_over_time((h[2s])))"),
        ("avg-nil-sum", "histogram_avg(sum_over_time((h[2s])))"),
        (
            "full-hist-block",
            "histogram_quantile(.5,histogram_count(sum_over_time((h[2s]))))",
        ),
        ("subquery-block", "histogram_count(sum_over_time(((h)[2s:500ms])))"),
    )
    for kind in ("float_histogram", "histogram"):
        for state in (
            "valid",
            "custom",
            "malformed-unknown",
            "malformed-known",
            "malformed-stale",
        ):
            for name, call in stats_calls:
                add(
                    "stats-ancestor",
                    kind + "-" + state + "-" + name,
                    f"quantile(scalar({call}),n @ 2)",
                    native_records(kind, state),
                    source=[
                        "promql/engine.go:1155",
                        "promql/engine.go:4755",
                        "promql/histogram_stats_iterator.go:92",
                        "promql/histogram_stats_iterator.go:148",
                    ],
                    claim="Whole-AST ancestry chooses stats decoding before discarded matrix acquisition. Malformed-span cases separately witness reset traversal; known-reset, stale, full-histogram and subquery blockers distinguish the phase.",
                )
    for name, call in (
        stats_calls[0],
        stats_calls[1],
        stats_calls[4],
        stats_calls[7],
        stats_calls[8],
    ):
        add(
            "stats-ancestor",
            "late-fault-malformed-" + name,
            f"quantile(scalar({call}),n @ 2)",
            native_records("float_histogram", "malformed-unknown"),
            {"h": iteration("h")},
            source=["promql/histogram_stats_iterator.go:148", "promql/engine.go:3057"],
            claim="A source reset-decoding failure on the prior point may precede a separately configured late iterator fault.",
        )
    assert len({row["request"]["id"] for row in rows}) == len(rows)
    assert all(row["request"]["profile"] in PROFILES for row in rows)
    return rows


def _unvisited_phases():
    """Preserve the independently declared unvisited-phases input roster."""
    PROFILES = ("server-default", "upstream-tests")
    MODES = ("instant", "range")
    STALE = "7ff0000000000002"
    rows, groups = ([], [])

    def add(family, witness, query, records, faults=None):
        ids = []
        for profile in PROFILES:
            for mode in MODES:
                ident = f"unvisited-phase-followup-{len(rows):04d}-{family}-{witness}-{profile}-{mode}"
                rows.append(
                    {
                        "request": {
                            "id": ident,
                            "query": query,
                            "profile": profile,
                            "mode": mode,
                            "time_ms": 2000,
                            "start_ms": 0,
                            "end_ms": 2000,
                            "step_ms": 1000,
                            "lookback_ms": 1000,
                            "subquery_step_ms": 500,
                        },
                        "dataset": {"schema_version": 1, "series": deepcopy(records)},
                        "faults": deepcopy(faults or {}),
                    }
                )
                ids.append(ident)
        groups.append(
            {"family": family, "witness": witness, "query": query, "ids": ids}
        )

    n = series(
        {"__name__": "n"},
        [(0, 11.0, "float"), (1000, 12.0, "float"), (2000, 13.0, "float")],
    )
    dense_q = series(
        {"__name__": "q"},
        [(t, float(t // 500 + 1), "float") for t in (0, 500, 1000, 1500, 2000)],
    )
    dense_bad = series(
        {"__name__": "bad", "le": "broken"},
        [(t, 1.0, "float") for t in (0, 500, 1000, 1500, 2000)],
    )
    for body_name, body in (("variable", "n"), ("fixed", "n @ 2")):
        for matrix_name, matrix in (
            ("generic", "((rate(q[1s]))[2s:500ms])"),
            ("ordinary", "(rate(q[1s]))[2s:500ms]"),
        ):
            add(
                "derived-info",
                body_name + "-" + matrix_name,
                f"quantile(scalar(sum_over_time({matrix})),{body})",
                [dense_q, n],
            )
        for matrix_name, matrix in (
            ("generic", "((histogram_quantile(.5,bad))[2s:500ms])"),
            ("ordinary", "(histogram_quantile(.5,bad))[2s:500ms]"),
        ):
            add(
                "derived-warning",
                body_name + "-" + matrix_name,
                f"quantile(scalar(sum_over_time({matrix})),{body})",
                [dense_bad, n],
            )

    def decoder_record(kind, decoder, state, times=(1000, 2000)):
        integer = kind == "histogram"
        h = series(
            {"__name__": "h"}, [(t, float(i + 3), kind) for i, t in enumerate(times)]
        )
        for i, point in enumerate(h["points"]):
            payload = point[kind]
            payload["schema"] = 9 if decoder == "schema9" else -53
            payload["zero_count"] = 1 if integer else bits(1.0)
            payload["zero_threshold"] = (
                bits(0.001 * (i + 1)) if decoder == "custom-bound" else bits(0.001)
            )
            payload["count"] = i + 3 if integer else bits(i + 3.0)
            payload["positive_spans"] = [
                {"offset": 0 if decoder == "schema9" else 2, "length": 1}
            ]
            payload["positive_buckets"] = [i + 2] if integer else [bits(i + 2.0)]
            payload["custom_values"] = None if decoder == "schema9" else [bits(1.0)]
        if state == "known":
            for point in h["points"][1:]:
                point[kind]["reset_hint"] = 2
        if state == "stale":
            h["points"][0][kind]["sum"] = STALE
        return h

    calls = (
        ("no-stats-rate", "rate((h[2s]))"),
        ("count-rate", "histogram_count(rate((h[2s])))"),
        ("count-nil-sum", "histogram_count(sum_over_time((h[2s])))"),
        ("sum-nil-sum", "histogram_sum(sum_over_time((h[2s])))"),
        (
            "full-hist-block",
            "histogram_quantile(.5,histogram_count(sum_over_time((h[2s]))))",
        ),
        ("subquery-block", "histogram_count(sum_over_time(((h)[2s:500ms])))"),
    )
    for kind in ("float_histogram", "histogram"):
        for decoder in ("schema9", "custom-bound"):
            for state in ("unknown", "known", "stale"):
                for call_name, call in calls:
                    add(
                        "decoder-phase",
                        kind + "-" + decoder + "-" + state + "-" + call_name,
                        f"quantile(scalar({call}),n @ 2)",
                        [decoder_record(kind, decoder, state), n],
                    )
    for decoder in ("schema9", "custom-bound"):
        for name, call in (calls[0], calls[1]):
            add(
                "decoder-late-fault",
                decoder + "-" + name,
                f"quantile(scalar({call}),n @ 2)",
                [
                    decoder_record(
                        "float_histogram", decoder, "unknown", (500, 1000, 2000)
                    ),
                    n,
                ],
                {"h": {"next_times": [2000], "message": "h late iterator fault"}},
            )
    assert len(rows) == 336 and len({row["request"]["id"] for row in rows}) == len(rows)
    return rows


def _native_valid():
    """Preserve the independently declared native-valid input roster."""

    def record(name, offset, times=(2000,), custom=False):
        value = series({"__name__": name}, [(t, 1.0, "float_histogram") for t in times])
        for point in value["points"]:
            payload = point["float_histogram"]
            payload.update(
                schema=-53 if custom else 0,
                zero_threshold=bits(0.0),
                zero_count=bits(0.0),
                count=bits(1.0),
                positive_spans=[{"offset": offset, "length": 1}],
                positive_buckets=[bits(1.0)],
                custom_values=[bits(1.0)] if custom else None,
            )
        return value

    a, b, c = (
        record("a", -(2**31)),
        record("b", 2**31 - 1),
        record("c", 0, custom=True),
    )
    s = record("s", -(2**31), (1000, 2000))
    s["points"][1]["float_histogram"]["positive_spans"][0]["offset"] = 2**31 - 1
    sr = deepcopy(s)
    sr["labels"][0]["value"] = base64.b64encode(b"sr").decode()
    sr["points"][0]["float_histogram"]["positive_spans"][0]["offset"] = 2**31 - 1
    sr["points"][1]["float_histogram"]["positive_spans"][0]["offset"] = -(2**31)
    definitions = [
        ("signed-add-forward", "a+b", [a, b]),
        ("signed-add-reverse", "b+a", [a, b]),
        ("signed-sum-forward", 'sum({__name__=~"a|b"})', [a, b]),
        ("signed-avg-forward", 'avg({__name__=~"a|b"})', [a, b]),
        ("signed-range-sum", "sum_over_time(s[2s])", [s]),
        ("signed-range-avg", "avg_over_time(s[2s])", [s]),
        ("signed-range-delta", "delta(s[2s])", [s]),
        ("signed-range-rate", "rate(sr[2s])", [sr]),
        ("custom-count-divzero", 'count_values("x",c/0)', [c]),
        ("custom-count-mulinf", 'count_values("x",c*Inf)', [c]),
        ("custom-quantile-divzero", "histogram_quantile(.25,c/0)", [c]),
        ("custom-count-sum-mulinf", 'count_values("x",sum(c*Inf))', [c]),
    ]
    rows = []
    for name, query, records in definitions:
        for profile in ("server-default", "upstream-tests"):
            rows.append(
                {
                    "request": {
                        "id": f"native-valid-contract-{len(rows):04d}-{name}-{profile}",
                        "query": query,
                        "profile": profile,
                        "mode": "instant",
                        "time_ms": 2000,
                        "start_ms": 0,
                        "end_ms": 0,
                        "step_ms": 1000,
                        "lookback_ms": 1000,
                        "subquery_step_ms": 500,
                    },
                    "dataset": {"schema_version": 1, "series": deepcopy(records)},
                    "faults": {},
                }
            )
    return rows


def _native_checked():
    """Preserve the independently declared native-checked input roster."""
    rows, groups = ([], [])

    def add(family, witness, query, records, profile="upstream-tests", mode="instant"):
        ident = (
            f"native-checked-panel-{len(rows):04d}-{family}-{witness}-{profile}-{mode}"
        )
        rows.append(
            {
                "request": {
                    "id": ident,
                    "query": query,
                    "profile": profile,
                    "mode": mode,
                    "time_ms": 2000,
                    "start_ms": 1000,
                    "end_ms": 2000,
                    "step_ms": 1000,
                    "lookback_ms": 1000,
                    "subquery_step_ms": 500,
                },
                "dataset": {"schema_version": 1, "series": deepcopy(records)},
                "faults": {},
            }
        )
        groups.append(
            {"id": ident, "family": family, "witness": witness, "query": query}
        )

    def histogram(name="h", variant="valid", times=(2000,), reset=0, drift=False):
        value = series({"__name__": name}, [(t, 1.0, "float_histogram") for t in times])
        for i, point in enumerate(value["points"]):
            h = point["float_histogram"]
            h.update(
                reset_hint=reset,
                schema=0,
                zero_threshold=bits(0.001 * (i + 1) if drift else 0.0),
                zero_count=bits(0.0),
                count=bits(1.0),
                positive_spans=[{"offset": 0, "length": 1}],
                positive_buckets=[bits(1.0)],
                negative_spans=None,
                negative_buckets=None,
                custom_values=None,
            )
            if variant.startswith("custom"):
                h.update(schema=-53, custom_values=[bits(1.0)])
            if variant == "custom-upper":
                h["positive_spans"][0]["offset"] = 2
            elif variant == "custom-lower":
                h["positive_spans"][0]["offset"] = -1
            elif variant == "custom-negative":
                h.update(
                    positive_spans=None,
                    positive_buckets=None,
                    negative_spans=[{"offset": 1, "length": 1}],
                    negative_buckets=[bits(1.0)],
                )
            elif variant == "custom-zero":
                h.update(
                    positive_spans=None, positive_buckets=None, zero_count=bits(1.0)
                )
            elif variant == "schema9":
                h["schema"] = 9
            elif variant == "schema-min":
                h["schema"] = -(2**31)
            elif variant == "short-counts":
                h["positive_spans"][0]["length"] = 2
            elif variant == "extra-counts":
                h["positive_buckets"] = [bits(1.0), bits(2.0)]
            elif variant == "signed-min":
                h["positive_spans"][0]["offset"] = -(2**31)
            elif variant == "signed-max":
                h["positive_spans"][0]["offset"] = 2**31 - 1
        return value

    kernels = (
        ("quantile-forward", "histogram_quantile(.25,h)"),
        ("quantile-reverse", "histogram_quantile(.75,h)"),
        ("fraction", "histogram_fraction(0,1,h)"),
        ("stdvar", "histogram_stdvar(h)"),
        ("trim-upper", "h </ 1"),
        ("trim-lower", "h >/ 1"),
    )
    for variant in (
        "custom-upper",
        "custom-lower",
        "custom-negative",
        "schema9",
        "schema-min",
    ):
        for name, query in kernels:
            add(
                "reached-kernel",
                variant + "-" + name,
                query,
                [histogram(variant=variant)],
            )
    for variant in ("short-counts", "extra-counts"):
        for name, query in kernels:
            add(
                "span-counts", variant + "-" + name, query, [histogram(variant=variant)]
            )
    for name, query in (kernels[0], kernels[2], kernels[3], kernels[4]):
        add("custom-zero", name, query, [histogram(variant="custom-zero")])
    add(
        "native-plural",
        "first-q-fault",
        'histogram_quantiles("q",h,.25,.75)',
        [histogram(variant="custom-upper")],
    )
    add(
        "native-plural",
        "zero-q-unused",
        'histogram_quantiles("q",h)',
        [histogram(variant="custom-upper")],
    )
    add(
        "native-stddev",
        "shared-stdvar-fault",
        "histogram_stddev(h)",
        [histogram(variant="custom-upper")],
    )
    bad = histogram(variant="custom-upper")
    zero_count = deepcopy(bad)
    zero_count["points"][0]["float_histogram"]["count"] = bits(0.0)
    empty_spans = deepcopy(bad)
    empty_spans["points"][0]["float_histogram"].update(
        positive_spans=None, positive_buckets=None
    )
    zero_bucket = deepcopy(bad)
    zero_bucket["points"][0]["float_histogram"]["positive_buckets"] = [bits(0.0)]
    guards = (
        ("q-negative", "histogram_quantile(-1,h)", bad),
        ("q-high", "histogram_quantile(2,h)", bad),
        ("q-nan", "histogram_quantile(NaN,h)", bad),
        ("q-count0", "histogram_quantile(.25,h)", zero_count),
        ("fraction-nan", "histogram_fraction(NaN,1,h)", bad),
        ("fraction-equal", "histogram_fraction(1,1,h)", bad),
        ("fraction-count0", "histogram_fraction(0,1,h)", zero_count),
        ("sum-unused-bounds", "histogram_sum(h)", bad),
        ("count-unused-bounds", "histogram_count(h)", bad),
        ("empty-spans-unused", "histogram_stdvar(h)", empty_spans),
        ("zero-productive-count-at", "histogram_stdvar(h)", zero_bucket),
        ("empty-selector-unused", 'histogram_quantile(.25,h{unused="yes"})', bad),
    )
    for name, query, record in guards:
        add("guards", name, query, [record])
    for variant in ("custom-upper", "schema-min", "short-counts"):
        records = [histogram("a"), histogram("h", variant)]
        for name, query in (
            ("sum", 'sum({__name__=~"a|h"})'),
            ("avg", 'avg({__name__=~"a|h"})'),
            ("add", "a+h"),
        ):
            add("native-arithmetic", variant + "-" + name, query, records)
        history = histogram(variant=variant, times=(500, 1500), drift=True)
        for name, query in (
            ("range-sum", "sum_over_time(h[2s])"),
            ("range-avg", "avg_over_time(h[2s])"),
        ):
            add("native-arithmetic", variant + "-" + name, query, [history])
    for variant in ("custom-upper", "schema9", "schema-min"):
        history = histogram(variant=variant, times=(-500, 500, 1500, 2500), drift=True)
        for name, query in (
            ("resets", "resets(h[2s])"),
            ("rate", "rate(h[2s])"),
            ("statistics", "histogram_count(rate(h[2s]))"),
        ):
            add("counter-and-statistics", variant + "-" + name, query, [history])
    for variant in ("custom-upper", "schema-min"):
        history = histogram(variant=variant, times=(-500, 500, 1500, 2500), drift=True)
        for modifier in ("anchored", "smoothed"):
            for function in ("rate", "increase", "delta"):
                add(
                    "extended-rate",
                    variant + "-" + function + "-" + modifier,
                    f"{function}(h[2s] {modifier})",
                    [history],
                )
    for hint in (0, 2):
        history = histogram(
            variant="custom-upper", times=(500, 1500), reset=hint, drift=True
        )
        add(
            "statistics-guards",
            "reset-hint-" + str(hint),
            "histogram_count(sum_over_time((h[2s])))",
            [history],
        )
        add("statistics-guards", "resets-hint-" + str(hint), "resets(h[2s])", [history])
    history = histogram(variant="custom-upper", times=(500, 1500), drift=True)
    history["points"][0]["float_histogram"]["sum"] = "7ff0000000000002"
    add(
        "statistics-guards",
        "stale-predecessor",
        "histogram_count(sum_over_time((h[2s])))",
        [history],
    )
    add(
        "statistics-guards",
        "full-histogram-ancestry",
        "histogram_quantile(.5,histogram_count(sum_over_time((h[2s]))))",
        [histogram(variant="custom-upper", times=(500, 1500), drift=True)],
    )
    bad_bucket = series(
        {"__name__": "bad", "le": "broken"},
        [(0, 1.0, "float"), (1000, 1.0, "float"), (2000, 1.0, "float")],
    )
    q = series(
        {"__name__": "q"},
        [(0, 1.0, "float"), (1000, 2.0, "float"), (2000, 3.0, "float")],
    )
    for note, left, record in (
        ("warning", "histogram_quantile(.5,bad)", bad_bucket),
        ("info", "rate(q[2s])", q),
    ):
        for fault in (False, True):
            h = histogram(variant="custom-upper" if fault else "valid")
            for mode in ("instant", "range"):
                add(
                    "recovery-notes",
                    note + "-" + ("fault" if fault else "success"),
                    left + "+histogram_stdvar(h)",
                    [record, h],
                    mode=mode,
                )
    signed = [histogram("a", "signed-min"), histogram("b", "signed-max")]
    add("signed-index", "negative-add", "a+b", signed)
    add("signed-index", "reverse-success", "b+a", signed)
    add("signed-index", "negative-sum", 'sum({__name__=~"a|b"})', signed)
    add("signed-index", "negative-avg", 'avg({__name__=~"a|b"})', signed)
    for query in ('count_values("x",c/0)', 'count_values("x",c*Inf)'):
        add(
            "generated-zero",
            "scalar-transform",
            query,
            [histogram("c", "custom-valid")],
        )
    for variant in ("custom-upper", "custom-lower", "schema9", "schema-min"):
        add(
            "server-profile",
            variant,
            "histogram_quantile(.25,h)",
            [histogram(variant=variant)],
            profile="server-default",
        )
    add("server-profile", "signed-add", "a+b", signed, profile="server-default")
    add(
        "server-profile",
        "custom-zero",
        'count_values("x",c/0)',
        [histogram("c", "custom-valid")],
        profile="server-default",
    )
    assert len(rows) == len({row["request"]["id"] for row in rows}) and len(rows) < 128
    return rows


def _native_notes():
    """Preserve the independently declared native-notes input roster."""

    def custom(name):
        value = series({"__name__": name}, [(2000, 1.0, "float_histogram")])
        value["points"][0]["float_histogram"].update(
            schema=-53,
            zero_threshold=bits(0.0),
            zero_count=bits(0.0),
            positive_spans=[{"offset": 0, "length": 1}],
            positive_buckets=[bits(1.0)],
            custom_values=[bits(1.0)],
        )
        return value

    c = custom("c")
    h = custom("h")
    h["points"][0]["float_histogram"]["positive_spans"][0]["offset"] = 2
    bad = series(
        {"__name__": "bad", "le": "broken"},
        [(0, 1.0, "float"), (1000, 1.0, "float"), (2000, 1.0, "float")],
    )
    q = series(
        {"__name__": "q"},
        [(0, 1.0, "float"), (1000, 2.0, "float"), (2000, 3.0, "float")],
    )
    rows = []

    def add(family, query, records, mode="instant", profile="upstream-tests"):
        rows.append(
            {
                "request": {
                    "id": f"native-count-followup-{len(rows):04d}-{family}-{profile}-{mode}",
                    "query": query,
                    "profile": profile,
                    "mode": mode,
                    "time_ms": 2000,
                    "start_ms": 1000,
                    "end_ms": 2000,
                    "step_ms": 1000,
                    "lookback_ms": 1000,
                    "subquery_step_ms": 500,
                },
                "dataset": {"schema_version": 1, "series": deepcopy(records)},
                "faults": {},
            }
        )

    for note, left, record in (
        ("warning", "histogram_quantile(.5,bad)", bad),
        ("info", "rate(q[2s])", q),
    ):
        for divisor in (1, 0):
            for mode in ("instant", "range"):
                add(
                    "direct-" + note + "-" + ("fault" if divisor == 0 else "success"),
                    left + f'+count_values("x",c/{divisor})',
                    [record, c],
                    mode,
                )
        for mode in ("instant", "range"):
            add(
                "derived-copy-" + note,
                left + '+count_values("x",last_over_time((c/0)[2s:1s]))',
                [record, c],
                mode,
            )
    add("plural-reached", 'histogram_quantiles(h,"q",.25,.75)', [h])
    add("plural-early-guards", 'histogram_quantiles(h,"q",-1,2)', [h])
    add("plural-later-fault", 'histogram_quantiles(h,"q",-1,.25)', [h])
    for note, left, record in (
        ("warning", "histogram_quantile(.5,bad)", bad),
        ("info", "rate(q[2s])", q),
    ):
        add(
            "server-direct-" + note,
            left + '+count_values("x",c/0)',
            [record, c],
            profile="server-default",
        )
    assert len(rows) == 17 and len({r["request"]["id"] for r in rows}) == 17
    return rows


def _matrix_copy():
    """Preserve the independently declared matrix-copy input roster."""
    rows, groups = ([], [])

    def hist(variant, times):
        record = series({"__name__": "h"}, [(t, 1.0, "float_histogram") for t in times])
        for i, point in enumerate(record["points"]):
            h = point["float_histogram"]
            h.update(
                reset_hint=0,
                schema=0,
                zero_threshold=bits(0.0),
                zero_count=bits(0.0),
                count=bits(1.0),
                sum=bits(1.0),
                positive_spans=[{"offset": 0, "length": 1}],
                positive_buckets=[bits(1.0)],
                negative_spans=None,
                negative_buckets=None,
                custom_values=None,
            )
            if variant.startswith("custom"):
                h.update(
                    schema=-53,
                    zero_threshold=bits(0.001 * (i + 1)),
                    custom_values=[bits(1.0)],
                )
            if variant == "custom-unused-fields":
                h.update(
                    zero_count=bits(7.0 + i),
                    negative_spans=[{"offset": 1, "length": 1}],
                    negative_buckets=[bits(2.0)],
                )
            elif variant == "custom-upper":
                h["positive_spans"][0]["offset"] = 2
            elif variant == "exponential-stray-bounds":
                h["custom_values"] = [bits(17.0)]
        return record

    def add(family, variant, phase, query, times, mode="instant"):
        ident = f"matrix-native-copy-{len(rows):04d}-{family}-{variant}-{phase}-{mode}"
        rows.append(
            {
                "request": {
                    "id": ident,
                    "query": query,
                    "profile": "upstream-tests",
                    "mode": mode,
                    "time_ms": 2000,
                    "start_ms": 2000,
                    "end_ms": 3000,
                    "step_ms": 1000,
                    "lookback_ms": 1000,
                    "subquery_step_ms": 500,
                },
                "dataset": {"schema_version": 1, "series": [hist(variant, times)]},
                "faults": {},
            }
        )
        groups.append(
            {
                "id": ident,
                "family": family,
                "variant": variant,
                "phase": phase,
                "query": query,
                "times_ms": list(times),
                "mode": mode,
            }
        )

    variants = ("custom-unused-fields", "custom-upper", "exponential-stray-bounds")
    for variant in variants:
        for phase, times in (("buffered", (500, 1500)), ("exact-maxt", (500, 2000))):
            for family, query in (
                ("raw-matrix", "h[2s]"),
                ("changes", "changes(h[2s])"),
                ("resets", "resets(h[2s])"),
                ("rate", "rate(h[2s])"),
            ):
                add(family, variant, phase, query, times)
    for modifier in ("anchored", "smoothed"):
        pairs = (
            (("buffered", (-500, 500, 1500)), ("exact-maxt", (-500, 500, 2000)))
            if modifier == "anchored"
            else (
                ("buffered", (-500, 500, 1500, 2500)),
                ("exact-maxt", (-500, 500, 1500, 3000)),
            )
        )
        for phase, times in pairs:
            for function in ("rate", "increase"):
                add(
                    "extended-" + modifier,
                    "custom-upper",
                    phase,
                    f"{function}(h[2s] {modifier})",
                    times,
                )
    for variant in variants:
        add(
            "retained-overlap",
            variant,
            "exact-first-then-retained",
            "changes(h[3s])",
            (500, 2000),
            "range",
        )
    for modifier in ("anchored", "smoothed"):
        add(
            "raw-matrix-extended",
            "custom-unused-fields",
            "buffered",
            "h[2s] " + modifier,
            (-500, 500, 1500, 2500),
        )
    for phase, times in (("buffered", (500, 1500)), ("exact-maxt", (500, 2000))):
        add(
            "statistics-before-copy",
            "custom-upper",
            phase,
            "histogram_count(rate(h[2s]))",
            times,
        )
    assert len(rows) == 39 and len({r["request"]["id"] for r in rows}) == len(rows)
    return rows


def _instant_copy():
    """Preserve the independently declared instant-copy input roster."""
    rows, groups = ([], [])

    def hist(variant, times):
        record = series({"__name__": "h"}, [(t, 1.0, "float_histogram") for t in times])
        for i, point in enumerate(record["points"]):
            h = point["float_histogram"]
            h.update(
                reset_hint=0,
                schema=0,
                zero_threshold=bits(0.0),
                zero_count=bits(0.0),
                count=bits(1.0),
                sum=bits(1.0),
                positive_spans=[{"offset": 0, "length": 1}],
                positive_buckets=[bits(1.0)],
                negative_spans=None,
                negative_buckets=None,
                custom_values=None,
            )
            if variant.startswith("custom"):
                h.update(
                    schema=-53,
                    zero_threshold=bits(0.001 * (i + 1)),
                    custom_values=[bits(1.0)],
                )
            if variant == "custom-unused-fields":
                h.update(
                    zero_count=bits(7.0 + i),
                    negative_spans=[{"offset": 1, "length": 1}],
                    negative_buckets=[bits(2.0)],
                )
            elif variant == "custom-upper":
                h["positive_spans"][0]["offset"] = 2
            elif variant == "exponential-stray-bounds":
                h["custom_values"] = [bits(17.0)]
        return record

    def add(family, variant, phase, query, times, mode="instant"):
        ident = f"matrix-native-copy-{len(rows):04d}-{family}-{variant}-{phase}-{mode}"
        rows.append(
            {
                "request": {
                    "id": ident,
                    "query": query,
                    "profile": "upstream-tests",
                    "mode": mode,
                    "time_ms": 2000,
                    "start_ms": 2000,
                    "end_ms": 3000,
                    "step_ms": 1000,
                    "lookback_ms": 1000,
                    "subquery_step_ms": 500,
                },
                "dataset": {"schema_version": 1, "series": [hist(variant, times)]},
                "faults": {},
            }
        )
        groups.append(
            {
                "id": ident,
                "family": family,
                "variant": variant,
                "phase": phase,
                "query": query,
                "times_ms": list(times),
                "mode": mode,
            }
        )

    for variant in ("custom-upper", "custom-unused-fields"):
        for phase, times in (("buffered", (500, 1500)), ("exact-maxt", (500, 2000))):
            for function in ("irate", "idelta"):
                add("instant-" + function, variant, phase, f"{function}(h[2s])", times)
    assert len(rows) == 8 and len({r["request"]["id"] for r in rows}) == len(rows)
    return rows


def _native_recovery():
    """Declare reached reads independently of source and candidate outcomes."""
    rows = []

    def histogram(variant):
        value = {
            "reset_hint": 0,
            "schema": 0,
            "zero_threshold": bits(0.0),
            "zero_count": bits(0.0),
            "count": bits(1.0),
            "sum": bits(1.0),
            "positive_spans": [{"offset": 0, "length": 1}],
            "positive_buckets": [bits(1.0)],
            "negative_spans": None,
            "negative_buckets": None,
            "custom_values": None,
        }
        if variant == "short-high":
            value.update(schema=1, positive_spans=[{"offset": 0, "length": 2}])
        elif variant == "empty":
            value["positive_buckets"] = []
        elif variant.startswith("custom"):
            value.update(schema=-53, custom_values=[bits(1.0)])
            if variant == "custom-negative":
                value["positive_spans"][0]["offset"] = -1
            elif variant == "custom-wide":
                value["positive_spans"] = [
                    {"offset": -(1 << 31), "length": 0},
                    {"offset": -(1 << 31), "length": 1},
                ]
            elif variant == "custom-other":
                value["custom_values"] = [bits(2.0)]
        return value

    def record(labels, points):
        result = series(labels, [(time, 1.0, "float_histogram") for time, _ in points])
        for point, (_, variant) in zip(result["points"], points, strict=True):
            point["float_histogram"] = histogram(variant)
        return result

    def add(family, query, records, modes=("instant", "range")):
        for profile in ("server-default", "upstream-tests"):
            for mode in modes:
                rows.append(
                    {
                        "request": {
                            "id": f"native-recovery-{len(rows):04d}-{family}-{profile}-{mode}",
                            "query": query,
                            "profile": profile,
                            "mode": mode,
                            "time_ms": 2000,
                            "start_ms": 2000,
                            "end_ms": 3000,
                            "step_ms": 1000,
                            "lookback_ms": 1000,
                            "subquery_step_ms": 500,
                        },
                        "dataset": {"schema_version": 1, "series": deepcopy(records)},
                        "faults": {},
                    }
                )

    pairs = {
        "lowering": ("short-high", "ordinary"),
        "raising": ("ordinary", "short-high"),
        "custom-negative": ("custom-negative", "custom-other"),
        "custom-wide": ("custom-wide", "custom-other"),
        "empty-receiver": ("empty", "ordinary"),
    }
    for family, (first, second) in pairs.items():
        records = [record({"__name__": "h"}, [(500, first), (1500, second)])]
        for query in (
            "sum_over_time(h[2s])",
            "avg_over_time(h[2s])",
            "histogram_count(sum_over_time(h[2s]))",
        ):
            add("range-" + family, query, records)
    for family in ("custom-negative", "custom-wide", "empty-receiver"):
        first, second = pairs[family]
        records = [
            record({"__name__": "h"}, [(2000, first)]),
            record({"__name__": "g"}, [(2000, second)]),
        ]
        for query in ("h+g", "h-g", "histogram_count(h+g)"):
            add("binary-" + family, query, records)
    for family, (first, second) in pairs.items():
        records = [
            record({"__name__": "h", "id": "0"}, [(2000, first)]),
            record({"__name__": "h", "id": "1"}, [(2000, second)]),
        ]
        for query in ("sum(h)", "avg(h)", "histogram_count(sum(h))"):
            add("aggregate-" + family, query, records, ("instant",))
    return rows


_BUILDERS = {
    "unvisited-primary": _primary,
    "unvisited-supplement": _unvisited_supplement,
    "unvisited-phases": _unvisited_phases,
    "native-valid": _native_valid,
    "native-checked": _native_checked,
    "native-notes": _native_notes,
    "matrix-copy": _matrix_copy,
    "instant-copy": _instant_copy,
    "native-recovery": _native_recovery,
}

_INPUTS = {
    "unvisited-primary": (
        1512,
        "2d87ed65e4cdc9709c6c11f8551927af92b64ce04fcd5ab561717f01ba4f2513",
    ),
    "unvisited-supplement": (
        1044,
        "a09bbee12aec4b729dd9612c9a1c8b8774532d685d095254c9e03fc7a02235a7",
    ),
    "unvisited-phases": (
        336,
        "f3a6d23e0438259e361daccac4010d236d749c7116fb028ef9ff6346c0f457bb",
    ),
    "native-valid": (
        24,
        "83e4e882441516a2b2e8a6910ed61a96d8833c6987a346d6ff3b1e5e51fb78bb",
    ),
    "native-checked": (
        123,
        "6959234668c9bd7f4132a4b4f0a3ee96096b31dbd22367268c3eb4a1a48019ce",
    ),
    "native-notes": (
        17,
        "b93fce8f4d7d474e13eb770eb96b66bc3c80fbb4547a4893efd783bf5d28111e",
    ),
    "matrix-copy": (
        39,
        "c5cd78619f8c331214123edbbdb06dd0d1da38130971a60f2daf73dfbde63d61",
    ),
    "instant-copy": (
        8,
        "98699e78eead7a4b3e2f8300e73a51213f5da8f77154c940841ffc3699635120",
    ),
    "native-recovery": (
        126,
        "31e38d0726a8b43948ebe7015be6fa1dff036642d352c8bd23634152f3ff19e5",
    ),
}

PANELS = tuple(_BUILDERS)


def cases(panel):
    """Return fresh, digest-bound inputs; no candidate or expected result is read."""
    if panel not in _BUILDERS:
        raise ValueError("unknown native matrix panel")
    rows = _BUILDERS[panel]()
    count, expected = _INPUTS[panel]
    encoded = json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()
    if (
        len(rows) != count
        or len({row["request"]["id"] for row in rows}) != count
        or hashlib.sha256(encoded).hexdigest() != expected
    ):
        raise ValueError("declared native matrix input roster changed")
    return rows
