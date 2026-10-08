"""Typed matrix argument controls declared independently of candidate outcomes."""

import json
from copy import deepcopy
from pathlib import Path

from info_cases import series


def cases():
    catalog = json.loads(Path(__file__).with_name("language-catalog.json").read_bytes())
    functions = [row for row in catalog["functions"] if "matrix" in row["arguments"]]
    rows = []
    for function in functions:
        for matrix in ("(m[2s])", "((m[2s:1s]))"):
            arguments = [
                matrix if kind == "matrix" else ".5" for kind in function["arguments"]
            ]
            call = function["name"] + "(" + ",".join(arguments) + ")"
            for owner in ("direct", "topk", "quantile"):
                expression = call if owner == "direct" else f"{owner}(scalar({call}),m)"
                for populated in (False, True):
                    for profile in ("server-default", "upstream-tests"):
                        for mode in ("instant", "range"):
                            records = (
                                [
                                    series(
                                        {"__name__": "m"},
                                        [
                                            (0, 1.0, "float"),
                                            (1000, 2.0, "float"),
                                            (2000, 3.0, "float"),
                                        ],
                                    )
                                ]
                                if populated
                                else []
                            )
                            rows.append(
                                {
                                    "request": {
                                        "id": f"unvisited-matrix-{len(rows):04d}-{function['name']}-{owner}",
                                        "query": expression,
                                        "profile": profile,
                                        "mode": mode,
                                        "time_ms": 2000,
                                        "start_ms": 0,
                                        "end_ms": 2000,
                                        "step_ms": 1000,
                                        "lookback_ms": 1000,
                                        "subquery_step_ms": 1000,
                                    },
                                    "dataset": {
                                        "schema_version": 1,
                                        "series": deepcopy(records),
                                    },
                                    "faults": {},
                                }
                            )
    for function in (
        "sum_over_time",
        "quantile_over_time",
        "absent_over_time",
        "rate",
        "changes",
        "last_over_time",
    ):
        for matrix in ("(m[2s])", "((m[2s:1s]))"):
            call = (
                f"{function}({matrix})"
                if function != "quantile_over_time"
                else f"{function}(scalar(q),{matrix})"
            )
            for owner in ("direct", "topk", "quantile"):
                expression = call if owner == "direct" else f"{owner}(scalar({call}),m)"
                for fault in ("m-selection", "m-iterator", "q-selection"):
                    for mode in ("instant", "range"):
                        metric, kind = fault.split("-")
                        settings = (
                            {"set_error": "matrix selection fault"}
                            if kind == "selection"
                            else {
                                "next_times": [2000],
                                "message": "matrix iterator fault",
                            }
                        )
                        rows.append(
                            {
                                "request": {
                                    "id": f"unvisited-matrix-fault-{len(rows):04d}-{function}-{owner}-{fault}",
                                    "query": expression,
                                    "profile": "upstream-tests",
                                    "mode": mode,
                                    "time_ms": 2000,
                                    "start_ms": 0,
                                    "end_ms": 2000,
                                    "step_ms": 1000,
                                    "lookback_ms": 1000,
                                    "subquery_step_ms": 1000,
                                },
                                "dataset": {
                                    "schema_version": 1,
                                    "series": [
                                        series(
                                            {"__name__": "m"},
                                            [
                                                (0, 1.0, "float"),
                                                (1000, 2.0, "float"),
                                                (2000, 3.0, "float"),
                                            ],
                                        ),
                                        series(
                                            {"__name__": "q"},
                                            [
                                                (0, 0.5, "float"),
                                                (1000, 0.5, "float"),
                                                (2000, 0.5, "float"),
                                            ],
                                        ),
                                    ],
                                },
                                "faults": {metric: settings},
                            }
                        )
    return rows
