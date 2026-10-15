"""Per-series window folds, name preservation and global-absence fallbacks."""

from copy import deepcopy

from info_cases import series
from mapped_rate_cases import _case
from mapped_rate_cases import cases as mapped_cases
from mapped_rate_cases import performance_cases as mapped_performance_cases
from range_rate_cases import _boundary_cases

FUNCTIONS = (
    "count_over_time",
    "present_over_time",
    "first_over_time",
    "last_over_time",
    "min_over_time",
    "max_over_time",
    "sum_over_time",
    "avg_over_time",
    "stdvar_over_time",
    "stddev_over_time",
    "ts_of_first_over_time",
    "ts_of_last_over_time",
    "ts_of_min_over_time",
    "ts_of_max_over_time",
    "changes",
    "resets",
)


def _function(case, function, prefix="fold-"):
    item = deepcopy(case)
    request = item["request"]
    request["id"] = prefix + function + "-" + request["id"]
    request["query"] = request["query"].replace("rate(", function + "(")
    if function.startswith("ts_of_") and request["profile"] == "server-default":
        request["profile"] = "upstream-tests"
        request["delayed_name_removal"] = False
        request["subquery_step_ms"] = 60000
    return item


def performance_cases():
    output = []
    for case in mapped_performance_cases():
        ident = case["request"]["id"]
        for shape in ("direct", "clamp"):
            if not ident.startswith("wrapped-rate-" + shape + "-"):
                continue
            count = ident.rsplit("pairs", 1)[1]
            for function in ("rate", *FUNCTIONS):
                item = _function(case, function)
                item["request"]["id"] = f"window-fold-{function}-{shape}-pairs{count}"
                output.append(item)
    return output


def _records(values):
    return [
        series(
            {"__name__": metric, "id": str(index), "job": "worker"},
            [
                ((tick + 1) * 1000, (index + 1) * value, "float")
                for tick, value in enumerate(values if metric == "m_total" else [1] * 3)
            ],
        )
        for metric, order in (("m_total", (1, 0)), ("n_total", (0, 1)))
        for index in order
    ]


def cases():
    output = performance_cases()
    controls = mapped_cases()[18:]
    for function in FUNCTIONS:
        output.extend(_function(case, function) for case in controls)
        for profile in ("server-default", "upstream-tests"):
            for fixture, values in (("reset", [10, 12, 1]), ("extrema", [8, 2, 4])):
                records = _records(values)
                for instant in (False, True):
                    output.append(
                        _function(
                            _case(
                                f"dispatch-{fixture}-{profile}-{instant}",
                                "rate(m_total[3s]) / on(id) n_total",
                                records,
                                profile,
                                instant=instant,
                            ),
                            function,
                        )
                    )
            for matching in ("on(__name__,id)", "ignoring(__name__)"):
                output.append(
                    _function(
                        _case(
                            f"names-{profile}-{matching}",
                            f"rate(m_total[3s]) / {matching} rate(n_total[3s])",
                            _records([10, 12, 1]),
                            profile,
                        ),
                        function,
                    )
                )
            started = _records([10, 12, 1])
            for record in started[:2]:
                for point, start in zip(
                    record["points"], (500, 500, 2500), strict=True
                ):
                    point["start_time_ms"] = start
            output.append(
                _function(
                    _case(
                        f"start-times-{profile}",
                        "rate(m_total[3s]) / on(id) n_total",
                        started,
                        profile,
                    ),
                    function,
                )
            )

    for profile in ("server-default", "upstream-tests"):
        for left, right in (
            ("sum_over_time", "avg_over_time"),
            ("min_over_time", "max_over_time"),
            ("first_over_time", "last_over_time"),
            ("count_over_time", "present_over_time"),
            ("stddev_over_time", "stdvar_over_time"),
            ("changes", "resets"),
            ("ts_of_first_over_time", "ts_of_last_over_time"),
            ("ts_of_min_over_time", "ts_of_max_over_time"),
            ("sum_over_time", "rate"),
            ("avg_over_time", "increase"),
            ("first_over_time", "delta"),
            ("last_over_time", "irate"),
        ):
            item = _function(
                _case(
                    f"mixed-{profile}-{left}-{right}",
                    "rate(m_total[3s]) / on(id) n_total",
                    _records([10, 12, 1]),
                    profile,
                ),
                left,
            )
            item["request"]["query"] = (
                f"{left}(m_total[3s]) / on(id) {right}(m_total[3s])"
            )
            output.append(item)
        for state in ("dense", "stale", "empty"):
            records = _records([10, 12, 1])
            if state != "dense":
                for record in records[:2]:
                    record["points"] = (
                        series(
                            {},
                            [
                                (tick * 1000, "7ff0000000000002", "float")
                                for tick in (1, 2, 3)
                            ],
                        )["points"]
                        if state == "stale"
                        else []
                    )
            for wrapped in (False, True):
                child = 'absent_over_time(m_total{id="0"}[3s])'
                if wrapped:
                    child = f"abs({child})"
                output.append(
                    _case(
                        f"fold-absence-{profile}-{state}-{wrapped}",
                        child + " / on(id) n_total",
                        records,
                        profile,
                    )
                )
    for case in _boundary_cases():
        if not case["request"]["id"].startswith("range-boundary-rate-"):
            continue
        for function in FUNCTIONS:
            output.append(_function(case, function))
    return output


def candidate_packets(inputs, page_size):
    from cursor_calls import candidate_packets as ordinary_packets

    groups, packets = ordinary_packets(inputs, page_size)
    requests = {case["request"]["id"]: case["request"] for case in inputs}
    for packet in packets:
        if packet.get("op") == "dataset":
            continue
        request = requests[packet["id"]]
        if "delayed_name_removal" in request:
            packet["delayed_name_removal"] = request["delayed_name_removal"]
    return groups, packets
