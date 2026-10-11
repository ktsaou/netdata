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
            ("quantile_over_time(0.5,m[100000s])",),
            time=count * 1000,
        )
    return output
