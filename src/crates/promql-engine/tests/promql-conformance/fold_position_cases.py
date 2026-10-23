"""Finite cache lookup targets, shared-path controls and exact original probes."""
from copy import deepcopy
from single_reset_cases import _records, _request
from native_endpoint_cases import _records as native_records
from window_ratio_cases import window_ratio_cases


def performance_cases():
    cases = []
    functions = ('rate', 'sum_over_time', 'count_over_time', 'deriv')
    for count, width in ((64, 4), (256, 4), (1, 4), (4, 4), (16, 256)):
        for function in functions:
            for mode in ('instant', 'range'):
                target = count >= 64
                cases.append({'shape': 'target' if target else 'control', 'dataset': _records(count, width),
                    'request': _request(f'position-{count}-{width}-{function}-{mode}',
                        f'{function}(m_total[{width}s])', width, mode)})
    for pattern in ('native', 'mixed'):
        for function in ('rate', 'sum_over_time', 'deriv'):
            for mode in ('instant', 'range'):
                cases.append({'shape': 'control', 'dataset': native_records(4, 8, pattern),
                    'request': _request(f'position-{pattern}-{function}-{mode}',
                        f'{function}(m_total[4s])', 4, mode)})
    for expression in ('m_total', 'abs(m_total)', 'sum by(job)(m_total)', 'm_total / on(id) m_total'):
        cases.append({'shape': 'control', 'dataset': _records(16, 4),
            'request': _request('position-control-' + expression, expression, 4, 'range')})
    assert len(cases) == 56
    return cases


def functional_cases():
    cases = performance_cases()
    for case in performance_cases():
        copy = deepcopy(case)
        copy['request']['id'] += '-upstream'
        copy['request']['profile'] = 'upstream-tests'
        cases.append(copy)
    for case in window_ratio_cases():
        copy = deepcopy(case)
        copy['shape'] = 'control'
        cases.append(copy)
    return cases
