"""Independent selection reducer inputs; all outcomes come from the original engine."""
from copy import deepcopy

from count_values_cases import FIRST, SECOND, STALE, WINDOWS
from info_cases import series


def cases():
    inputs = []

    def add(ident, records, queries, windows=WINDOWS, faults=None, lookback_ms=1,
            profiles=("server-default", "upstream-tests")):
        for profile in profiles:
            for qi, query in enumerate(queries):
                for wi, (mode, time, start, end, step) in enumerate(windows):
                    inputs.append({"request": {"id": f"topk:{ident}:{profile}:{qi}:{wi}",
                        "query": query, "profile": profile, "mode": mode, "time_ms": time,
                        "start_ms": start, "end_ms": end, "step_ms": step,
                        "lookback_ms": lookback_ms, "subquery_step_ms": 1000},
                        "dataset": {"schema_version": 1, "series": deepcopy(records)},
                        "faults": deepcopy(faults or {})})

    ordinary = [series({"__name__": "m", "job": "one" if index % 2 else "two", "z": str(index)},
        [(0, value, "float"), (1000, 6 - index, "float"),
         (2000, STALE if index == 0 else index % 3, "float")])
        for index, value in enumerate((0.0, -0.0, 2, 2, -1, 6, 1))]
    for operator in ("topk", "bottomk"):
        add("parameters-" + operator, ordinary,
            tuple(f"{operator}({k},m)" for k in
                ("-Inf", "-2", "-0.9", "-0", "0", "0.9", "1", "1.9", "2", "7", "8",
                 "NaN", "Inf", "9223372036854774784", "9223372036854773760",
                 "-9223372036854775808", "-9223372036854774784")))
        add("groups-" + operator, ordinary,
            (f"{operator} by(job)(2,m)", f"{operator} without(z)(2,m)",
             f"{operator} by(job,job)(2,m)", f"{operator} without()(2,m)",
             f"{operator} by(__name__)(1,m)", f"{operator} by()(2,m)"))
        add("parents-" + operator, ordinary,
            (f"sum({operator}(2,m))", f"count({operator}(2,m))", f"abs({operator}(2,m))",
             f"timestamp({operator}(2,m))", f"{operator}(1,{operator}(3,m))",
             f'count_values("v",{operator}(3,m))', f"{operator}(2,m) + on(z) m",
             f'label_replace({operator}(2,m),"new","$1","job","(.*)")'))
        add("fixed-" + operator, ordinary,
            (f"{operator}(time()+1,m @0)", f"{operator}(time()+1,m @end())",
             f"{operator}(2,m offset 1s)", f"({operator}((2),m))",
             f"{operator}(scalar(m{{z=\"2\"}} @end()),m)",
             f"{operator}(scalar(timestamp(m{{z=\"2\"}} @0))+1,m)",
             f"{operator}(scalar(timestamp((m{{z=\"2\"}} @0)))+1,m)"))
        add("empty-" + operator, [], (f"{operator}(2,m)", f"{operator}(NaN,m)",
             f"{operator}(Inf,m)", f"{operator}(-Inf,m)", f"{operator}(0,m)"))
        add("stale-" + operator, [series({"__name__": "m"}, [(0, STALE, "float")])],
            (f"{operator}(1,m)",))
        add("body-producers-" + operator, ordinary,
            (f"{operator}(2,rate(m[2s]))", f"{operator}(2,avg by(job)(m))",
             f"{operator}(2,sum by(z)(m))", f'{operator}(2,count_values("v",m))',
             f"{operator}(2,delta(m[2s]))", f"{operator}(2,abs(m))"))
        add("contextual-parameter-" + operator, ordinary,
            tuple(f"{operator}({function}()+1,m)" for function in ("start", "end", "range", "step")),
            profiles=("upstream-tests",))
        clock = [series({"__name__": "p"},
            [(time, value, "float") for time, value in
             ((-2000, 1), (-1000, 2), (0, 4), (1000, 8), (2000, 16))])]
        add("parameter-clock-" + operator, ordinary + clock,
            (f"{operator}(scalar(timestamp(p @0))+2,m)",
             f"{operator}(scalar(timestamp((p @0)))+2,m)",
             f"{operator}(scalar(delta(p[2s] @0))+2,m)",
             f"{operator}(scalar(rate(p[2s] @0))+2,m)",
             f"{operator}(scalar(predict_linear(p[2s] @0,1))+2,m)",
             f"{operator}(scalar({operator}(time()+1,p @0))+2,m)",
             f"{operator}(scalar({operator}(time()+1,p @0))+2,m @0)"),
            windows=WINDOWS + (("range", 0, 1000, 2000, 1000),))

    special = ["8000000000000000", "0000000000000000", "7ff8000000000063",
               "fff8000000000027", "7ff0000000000044", "7ff0000000000000",
               "fff0000000000000", "3ff0000000000000", "bff0000000000000"]
    for length in (1, 2, 3, 12, 13, 49, 50, 51, 127, 129, 1023, 1024, 1025):
        for pattern in ("ascending", "descending", "equal", "zeros", "nan", "special"):
            values = {"ascending": list(range(length)), "descending": list(range(length))[::-1],
                      "equal": [1] * length, "zeros": [special[index % 2] for index in range(length)],
                      "nan": [special[2 + index % 3] for index in range(length)],
                      "special": [special[index % len(special)] for index in range(length)]}[pattern]
            records = [series({"__name__": "m", "z": f"{index:05d}"}, [(0, value, "float")])
                       for index, value in enumerate(values)]
            parameters = sorted({1, 2, max(1, length // 2), length, length + 1})
            add(f"threshold-{length}-{pattern}", records,
                tuple(f"{operator}({k},m)" for operator in ("topk", "bottomk") for k in parameters),
                windows=WINDOWS[:1])

    for operator in ("topk", "bottomk"):
        for ident, values in (
            ("changing", (0, 2, 1)), ("negative", (-2, -1, -0.5)),
            ("fraction", (0.9, 1.9, 2.9)), ("nan", (1, "7ff8000000000063", 0)),
            ("nan-low", (0, "7ff8000000000063", -1)),
            ("overflow", (1, "43e0000000000000", 0)),
            ("underflow", (1, "c3e0000000000000", 0)),
            ("stale", (1, STALE, 2)),
        ):
            records = ordinary + [series({"__name__": "p"},
                [(time, value, "float") for time, value in zip((0, 1000, 2000), values, strict=True)])]
            add("parameter-sequence-" + operator + "-" + ident, records,
                (f"{operator}(scalar(p),m)", f"{operator}(scalar(p),m @0)",
                 f"count({operator}(scalar(p),m))"))
        for kind in ("histogram", "float_histogram"):
            records = ordinary + [series({"__name__": "m", "job": "native", "z": "native"},
                [(0, 10, kind), (1000, 20, "float"), (2000, 30, kind)])]
            add("native-" + operator + "-" + kind, records,
                (f"{operator}(2,m)", f"{operator} by(job)(2,m)", f"{operator}(0,m)",
                 f"{operator}(-Inf,m)", f"{operator}(NaN,m)", f"histogram_count({operator}(2,m))"))
            add("native-only-" + operator + "-" + kind, records[-1:],
                (f"{operator}(2,m)", f"{operator}(0,m)"))
        for k in ("0", "NaN", "Inf", "scalar(p)"):
            records = ordinary + [series({"__name__": "p"}, [(0, 1, "float"), (1000, 2, "float")])]
            for name in ("m", "p"):
                add(f"set-fault-{operator}-{k}-{name}", records, (f"{operator}({k},m)",),
                    faults={name: {"set_error": "selection fault"}})
            add(f"iterator-fault-{operator}-{k}", records, (f"{operator}({k},m)",),
                faults={"m": {"next_times": [1000], "message": "iterator fault"}})

    group_collision = [series({"__name__": "m", "a": b"b\xffx\xffy", "z": "a"}, [(0, 1, "float")]),
        series({"__name__": "m", "a": b"b", "x": "y", "z": "b"}, [(0, 2, "float")])]
    add("group-hash-alias", group_collision, ("topk by(a,x)(1,m)", "bottomk by(a,x)(1,m)"))
    for overlap in (False, True):
        records = [series({"a": FIRST}, [(0, 1, "float"), (1000, 1 if overlap else STALE, "float")]),
                   series({"a": SECOND}, [(0, 2 if overlap else STALE, "float"), (1000, 2, "float")])]
        for operator in ("topk", "bottomk"):
            inner = f'{operator}(2,{{a=~".+"}})'
            add(f"full-hash-alias-{operator}-{overlap}", records,
                (inner, f"count({inner})", f"sum({inner})", f"-{inner}", f"+{inner}",
                 f"abs({inner})", f"timestamp({inner})", f"{operator}(1,{inner})",
                 f'label_join({inner},"new","","a")',
                 f'label_replace({inner},"new","ok","a",".*")'))
    if len(inputs) != len({row["request"]["id"] for row in inputs}):
        raise ValueError("duplicate independently declared selection identity")
    return inputs


def grouped_input_cases():
    """Selection over paged groups, with complete outcomes from the original engine."""
    from base64 import b64decode
    from quantile_aggregate_cases import result_label_records

    ordinary = result_label_records(4, 96)
    aliases, mixed, histograms = [], [], []
    for index, row in enumerate(ordinary):
        labels = {
            b64decode(label["name"]): b64decode(label["value"])
            for label in row["labels"]
        }
        alias_labels = dict(labels)
        if index % 2:
            alias_labels.pop(b"empty")
        else:
            alias_labels[b"empty"] = b""
        aliases.append(series(alias_labels, [
            (time * 1000, index + time + 1, "float") for time in range(3)
        ]))
        histogram = series(labels, [
            (time * 1000, index + time + 1, "float_histogram") for time in range(3)
        ])
        histograms.append(histogram)
        mixed.append(deepcopy(histogram if index % 2 else row))

    result = []
    for operator, parameter, profiles in (
        ("topk", "2", ("server-default", "upstream-tests")),
        ("bottomk", "2", ("server-default", "upstream-tests")),
        ("limitk", "64", ("upstream-tests",)),
        ("limit_ratio", ".5", ("upstream-tests",)),
    ):
        queries = []
        for reducer in ("sum", "avg", "count", "min", "max", "group", "stdvar", "stddev"):
            for body in ("m", "rate(m[2s])"):
                queries.append(
                    f"sum({operator}({parameter},{reducer} by(job,empty)({body})))"
                )
        queries.extend((
            f"sum({operator}({parameter},abs(sum by(job,empty)(sum by(job,empty,member)(m)))))",
            f"count({operator}({'2' if operator != 'limit_ratio' else '.5'},sum by(job)(m)))",
            f"sum({operator}(0,sum by(job,empty)(m)))",
            f"sum({operator}(NaN,sum by(job,empty)(m)))",
            f"sum({operator}(Inf,sum by(job,empty)(m)))",
            f"sum({operator}(-Inf,sum by(job,empty)(m)))",
            f"sum({operator}(time()+1,sum by(job,empty)(m)))",
            f"sum({operator}({parameter},sum by(job,empty)(m @ 1)))",
            f"sum({operator}({parameter},sum by(job,empty)(m offset 1s)))",
            f"sum({operator}({parameter},sum by(job,empty)(timestamp(m))))",
            f"sum({operator}({parameter},sum by(job,empty)(m*scalar(m))))",
            f"sum_over_time((sum({operator}({parameter},sum by(job,empty)(m))))[2s:1s] @ 2)",
            f"sum(topk(scalar(count({operator}({parameter},sum by(job,empty)(m)))),sum by(job)(m)))",
        ))
        if operator in ("topk", "bottomk"):
            queries.append(f"{operator}(2,sum by(job,empty)(m))")
        if operator == "limit_ratio":
            queries.append("sum(limit_ratio(-.5,sum by(job,empty)(m)))")
        for shape, records in (
            ("empty", []),
            ("singleton", result_label_records(1)),
            ("ordinary", ordinary),
            ("raw-empty-aliases", aliases),
            ("mixed", mixed),
            ("native", histograms),
        ):
            for profile in profiles:
                for delayed in (False, True):
                    for mode in ("instant", "equal", "range"):
                        for index, query in enumerate(queries):
                            result.append({
                                "request": {
                                    "id": f"selection-group:{operator}:{shape}:{profile}:{delayed}:{mode}:{index}",
                                    "query": query,
                                    "profile": profile,
                                    "delayed_name_removal": delayed,
                                    "mode": "instant" if mode == "instant" else "range",
                                    "time_ms": 2000,
                                    "start_ms": 0 if mode == "range" else 2000,
                                    "end_ms": 2000,
                                    "step_ms": 1000,
                                    "lookback_ms": 1,
                                    "subquery_step_ms": 1000,
                                },
                                "dataset": {"schema_version": 1, "series": deepcopy(records)},
                                "faults": {},
                            })
    if len(result) != len({row["request"]["id"] for row in result}):
        raise ValueError("duplicate grouped selection input case")
    return result
