"""Independent inputs for page-crossing and nested temporal-quantile backing."""

from copy import deepcopy

from count_values_cases import STALE
from info_cases import bits, series


def cases():
    output = []

    def add(ident, points, queries, *, time=0, mode="instant", end=0, faults=None):
        for profile in ("server-default", "upstream-tests"):
            for qi, query in enumerate(queries):
                output.append(
                    {
                        "request": {
                            "id": f"temporary-quantile:{ident}:{profile}:{qi}",
                            "query": query,
                            "profile": profile,
                            "mode": mode,
                            "time_ms": time,
                            "start_ms": time,
                            "end_ms": end,
                            "step_ms": 1,
                            "lookback_ms": 1,
                            "subquery_step_ms": 1,
                        },
                        "dataset": {
                            "schema_version": 1,
                            "series": [
                                series(
                                    {"__name__": "m", "job": "one"}, deepcopy(points)
                                )
                            ],
                        },
                        "faults": deepcopy(faults or {}),
                    }
                )

    def history(count, nan=False):
        zeros = [bits(-0.0), bits(0.0), bits(1.0)]
        return [
            (
                index - count + 1,
                f"{0x7FF8000000010000 + index:016x}"
                if nan and index % 3 != 2
                else zeros[index % 3],
                "float",
            )
            for index in range(count)
        ]

    for count in (1023, 1024, 1025, 2049):
        for nan in (False, True):
            points = history(count, nan)
            queries = tuple(
                f"quantile_over_time({q},m[{count}ms])" for q in ("0", "0.5", "1")
            )
            add(f"raw-{'nan' if nan else 'zero'}-{count}", points, queries)
            if count == 2049:
                add(
                    f"computed-{'nan' if nan else 'zero'}",
                    points,
                    (
                        f"quantile_over_time(0.5,(m*1)[{count}ms:1ms])",
                        f"quantile_over_time(0.666748046875,(m*1)[{count}ms:1ms])",
                    ),
                )
                add(
                    f"overlap-{'nan' if nan else 'zero'}",
                    points,
                    (
                        "quantile_over_time(0.5,m[2049ms:1ms])",
                        "quantile_over_time(0.5,(m*1)[2049ms:1ms] @ 0)",
                        "quantile_over_time(0.5,(m*1)[2049ms:1ms] offset 1ms)",
                    ),
                    mode="range",
                    end=2,
                )

    transitions = history(1025, True)
    transitions[300] = (-724, STALE, "float")
    transitions[500] = (-524, 7.0, "histogram")
    transitions[700] = (-324, 11.0, "float_histogram")
    add(
        "transitions",
        transitions,
        (
            "quantile_over_time(0,m[1025ms])",
            "quantile_over_time(0,(m*1)[1025ms:1ms])",
        ),
    )
    add(
        "source-error",
        history(2049, True),
        ("quantile_over_time(0.5,m[2049ms])",),
        faults={
            "m": {"next_times": [0], "message": "temporary quantile source failure"}
        },
    )

    nested = []
    for block in range(14):
        endpoint = -260 + 20 * block
        for index in range(17):
            value = (
                f"{0x7FF8000000100000 + 256 * block + index + 1:016x}"
                if block < 13 and index < 2
                else bits(1.0)
            )
            nested.append((endpoint - 16 + index, value, "float"))
    add(
        "nested",
        nested,
        ("quantile_over_time(0,quantile_over_time(0,m[17ms])[280ms:20ms])",),
    )

    for count in (2048, 100000):
        values = [bits(value) for value in (-1.0, -0.0, 0.0, 1.0)]
        generated = [
            ((index + 1) * 1000, values[index % 4], "float") for index in range(count)
        ]
        add(
            f"generated-{count}",
            generated,
            (
                "quantile_over_time(0.5,m[100000s])",
                f"quantile_over_time(0.5,(m*1)[{count}s:1s])",
            ),
            time=count * 1000,
        )
    return output


def derived_kind_cases():
    """Small generic and scheduled subqueries with actual output-kind transitions."""
    dense = [(index * 1000, (-1.0, -0.0, 0.0, 1.0)[(index - 1) % 4], "float")
             for index in range(1, 17)]
    labels = {"__name__": "m", "job": "one"}
    populations = {
        "dense": [series(labels, dense)],
        "sparse": [series(labels, dense[::2])],
        "stale": [series(labels, [(time, STALE, kind) for time, _, kind in dense])],
        "empty-selector": [series({"__name__": "n", "job": "one"}, dense)],
        "nonfinite": [series(labels, [(time, value, kind) for (time, _, kind), value in zip(
            dense, [bits(float("inf")), bits(float("-inf")), "7ff8000000012345", bits(-0.0)] * 4)])],
        "integer-native": [series(labels, [(time, value, "histogram") for time, value, _ in dense])],
        "float-native": [series(labels, [(time, value, "float_histogram") for time, value, _ in dense])],
        "mixed": [series(labels, [(time, value, "float" if index % 2 else "float_histogram")
                                 for index, (time, value, _) in enumerate(dense)])],
        "native-then-float": [series(labels, [(time, value, "histogram" if time < 8000 else "float")
                                             for time, value, _ in dense])],
        "aliases": [series(labels, dense), series(labels | {"empty": ""}, dense)],
        "plural-late": [series(labels, dense), series(labels | {"job": "two"}, dense[8:])],
        "negative-clock": [series(labels, [(time - 20000, value, kind) for time, value, kind in dense])],
    }
    backing_floats = iter([-1.0] * 4 + [-0.0] * 3 + [0.0] * 3 + [1.0] * 4)
    populations["mixed-backing"] = [series(labels, [
        (index * 1000, 1.0 if index in (8, 9) else next(backing_floats),
         "float_histogram" if index in (8, 9) else "float")
        for index in range(1, 17)
    ])]
    ordinary = [f"{function}((m or m)[16s:1s])" for function in (
        "sum_over_time", "count_over_time", "avg_over_time", "min_over_time", "max_over_time",
        "last_over_time", "rate", "resets", "changes")]
    expressions = ordinary + [
        "quantile_over_time(0.5,(m or m)[16s:1s])",
        "sum_over_time((sum by(job)(m))[16s:1s])",
        "quantile_over_time(0.5,(m*1)[16s:1s])",
        "sum_over_time((m*1)[16s:1s])",
        "sum_over_time((m or m)[16s:1s] offset 1s)",
        "sum_over_time((m or m)[16s:1s] @ end())",
    ]
    output = []

    def add(population, records, faults=None):
        time = -4000 if population == "negative-clock" else 16000
        for delayed in (False, True):
            for expression_index, expression in enumerate(expressions):
                for mode in ("instant", "equal", "range"):
                    output.append({
                        "request": {
                            "id": f"derived-kind:{population}:{int(delayed)}:{expression_index}:{mode}",
                            "query": expression, "profile": "server-default", "delayed_name_removal": delayed,
                            "mode": "instant" if mode == "instant" else "range", "time_ms": time,
                            "start_ms": time, "end_ms": time + (2000 if mode == "range" else 0),
                            "step_ms": 1000, "lookback_ms": 1, "subquery_step_ms": 1000,
                        },
                        "dataset": {"schema_version": 1, "series": deepcopy(records)},
                        "faults": deepcopy(faults or {}),
                    })

    for population, records in populations.items():
        add(population, records)
    for name, fault in (
        ("next-first", {"next_times": [2000], "message": "derived required first read"}),
        ("next-later", {"next_times": [9000], "message": "derived required later read"}),
        ("next-eof", {"next_eof": True, "message": "derived required cursor completion"}),
    ):
        add(name, populations["dense"], {"m": fault})
    return output


def plain_lookup_cases():
    """Later Plain lookups with original headers, predecessors and stateful controls."""
    labels = {"__name__": "m", "job": "one"}
    dense = [(index * 1000, (-1.0, -0.0, 0.0, 1.0)[(index - 1) % 4], "float")
             for index in range(1, 17)]
    populations = {
        "dense-starts": [series(labels, dense)],
        "sparse-starts": [series(labels, [(time, float(index + 1), "float")
                                          for index, time in enumerate((900, 2000, 5000, 8000, 16000))])],
        "stale-starts": [series(labels, [(time, STALE if time == 8000 else value, kind)
                                         for time, value, kind in dense])],
        "integer-native": [series(labels, [(time, value, "histogram") for time, value, _ in dense])],
        "float-native": [series(labels, [(time, value, "float_histogram") for time, value, _ in dense])],
        "mixed": [series(labels, [(time, value, "float" if index % 2 else "float_histogram")
                                 for index, (time, value, _) in enumerate(dense)])],
        "native-future": [series(labels, [(900, 8.0, "float_histogram"),
                                          (5000, 4.0, "float_histogram")])],
    }
    for records in populations.values():
        for record in records:
            for point in record["points"]:
                point["start_time_ms"] = point["time_ms"] - 250
    native = populations["native-future"][0]["points"]
    for point, count, hint in zip(native, (8.0, 4.0), (2, 0)):
        point["float_histogram"].update(count=bits(count), zero_count=bits(count), reset_hint=hint)
    populations["initial-error"] = deepcopy(populations["dense-starts"])
    populations["required-eof"] = deepcopy(populations["dense-starts"])
    expressions = (
        "sum_over_time((m or m)[8s:1s])",
        "quantile_over_time(0.5,(m or m)[8s:1s])",
        "sum_over_time((timestamp(m) or timestamp(m))[8s:1s])",
        "sum_over_time((start_timestamp(m) or start_timestamp(m))[8s:1s])",
        "sum_over_time((m @start() or m @end())[8s:1s])",
        "sum_over_time((timestamp(m @start()) or timestamp(m @end()))[8s:1s])",
        "sum_over_time((m offset 1ms or m offset 1ms)[8s:1s])",
        "sum_over_time((m offset -1ms or m offset -1ms)[8s:1s])",
        "sum_over_time((m offset 0.0005 or m offset 0.0005)[8s:1s])",
        "sum_over_time((m @0.001 or m @0.001)[8s:1s])",
        "histogram_count(m)",
        "m",
    )
    output = []
    for population, records in populations.items():
        time = 1000 if population == "native-future" else 16000
        end = 5000 if population == "native-future" else 18000
        faults = ({"m": {"next_times": [1000], "message": "required initial lookup"}}
                  if population == "initial-error" else
                  {"m": {"next_eof": True, "message": "required lookup completion"}}
                  if population == "required-eof" else {})
        for lookback in (1, 1001, 0):
            for delayed in (False, True):
                for index, expression in enumerate(expressions):
                    for mode in ("instant", "equal", "range"):
                        output.append({
                            "request": {
                                "id": f"plain-lookup:{population}:{lookback}:{int(delayed)}:{index}:{mode}",
                                "query": expression, "profile": "upstream-tests",
                                "delayed_name_removal": delayed,
                                "mode": "instant" if mode == "instant" else "range",
                                "time_ms": time, "start_ms": time,
                                "end_ms": end if mode == "range" else time,
                                "step_ms": 1000, "lookback_ms": lookback, "subquery_step_ms": 1000,
                            },
                            "dataset": {"schema_version": 1, "series": deepcopy(records)},
                            "faults": deepcopy(faults),
                        })
    return output
