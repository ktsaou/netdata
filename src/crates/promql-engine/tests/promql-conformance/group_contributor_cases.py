"""Ordinary grouped contributors: distinct values, joins and retained fallbacks."""

from info_cases import series
from mapped_rate_cases import _case

OPERATORS = ("sum", "avg", "count", "min", "max", "group", "stdvar", "stddev")


def _records(state="dense", groups=3, members=4):
    output = []
    for metric, side in (("m_total", 0), ("n_total", 1)):
        order = range(groups) if side == 0 else reversed(range(groups))
        for group in order:
            for member in reversed(range(members)) if side else range(members):
                labels = {
                    "__name__": metric,
                    "cluster": str(group % 2),
                    "zone": str(group),
                    "member": str(member),
                }
                points = []
                for tick in range(6):
                    kind = "float"
                    value = (group + 1) * (member + 1) * (tick + 1)
                    if side:
                        value += (member + 1) ** 2 + 3
                    if group == 1 and member == 1:
                        if state == "stale" and tick in (2, 3):
                            value = "7ff0000000000002"
                        elif state == "late" and tick < 3:
                            continue
                        elif state == "empty":
                            continue
                        elif state == "native":
                            kind = "float_histogram"
                        elif state == "mixed" and tick in (2, 3):
                            kind = "float_histogram"
                        elif state == "nan":
                            value = "7ff8000000000042"
                        elif state == "infinity":
                            value = "7ff0000000000000" if side else "fff0000000000000"
                        elif state == "zero":
                            value = "0000000000000000" if side else "8000000000000000"
                    if state == "all-native":
                        kind = "float_histogram"
                    points.append((tick * 1000, value, kind))
                output.append(series(labels, points))
    return output


def cases():
    output = []
    for profile in ("server-default", "upstream-tests"):
        for operator in OPERATORS:
            left = f"{operator} by(cluster,zone)(m_total)"
            right = f"{operator} by(cluster,zone)(n_total)"
            window = f"{operator} by(cluster,zone)(rate(m_total[3s]))"
            fold = f"{operator} by(cluster,zone)(sum_over_time(m_total[3s]))"
            nested = f"{operator} by(cluster)(avg by(cluster,zone)(m_total))"
            expressions = {
                "standalone-by": left,
                "standalone-without": f"{operator} without(member)(m_total)",
                "raw": f"{left} / on(cluster,zone) {right}",
                "without": (
                    f"{operator} without(member)(m_total)"
                    f" / on(cluster,zone) {operator} without(member)(n_total)"
                ),
                "mapped-inner": (
                    f"{operator} by(cluster,zone)(abs(m_total*2))"
                    f" / on(cluster,zone) {right}"
                ),
                "mapped-outer": f"abs({left}*2) / on(cluster,zone) {right}",
                "rate": f"({window}+1) / on(cluster,zone) ({window.replace('m_total', 'n_total')}+1)",
                "fold": f"{fold} / on(cluster,zone) {fold.replace('m_total', 'n_total')}",
                "mixed-left": f"({window}+1) / on(cluster,zone) ({right}+1)",
                "mixed-right": f"({left}+1) / on(cluster,zone) ({window.replace('m_total', 'n_total')}+1)",
                "nested": f"({nested}+1) / on(cluster) ({nested.replace('m_total', 'n_total')}+1)",
                "boolean": f"{left} > bool on(cluster,zone) {right}",
                "fixed-left": f"{operator} by(cluster,zone)(m_total @ 2) / on(cluster,zone) {right}",
                "fixed-right": f"{left} / on(cluster,zone) {operator} by(cluster,zone)(n_total @ 2)",
                "offset": f"{operator} by(cluster,zone)(m_total offset 1s) / on(cluster,zone) {right}",
                "group-left": f"{left} / on(cluster) group_left {operator} by(cluster)(n_total)",
                "group-right": f"{operator} by(cluster)(m_total) / on(cluster) group_right {right}",
            }
            for name, expression in expressions.items():
                output.append(_case(f"contributors-{profile}-{operator}-{name}", expression, _records(), profile))
            for state in ("stale", "late", "empty", "native", "mixed", "all-native", "nan", "infinity", "zero"):
                for name in ("raw", "rate", "boolean"):
                    output.append(_case(f"contributors-{profile}-{operator}-{state}-{name}", expressions[name], _records(state), profile))
            output.append(_case(f"contributors-{profile}-{operator}-instant", expressions["raw"], _records(), profile, instant=True))
    return output


def performance_cases():
    output = []
    for groups, members in ((4, 4), (8, 4), (16, 4), (32, 4), (8, 1), (8, 16), (8, 64)):
        records = _records(groups=groups, members=members)
        for name, body in (("raw", "m_total"), ("rate", "rate(m_total[3s])")):
            left = f"sum by(cluster,zone)({body})"
            for joined in (False, True):
                expression = left
                if joined:
                    expression += " / on(cluster,zone) " + left.replace("m_total", "n_total")
                output.append(_case(f"contributors-perf-{name}-{'join' if joined else 'alone'}-g{groups}-m{members}", expression, records, end=3000))
    return output
