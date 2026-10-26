# SPDX-License-Identifier: GPL-3.0-or-later
"""Fixed end-of-range ranking, sparse histories and ordinary-path controls."""

from info_cases import series


def datasets():
    for pattern in [
        "changing",
        "sparse",
        "stale",
        "empty",
        "negative",
        "nan",
        "infinity",
        "zero",
        "ties",
        "native",
        "mixed",
    ]:
        records = []
        for g in range(6):
            for d in range(2):
                pts = []
                for t in range(6):
                    if pattern == "empty" or (
                        pattern == "sparse" and (g == 0 and t < 3 or g == 5 and t > 2)
                    ):
                        continue
                    v = (g + 1) * 10 + d + t * (6 - g)
                    kind = "float"
                    if pattern == "negative":
                        v = -v if d == 0 else v
                    if pattern == "stale" and t in [2, 3]:
                        v = "7ff0000000000002"
                    if pattern == "nan" and g == 0:
                        v = "7ff8000000000042"
                    if pattern == "infinity" and g < 2:
                        v = ["7ff0000000000000", "fff0000000000000"][g]
                    if pattern == "zero":
                        v = -0.0 if d == 0 else 0.0
                    if pattern == "ties":
                        v = 1
                    if pattern == "native" or pattern == "mixed" and d == 1:
                        kind = "float_histogram"
                    pts.append((t * 1000, v, kind))
                records.append(
                    series({"__name__": "m", "id": str(g), "dim": str(d)}, pts)
                )
        yield pattern, {"schema_version": 1, "series": records}


def requests(pattern):
    k = 10 if pattern in ["ties", "zero"] else 3
    exprs = [
        f"sum by(id)(m) and topk({k},sum by(id)(m @ end()))",
        f"sum by(id)(abs(m)) and topk({k},sum by(id)(abs(m @ end())))",
        f"topk({k},sum by(id)(m @ end()))",
        "sum by(id)(m)",
        "topk(3,sum by(id)(m))",
        "sum by(id)(m) and topk(3,sum by(id)(m @ start()))",
        "sum by(id)(m offset 1s) and topk(3,sum by(id)(m @ end()))",
        "sum by(id)(m) and topk(scalar(vector(3)),sum by(id)(m @ end()))",
        "sum by(id)(m) and sum by(id)(m @ end())",
        "sum by(id)(m) or sum by(id)(m @ end())",
        "sum by(id)(m) unless sum by(id)(m @ end())",
        "sum by(id)(sum by(id)(m)) and topk(3,sum by(id)(m @ end()))",
    ]
    # Avoid unspecified cutoff ties in strict Go membership comparison.
    if pattern in ["ties", "zero"]:
        exprs = exprs[:4]
    for profile in ["server-default", "upstream-tests"]:
        for i, expr in enumerate(exprs):
            for mode in ["range", "instant"]:
                yield {
                    "id": f"{pattern}-{profile}-{i}-{mode}",
                    "query": expr,
                    "profile": profile,
                    "mode": mode,
                    "time_ms": 5000,
                    "start_ms": 0,
                    "end_ms": 5000,
                    "step_ms": 1000,
                    "lookback_ms": 999,
                    "subquery_step_ms": 1000,
                }


def cases():
    return [
        {"id": name, "dataset": data, "requests": list(requests(name))}
        for name, data in datasets()
    ]
