#!/usr/bin/env python3
"""Additive complete queries for early and full-depth temporal rank selection."""

import struct

from info_cases import series


def _float(bits):
    return struct.unpack(">d", int(bits).to_bytes(8, "big"))[0]


def dataset(pattern, count):
    def value(index):
        if pattern == "distinct":
            return float(index + 1)
        if pattern == "two":
            return (1.0, 2.0)[index % 2]
        if pattern == "skew":
            return 1.0 if index % 4 else 2.0
        if pattern == "near":
            return _float(0x3FF0000000000002 - index % 3)
        if pattern in ("subnormal", "subnormal-offset"):
            return _float(index + (2 if pattern == "subnormal-offset" else 1))
        if pattern == "mixed-zero":
            return (-1.0, -0.0, 0.0, 1.0)[index % 4]
        if pattern == "nan":
            return _float(0x7FF8000000000042 + index % 2) if index % 3 else 1.0
        if pattern == "one":
            return 1.0
        if pattern == "zero":
            return (-0.0, 0.0)[index % 2]
        raise ValueError("unknown rank-selection pattern")

    return {"schema_version": 1, "series": [series(
        {"__name__": "m", "instance": "0", "job": "worker"},
        [(tick * 1000, value(tick - 1), "float") for tick in range(1, count + 3)]
        if count else [],
    )]}


def query_requests(family, count, *, prefix="rank"):
    width = max(count, 1)
    for route in ("raw", "computed"):
        body = f"m[{width}s]" if route == "raw" else f"(m*1)[{width}s:1s]"
        expressions = [(f"q{q}", f"quantile_over_time({q},{body})", profile)
                       for q in (0, .5, .95, 1)
                       for profile in ("server-default", "upstream-tests")]
        expressions.append(("mad", f"mad_over_time({body})", "upstream-tests"))
        for name, expression, profile in expressions:
            for mode in ("instant", "range"):
                ident = f"{prefix}:{family}:{route}:{name}:{profile}:{mode}"
                yield {
                    "id": ident, "query": expression, "profile": profile, "mode": mode,
                    "time_ms": width * 1000, "start_ms": width * 1000,
                    "end_ms": (width + 2) * 1000, "step_ms": 1000,
                    "lookback_ms": 1, "subquery_step_ms": 1000,
                }


def cases():
    result = []
    panels = [("empty", "one", 0), ("singleton", "one", 1), ("two-tiny", "two", 2),
              ("two", "two", 32), ("skew", "skew", 35), ("near", "near", 35),
              ("subnormal", "subnormal", 35), ("mixed-zero", "mixed-zero", 32),
              ("nan", "nan", 32), ("one", "one", 32), ("zero", "zero", 32),
              ("distinct", "distinct", 129), ("two-long", "two", 512),
              ("near-long", "near", 512), ("subnormal-offset", "subnormal-offset", 4)]
    for family, pattern, count in panels:
        data = dataset(pattern, count)
        result.extend({"dataset": data, "request": request, "faults": {}}
                      for request in query_requests(family, count))
    assert len(result) == 540 and len({c["request"]["id"] for c in result}) == 540
    return result
