"""Literal histogram ordering, sparse ticks and explicit fallback expressions."""
from copy import deepcopy

from histogram_contributor_cases import cases as contributor_cases
from info_cases import series


def cases():
    dataset, requests = contributor_cases()
    stale = "7ff0000000000002"
    for index, job in enumerate(("a", "b", "c")):
        for bound, value in (("1", 1), ("+Inf", 2)):
            points = [(0, value, "float"), ((index + 1) * 1000, stale, "float"),
                      ((index + 4) * 1000, value, "float")]
            dataset["series"].append(series(
                {"__name__": "stream_sparse_bucket", "job": job, "le": bound}, points))
        for bound, value in (("1", 3), ("1.0", 2), ("+Inf", 3)):
            dataset["series"].append(series(
                {"__name__": "stream_equal_bucket", "job": job, "le": bound},
                [(tick * 1000, value, "float") for tick in range(7)]))
    dataset["series"].append(series({"__name__": "stream_q"},
        [(tick * 1000, .25 if tick % 2 else .75, "float") for tick in range(7)]))
    native = next(row["points"] for row in dataset["series"]
                  if row["points"] and row["points"][0]["kind"] == "float_histogram")
    for job in ("a", "b"):
        row = series({"__name__": "stream_native", "job": job}, [])
        row["points"] = deepcopy(native)
        dataset["series"].append(row)

    def add(name, expression, profile="server-default"):
        for delayed in (False, True):
            for mode, start, end in (("instant", 3000, 3000), ("range", 3000, 3000),
                                     ("range", 3000, 3001), ("range", 0, 6000)):
                requests.append({"id": f"histogram-stream:{name}:d{int(delayed)}:{mode}:{start}:{end}",
                                 "query": expression, "profile": profile,
                                 "delayed_name_removal": delayed, "mode": mode,
                                 "time_ms": start, "start_ms": start, "end_ms": end,
                                 "step_ms": 1000, "lookback_ms": 300000,
                                 "subquery_step_ms": 60000})

    for metric in ("stream_sparse_bucket", "stream_equal_bucket", "stream_native"):
        for function, consume in (
            ("quantile", lambda child: f"histogram_quantile(.5,{child})"),
            ("fraction", lambda child: f"histogram_fraction(0,2,{child})"),
        ):
            body = consume(metric)
            add(f"{function}-{metric}", body)
            add(f"{function}-{metric}-sum", f"sum({body})")
            add(f"{function}-{metric}-scalar", f"scalar(sum({body}))")
            add(f"{function}-{metric}-map", f"abs({body}*2)")
    for name, expression in {
        "dynamic-quantile": "histogram_quantile(scalar(stream_q),stream_sparse_bucket)",
        "dynamic-fraction": "histogram_fraction(scalar(stream_q),2,stream_sparse_bucket)",
        "plural": 'histogram_quantiles(stream_sparse_bucket,"q",.25,.75)',
        "wrapped-number": "histogram_quantile((.5),stream_sparse_bucket)",
        "arithmetic-parameter": "histogram_quantile(1/2,stream_sparse_bucket)",
        "invalid-number": "histogram_quantile(2,stream_sparse_bucket)",
        "selected-join": "histogram_quantile(.5,stream_sparse_bucket)+on(job)histogram_fraction(0,2,stream_sparse_bucket)",
        "rank-selected": "topk(1,histogram_quantile(.5,stream_sparse_bucket))",
        "fraction-both-dynamic": "histogram_fraction(scalar(stream_q),scalar(stream_q)+1,stream_sparse_bucket)",
        "empty": "histogram_quantile(.5,stream_missing)",
        "fixed": "histogram_quantile(.5,stream_sparse_bucket @ 2)",
        "subquery": "sum_over_time((histogram_quantile(.5,stream_sparse_bucket))[2s:1s])",
    }.items():
        add(name, expression, profile="upstream-tests" if name == "plural" else "server-default")
    return dataset, requests
