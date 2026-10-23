# SPDX-License-Identifier: GPL-3.0-or-later
"""Literal Sum/Avg preparation targets with retained Rate/Increase controls."""
from copy import deepcopy
import re
from literal_rate_cases import literal_rate_cases, literal_rate_performance_cases
from single_reset_cases import _records, _request
from native_endpoint_cases import _records as native_records

NAMES = {'rate': 'sum_over_time', 'increase': 'avg_over_time'}


def transform(case):
    case = deepcopy(case)
    request = case['request']
    old, = {name for name in NAMES if name + '(' in request['query']}
    name = NAMES[old]
    request['query'] = re.sub(r'\b' + old + r'\(', name + '(', request['query'])
    request['id'] = 'fold-' + request['id'].replace(old, name)
    if case['shape'] in NAMES or '-bool-' in request['id']:
        case['shape'] = name
    return case


def literal_fold_performance_cases():
    old = literal_rate_performance_cases()
    cases = [transform(case) for case in old]
    controls = [deepcopy(case) for case in old if case['shape'] in NAMES
                and case['request']['query'].startswith('2 *')]
    assert len(controls) == 4
    for case in controls:
        case['shape'] = 'control'
        case['request']['id'] = 'retained-' + case['request']['id']
    return cases + controls


def literal_fold_cases():
    primary = literal_fold_performance_cases()
    controls = [transform(case) for case in literal_rate_cases()[100:]]
    for name in NAMES.values():
        for profile in ('server-default', 'upstream-tests'):
            for operator in ('==', '!=', '>', '<', '>=', '<='):
                for left in (False, True):
                    child = name + '(m_total[4s])'
                    expression = f'2.5 {operator} bool {child}' if left else f'{child} {operator} bool 2.5'
                    ident = f'bool-fold-{name}-{profile}-{operator}-{left}'
                    controls.append({'shape': 'edge', 'dataset': _records(4, 4),
                                     'request': _request(ident, expression, 4, 'instant', profile)})
            for pattern in ('singleton', 'empty-series', 'incompatible-native'):
                data = _records(4, 4)
                if pattern == 'singleton':
                    for record in data['series']:
                        record['points'] = record['points'][:1]
                elif pattern == 'empty-series':
                    for record in data['series']:
                        record['points'] = []
                else:
                    data = native_records(4, 2)
                    custom = native_records(4, 2, 'custom')
                    for record, other in zip(data['series'], custom['series'], strict=True):
                        record['points'][1] = other['points'][1]
                for suffix in ('+ 2', '* 2', '> bool 0'):
                    ident = f'edge-fold-{pattern}-{name}-{profile}-{suffix}'
                    controls.append({'shape': 'edge', 'dataset': data,
                                     'request': _request(ident, f'{name}(m_total[4s]) {suffix}',
                                                         4, 'instant', profile)})
    cases = primary + controls
    assert len({c['request']['id'] for c in cases}) == len(cases)
    return cases
