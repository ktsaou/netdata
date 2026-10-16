"""Fixed rate-acquisition scaling, fallback and empty-input controls."""
import re

from fold_join_cases import cases as window_cases
from info_cases import series
from mapped_rate_cases import _case
from pair_frontier_cases import performance_cases as pair_cases
from wrapped_group_cases import cases as wrapped_cases


def performance_cases():
    scaling = pair_cases()
    wrapped = wrapped_cases()
    window = window_cases()
    selected = [case for case in scaling if "-rate-abs-" in case["request"]["id"]]
    index = {case["request"]["id"]: case for case in scaling}
    for ident in [
        "pair-frontier-raw-root-g4-m1",
        "pair-frontier-raw-root-g64-m1",
        "pair-frontier-raw-root-g1024-m1",
        "pair-frontier-raw-abs-g1024-m1",
        "pair-frontier-leaf-abs-g1024-m1",
    ]:
        selected.append(index[ident])
    for needle in [
        "-histogram-server-default-0",
        "-float_histogram-server-default-0",
        "-sum-mixed-left-abs",
        "-sum-mixed-right-abs",
        "-sum-native-rate-abs",
        "-sum-all-native-rate-abs",
        "-sum-stale-rate-abs",
        "-sparse-server-default-0",
        "-instant-server-default-0",
    ]:
        selected.append(next(case for case in wrapped if needle in case["request"]["id"]))
    for name in ["increase", "delta", "avg_over_time"]:
        selected.append(next(case for case in window
                             if re.search(r"\b" + name + r"\s*\(", case["request"]["query"])))
    for name in ["increase", "delta"]:
        for count in [256, 1024]:
            original = index[f"pair-frontier-rate-abs-g{count}-m1"]
            request = {**original["request"],
                       "id": f"rate-acquisition-{name}-g{count}",
                       "query": original["request"]["query"].replace("rate(", name + "(")}
            selected.append({**original, "request": request})
    return selected


def empty_cases():
    output = []
    for state in ["empty", "float", "histogram", "float_histogram"]:
        points = [] if state == "empty" else [
            (time, "7ff0000000000002", state) for time in [1000, 2000]
        ]
        records = [series({"__name__": "m_total", "job": "worker"}, points)]
        for name in ["rate", "increase", "delta"]:
            for absent in [False, True]:
                expression = f"{name}(m_total[3s])"
                if absent:
                    expression = f"absent({expression})"
                for instant in [False, True]:
                    ident = f"rate-acquisition-{state}-{name}-{int(absent)}-{int(instant)}"
                    output.append(_case(ident, expression, records, instant=instant, end=3000))
    return output
