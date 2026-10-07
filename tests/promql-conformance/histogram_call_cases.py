"""Original-query inputs exercise call-map aliases, silent steps and integer wrapping."""
import base64
import struct


def cases():
    def encoded(value):
        return base64.b64encode(value.encode()).decode()

    def number(value):
        return struct.pack(">d", value).hex()

    dataset = {"schema_version": 1, "series": []}
    for bound, values in [("1", [3, 3, 3, 3]), ("2", [2, 3, 2, 3]), ("+Inf", [3, 3, 3, 3])]:
        dataset["series"].append({"labels": [
            {"name": encoded("__name__"), "value": encoded("m")},
            {"name": encoded("le"), "value": encoded(bound)},
        ], "points": [{"time_ms": index * 1000, "kind": "float", "float": number(value)}
                      for index, value in enumerate(values)]})
    queries = [
        "histogram_quantile(0.1,m)",
        "histogram_quantile(scalar(histogram_quantile(0.1,m)),m)",
        "histogram_quantile(0.1,histogram_quantile(0.1,m))",
        "min(histogram_quantile(0.1,m))",
        "sum(histogram_quantile(0.1,m))",
        "histogram_fraction(0,1,histogram_quantile(0.1,m))",
    ]
    windows = [(0, 2000, 1000), (1000, 3000, 1000), (1000, 2000, 1000),
               (0, 3000, 1000), (2000, 2000, 1000), (0, 100000, 1000)]
    requests = []
    for profile in ["server-default", "upstream-tests"]:
        for query_index, query in enumerate(queries):
            for window_index, (start, end, step) in enumerate(windows):
                requests.append({"id": f"histogram-call:{profile}:{query_index}:{window_index}",
                                 "query": query, "profile": profile, "mode": "range", "time_ms": 0,
                                 "start_ms": start, "end_ms": end, "step_ms": step,
                                 "lookback_ms": 300000, "subquery_step_ms": 60000})
    return dataset, requests
