"""Additive complete inputs for shared-prefix refinement and genuine full depth."""

import math

from info_cases import series
from rank_completion_cases import _float, query_requests


def panels():
    ladder = [_float(1 << (4 * index)) for index in range(16)]
    result = [("ladder", [0.0, *ladder]),
              ("mad-ladder", [0.0] * 19 + ladder),
              ("unselected-first", [1.0, _float(0x3000000000000000), 2.0])]
    first = 0xBFF0000000000000
    for length in (0, 3, 4, 7, 8, 11, 12, 15, 16, 59, 60, 63):
        second = first ^ (1 << (63 - length))
        assert 64 - (first ^ second).bit_length() == length
        bits = second ^ (1 << 63) if second >> 63 else ~second & ((1 << 64) - 1)
        result.append((f"common-{length}", [1.0, _float(bits)]))
    assert len(result) == 15 and all(math.isfinite(v) for _, values in result for v in values)
    return result


def cases():
    result = []
    for family, values in panels():
        count = len(values)
        data = {"schema_version": 1, "series": [series(
            {"__name__": "m", "instance": "0", "job": "worker"},
            [(tick * 1000, values[(tick - 1) % count], "float")
             for tick in range(1, count + 3)],
        )]}
        result.extend({"dataset": data, "request": request, "faults": {}}
                      for request in query_requests(family, count, prefix="rank-prefix"))
    assert len(result) == 540 and len({c["request"]["id"] for c in result}) == 540
    return result
