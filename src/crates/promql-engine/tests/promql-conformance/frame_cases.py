"""Complete scheduled-frame allocation, clock and fresh-parameter controls."""

from info_cases import series


def _case(shape, expression, count):
    labels = {
        "__name__": "m",
        "instance": "0",
        "job": "worker",
        "payload": "Q" * 16,
    }
    records = [
        series(
            labels,
            [((index + 1) * 1000, index + 1, "float") for index in range(count)],
        )
    ]
    return {
        "request": {
            "id": f"frame-{shape}-ticks{count}",
            "query": expression,
            "profile": "server-default",
            "mode": "range",
            "time_ms": 1000,
            "start_ms": 1000,
            "end_ms": count * 1000,
            "step_ms": 1000,
            "lookback_ms": 1000,
            "subquery_step_ms": 1000,
        },
        "dataset": {"schema_version": 1, "series": records},
        "faults": {},
    }


def cases():
    expressions = [
        ("raw", "m"),
        ("map", "m+0"),
        ("absolute", "abs(m)"),
        ("deep8", "abs(" * 8 + "m" + ")" * 8),
        ("parameter", "round(abs(m),.5)+time()"),
        ("filtered", "m>128"),
    ]
    inputs = [
        _case(shape, expression, count)
        for count in [1, 16, 64, 256]
        for shape, expression in expressions
    ]
    inputs += [
        _case(shape, expression, 16)
        for shape, expression in [
            ("fixed-scalar", "m @ 1 + time()"),
            ("fixed-timestamp", "timestamp(m @ 1)"),
            ("start-scalar", "m @ start() + time()"),
            ("end-timestamp", "timestamp(m @ end())"),
            ("changing-clamp", "clamp(m,time()-.5,time()+.5)"),
            ("changing-round", "round(m,time()+.5)"),
        ]
    ]
    return inputs
