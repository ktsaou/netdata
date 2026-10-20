"""Scalar reductions consumed by ordered vectors and ordinary fallback shapes."""

from group_contributor_cases import OPERATORS, _records


def cases():
    panels = []
    for state in ("dense", "all-native", "mixed", "stale", "late", "nan", "infinity", "zero", "empty"):
        records = _records(state, groups=0 if state == "empty" else 3, members=2)
        requests = []

        def add(expression, tag, mode="range", start=1000, end=4000):
            for delayed in (False, True):
                requests.append({
                    "id": f"scalar-group:{state}:{tag}:d{int(delayed)}",
                    "query": expression, "profile": "server-default",
                    "delayed_name_removal": delayed, "mode": mode, "time_ms": 3000,
                    "start_ms": start, "end_ms": end, "step_ms": 1000,
                    "lookback_ms": 300000, "subquery_step_ms": 60000,
                })

        for reducer in OPERATORS:
            for body in ("n_total", "rate(n_total[3s])"):
                denominator = f"scalar({reducer}({body}))"
                add(f"m_total/{denominator}", f"{reducer}-{body}-right")
                add(f"{denominator}/m_total", f"{reducer}-{body}-left")
        for tag, expression in {
            "parens": "m_total/((scalar((sum by()(n_total)))))",
            "nested": "m_total/scalar(sum(sum by(zone)(n_total)))",
            "grouped": "m_total/scalar(sum by(zone)(n_total))",
            "arithmetic": "m_total/(scalar(sum(n_total))+1)",
            "fixed": "m_total/scalar(sum(n_total @ 2))",
            "offset": "m_total/scalar(sum(n_total offset 1s))",
            "single": 'm_total{zone="0",member="0"}/scalar(sum(n_total))',
            "empty-denominator": 'm_total/scalar(sum(n_total{member="missing"}))',
            "empty-denominator-left": 'scalar(sum(n_total{member="missing"}))/m_total',
            "empty-numerator": 'm_total{member="missing"}/scalar(sum(n_total))',
            "standalone": "scalar(sum(n_total))",
        }.items():
            add(expression, tag)
        add("m_total/scalar(sum(n_total))", "instant", mode="instant")
        add("m_total/scalar(sum(n_total))", "equal", start=3000, end=3000)
        panels.append({"id": state, "dataset": {"schema_version": 1, "series": records},
                       "requests": requests})
    return panels
