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


def mapped_cases():
    """Ordered scalar maps over groups, including nested consumers and fallback clocks."""
    output = []
    for profile in ("server-default", "upstream-tests"):
        for operator in OPERATORS:
            for grouping in ("by(cluster,zone)", "without(__name__,member)"):
                grouped = f"{operator} {grouping}(m_total)"
                expressions = (
                    grouped,
                    f"({grouped}) > 5",
                    f"({grouped}) > bool 5",
                    f"5 < ({grouped})",
                    f"abs({grouped})",
                    f"({grouped}) ^ 2",
                    f"2 * ({grouped})",
                    f"clamp_min({grouped},time()-3)",
                    f"sum(abs({grouped}))",
                    f"scalar(abs({grouped}))",
                    f"abs(sum by(cluster)({grouped}))",
                    f"({grouped}) > 1000000",
                )
                for index, expression in enumerate(expressions):
                    output.append(_case(f"mapped-groups-{profile}-{operator}-{grouping}-{index}",
                                        expression, _records(), profile))
            window = f"{operator} by(cluster,zone)(rate(m_total[3s]))"
            for index, expression in enumerate((
                f"({window}) > 5",
                f"({window}) > bool 5",
                f"abs({window})",
                f"sum(abs({window}))",
            )):
                output.append(_case(f"mapped-window-groups-{profile}-{operator}-{index}",
                                    expression, _records(), profile))
        grouped = "sum by(cluster,zone)(m_total)"
        for state in ("stale", "late", "empty", "native", "mixed", "all-native", "nan", "infinity", "zero"):
            for index, expression in enumerate((
                f"({grouped}) > 5",
                f"({grouped}) > bool 5",
                f"5 < ({grouped})",
                f"abs({grouped})",
                f"sum(abs({grouped}))",
                f"abs(avg by(cluster)(sum by(cluster,zone)(m_total)))",
                "abs(sum by(cluster,zone)(rate(m_total[3s])))",
                "abs(sum by(cluster,zone)(sum_over_time(m_total[3s])))",
            )):
                output.append(_case(f"mapped-state-groups-{profile}-{state}-{index}",
                                    expression, _records(state), profile))
        for index, expression in enumerate((
            f"-({grouped})",
            f"-(({grouped}) > 5)",
            f"({grouped}) * scalar(n_total{{zone=\"0\",member=\"0\"}})",
            "sum by(cluster,zone)(rate(m_total[3s] @ 2)) > 1",
            "sum by(cluster,zone)(rate(m_total[3s] offset 1s)) > 1",
            "abs(sum by(cluster,zone)(rate((m_total*1)[3s:1s])))",
            f"timestamp({grouped})",
            f"label_join({grouped},\"site\",\"/\",\"cluster\",\"zone\")",
        )):
            for instant in (False, True):
                output.append(_case(f"mapped-fallback-groups-{profile}-{index}-{instant}",
                                    expression, _records(), profile, instant=instant))
    return output


def identity_cases():
    output = []
    selector = '{__name__=~"m.+"}'
    expressions = (
        f'-{selector}', f'abs({selector})', f'{selector}*2',
        f'{selector}>0', f'{selector}>bool 0', f'0<{selector}',
        f'0<bool {selector}', f'abs({selector})<0', f'-{selector}+0',
        f'histogram_count({selector})', f'histogram_sum({selector})',
        f'timestamp({selector})', f'scalar(abs({selector}))',
        f'sum(abs({selector}))', f'abs({selector} @2)',
        f'sum_over_time((-{selector})[2s:1s])',
    )
    for state in ('unique', 'alias-float', 'alias-native', 'alias-mixed',
                  'disjoint', 'filtered', 'special', 'empty'):
        records = []
        for index in range(4):
            labels = {'__name__': f'm{index}', 'instance': str(index if state == 'unique' else index // 2), 'job': 'worker'}
            points = []
            for tick in range(6):
                kind, value = 'float', float((index + 1) * (tick + 1))
                if state == 'alias-native' or state == 'alias-mixed' and index % 2:
                    kind = 'float_histogram'
                elif state == 'disjoint' and (tick + index) % 2:
                    value = '7ff0000000000002'
                elif state == 'filtered' and index % 2:
                    value = -value
                elif state == 'special':
                    value = ('8000000000000000', '0000000000000000', '7ff8000000000042', '7ff0000000000000')[index]
                if state != 'empty':
                    points.append((tick * 1000, value, kind))
            records.append(series(labels, points))
        for profile in ('server-default', 'upstream-tests'):
            for instant in (False, True):
                for index, expression in enumerate(expressions):
                    output.append(_case(f'map-identity:{state}:{profile}:{instant}:{index}', expression, records, profile, instant=instant))
    return output
