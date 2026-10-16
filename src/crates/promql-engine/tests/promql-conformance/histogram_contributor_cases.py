"""Histogram consumers of grouped outputs, including local clocks and fallbacks."""

from copy import deepcopy

from group_contributor_cases import OPERATORS
from info_cases import series
from plural_histogram_cases import cases as plural_cases


def cases():
    native = plural_cases()[0]["series"]
    native = next(row["points"] for row in native if row["points"][0]["kind"] == "float_histogram")
    records = []
    for job in (3, 1, 2, 0):
        for worker in (1, 0):
            for metric in ("buckets", "mixed", "sparse"):
                for bound, increment in (("1", 1), ("2", 2), ("+Inf", 4)):
                    points = [(tick * 1000, increment * (worker + 1) * (tick + 1), "float")
                              for tick in range(6)
                              if metric != "sparse" or job != 1 or tick >= 3]
                    records.append(series({"__name__": metric, "job": f"j{job}",
                                           "le": bound, "worker": f"w{worker}"}, points))
            for metric in ("native", "mixed"):
                record = series({"__name__": metric, "job": f"j{job}",
                                 "worker": f"w{worker}"}, [])
                record["points"] = deepcopy(native)
                records.append(record)

    requests = []

    def add(name, expression, profile="upstream-tests", modes=("instant", "range")):
        for delayed in (False, True):
            for mode in modes:
                requests.append({"id": f"histogram-contributors:{name}:{profile}:d{int(delayed)}:{mode}",
                                 "query": expression, "profile": profile,
                                 "delayed_name_removal": delayed, "mode": mode,
                                 "time_ms": 3000, "start_ms": 2000, "end_ms": 4000,
                                 "step_ms": 1000, "lookback_ms": 300000,
                                 "subquery_step_ms": 60000})

    consumers = {
        "quantile": lambda child: f"histogram_quantile(.5,{child})",
        "fraction": lambda child: f"histogram_fraction(0,2,{child})",
        "quantiles": lambda child: f'histogram_quantiles({child},"q",.25,.75)',
    }
    for operator in OPERATORS:
        for name, consume in consumers.items():
            add(f"{name}-{operator}-classic", consume(f"{operator} by(job,le)(buckets)"))
            for kind in ("native", "mixed"):
                add(f"{name}-{operator}-{kind}", consume(f"{operator} by(job,le)({kind})"),
                    modes=("instant",))

    children = {
        "rate": "sum by(job,le)(rate(buckets[3s]))",
        "without": "sum without(worker)(buckets)",
        "inner-map": "sum by(job,le)(abs(buckets*2))",
        "outer-map": "abs(sum by(job,le)(buckets)*2)",
        "nested": "sum by(job,le)(avg by(job,le,worker)(buckets))",
        "sparse": "sum by(job,le)(sparse)",
        "fixed": "sum by(job,le)(buckets @ 2)",
        "offset": "sum by(job,le)(buckets offset 1s)",
        "empty": "sum by(job,le)(missing)",
        "unsupported-sort": "sort(sum by(job,le)(buckets))",
        "unsupported-topk": "topk by(job,le)(1,buckets)",
        "subquery-barrier": "sum_over_time((sum by(job,le)(buckets))[2s:1s])",
        "grouping-alias": "sum by(job,le,job)(buckets)",
    }
    for name, consume in consumers.items():
        for shape, child in children.items():
            add(f"{name}-{shape}", consume(child))
        body = consume(children["rate"])
        for shape, expression in {
            "scalar-wrapper": f"scalar(sum({body}))",
            "outer-wrapper": f"sum(abs({body}*2))",
            "local-subquery": f"sum_over_time(({body})[2s:1s])",
            "local-fixed-subquery": f"sum_over_time(({body})[2s:1s] @ 3)",
        }.items():
            add(f"{name}-{shape}", expression)
        if name != "quantiles":
            add(f"{name}-stable", consume(children["rate"]), profile="server-default")
        for shape, end in (("equal-bound", 3000), ("partial-step", 3001)):
            add(f"{name}-{shape}", body, modes=("range",))
            for request in requests[-2:]:
                request.update(start_ms=3000, end_ms=end)
    for destination in ("le", "job", "__name__", ""):
        add(f"quantiles-destination-{destination}",
            f'histogram_quantiles(sum by(job,le)(buckets),"{destination}",.25,.75)')
    add("scalar-argument", "histogram_quantile(scalar(sum(histogram_fraction(0,2,sum by(job,le)(buckets))))/4,sum by(job,le)(buckets))")
    return {"schema_version": 1, "series": records}, requests
