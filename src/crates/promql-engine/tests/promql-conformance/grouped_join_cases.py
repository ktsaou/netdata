"""Complete grouped-join interactions for the pinned query oracle."""
from copy import deepcopy
from info_cases import series


def cases():
    output = []
    times = [0, 1000, 2000, 3000, 4000]
    stale = '7ff0000000000002'
    left = 'm / on(host) group_left(site) n'
    right = 'n / on(host) group_right(site) m'

    def points(base):
        return [(time, base + index, 'float') for index, time in enumerate(times)]

    def ordinary(count=3):
        records = [series({'__name__': 'm', 'host': 'h', 'cpu': str(index)}, points(2 + 6 * index))
                   for index in range(count)]
        records.append(series({'__name__': 'n', 'host': 'h', 'site': 'north'},
                              [(time, 4, 'float') for time in times]))
        return records

    def add(tag, records, expressions=(left, right)):
        for index, expression in enumerate(expressions):
            for profile in ['server-default', 'upstream-tests']:
                for mode in ['instant', 'range']:
                    output.append({'request': {
                        'id': f'grouped:{tag}:{index}:{profile}:{mode}', 'query': expression,
                        'profile': profile, 'mode': mode, 'time_ms': 4000,
                        'start_ms': 2000, 'end_ms': 4000, 'step_ms': 1000,
                        'lookback_ms': 1000, 'subquery_step_ms': 1000},
                        'dataset': {'schema_version': 1, 'series': records}, 'faults': {}})

    expressions = []
    for operator in ['+', '-', '*', '/', '%', '^', 'atan2', '>', '> bool', '== bool']:
        expressions += [f'm {operator} on(host) group_left(site) n',
                        f'n {operator} on(host) group_right(site) m']
    add('operators', ordinary(), expressions)
    add('permuted', list(reversed(ordinary())), (
        left, right, 'm / ignoring(cpu,site) group_left(site) n',
        'n / ignoring(cpu,site) group_right(site) m',
        'm / on(host) group_left n', 'n / on(host) group_right m'))
    add('maps', ordinary(), (
        'abs(m) / on(host) group_left(site) round(n,.5)',
        '(n*4) / on(host) group_right(site) (-m)',
        'label_join(m,"cpu","","cpu") / on(host) group_left(site) n',
        'm / on(host) group_left(site) (n*4)'))
    add('singleton', ordinary(1))
    add('empty', [])
    add('missing-one', ordinary()[:-1])
    add('missing-many', ordinary()[-1:])
    add('fixed-time', ordinary(), (
        'm @end() / on(host) group_left(site) n',
        'n / on(host) group_right(site) m offset 1s'))

    records = ordinary(2)
    records += [series({'__name__': 'm', 'host': 'other', 'cpu': 'x'}, points(10)),
                series({'__name__': 'n', 'host': 'other', 'site': 'south'}, points(5)),
                series({'__name__': 'm', 'host': 'unmatched', 'cpu': 'z'}, points(20)),
                series({'__name__': 'n', 'host': 'unused', 'site': 'unused'}, points(30))]
    add('multiple-signatures', records)

    records = ordinary(2)
    for index in [0, 2]:
        records[index]['points'] = series({}, [(0, stale, 'float'), (2000, stale, 'float'),
            (3000, index + 2, 'float'), (4000, stale, 'float')])['points']
    add('changing-membership', records)

    records = ordinary(2)[:-1]
    records += [series({'__name__': 'n', 'host': 'h', 'site': 'old'},
                      [(0, 4, 'float'), (2000, 4, 'float'), (3000, stale, 'float'), (4000, stale, 'float')]),
                series({'__name__': 'n', 'host': 'h', 'site': 'new'},
                      [(0, stale, 'float'), (2000, stale, 'float'), (3000, 8, 'float'), (4000, 8, 'float')])]
    add('one-side-history', records)
    records = ordinary(2)
    records.append(series({'__name__': 'n', 'host': 'h', 'site': 'duplicate'}, points(8)))
    add('one-side-ambiguous', records)

    records = ordinary(2)
    records[-1] = series({'__name__': 'n', 'host': 'h', 'site': 'north', 'cpu': 'same'}, points(5))
    add('include-collision', records, (
        'm / on(host) group_left(cpu) n', 'n / on(host) group_right(cpu) m',
        'm > on(host) group_left(cpu) n', 'n > on(host) group_right(cpu) m'))

    add('false-first-writer', [
        series({'__name__': 'a', 'k': 'x'}, [(2000, 2, 'float'), (4000, stale, 'float')]),
        series({'__name__': 'a'}, [(2000, stale, 'float'), (4000, 7, 'float')]),
        series({'__name__': 'b'}, [(2000, 4, 'float'), (4000, stale, 'float')]),
        series({'__name__': 'b', 'k': 'x'}, [(2000, stale, 'float'), (4000, 5, 'float')])],
        ('a > on() group_left(k) sum by(k)(b)',))

    records = ordinary(2)
    records[0]['points'] = series({}, [(time, '8000000000000000', 'float') for time in times])['points']
    records[1]['points'] = series({}, [(time, '7ff8000000000042', 'float') for time in times])['points']
    add('float-bits', records, (left, right, 'm > bool on(host) group_left(site) n'))
    records = [series({'__name__': 'm', 'host': b'\xff', 'cpu': str(index)}, points(index + 2))
               for index in range(2)]
    records.append(series({'__name__': 'n', 'host': b'\xff', 'site': b'\xfe'}, points(4)))
    add('byte-labels', records)

    for kind in ['histogram', 'float_histogram']:
        records = ordinary(2)
        records[0]['points'] = series({}, [(time, 2, kind) for time in times])['points']
        add('native-' + kind, records)
    records = ordinary(2)
    records[0]['points'] = series({}, [(0, 2, 'float_histogram'), (1000, 2, 'float'),
        (2000, 2, 'float_histogram'), (3000, 3, 'float'), (4000, 4, 'float')])['points']
    add('mixed-history', records)

    records = ordinary(2)
    counters = deepcopy(records[:-1])
    import base64
    for record in counters:
        record['labels'][0]['value'] = base64.b64encode(b'm_total').decode()
    add('rates', [*counters, records[-1]], (
        'rate(m_total[3s]) / on(host) group_left(site) n',
        'n / on(host) group_right(site) rate(m_total[3s])'))
    return output


def ordered_parent_cases():
    """Complete joins beneath ordinary All-consuming parent expressions."""
    from owned_label_cases import _case

    output = []
    for original in cases():
        supplied = original["request"]
        delayed_options = (supplied["delayed_name_removal"],) if "delayed_name_removal" in supplied else (False, True)
        for reducer in ("sum", "count"):
            for delayed in delayed_options:
                case = deepcopy(original)
                case["request"].update(
                    id=f'ordered-parent:{reducer}:{supplied["id"]}:d{int(delayed)}',
                    query=f'{reducer} by(host)({supplied["query"]})',
                    delayed_name_removal=delayed,
                )
                output.append(case)

    expressions = (
        "sum by(job)(m+n)",
        "abs(sum by(job)(m+n))",
        "avg by(job)(m/n)",
        "count by(job)(abs(m-n))",
        "max by(job)(m > bool on(id) n)",
        "sum by(job)(m < on(id) n)",
        "quantile(.5,m/n)",
        "topk(2,m+n)",
        'count_values("result",m+n)',
        "scalar(sum(m+n))",
        "sum_over_time((m+n)[2s:1s])",
        "sum by(job)(rate(m[2s])+rate(n[2s]))",
        "sum by(job)(m offset 1s+n)",
        "sum by(job)(m @ 2+n)",
        "clamp_min(m/n,scalar(vector(time())))",
    )
    for state in ("dense", "sparse", "long", "native", "mixed", "empty"):
        records = []
        if state != "empty":
            for metric in ("m", "n"):
                for identity in (3, 1, 0, 2):
                    labels = {"__name__": metric, "id": str(identity), "job": "worker"}
                    if state == "long":
                        labels["pad"] = "x" * 512
                    scale = 4 if metric == "n" else (2 if identity % 2 == 0 else 8)
                    points = []
                    for tick in range(5):
                        value = float((tick + 1) * scale)
                        kind = "float_histogram" if state == "native" or state == "mixed" and identity % 2 else "float"
                        if state == "sparse" and (identity + (metric == "n")) % 3 != tick % 3:
                            value, kind = "7ff0000000000002", "float"
                        points.append((tick * 1000, value, kind))
                    records.append(series(labels, points))
        for profile in ("server-default", "upstream-tests"):
            for delayed in (False, True):
                for mode in ("instant", "range"):
                    for index, expression in enumerate(expressions):
                        case = _case(f"ordered-parent:direct:{state}:{profile}:d{int(delayed)}:{mode}:{index}",
                                     expression, records)
                        case["request"].update(profile=profile, delayed_name_removal=delayed, mode=mode,
                            time_ms=4000, start_ms=2000, end_ms=4000, step_ms=1000, lookback_ms=1000)
                        output.append(case)
    assert len({case["request"]["id"] for case in output}) == len(output)
    return output
