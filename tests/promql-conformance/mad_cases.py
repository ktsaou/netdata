"""Independent MAD query inputs; original Prometheus executions supply outcomes."""

import math
import random
import struct
from copy import deepcopy

from count_values_cases import STALE, WINDOWS
from info_cases import bits, series


def arrays():
    small = struct.unpack(">d", bytes.fromhex("0000000000000001"))[0]
    large = struct.unpack(">d", bytes.fromhex("7fefffffffffffff"))[0]
    bases = [
        ("signedzero-three", [-0.0, -0.0, 0.0]),
        ("signedzero-four", [-0.0, -0.0, 0.0, 0.0]),
        ("subnormal-zero", [-small, -0.0, -0.0, 0.0]),
        ("subnormal-two", [-small, small]),
        ("subnormal-three", [-small, 0.0, small]),
        ("subnormal-four", [-small, -small, small, small]),
        ("single-finite", [7.0]),
        ("single-inf", [math.inf]),
        ("single-negative-inf", [-math.inf]),
        ("all-positive-inf-two", [math.inf, math.inf]),
        ("all-positive-inf-three", [math.inf] * 3),
        ("all-negative-inf-two", [-math.inf] * 2),
        ("median-inf", [0.0, 0.0, math.inf, math.inf]),
        ("median-negative-inf", [-math.inf, -math.inf, 0.0, 0.0]),
        ("first-median-inf-neighbor", [0.0, 0.0, math.inf]),
        ("second-median-inf-neighbor", [-math.inf, 0.0, 0.0]),
        ("finite-inf-mad", [-math.inf, -math.inf, 0.0, 1.0, 2.0, 3.0]),
        ("both-infinities", [-math.inf, 0.0, math.inf]),
        ("finite-overflow-deviations", [-large, -large, large]),
        ("largest-even", [-large, -large, large, large]),
        ("huge-finite", [-1e308, 1e308]),
        ("nan-positive", ["7ff8000000000042", 1.0, 2.0]),
        ("nan-negative", ["fff8000000000043", 1.0, 2.0]),
        ("nan-signaling", ["7ff0000000000043", 1.0, 2.0]),
        ("pdq-zeros", [0.0 if i == 0 else 1.0 if i % 2 else -0.0 for i in range(13)]),
        (
            "pdq-nonfinite",
            [
                math.inf
                if i % 5 == 0
                else -math.inf
                if i % 7 == 0
                else -0.0
                if i % 3 == 0
                else float(i % 9 - 4)
                for i in range(51)
            ],
        ),
        ("page-boundaries", [float(i % 11 - 5) for i in range(1030)]),
        ("ordinary", [0.0, 1.0, 2.0, 3.0, 7.0, 11.0]),
    ]
    for name, values in list(bases):
        if name in {
            "first-median-inf-neighbor",
            "second-median-inf-neighbor",
            "subnormal-two",
            "pdq-zeros",
        }:
            for length in (13, 50, 51, 127):
                bases.append(
                    (
                        f"{name}-repeat-{length}",
                        (values * ((length + len(values) - 1) // len(values)))[:length],
                    )
                )
    output = []
    for name, source in bases:
        for permutation in ("input", "reverse", "rotate", "shuffle"):
            values = list(source)
            if permutation == "reverse":
                values.reverse()
            elif permutation == "rotate":
                values = values[1:] + values[:1]
            elif permutation == "shuffle":
                random.Random(71081).shuffle(values)
            output.append(
                {
                    "id": f"{name}-{permutation}",
                    "base": name,
                    "permutation": permutation,
                    "bits": [
                        value if isinstance(value, str) else bits(value)
                        for value in values
                    ],
                }
            )
    return output


def cases():
    inputs = []

    def add(
        ident, records, queries=("mad_over_time(m[1h])",), windows=WINDOWS, faults=None
    ):
        for profile in ("server-default", "upstream-tests"):
            for qi, query in enumerate(queries):
                for wi, (mode, time, start, end, step) in enumerate(windows):
                    inputs.append(
                        {
                            "request": {
                                "id": f"mad:{ident}:{profile}:{qi}:{wi}",
                                "query": query,
                                "profile": profile,
                                "mode": mode,
                                "time_ms": time,
                                "start_ms": start,
                                "end_ms": end,
                                "step_ms": step,
                                "lookback_ms": 1,
                                "subquery_step_ms": 1000,
                            },
                            "dataset": {
                                "schema_version": 1,
                                "series": deepcopy(records),
                            },
                            "faults": deepcopy(faults or {}),
                        }
                    )

    retained = arrays()
    for item in retained:
        n = len(item["bits"])
        record = series(
            {
                "__name__": "m",
                "__type__": "gauge",
                "__unit__": "seconds",
                "id": item["id"],
            },
            [
                ((i - n + 1) * 1000, value, "float")
                for i, value in enumerate(item["bits"])
            ],
        )
        add("array-" + item["id"], [record])
    selected = {
        "signedzero-three",
        "subnormal-two",
        "first-median-inf-neighbor",
        "second-median-inf-neighbor",
        "finite-overflow-deviations",
        "nan-positive",
        "nan-negative",
        "nan-signaling",
    }
    for item in retained:
        if item["base"] in selected and item["permutation"] == "input":
            n = len(item["bits"])
            add(
                "consumers-" + item["id"],
                [
                    series(
                        {"__name__": "m"},
                        [
                            ((i - n + 1) * 1000, value, "float")
                            for i, value in enumerate(item["bits"])
                        ],
                    )
                ],
                (
                    "mad_over_time(m[1h]) >= 0",
                    "1/mad_over_time(m[1h]) < bool 0",
                    "timestamp(mad_over_time(m[1h]))",
                    "mad_over_time(m[1h]) == bool mad_over_time(m[1h])",
                ),
            )
    changing = [
        series(
            {"__name__": "m", "job": "a", "empty": ""},
            [
                (-2000, 1, "float"),
                (-1000, 9, "float"),
                (0, -1, "float"),
                (1000, STALE, "float"),
                (2000, 2, "float"),
            ],
        ),
        series(
            {"__name__": "m", "job": "b"},
            [(-1000, -0.0, "float"), (0, 0.0, "float"), (2000, 7, "float")],
        ),
    ]
    add(
        "windows",
        changing,
        (
            "mad_over_time(m[1s])",
            "mad_over_time(m[2s])",
            "mad_over_time(m[3s])",
            "mad_over_time(m[1h])",
        ),
    )
    add(
        "fixed",
        changing,
        (
            "mad_over_time(m[1h] @0)",
            "mad_over_time(m[1h] @start())",
            "mad_over_time(m[1h] @end())",
            "mad_over_time(m[1h] offset 1s)",
            "mad_over_time(m[1h] @end() offset step())",
            "(mad_over_time((m[1h])))",
        ),
    )
    add(
        "parents",
        changing,
        (
            "sum(mad_over_time(m[1h]))",
            "quantile(0.5,mad_over_time(m[1h]))",
            'count_values("v",mad_over_time(m[1h]))',
            "abs(mad_over_time(m[1h]))",
            'label_replace(mad_over_time(m[1h]),"copy","$1","job","(.*)")',
        ),
    )
    add("empty", [])
    add(
        "stale",
        [series({"__name__": "m"}, [(-1000, STALE, "float"), (0, STALE, "float")])],
    )
    for kind in ("histogram", "float_histogram"):
        timelines = {
            "all-native": [(-1000, 1, kind), (0, 7, kind)],
            "float-native": [(-1000, 7, "float"), (0, 2, kind)],
            "native-float": [(-1000, 2, kind), (0, 7, "float")],
            "nan-native": [(-1000, "fff8000000000042", "float"), (0, 2, kind)],
            "native-nan": [(-1000, 2, kind), (0, "7ff0000000000043", "float")],
            "stale-native": [(-1000, STALE, "float"), (0, 2, kind)],
            "native-stale": [(-1000, 2, kind), (0, STALE, "float")],
            "float-stale-native": [(-1000, 7, "float"), (0, STALE, kind)],
        }
        for name, points in timelines.items():
            add(
                f"{kind}-{name}",
                [series({"__name__": "m", "job": "one"}, points)],
                (
                    "mad_over_time(m[1h])",
                    "(mad_over_time((m[1h])))",
                    "mad_over_time(m[1s])",
                ),
            )
    collision = [
        series(
            {"__name__": name, "job": "same"},
            [(-1000, 1, "float"), (0, value, "float")],
        )
        for name, value in (("m", 3), ("n", 7))
    ]
    add(
        "name-collision",
        collision,
        (
            'mad_over_time({__name__=~"m|n"}[1h])',
            'abs(mad_over_time({__name__=~"m|n"}[1h]))',
        ),
    )
    fault_body = [
        series(
            {"__name__": "m"},
            [
                (-1000, "7ff8000000000042", "float"),
                (0, 7, "float"),
                (1000, 11, "float"),
            ],
        )
    ]
    for fault, settings in (
        ("selection", {"set_error": "MAD selection fault"}),
        ("iterator-after-nan", {"next_times": [0], "message": "MAD iterator fault"}),
        ("seek", {"seek_targets": [-3599999], "message": "MAD seek fault"}),
    ):
        add(
            "fault-" + fault,
            fault_body,
            ("mad_over_time(m[1h])", "(mad_over_time((m[1h])))"),
            faults={"m": settings},
        )
    return inputs
