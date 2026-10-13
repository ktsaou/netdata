"""Independently declared COUNT_VALUES inputs for source text, grouping and scopes."""
import struct
from copy import deepcopy

from info_cases import bits, series

STALE = "7ff0000000000002"
FIRST = b"Aaaaa\x00\x00\x00\x00\x00\x00\x00\x00"
SECOND = b"Bbbbb\x1d\x3c\xa5\x95\x96\x2f\x43\xdd"
WINDOWS = (("instant", 0, 0, 0, 0), ("instant", 2000, 0, 0, 0),
           ("range", 0, 0, 0, 1000), ("range", 0, 0, 2000, 1000))


def native(kind="float_histogram", **changes):
    integer = kind == "histogram"
    value = {"reset_hint": 0, "schema": 0, "count": 1 if integer else bits(1),
             "sum": bits(1), "zero_threshold": bits(0.001), "zero_count": 0 if integer else bits(0),
             "positive_spans": None, "negative_spans": None, "positive_buckets": None,
             "negative_buckets": None, "custom_values": None}
    value.update(changes)
    return value


def native_series(payload, kind="float_histogram", labels=None):
    row = series(labels or {"__name__": "m"}, [])
    row["points"] = [{"time_ms": 0, "kind": kind, kind: payload}]
    return row


def span(offset, length):
    return {"offset": offset, "length": length}


def cases():
    inputs = []

    def add(ident, records, queries=('count_values("v",m)',), windows=WINDOWS, faults=None):
        for profile in ("server-default", "upstream-tests"):
            for qi, query in enumerate(queries):
                for wi, (mode, time, start, end, step) in enumerate(windows):
                    inputs.append({"request": {"id": f"count-values:{ident}:{profile}:{qi}:{wi}",
                        "query": query, "profile": profile, "mode": mode, "time_ms": time,
                        "start_ms": start, "end_ms": end, "step_ms": step,
                        "lookback_ms": 1, "subquery_step_ms": 1000},
                        "dataset": {"schema_version": 1, "series": deepcopy(records)},
                        "faults": deepcopy(faults or {})})

    specials = [0, 1 << 63, 1, (1 << 63) | 1, 0x000fffffffffffff, 0x0010000000000000,
        0x0010000000000001, 0x3ff0000000000000, 0xbff0000000000000,
        0x7fefffffffffffff, 0xffefffffffffffff, 0x7ff0000000000000, 0xfff0000000000000,
        0x7ff8000000000000, 0x7ff8000000000001, 0xfff8000000000042,
        0x7ff0000000000001, 0xfff0000000000044, 0x7fffffffffffffff, 0xffffffffffffffff]
    add("float-specials", [series({"__name__": "m", "z": str(index)},
        [(0, f"{value:016x}", "float")]) for index, value in enumerate(specials)])
    decades = []
    for exponent in (-324, -323, -308, -100, -7, -6, -5, -4, -1, 0, 1, 5, 6, 7, 100, 308):
        center = int.from_bytes(struct.pack(">d", float(f"1e{exponent}")), "big")
        for delta in (-1, 0, 1):
            if 0 <= center + delta <= 0x7fefffffffffffff:
                for sign in (0, 1 << 63):
                    decades.append(series({"__name__": "m", "z": str(len(decades))},
                        [(0, f"{sign | (center + delta):016x}", "float")]))
    add("float-decades", decades, windows=WINDOWS[:1])
    carries = [0.0000995, 0.0009995, 0.00995, 0.0995, 0.995, 1.25, 1.35,
               9.95, 12.5, 13.5, 99.5, 125.0, 135.0, 995.0, 999500.0]
    add("float-carries", [series({"__name__": "m", "z": str(index)}, [(0, value, "float")])
                          for index, value in enumerate(carries)], windows=WINDOWS[:1])
    ordinary = [series({"__name__": "m", "job": "one", "z": "a", "v": "old", "empty": ""},
                      [(0, 1, "float"), (1000, 2, "float"), (2000, -0.0, "float")]),
                series({"__name__": "m", "job": "one", "z": "b", "v": ""},
                      [(0, 1, "float"), (1000, 2, "float"), (2000, "fff8000000000042", "float")]),
                series({"__name__": "m", "job": "two", "z": "c"},
                      [(0, 2, "float"), (1000, STALE, "float"), (2000, 1, "float")])]
    add("grouping", ordinary, ('count_values by(job)("v",m)',
        'count_values by(job,v,v)("v",m)', 'count_values without(z,v)("v",m)',
        'count_values without()("v",m)', 'count_values by(__name__)("__name__",m)',
        'count_values without(v)("__name__",m)', 'count_values by(job)("job",m)'))
    add("consumers", ordinary, ('sum(count_values("v",m))', 'count(count_values("v",m))',
        'timestamp(count_values("v",m))', 'abs(count_values("v",m))',
        'count_values("w",count_values("v",m))', 'sum by(v)(count_values("v",m))',
        'count_values("v",m) + on(v) count_values("v",m)',
        'label_replace(count_values("v",m),"new","$1","v","(.*)")'))
    add("fixed", ordinary, ('count_values(("v"),m @0)', 'count_values("v",m @end())',
        'count_values("v",m offset 1s)', '(count_values("v",m))'))
    add("empty", [], ('count_values("v",m)', 'count_values("",m)'))
    add("stale", [series({"__name__": "m"}, [(0, STALE, "float")])])
    for name in ('""', '"a\\xc5z"', '"λ🙂"', '"a\\x00b"', '"a b"', '"a\\nb"'):
        add("name-" + name.encode().hex(), ordinary, (f"count_values({name},m)",))
    for ident, payload in {
        "empty": native(),
        "negative-prefix": native(negative_spans=[span(-1, 3)], negative_buckets=[bits(1), bits(0)]),
        "excess-counts": native(negative_spans=[span(-1, 1)], negative_buckets=[bits(2), bits(5)]),
        "empty-spans": native(negative_spans=[span(-3, 0), span(1, 2), span(2, 0), span(1, 1)],
                              negative_buckets=[bits(1), bits(0), bits(2)]),
        "uint-length": native(negative_spans=[span(-1, (1 << 32) - 1)], negative_buckets=[bits(1)]),
        "wrap-positive": native(positive_spans=[span((1 << 31) - 1, 2)],
                                positive_buckets=[bits(1), bits(2)]),
        "wrap-negative": native(negative_spans=[span((1 << 31) - 1, 2)],
                                negative_buckets=[bits(1), bits(2)]),
        "zero-negative": native(zero_count=bits(-2), zero_threshold=bits(1)),
        "zero-nan": native(zero_count="fff8000000000042"),
        "nonfinite": native(count="fff8000000000042", sum="7ff0000000000000",
                            positive_spans=[span(0, 3)], positive_buckets=[bits(-1), bits(-0.0), "fff0000000000000"]),
        "unclipped": native(zero_count=bits(1), zero_threshold=bits(2),
            positive_spans=[span(0, 2)], positive_buckets=[bits(1), bits(2)],
            negative_spans=[span(0, 2)], negative_buckets=[bits(1), bits(2)]),
        "custom": native(schema=-53, custom_values=[bits(-1), bits(1)],
            positive_spans=[span(0, 3)], positive_buckets=[bits(1), bits(2), bits(3)]),
        "custom-zero": native(schema=-53, zero_count=bits(1)),
        "custom-zero-nan": native(schema=-53, zero_count="fff8000000000042"),
        "custom-negative-at-zero": native(schema=-53, negative_spans=[span(1, 1)], negative_buckets=[bits(0)]),
        "custom-positive-bounds": native(schema=-53, positive_spans=[span(2, 1)], positive_buckets=[bits(1)]),
        "schema-nine": native(schema=9, negative_spans=[span(0, 1)], negative_buckets=[bits(0)]),
        "schema-min": native(schema=-(1 << 31), positive_spans=[span(0, 1)], positive_buckets=[bits(1)]),
        "schema-empty-nine": native(schema=9),
    }.items():
        add("native-" + ident, [native_series(payload)])
    integer = native("histogram", count=4, zero_count=1,
        negative_spans=[span(-1, 3)], negative_buckets=[1, -1, 2])
    floating = native(count=bits(4), zero_count=bits(1),
        negative_spans=[span(-1, 3)], negative_buckets=[bits(1), bits(0), bits(2)])
    add("integer-and-float", [native_series(integer, "histogram", {"__name__": "m", "z": "int"}),
        native_series(floating, labels={"__name__": "m", "z": "float"})])
    for schema in (-4, -1, 1, 4, 8):
        add("schema-" + str(schema), [native_series(native(schema=schema,
            negative_spans=[span(-2, 3)], negative_buckets=[bits(1), bits(0), bits(2)],
            positive_spans=[span(-2, 3)], positive_buckets=[bits(1), bits(0), bits(2)]))])
    add("grouping-hash-collision", [series({"__name__": "m", "a": b"b\xffx\xffy", "z": "a"}, [(0, 1, "float")]),
        series({"__name__": "m", "a": b"b", "x": "y", "z": "b"}, [(0, 1, "float")])],
        ('count_values by(a,x)("v",m)', 'sum(count_values by(a,x)("v",m))',
         'count_values by(a,x)("z",m)', 'sum(count_values by(a,x)("z",m))'))
    inner = 'count_values without(id,v)("v",{id=~".+"})'
    for overlap in (False, True):
        records = [series({"a": FIRST, "id": "first"}, [(0, 1, "float"), (1000, 1 if overlap else STALE, "float")]),
                   series({"a": SECOND, "id": "second"}, [(0, 2 if overlap else STALE, "float"), (1000, 2, "float")])]
        add("full-hash-" + str(overlap), records, (inner, f'sum({inner})',
            'count_values("w",' + inner + ')',
            'count_values without(id,v)("v",{id=~".+"} @0)',
            'sum(count_values without(id,v)("v",{id=~".+"} @0))'))
    for name in ('"v"', '""', '"a\\xc5z"'):
        add("set-error-" + name.encode().hex(), ordinary, (f"count_values({name},m)",),
            faults={"m": {"set_error": "selection fault"}})
    add("iterator-error", ordinary, faults={"m": {"next_times": [1000], "message": "iterator fault"}})
    for ident, payload in (
        ("custom-zero", native(schema=-53, zero_count=bits(1))),
        ("custom-nan", native(schema=-53, zero_count="fff8000000000042")),
        ("custom-negative", native(schema=-53, negative_spans=[span(1, 1)], negative_buckets=[bits(0)])),
    ):
        add("raw-fields-" + ident, [native_series(payload)],
            ("m", "+m", "-m", 'count_values("v",+m)', 'count_values("v",-m)',
             "sum(m)", 'count_values("v",sum(m))', 'count_values("v",m*1)', "histogram_count(m)"))
    if len({row["request"]["id"] for row in inputs}) != len(inputs):
        raise ValueError("duplicate independently declared count_values identity")
    return inputs
