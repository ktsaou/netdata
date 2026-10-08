"""Independent temporal quantile inputs; unchanged Prometheus supplies outcomes."""

import math
import random
from copy import deepcopy

from count_values_cases import STALE, WINDOWS
from info_cases import bits, series
from mad_cases import arrays


def cases():
    output = []

    def add(ident, records, queries, windows=WINDOWS[:1], faults=None):
        for profile in ("server-default", "upstream-tests"):
            for qi, query in enumerate(queries):
                for wi, (mode, time, start, end, step) in enumerate(windows):
                    output.append({
                        "request": {
                            "id": f"temporal-quantile:{ident}:{profile}:{qi}:{wi}",
                            "query": query, "profile": profile, "mode": mode,
                            "time_ms": time, "start_ms": start, "end_ms": end,
                            "step_ms": step, "lookback_ms": 1, "subquery_step_ms": 1000,
                        },
                        "dataset": {"schema_version": 1, "series": deepcopy(records)},
                        "faults": deepcopy(faults or {}),
                    })

    def record(values):
        count = len(values)
        return series(
            {"__name__": "m", "job": "one", "__type__": "gauge", "__unit__": "seconds"},
            [((index - count + 1) * 1000, value, "float") for index, value in enumerate(values)],
        )

    basic = tuple(f"quantile_over_time({q},m[1h])" for q in ("0", "0.5", "1"))
    for item in arrays():
        add("array-" + item["id"], [record(item["bits"])], basic)

    # Distinct raw payloads expose source-sort provenance, including its heap fallback.
    for count in (1, 2, 3, 12, 13, 49, 50, 51, 64, 127, 128, 2048):
        values = [f"{0x7ff8000000000001 + index:016x}" for index in range(count)]
        quantiles = ("0", "0.1", "0.5", "0.9", "1", "-1", "2", "NaN")
        add(f"all-nan-{count}", [record(values)],
            tuple(f"quantile_over_time({q},m[1h])" for q in quantiles))
        signed = [f"{0x7ff0000000000010 + index | ((index % 2) << 63):016x}"
                  for index in range(count)]
        add(f"signed-signaling-nan-{count}", [record(signed)], basic)

    randomizer = random.Random(81297)
    pool = [bits(value) for value in (-math.inf, -3.0, -0.0, 0.0, 1.0, math.inf)]
    pool += ["7ff8000000000042", "fff8000000000043", "7ff0000000000044", "fff0000000000045"]
    for index in range(32):
        count = (12, 13, 49, 50, 51, 64, 127, 128)[index % 8]
        add(f"mixed-ties-{index}", [record([randomizer.choice(pool) for _ in range(count)])], basic)

    changing = [
        series({"__name__": "m", "job": "one"},
               [(-2000, -0.0, "float"), (-1000, 0.0, "float"),
                (0, 7.0, "float"), (1000, STALE, "float"), (2000, -3.0, "float")]),
        series({"__name__": "q"},
               [(0, 0.5, "float"), (1000, -1.0, "float"), (2000, 2.0, "float")]),
    ]
    add("changing", changing, (
        "quantile_over_time(scalar(q),m[1h])",
        "quantile_over_time(time()/2,m[2s])",
        "quantile_over_time(0.5,m[1h] @start())",
        "quantile_over_time(0.5,m[1h] @end() offset step())",
        "quantile_over_time(NaN,m[3s:1s])",
        "quantile_over_time(0.5,m[3s:1s])",
        "quantile_over_time(0.5,(m*1)[3s:1s])",
        "1/quantile_over_time(1,m[3s]) < bool 0",
        "sum(quantile_over_time(0.5,m[3s]))",
        "timestamp(quantile_over_time(0.5,m[3s]))",
    ), WINDOWS)
    queries = tuple(f"quantile_over_time({q},m[1h])" for q in ("-1", "2", "NaN", "0.5"))
    add("empty", [], queries, WINDOWS)
    add("stale", [record([STALE, STALE])], queries, WINDOWS)
    for kind in ("histogram", "float_histogram"):
        for ident, points in {
            "native-only": [(-1000, 1, kind), (0, 7, kind)],
            "float-native": [(-1000, 7, "float"), (0, 2, kind)],
            "native-float": [(-1000, 2, kind), (0, -3, "float")],
            "nan-native": [(-1000, "fff8000000000042", "float"), (0, 2, kind)],
            "native-stale": [(-1000, 2, kind), (0, STALE, "float")],
        }.items():
            add(f"{kind}-{ident}", [series({"__name__": "m", "job": "one"}, points)],
                queries + ("quantile_over_time(NaN,m[3s:1s])",), WINDOWS)
    for ident, settings in (
        ("selection", {"set_error": "temporal quantile selection fault"}),
        ("iterator", {"next_times": [0], "message": "temporal quantile iterator fault"}),
        ("seek", {"seek_targets": [-3599999], "message": "temporal quantile seek fault"}),
    ):
        add("fault-" + ident, [record(["7ff8000000000042", 7.0, 11.0])], queries,
            WINDOWS, {"m": settings})
    return output
