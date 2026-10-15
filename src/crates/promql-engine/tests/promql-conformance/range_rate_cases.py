"""Closed range-rate family: discovery reuse without changing numerical dispatch."""

from copy import deepcopy

from info_cases import series
from mapped_rate_cases import _case
from mapped_rate_cases import cases as mapped_cases
from mapped_rate_cases import performance_cases as mapped_performance_cases

FUNCTIONS = ("rate", "increase", "delta", "irate", "idelta")


def _function(case, function, prefix):
    case = deepcopy(case)
    case["request"]["id"] = prefix + case["request"]["id"]
    case["request"]["query"] = case["request"]["query"].replace("rate(", function + "(")
    return case


def performance_cases():
    output = []
    for case in mapped_performance_cases():
        ident = case["request"]["id"]
        for shape in ("direct", "clamp"):
            if not ident.startswith("wrapped-rate-" + shape + "-"):
                continue
            count = ident.rsplit("pairs", 1)[1]
            for function in FUNCTIONS:
                item = _function(case, function, "")
                item["request"]["id"] = (
                    f"range-producer-{function}-{shape}-pairs{count}"
                )
                output.append(item)
    return output


def _reset_records():
    output = []
    for metric, indices in [("m_total", [1, 0]), ("n_total", [0, 1])]:
        for index in indices:
            values = [10, 12, 1] if metric == "m_total" else [1, 1, 1]
            output.append(
                series(
                    {"__name__": metric, "id": str(index), "job": "worker"},
                    [
                        ((tick + 1) * 1000, (index + 1) * value, "float")
                        for tick, value in enumerate(values)
                    ],
                )
            )
    return output


def _boundary_cases():
    records = [
        series(
            {"__name__": "m_total", "id": "0"},
            [(1000, 10, "float"), (2000, 15, "float"), (3000, 20, "float")],
        ),
        series(
            {"__name__": "n_total", "id": "0"}, [(3500, 1, "float"), (4500, 1, "float")]
        ),
    ]
    output = []
    offset_ms = 200 * 365 * 24 * 60 * 60 * 1000
    ancient = [
        series(
            {"__name__": metric, "id": "0"},
            [(tick * 1000 - offset_ms, tick, "float") for tick in (1, 2, 3)],
        )
        for metric in ("m_total", "n_total")
    ]
    for function in FUNCTIONS:
        for name, offset in [("positive", "0.0015"), ("negative", "-0.0015")]:
            item = _case(
                f"range-boundary-{function}-{name}",
                f"abs({function}(m_total[4.0005] offset {offset})) / on(id) n_total",
                records,
                end=4500,
            )
            item["request"].update(time_ms=3500, start_ms=3500)
            output.append(item)
        output.append(
            _case(
                f"range-boundary-{function}-duration-add-overflow",
                f"abs({function}(m_total[200y] offset 200y)) / on(id) (n_total offset 200y)",
                ancient,
                end=3000,
            )
        )
    return output


def cases():
    output = performance_cases()
    for function in FUNCTIONS:
        for case in mapped_cases()[18:]:
            output.append(_function(case, function, "range-" + function + "-"))

    for profile in ("server-default", "upstream-tests"):
        records = _reset_records()
        for function in FUNCTIONS:
            expression = function + "(m_total[3s]) / on(id) n_total"
            for instant in (False, True):
                output.append(
                    _case(
                        f"range-reset-{profile}-{function}-{instant}",
                        expression,
                        records,
                        profile,
                        instant=instant,
                    )
                )
            for state in ("empty", "singleton", "start", "gauge", "noncounter"):
                changed = deepcopy(records)
                query = expression
                for item in changed[:2]:
                    if state in ("empty", "singleton"):
                        item["points"] = item["points"][: int(state == "singleton")]
                    elif state == "start":
                        for point, start in zip(item["points"], [500, 500, 2500]):
                            point["start_time_ms"] = start
                    elif state == "noncounter":
                        item["labels"][0]["value"] = "bQ=="
                        query = function + "(m[3s]) / on(id) n_total"
                    else:
                        item["points"] = series(
                            {},
                            [
                                (1000, 10, "float_histogram"),
                                (2000, 12, "float_histogram"),
                                (3000, 1, "float_histogram"),
                            ],
                        )["points"]
                        for point in item["points"]:
                            point["float_histogram"]["reset_hint"] = 3
                output.append(
                    _case(
                        f"range-tail-{profile}-{function}-{state}",
                        query,
                        changed,
                        profile,
                    )
                )
        for left in FUNCTIONS:
            for right in FUNCTIONS:
                output.append(
                    _case(
                        f"range-mixed-{profile}-{left}-{right}",
                        f"abs({left}(m_total[3s])) / on(id) "
                        f"clamp_min({right}(m_total[3s]),.1)",
                        records,
                        profile,
                    )
                )
    return output + _boundary_cases()
