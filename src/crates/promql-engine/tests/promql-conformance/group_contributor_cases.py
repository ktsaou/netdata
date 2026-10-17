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


def empty_owner_cases():
    """Globally empty owners beside productive singleton and later-range controls."""
    from base64 import b64decode
    from copy import deepcopy

    ordinary = _records(groups=4, members=2)
    later = deepcopy(ordinary)
    for record in later:
        labels = {b64decode(label["name"]): b64decode(label["value"])
                  for label in record["labels"]}
        if labels[b"__name__"] == b"m_total":
            for point in record["points"]:
                if point["time_ms"] < 2000:
                    point["float"] = "7ff0000000000002"
    queries = []
    for operator in OPERATORS:
        empty = f'{operator} by(cluster,zone)(m_total{{member="missing"}})'
        selected = f"sum(topk(64,{empty}))"
        ordinary_group = f"{operator} by(cluster,zone)(m_total)"
        queries.extend((
            selected,
            f"absent({selected})",
            f"quantile(.5,{operator} by(cluster)({empty}))",
            f"topk(64,{empty}) or on(cluster,zone) sum by(cluster,zone)(n_total)",
            f"vector(scalar({selected}))",
            f"{empty} / on() group_left sum(n_total)",
            f"sum(n_total) / on() group_right {empty}",
            f"sum(topk(64,{ordinary_group}))",
            f"sum(topk(64,{operator} by(cluster,zone)(rate(m_total[2s]))))",
            f"{ordinary_group} / on() group_left sum(n_total)",
            f"sum(n_total) / on() group_right {ordinary_group}",
        ))
    empty = 'sum by(cluster,zone)(m_total{member="missing"})'
    queries.extend((
        f"topk(NaN,{empty})",
        f"bottomk(Inf,{empty})",
        f"topk(scalar(sum(n_total)),{empty})",
        f"quantile(NaN,{empty})",
        'histogram_quantile(.5,sum by(le)(m_total{member="missing"}))',
        "sum(topk(64,sum by(cluster,zone)(m_total @ 1)))",
        "sum(topk(64,sum by(cluster,zone)(m_total offset 1s)))",
        "sum_over_time((sum(topk(64,sum by(cluster,zone)(m_total))))[2s:1s] @ 3)",
    ))
    output = []
    for shape, records in (("empty", []), ("ordinary", ordinary), ("later", later),
                           ("singleton", _records(groups=1, members=2))):
        declarations = [("server-default", index, expression) for index, expression in enumerate(queries)]
        declarations.extend(("upstream-tests", len(queries) + index, expression) for index, expression in enumerate((
            f"sum(limitk(64,{empty}))", f"sum(limit_ratio(1,{empty}))",
            "sum(limitk(64,sum by(cluster,zone)(m_total)))",
            "sum(limit_ratio(1,sum by(cluster,zone)(m_total)))",
        )))
        for profile, index, expression in declarations:
            for delayed in (False, True):
                for mode in ("instant", "equal", "range"):
                    output.append({
                        "request": {"id": f"empty-owner:{shape}:{profile}:{delayed}:{mode}:{index}",
                            "query": expression, "profile": profile, "delayed_name_removal": delayed,
                            "mode": "instant" if mode == "instant" else "range", "time_ms": 3000,
                            "start_ms": 0 if mode == "range" else 3000, "end_ms": 3000, "step_ms": 1000,
                            "lookback_ms": 1, "subquery_step_ms": 1000},
                        "dataset": {"schema_version": 1, "series": deepcopy(records)}, "faults": {},
                    })
    if len(output) != len({case["request"]["id"] for case in output}):
        raise ValueError("duplicate empty group owner input identity")
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


def local_join_cases():
    """Nested consumers of the complete ordinary grouped-operand panel."""
    output = []
    for case in cases():
        for parent in ("sum", "count"):
            for delayed in (False, True):
                request = case["request"]
                output.append({
                    **case,
                    "request": {
                        **request,
                        "id": f"local-join:{parent}:{delayed}:{request['id']}",
                        "query": f"{parent} by(cluster)({request['query']})",
                        "delayed_name_removal": delayed,
                    },
                })
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


def output_identity_cases():
    output = []
    grouped = 'sum by(cluster,zone)(m_total)'
    expressions = (
        f'-{grouped}', f'(-{grouped})', f'+(-{grouped})',
        f'abs({grouped})', f'-({grouped}>5)', f'-({grouped}>bool 5)',
        '-sum by(cluster,zone)(rate(m_total[3s]))',
        '-sum by(cluster,zone)(sum_over_time(m_total[3s]))',
        f'-({grouped} @2)', f'-({grouped} offset 1s)',
        f'label_join(-{grouped},"site","/","cluster","zone")',
        f'sum_over_time((-{grouped})[2s:1s])',
    )
    for state in ('dense', 'late', 'stale', 'native', 'mixed', 'all-native', 'nan', 'empty'):
        records = [] if state == 'empty' else _records(state, groups=4)
        for profile in ('server-default', 'upstream-tests'):
            for instant in (False, True):
                for index, expression in enumerate(expressions):
                    output.append(_case(f'output-identity:{state}:{profile}:{instant}:{index}',
                                        expression, records, profile, instant=instant))
    return output


def projected_cases():
    """Ordered label consumers over grouped values, scopes and merged aliases."""
    output = []

    def joined(body):
        return f'label_join({body},"site","/","cluster","zone")'

    def replaced(body):
        return f'label_replace({body},"site","zone_$1","zone","(.*)")'

    def add(ident, expression, records, profile, delayed, mode="range"):
        case = _case(f"projected-groups:{profile}:{delayed}:{ident}:{mode}",
                     expression, records, profile, instant=mode == "instant")
        case["request"]["delayed_name_removal"] = delayed
        if mode == "equal":
            case["request"].update(mode="range", start_ms=3000, end_ms=3000)
        output.append(case)

    for profile in ("server-default", "upstream-tests"):
        for delayed in (False, True):
            dense = _records(groups=4, members=2)
            for operator in OPERATORS:
                grouped = f"{operator} by(cluster,zone)(m_total)"
                rate = f"{operator} by(cluster,zone)(rate(m_total[3s]))"
                fold = f"{operator} by(cluster,zone)(sum_over_time(m_total[3s]))"
                expressions = {
                    "join": joined(grouped),
                    "replace": replaced(grouped),
                    "without": joined(f"{operator} without(member)(m_total)"),
                    "rate": replaced(rate),
                    "fold": joined(fold),
                    "mapped-input": joined(f"{operator} by(cluster,zone)(abs(m_total*2))"),
                    "mapped-output": f"abs({joined(grouped)})",
                    "nested-group": joined(f"{operator} by(cluster)(avg by(cluster,zone)(m_total))"),
                    "nested-label": replaced(joined(grouped)),
                    "filtered": joined(f"({grouped}) > 10"),
                    "no-survivors": replaced(f"({grouped}) < -1000"),
                    "metric-name": f'label_replace({grouped},"__name__","projected_$1","zone","(.*)")',
                }
                for shape, expression in expressions.items():
                    add(f"{operator}:{shape}", expression, dense, profile, delayed)
                for shape in ("join", "rate"):
                    for mode in ("instant", "equal"):
                        add(f"{operator}:{shape}", expressions[shape], dense, profile, delayed, mode)
            grouped = "sum by(cluster,zone)(m_total)"
            for state in ("stale", "late", "empty", "native", "mixed", "all-native", "nan", "infinity", "zero"):
                records = _records(state, groups=4, members=2)
                for shape, expression in enumerate((joined(grouped), replaced(grouped),
                        joined("sum by(cluster,zone)(rate(m_total[3s]))"),
                        replaced("avg by(cluster,zone)(sum_over_time(m_total[3s]))"))):
                    add(f"state:{state}:{shape}", expression, records, profile, delayed)
            fallbacks = (
                joined("sum by(cluster,zone)(m_total @ 2)"),
                replaced("sum by(cluster,zone)(m_total offset 1s)"),
                joined("sum by(cluster,zone)(rate(m_total[3s] @ 2))"),
                replaced("sum by(cluster,zone)(rate(m_total[3s] offset 1s))"),
                joined("sum by(cluster,zone)(timestamp(m_total))"),
                joined('sum by(cluster,zone)(m_total * scalar(n_total{zone="0",member="0"}))'),
                joined("sum by(cluster,zone)(rate((m_total*1)[3s:1s]))"),
                f"sum_over_time(({joined(grouped)})[2s:1s])",
                f"topk(2,{replaced(grouped)})",
                f"quantile(.5,{joined(grouped)})",
            )
            for index, expression in enumerate(fallbacks):
                for mode in ("instant", "range"):
                    add(f"scope:{index}", expression, dense, profile, delayed, mode)
            for state in ("disjoint", "overlap", "mixed", "single", "empty"):
                records = []
                count = 1 if state == "single" else 4
                for group in range(count):
                    for member in range(2):
                        points = []
                        if state != "empty":
                            for tick in range(6):
                                value = float((group + 1) * (member + 1) * (tick + 1))
                                kind = "float_histogram" if state == "mixed" and group % 2 else "float"
                                if state == "disjoint" and group != tick % count:
                                    value, kind = "7ff0000000000002", "float"
                                points.append((tick * 1000, value, kind))
                        records.append(series({"__name__": "m_total", "zone": str(group),
                                               "member": str(member)}, points))
                aggregate = "sum by(zone)(m_total)"
                for shape, expression in enumerate((
                    f'label_join({aggregate},"zone","","missing")',
                    f'label_replace({aggregate},"zone","","zone",".*")',
                )):
                    for mode in ("instant", "range"):
                        add(f"aliases:{state}:{shape}", expression, records, profile, delayed, mode)
            for shape, expression in enumerate((joined(grouped), replaced(grouped))):
                add(f"absent:{shape}", expression, [], profile, delayed)
    assert len({case["request"]["id"] for case in output}) == len(output)
    return output
