"""Independent source-input panel for histogram annotation callers and UTC formatting."""
import datetime
import random
import struct

MONOTONIC = (b"PromQL info: input to histogram_quantile needed to be fixed for monotonicity "
             b"(see https://prometheus.io/docs/prometheus/latest/querying/functions/#histogram_quantile)")


def bits(value):
    return struct.pack(">d", value).hex()


def mono(timestamp=0, low="3ff0000000000000", high="4000000000000000",
         decrease="3ff0000000000000", metric=b"m", start=0, end=1, op="add"):
    return {"op": op, "kind": "monotonic", "metric": metric.hex(), "start": start, "end": end,
            "time_ms": timestamp, "min": low, "max": high, "diff": decrease}


def generate():
    cases = []

    def add(ident, events, query=b"a+b+c"):
        cases.append({"id": ident, "query": query.hex(), "events": events})

    # Original annotations_test.go:43-45 constructor arguments, independently checked by its untouched assertion.
    add("original-go-monotonic", [mono(ts, bits(low), bits(high), bits(diff), b"series_1", 3, 8)
                                for ts, low, high, diff in ((1735084800000, 5.0, 50.0, 5.5),
                                                           (1703462400000, 10.0, 100.0, 10.0),
                                                           (1733011200000, 2.5, 75.0, 7.5))],
        b"lorem ipsum dolor sit amet")
    for index, query in enumerate((b"", b"a", b"a\n\xc3\xa9b")):
        for position in (-1, 0, 1, 2, 4, 5, 32):
            add(f"position-{index}-{position}", [mono(start=position, end=position+1)], query)

    times = {-(1 << 63), (1 << 63)-1, -1001, -1000, -999, -1, 0, 1, 999, 1000, 1001,
             -62167219200000, -62198755200000, 253402300799999, 253402300800000}
    for power in range(64):
        for sign in (-1, 1):
            for adjustment in (-1, 0, 1):
                value = sign*(1 << power)+adjustment
                if -(1 << 63) <= value < (1 << 63):
                    times.add(value)
    epoch = datetime.datetime(1970, 1, 1, tzinfo=datetime.timezone.utc)
    for year in (1, 4, 100, 400, 1582, 1900, 1970, 2000, 2038, 2100, 2400, 9999):
        for month in (1, 2, 3, 12):
            value = int((datetime.datetime(year, month, 1, tzinfo=datetime.timezone.utc)-epoch).total_seconds())*1000
            for adjustment in (-1001, -1000, -999, -1, 0, 1, 999, 1000, 1001):
                times.add(value+adjustment)
    rng = random.Random(0xA6607A710)
    for _ in range(256):
        value = rng.getrandbits(64)
        times.add(value-(1 << 64) if value >= (1 << 63) else value)
    for index, timestamp in enumerate(sorted(times)):
        add(f"utc-{index:04d}", [mono(timestamp)])

    values = ["0000000000000000", "8000000000000000", "0000000000000001", "8000000000000001",
              "3ff0000000000000", "bff0000000000000", "7ff0000000000000", "fff0000000000000",
              "7ff8000000000042", "fff8000000000043", "7ff0000000000042", "7ff0000000000002"]
    for index, first in enumerate(values):
        for second_index, second in enumerate(values):
            for order in (0, 1):
                left, right = (first, second) if order == 0 else (second, first)
                add(f"strict-extrema-{index:02d}-{second_index:02d}-{order}",
                    [mono(1001, left, left, left, start=0), mono(-1001, right, right, right, start=2)])
    add("source-infinite-sentinels", [mono(0, "7ff0000000000000", "fff0000000000000")])

    names = (b"", b"m", b"m\xff\n", "α🙂".encode(), b"metric"*400)
    kinds = ("mixed_classic_native", "quantile_nan_result", "quantile_nan_skew", "fraction_nans")
    for index, name in enumerate(names):
        for kind in kinds:
            add(f"optional-name-{kind}-{index}",
                [{"op": "add", "kind": kind, "metric": name.hex(), "start": 0, "end": 1},
                 {"op": "add", "kind": kind, "metric": name.hex(), "start": 2, "end": 3}])
        for label_index, label in enumerate((b"", b"NaN!", b"1e99999", b"\xff\n", "λ".encode())):
            add(f"bad-bucket-{index}-{label_index}",
                [{"op": "add", "kind": "bad_bucket", "metric": name.hex(), "label": label.hex(),
                  "start": 0, "end": 1},
                 {"op": "add", "kind": "bad_bucket", "metric": name.hex(), "label": label.hex(),
                  "start": 2, "end": 3}])
        add(f"monotonic-name-{index}", [mono(metric=name)])
    quantiles = values + [bits(value) for value in (-1.1, 0.5, 1.1, 1e-10, 1e6)]
    quantiles += [f"{rng.getrandbits(64):016x}" for _ in range(128)]
    for index, value in enumerate(quantiles):
        add(f"invalid-q-{index:03d}", [{"op": "add", "kind": "invalid_quantile", "q": value,
                                       "start": 0, "end": 1},
                                      {"op": "add", "kind": "invalid_quantile", "q": value,
                                       "start": 2, "end": 3}])
    raw = MONOTONIC+b' for metric name "m"'
    provider = {"op": "add", "kind": "opaque", "raw": raw.hex(), "severity": "warning",
                "no_position": True}
    add("opaque-replaces-monotonic", [mono(), mono(start=2), provider])
    add("generated-replaces-opaque", [mono(), provider, mono(start=4)])
    add("generated-restarts-count", [mono(), provider, mono(start=4), mono(start=2)])
    add("child-merge-first-position", [mono(1000), mono(-1000, start=2, op="child_add"),
                                        mono(2000, start=4, op="child_add"), {"op": "merge"}])
    add("child-merge-replacement", [mono(), dict(provider, op="child_add"), {"op": "merge"}, mono(start=4)])
    add("unmerged-child", [mono(op="child_add")])
    add("empty-merge", [{"op": "merge"}])
    add("all-callers", [mono()] + [{"op": "add", "kind": kind, "metric": b"m".hex(), "start": 2, "end": 3}
                                   for kind in kinds])
    for steps in (1, 2, 3, 10, 100, 1025):
        add(f"occurrences-{steps}", [mono(index*1000, start=index % 5, end=index % 5 + 1)
                                    for index in range(steps)])
    assert len({case["id"] for case in cases}) == len(cases)
    return cases
