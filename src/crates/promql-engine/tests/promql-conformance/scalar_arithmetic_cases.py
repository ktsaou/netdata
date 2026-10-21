"""Literal-sided scalar arithmetic and its ordinary evaluation boundaries."""

from group_contributor_cases import OPERATORS, _records


def cases():
    panels = []
    for state in ("dense", "all-native", "mixed", "stale", "late", "nan", "infinity", "zero", "empty"):
        requests = []

        def add(expression, tag, mode="range", start=1000, end=4000, profile="server-default"):
            for delayed in (False, True):
                requests.append({
                    "id": f"scalar-arithmetic:{state}:{tag}:d{int(delayed)}",
                    "query": expression, "profile": profile,
                    "delayed_name_removal": delayed, "mode": mode, "time_ms": 3000,
                    "start_ms": start, "end_ms": end, "step_ms": 1000,
                    "lookback_ms": 300000, "subquery_step_ms": 60000,
                })

        for reducer in OPERATORS:
            for body in ("n_total", "rate(n_total[3s])"):
                scalar = f"scalar({reducer}({body}))"
                for literal_left in (False, True):
                    wrapped = f"(1+{scalar})" if literal_left else f"({scalar}+1)"
                    for reverse in (False, True):
                        add(f"{wrapped}/m_total" if reverse else f"m_total/{wrapped}",
                            f"{reducer}-{body}-literal{literal_left}-reverse{reverse}")
        scalar = "scalar(sum(n_total))"
        for operator in ("-", "*", "/", "%", "^", "atan2",
                         "> bool", "< bool", ">= bool", "<= bool", "== bool", "!= bool", ">/", "</"):
            for literal_left in (False, True):
                wrapped = f"(2 {operator} {scalar})" if literal_left else f"({scalar} {operator} 2)"
                add(f"m_total/{wrapped}", f"operator-{operator}-{literal_left}")
        for literal in ("0", "-0", "NaN", "Inf", "(1)", "(1+1)"):
            add(f"m_total/({scalar}+{literal})", f"literal-right-{literal}")
            add(f"m_total/({literal}+{scalar})", f"literal-left-{literal}")
        for tag, expression in {
            "nested": f"m_total/(2-({scalar}-1))",
            "nested-power": f"m_total/(2^({scalar}%3))",
            "dependent": f"m_total/({scalar}+{scalar})",
            "time": f"m_total/({scalar}+time())",
            "negated": f"m_total/(-{scalar}+1)",
            "aggregate-nested": "m_total/(scalar(sum(sum by(zone)(n_total)))+1)",
            "grouped": "m_total/(scalar(sum by(zone)(n_total))+1)",
            "fixed": "m_total/(scalar(sum(n_total @ 2))+1)",
            "offset": "m_total/(scalar(sum(n_total offset 1s))+1)",
            "single": f'm_total{{zone="0",member="0"}}/({scalar}+1)',
            "empty-denominator": 'm_total/(scalar(sum(n_total{member="missing"}))+1)',
            "empty-numerator": f'm_total{{member="missing"}}/({scalar}+1)',
            "standalone": f"{scalar}+1",
        }.items():
            add(expression, tag)
        for function in ("min_of", "max_of"):
            add(f"m_total/{function}({scalar},1)", function, profile="upstream-tests")
        add(f"m_total/({scalar}+1)", "instant", mode="instant")
        add(f"m_total/({scalar}+1)", "equal", start=3000, end=3000)
        panels.append({
            "id": state,
            "dataset": {"schema_version": 1, "series": _records(
                state, groups=0 if state == "empty" else 3, members=2)},
            "requests": requests,
        })
    return panels
