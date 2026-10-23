# SPDX-License-Identifier: GPL-3.0-or-later
"""Snapshot-memory targets and complete range-function geometry controls."""
from copy import deepcopy
from fold_position_cases import performance_cases
from info_cases import series
from single_reset_cases import _records, _request
from native_endpoint_cases import _records as native_records
from window_ratio_cases import window_ratio_cases

FUNCTIONS = (
    'rate', 'increase', 'delta', 'irate', 'idelta', 'deriv', 'predict_linear',
    'double_exponential_smoothing', 'mad_over_time', 'quantile_over_time',
    'count_over_time', 'present_over_time', 'absent_over_time', 'first_over_time',
    'last_over_time', 'min_over_time', 'max_over_time', 'sum_over_time', 'avg_over_time',
    'stdvar_over_time', 'stddev_over_time', 'ts_of_first_over_time', 'ts_of_last_over_time',
    'ts_of_min_over_time', 'ts_of_max_over_time', 'changes', 'resets',
)
OPTIONAL = {'double_exponential_smoothing', 'mad_over_time', 'ts_of_first_over_time',
            'ts_of_last_over_time', 'ts_of_min_over_time', 'ts_of_max_over_time'}


def expression(function, matrix):
    if function == 'predict_linear':
        return f'{function}({matrix}, 2)'
    if function == 'double_exponential_smoothing':
        return f'{function}({matrix}, 0.4, 0.6)'
    if function == 'quantile_over_time':
        return f'{function}(0.5, {matrix})'
    return f'{function}({matrix})'


def geometry_performance_cases():
    return performance_cases()


def geometry_cases():
    cases = geometry_performance_cases()
    for pattern in ('float', 'native', 'mixed', 'custom', 'stale', 'empty'):
        data = native_records(4, 8, 'float' if pattern == 'empty' else pattern)
        if pattern == 'empty':
            for row in data['series']:
                row['points'] = []
        for function in FUNCTIONS:
            profiles = ('upstream-tests',) if function in OPTIONAL else ('server-default', 'upstream-tests')
            for profile in profiles:
                for mode in ('instant', 'range'):
                    cases.append({'shape': 'control', 'dataset': data,
                        'request': _request(f'geometry-{pattern}-{function}-{profile}-{mode}',
                            expression(function, 'm_total[4s]'), 4, mode, profile)})
    for modifier in ('@ 4', '@ start()', '@ end()', 'offset 1s', 'offset -1s'):
        for function in ('rate', 'sum_over_time', 'deriv', 'quantile_over_time'):
            for mode in ('instant', 'range'):
                cases.append({'shape': 'control', 'dataset': _records(4, 12, [3]),
                    'request': _request(f'geometry-modified-{modifier}-{function}-{mode}',
                        expression(function, f'm_total[4s] {modifier}'), 8, mode)})
    for modifier, functions in [('anchored', ('rate', 'increase', 'delta', 'resets', 'changes')),
                                ('smoothed', ('rate', 'increase', 'delta'))]:
        for function in functions:
            for pattern in ('float', 'native'):
                for mode in ('instant', 'range'):
                    cases.append({'shape': 'control', 'dataset': native_records(12, 8, pattern),
                        'request': _request(f'geometry-{modifier}-{function}-{pattern}-{mode}',
                            expression(function, f'm_total[4s] {modifier}'), 8, mode, 'upstream-tests')})
    for function in ('rate', 'sum_over_time', 'deriv', 'quantile_over_time', 'mad_over_time'):
        for profile in (('upstream-tests',) if function in OPTIONAL else ('server-default', 'upstream-tests')):
            for mode in ('instant', 'range'):
                cases.append({'shape': 'control', 'dataset': _records(4, 8),
                    'request': _request(f'geometry-derived-{function}-{profile}-{mode}',
                        expression(function, '(m_total+1)[4s:1s]'), 4, mode, profile)})
    for function in ('rate', 'sum_over_time', 'count_over_time', 'deriv', 'quantile_over_time'):
        for profile in ('server-default', 'upstream-tests'):
            request = _request(f'geometry-disjoint-{function}-{profile}',
                expression(function, 'm_total[2s]'), 4, 'range', profile)
            request.update(end_ms=16000, step_ms=4000)
            cases.append({'shape': 'control', 'dataset': _records(4, 18), 'request': request})
    for function in ('rate', 'sum_over_time', 'deriv'):
        cases.append({'shape': 'control', 'dataset': _records(4, 8),
            'request': _request(f'geometry-duration-{function}',
                expression(function, 'm_total[2s+2s]'), 4, 'range', 'upstream-tests')})
    # Direct quantiles cross the fixture's temporary-storage threshold without derived replay expansion.
    for count in (1025, 2049):
        data = {'schema_version': 1, 'series': [series({'__name__': 'm_total', 'id': '0'},
            [(tick, ((tick * 37) % 257) - 128, 'float') for tick in range(1, count + 1)])]}
        for function in ('quantile_over_time', 'mad_over_time'):
            request = _request(f'geometry-backed-{count}-{function}',
                expression(function, f'm_total[{count}ms]'), 1, 'instant', 'upstream-tests')
            request['time_ms'] = count
            cases.append({'shape': 'control', 'dataset': data, 'request': request})
    for case in window_ratio_cases():
        if case.get('faults'):
            cases.append(deepcopy(case))
    assert len({c['request']['id'] for c in cases}) == len(cases)
    return cases
