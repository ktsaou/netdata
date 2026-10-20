"""Native field maps over groups at ordinary range clocks and retained fallbacks."""

from copy import deepcopy

from group_contributor_cases import OPERATORS, _records
from info_cases import bits
from plural_histogram_cases import cases as plural_cases


def cases():
    custom = next(
        point["float_histogram"]
        for row in plural_cases()[0]["series"]
        for point in row["points"]
        if point["kind"] == "float_histogram"
    )
    panels = []
    for state in ("native", "mixed", "stale", "absent", "schema", "custom", "incompatible"):
        records = _records("all-native", groups=3, members=2)[:6]
        for index, record in enumerate(records):
            for point in record["points"]:
                tick = point["time_ms"] // 1000
                histogram = point["float_histogram"]
                if state == "mixed" and index == 1 and tick in (2, 3):
                    point.clear()
                    point.update(time_ms=tick * 1000, kind="float", float=bits(5))
                elif state == "stale" and index < 2 and tick in (2, 3):
                    histogram["sum"] = "7ff0000000000002"
                elif state == "schema":
                    histogram["schema"] = (index + tick) % 3
                    histogram["positive_spans"] = [{"offset": 0, "length": 2}]
                    histogram["positive_buckets"] = [bits(1), bits(2)]
                    histogram["count"] = bits(4)
                elif state in ("custom", "incompatible"):
                    point["float_histogram"] = deepcopy(custom)
                    point["float_histogram"]["sum"] = bits((index + 1) * (tick + 1))
                    if state == "incompatible" and index == 1:
                        point["float_histogram"]["custom_values"] = [bits(1), bits(3)]
            if state == "absent" and index < 2:
                record["points"] = []
        requests = []

        def add(expression, tag, mode="range", start=1000, end=4000):
            for delayed in (False, True):
                requests.append({
                    "id": f"native-group-map:{state}:{tag}:d{int(delayed)}",
                    "query": expression, "profile": "server-default",
                    "delayed_name_removal": delayed, "mode": mode, "time_ms": 3000,
                    "start_ms": start, "end_ms": end, "step_ms": 1000,
                    "lookback_ms": 300000, "subquery_step_ms": 60000,
                })

        for function in ("histogram_sum", "histogram_count", "histogram_avg"):
            for operator in OPERATORS:
                add(f"{function}({operator} by(zone)(m_total))", f"{function}-{operator}")
            body = f"{function}(sum by(zone)(m_total))"
            for tag, expression in {
                "wrapped": f"abs({body})",
                "nested": f"sum({body})",
                "without": f"{function}(avg without(member)(m_total))",
                "offset": f"{function}(sum by(zone)(m_total offset 1s))",
                "fixed": f"{function}(sum by(zone)(m_total @ 2))",
                "empty": f'{function}(sum by(zone)(m_total{{member="missing"}}))',
            }.items():
                add(expression, f"{function}-{tag}")
            add(body, f"{function}-instant", mode="instant")
            add(body, f"{function}-equal", start=3000, end=3000)
        panels.append({"id": state, "dataset": {"schema_version": 1, "series": records},
                       "requests": requests})
    return panels
