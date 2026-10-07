"""Independently declared single-call arithmetic inputs for original Go helpers."""
import struct
from itertools import product


def bits(value):
    return struct.pack(">d", value).hex()


def requests():
    counts = ("0000000000000000", "8000000000000000", "3ff0000000000000", "bff0000000000000",
              "7ff0000000000000", "fff0000000000000", "7ff8000000000099", "fff8000000000027",
              "7ff0000000000002", "fff0000000000003")
    operations = ([{"kind": "quantile", "quantile": bits(q)} for q in (0.25, 0.75)]
                  + [{"kind": "fraction", "lower": bits(lower), "upper": bits(upper)}
                     for lower, upper in ((-float("inf"), 2), (0.5, 1.5), (-0.5, 0.5))])
    rows = []
    for panel, bounds in enumerate(((-1, 1, float("inf")), (0, 1, float("inf")), (1, 1, float("inf")))):
        for index, values in enumerate(product(counts, repeat=3)):
            buckets = [{"upper_bound": bits(bound), "count": count} for bound, count in zip(bounds, values)]
            for ordinal, operation in enumerate(operations):
                rows.append({"id": f"operand:{panel}:{index:04d}:{ordinal}", "buckets": buckets,
                             "operation": operation})
    return rows
