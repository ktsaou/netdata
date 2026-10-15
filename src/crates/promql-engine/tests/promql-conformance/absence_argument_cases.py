"""Absence inference must observe the source's visited argument edges."""

from copy import deepcopy

from info_cases import series


def cases():
    forms = []
    for matcher in (r'job="a"', r'job="",foo="b"', r'job="a",job="a"', r'job="\xff"'):
        selector = f"missing{{{matcher}}}"
        for depth in (0, 1, 2):
            argument = "(" * depth + selector + ")" * depth
            call = f"absent({argument})"
            for owner, value in (("topk", 1), ("quantile", 0)):
                expression = f"{owner}(scalar({call} + on(job) vector({value})), m)"
                forms.append((expression, False))
                if matcher == r'job="a"' and depth < 2:
                    forms.append((expression, True))
    forms += [(expression, False) for expression in (
        'absent(missing{job="a"})',
        r'absent(((missing{job="\xff"})))',
        'absent_over_time(missing{job="a"}[2s])',
        'absent_over_time((missing{job="a"}[2s]))',
        'quantile(scalar(absent_over_time((missing{job="a"}[2s])) + on(job) vector(0)),m)',
        'quantile(scalar(absent_over_time((missing{job="a"}[2s:1s])) + on(job) vector(0)),m)',
        'quantile(scalar(absent((abs(missing{job="a"}))) + on(job) vector(0)),m)',
        'quantile(scalar(absent_over_time(missing{job="a"}[2s]) + on(job) vector(0)),m)',
    )]
    base = [series({'__name__': 'm', 'id': name, 'job': 'worker'},
                   [(1000, value, 'float'), (2000, value, 'float')])
            for name, value in [('a', 1), ('b', 2), ('c', 3)]]
    output = []
    for expression, present in forms:
        records = deepcopy(base)
        if present:
            records.append(series({'__name__': 'missing', 'job': 'a'},
                                  [(1000, 1, 'float'), (2000, 1, 'float')]))
        for profile in ('server-default', 'upstream-tests'):
            for mode in ('instant', 'range'):
                output.append({
                    'request': {
                        'id': f'absence-edge-{len(output):03d}', 'query': expression,
                        'profile': profile, 'mode': mode, 'time_ms': 2000,
                        'start_ms': 1000, 'end_ms': 2000, 'step_ms': 1000,
                        'lookback_ms': 1000, 'subquery_step_ms': 1000,
                    },
                    'dataset': {'schema_version': 1, 'series': records},
                    'faults': {},
                })
    return output
