"""Mapped rate joins, original row order, changing scopes and ordinary fallbacks."""

from info_cases import series


def _case(
    ident, expression, records, profile="server-default", *, instant=False, end=5000
):
    return {
        "request": {
            "id": ident,
            "query": expression,
            "profile": profile,
            "mode": "instant" if instant else "range",
            "time_ms": 3000 if instant else 2000,
            "start_ms": 0 if instant else 2000,
            "end_ms": 0 if instant else end,
            "step_ms": 1000,
            "lookback_ms": 300000,
            "subquery_step_ms": 1000,
        },
        "dataset": {"schema_version": 1, "series": records},
        "faults": {},
    }


def performance_cases():
    expressions = [
        ("direct", "rate(m_total[5m]) / on(id) n_total"),
        ("raw-map", "(m_total*100) / on(id) n_total"),
        ("clamp", "clamp_min(rate(m_total[5m]),0) / on(id) n_total"),
        ("scale", "(rate(m_total[5m])*100) / on(id) n_total"),
        ("absolute", "abs(rate(m_total[5m])) / on(id) n_total"),
        ("both", "clamp_min(rate(m_total[5m]),0) / on(id) rate(n_total[5m])"),
    ]
    output = []
    for count in [16, 32, 64]:
        records = []
        for metric in ["m_total", "n_total"]:
            indices = range(count) if metric == "m_total" else reversed(range(count))
            for index in indices:
                records.append(
                    series(
                        {"__name__": metric, "id": str(index), "job": "worker"},
                        [(1000, 1.0, "float"), (2000, 2.0, "float")],
                    )
                )
        for shape, expression in expressions:
            output.append(
                _case(
                    f"wrapped-rate-{shape}-pairs{count}", expression, records, end=3000
                )
            )
    return output


def _records(state="dense"):
    output = []
    for metric, order in [("m_total", [2, 0, 3, 1]), ("n_total", [3, 1, 0, 2])]:
        for index in order:
            labels = {"__name__": metric, "id": str(index), "job": "worker"}
            points = [
                (tick * 1000, (index + 1) * value, "float")
                for tick, value in enumerate([1, 2, 3, 1, 2, 3])
            ]
            if metric == "m_total" and index == 1:
                if state == "sparse":
                    points = [
                        (0, 1, "float"),
                        (1000, 2, "float"),
                        (2000, "7ff0000000000002", "float"),
                        (3000, "7ff0000000000002", "float"),
                        (4000, 3, "float"),
                        (5000, 4, "float"),
                    ]
                elif state in ("histogram", "float_histogram"):
                    points = [(tick * 1000, tick + 1, state) for tick in range(6)]
                elif state == "mixed":
                    points = [
                        (
                            tick * 1000,
                            tick + 1,
                            "float_histogram" if tick in (2, 3) else "float",
                        )
                        for tick in range(6)
                    ]
                elif state == "nan":
                    points = [
                        (
                            tick * 1000,
                            "7ff8000000000042" if tick in (1, 3) else tick + 1,
                            "float",
                        )
                        for tick in range(6)
                    ]
            output.append(series(labels, points))
    return output


def cases():
    output = performance_cases()
    dense = _records()
    expressions = [
        "clamp_min(rate(m_total[3s]),0) / on(id) n_total",
        "rate(m_total[3s]) / on(id) abs(rate(n_total[3s]))",
        "clamp_min(rate(m_total[3s]),0) / on(id) abs(rate(n_total[3s]))",
        "round(abs(clamp_min(rate(m_total[3s]),-time())),.25) / on(id) n_total",
        "(100*rate(m_total[3s])) / on(id) n_total",
        "(rate(m_total[3s])*100) / on(id) n_total",
        "(-rate(m_total[3s])) / on(id) (-rate(n_total[3s]))",
        "((+rate(m_total[3s]))) / on(id) n_total",
        "clamp(rate(m_total[3s]),0,.5-(time()-3)^2) / on(id) n_total",
        "(rate(m_total[3s]) > time()-2) / on(id) n_total",
        "((time()-2) < bool rate(m_total[3s])) / on(id) n_total",
        "clamp_min(rate(m_total[3s]),NaN) / on(id) n_total",
        'clamp_min(rate(m_total[3s]),scalar(n_total{id="0"})) / on(id) n_total',
        "clamp_min(rate(m_total[3s]),scalar(vector(time()))) / on(id) n_total",
        "abs(rate(m_total[3s] @ start())) / on(id) n_total",
        "abs(rate(m_total[3s] @ end())) / on(id) n_total",
        "abs(rate(m_total[3s] @ 2)) / on(id) n_total",
        "clamp_min(rate(m_total[3s] offset 1s),0) / on(id) n_total",
        "abs(rate(m_total[3s] offset 0.000000001)) / on(id) n_total",
        "abs(rate(m_total[0.000000001])) / on(id) n_total",
        "abs(rate(m_total[0.000999999] offset 0.000000001)) / on(id) n_total",
        "abs(rate(m_total[2.999999999] offset 0.000000001)) / on(id) n_total",
        "abs(rate(m_total[3s] offset -0.0015)) / on(id) n_total",
        "abs(rate(m_total[3s] offset 0.0005)) / on(id) n_total",
        "year(rate(m_total[3s])) / on(id) n_total",
        "abs(rate((m_total*1)[3s:1s])) / on(id) n_total",
        "sum by(id)(clamp_min(rate(m_total[3s]),0)) / on(id) n_total",
        'label_join(abs(rate(m_total[3s])),"extra","","job") / on(id) n_total',
    ]
    state_expressions = [expressions[index] for index in (0, 1, 2, 8)]
    for profile in ["server-default", "upstream-tests"]:
        for index, expression in enumerate(expressions):
            output.append(
                _case(
                    f"mapped-rate-dense-{profile}-{index}", expression, dense, profile
                )
            )
        for state in ["sparse", "histogram", "float_histogram", "mixed", "nan"]:
            records = _records(state)
            for index, expression in enumerate(state_expressions):
                output.append(
                    _case(
                        f"mapped-rate-{state}-{profile}-{index}",
                        expression,
                        records,
                        profile,
                    )
                )
        for index in (0, 2, 8):
            output.append(
                _case(
                    f"mapped-rate-instant-{profile}-{index}",
                    expressions[index],
                    dense,
                    profile,
                    instant=True,
                )
            )

        if profile == "upstream-tests":
            for modifier in ["anchored", "smoothed"]:
                output.append(
                    _case(
                        f"mapped-rate-extended-{modifier}",
                        f"abs(rate(m_total[3s] {modifier})) / on(id) n_total",
                        dense,
                        profile,
                    )
                )

        group = [
            series(
                {"__name__": "m_total", "id": str(index), "job": "worker"},
                [(tick * 1000, (index + 1) * (tick + 1), "float") for tick in range(6)],
            )
            for index in (2, 0, 3, 1)
        ]
        group.append(
            series(
                {"__name__": "n_total", "job": "worker", "site": "north"},
                [(tick * 1000, tick + 1, "float") for tick in range(6)],
            )
        )
        for index, expression in enumerate(
            [
                "clamp_min(rate(m_total[3s]),0) / on(job) group_left(site) rate(n_total[3s])",
                "rate(n_total[3s]) / on(job) group_right(site) abs(rate(m_total[3s]))",
                "(rate(m_total[3s])*100) / on(job) group_left(site) (100*rate(n_total[3s]))",
            ]
        ):
            output.append(
                _case(
                    f"mapped-rate-group-{profile}-{index}", expression, group, profile
                )
            )

        duplicate = list(dense)
        duplicate.append(
            series(
                {"__name__": "other_total", "id": "0", "job": "worker"},
                [(tick * 1000, tick + 1, "float") for tick in range(6)],
            )
        )
        for index, expression in enumerate(
            [
                'abs(rate({__name__=~"m_total|other_total"}[3s])) / on(id) n_total',
                '(abs(rate({__name__=~"m_total|other_total"}[3s])) > Inf) / on(id) n_total',
                'clamp(rate({__name__=~"m_total|other_total"}[3s]),1,0) / on(id) n_total',
            ]
        ):
            output.append(
                _case(
                    f"mapped-rate-collision-{profile}-{index}",
                    expression,
                    duplicate,
                    profile,
                )
            )
    return output
