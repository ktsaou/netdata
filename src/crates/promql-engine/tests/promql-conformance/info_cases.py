"""Independent INFO inputs for labels, scopes, native kinds and timestamp encoding."""
import base64
import struct


def bits(value):
    return struct.pack(">d", value).hex()


def series(labels, points):
    encoded = lambda value: base64.b64encode(value if isinstance(value, bytes) else value.encode()).decode()
    output = []
    for time, value, kind in points:
        point = {"time_ms": time, "kind": kind}
        value = value if isinstance(value, str) else bits(value)
        if kind == "float":
            point["float"] = value
        else:
            integer = kind == "histogram"
            point[kind] = {"reset_hint": 0, "schema": 0, "zero_threshold": bits(0.001),
                           "zero_count": 1 if integer else bits(1),
                           "count": 1 if integer else bits(1), "sum": value,
                           "positive_spans": None, "negative_spans": None,
                           "positive_buckets": None, "negative_buckets": None,
                           "custom_values": None}
        output.append(point)
    return {"labels": [{"name": encoded(name), "value": encoded(value)}
                       for name, value in labels.items()], "points": output}


def cases():
    groups = []
    stale = "7ff0000000000002"
    base_labels = {"__name__": "metric", "instance": "a", "job": "worker"}
    info_labels = {"__name__": "target_info", "instance": "a", "job": "worker", "data": "info"}
    base_points = [(0, 10, "float"), (1000, 20, "float"), (2000, 30, "float")]
    info_points = [(0, 99, "float"), (1000, 99, "float"), (2000, 99, "float")]
    windows = [("instant", 0, 0, 0, 1000), ("instant", 2000, 0, 0, 1000),
               ("range", 0, 0, 0, 1000), ("range", 0, 0, 2000, 1000)]

    def add(ident, records, queries=("info(metric)",), contexts=windows):
        requests = []
        for qi, query in enumerate(queries):
            for wi, (mode, time, start, end, step) in enumerate(contexts):
                requests.append({"id": f"info:{ident}:{qi}:{wi}", "query": query,
                                 "profile": "upstream-tests", "mode": mode, "time_ms": time,
                                 "start_ms": start, "end_ms": end, "step_ms": step,
                                 "lookback_ms": 300000, "subquery_step_ms": 60000})
        groups.append({"id": ident, "dataset": {"schema_version": 1, "series": records},
                       "requests": requests})

    ordinary = [series(base_labels, base_points), series(info_labels, info_points)]
    add("default", ordinary, ("info(metric)", 'info(metric,{data=~".*"})',
        'info(metric,{data=~".+"})', 'info(metric,{data="missing"})',
        "info(info(metric))", "timestamp(info(metric))", "start_timestamp(info(metric))",
        "sum(info(metric))", "abs(info(metric))", "info(metric+0)",
        "info(metric @start())", "info(metric @end())", "info(metric offset 1s)",
        "info(metric @end() offset step())"))
    add("changing", [ordinary[0], series(dict(info_labels, data="old"), [(0, 1, "float")]),
        series(dict(info_labels, data="new"), [(1000, 1, "float"), (2000, 1, "float")])])
    add("duplicates", [ordinary[0], series(dict(info_labels, data="one"), info_points),
        series(dict(info_labels, data="two"), info_points)])
    add("empty-step-duplicates", [series(base_labels, [(0, stale, "float"), (1000, 1, "float")]),
        series(dict(info_labels, data="old"), [(0, 1, "float")]),
        series(dict(info_labels, data="new"), [(0, 1, "float"), (1000, 1, "float")])])
    add("empty-whole-base", [series(base_labels, [(0, stale, "float")]),
        series(info_labels, [(0, 1, "float_histogram")])])
    for kind in ("histogram", "float_histogram"):
        add("native-info-" + kind, [ordinary[0], series(info_labels, [(0, 1, kind)])])
        add("native-empty-step-" + kind, [series(base_labels, [(0, stale, "float"), (1000, 1, "float")]),
            series(info_labels, [(0, 1, kind), (1000, 1, "float")])])
        add("stale-native-" + kind, [ordinary[0], series(info_labels, [(0, stale, kind)])])
        add("native-base-" + kind, [series(base_labels, [(0, 2, kind), (1000, 3, kind)]), ordinary[1]],
            ("info(metric)", "histogram_count(info(metric))", "start_timestamp(info(metric))"))
    for presence in ("job", "instance", "both", "neither", "empty"):
        labels = dict(base_labels)
        inf = dict(info_labels)
        if presence in ("job", "neither"):
            labels.pop("instance")
            inf.pop("instance")
        if presence in ("instance", "neither"):
            labels.pop("job")
            inf.pop("job")
        if presence == "empty":
            labels["job"] = ""
            inf["job"] = ""
        add("presence-" + presence, [series(labels, base_points), series(inf, info_points)])
    pattern_records = []
    for label in ("job", "instance"):
        pattern_records += [series({"__name__": "metric", label: "a"}, base_points),
                            series({"__name__": "target_info", label: "a", "data": label}, info_points)]
    pattern_records += ordinary
    add("all-presence-patterns", pattern_records)
    add("base-label", [series(dict(base_labels, data="base"), base_points), ordinary[1]])
    add("base-empty-label", [series(dict(base_labels, data=""), base_points), ordinary[1]])
    multiple = [ordinary[0], ordinary[1], series(dict(info_labels, __name__="other_info", version="v"), info_points)]
    add("names", multiple, ('info(metric,{__name__=~".*_info"})',
        'info(metric,{__name__!="missing"})', 'info(metric,{__name__!~"other.*"})',
        'info(metric,{__name__=~".*_info",__name__!="other_info"})',
        'info(metric,{__name__="other_info",version=~".*"})'))
    add("conflict", [ordinary[0], series(dict(info_labels, data="one"), info_points),
        series(dict(info_labels, __name__="other_info", data="two"), info_points)],
        ('info(metric,{__name__=~".*_info"})',))
    for index, value in enumerate(("a.+*?()|[]{}^$\\", "λ🙂", "a\n\x00", b"\xff", b"\xff+")):
        add(f"identifying-bytes-{index}", [series(dict(base_labels, instance=value), base_points),
            series(dict(info_labels, instance=value), info_points)])
    add("data-bytes", [ordinary[0], series(dict(info_labels, data=b"\xff\x00\n"), info_points)])
    add("empty-name", [ordinary[0], series({"instance": "a", "job": "worker", "data": "nameless"}, info_points)],
        ('info(metric,{__name__="",data=~".*"})',))
    add("ignored-info-base", ordinary, ('info({__name__=~"metric|target_info"})',))
    for index, value in enumerate(("8000000000000000", "7ff0000000000000", "fff0000000000000", "fff8000000000042")):
        add(f"base-bits-{index}", [series(base_labels, [(0, value, "float")]), ordinary[1]])
    huge = 1 << 53
    add("timestamp-rounding", [series(base_labels, [(huge + 2, 1, "float")]),
        series(dict(info_labels, data="one"), [(huge, 1, "float")]),
        series(dict(info_labels, data="two"), [(huge + 1, 1, "float")])],
        contexts=[("instant", huge + 2, 0, 0, 1000)])
    add("negative-time", [series(base_labels, [(-1000, 1, "float")]),
        series(info_labels, [(-1001, 1, "float")])],
        contexts=[("instant", -1000, 0, 0, 1000), ("range", 0, -1000, 0, 1000)])
    fixed_contexts = [windows[0], windows[1], windows[3]]
    for ident, reference, older, newer in (
            ("positive", "9007199254740.994", huge, huge + 1),
            ("negative", "-9007199254740.992", -huge - 1, -huge)):
        query = f'info(absent(missing{{instance="a",job="worker"}} @{reference}))'
        add(f"timestamp-fixed-{ident}-duplicate", [
            series(dict(info_labels, data="one"), [(older, 1, "float")]),
            series(dict(info_labels, data="two"), [(newer, 1, "float")])],
            (query,), contexts=fixed_contexts)
        add(f"timestamp-fixed-{ident}-newest", [
            series(dict(info_labels, data="old"), [(older - 2, 1, "float")]),
            series(dict(info_labels, data="new"), [(newer, 1, "float")])],
            (query,), contexts=fixed_contexts)
    return groups
