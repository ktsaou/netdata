# SPDX-License-Identifier: GPL-3.0-or-later
"""Literal Count/Min/Max preparation targets and ordinary preparation controls."""
from copy import deepcopy
import re
from literal_rate_cases import literal_rate_cases, literal_rate_performance_cases
from literal_fold_cases import literal_fold_performance_cases
from single_reset_cases import _records, _request
from native_endpoint_cases import _records as native_records

FUNCTIONS = ('count_over_time', 'min_over_time', 'max_over_time')


def transform(case, function):
    case = deepcopy(case)
    request = case['request']
    assert re.search(r'\brate\(', request['query'])
    request['query'] = re.sub(r'\brate\(', function + '(', request['query'])
    request['id'] = 'summary-' + request['id'].replace('rate', function)
    if case['shape'] == 'rate' or '-bool-' in request['id']:
        case['shape'] = function
    return case


def literal_count_extrema_performance_cases():
    base = [c for c in literal_rate_performance_cases() if re.search(r'\brate\(', c['request']['query'])]
    cases = [transform(case, function) for function in FUNCTIONS for case in base]
    for source in literal_fold_performance_cases():
        if source['request']['query'].startswith('2 *') and (
            source['shape'] in ('sum_over_time', 'avg_over_time')
            or source['request']['id'].startswith('retained-')
        ):
            case = deepcopy(source)
            case['shape'] = 'control'
            case['request']['id'] = 'summary-retained-' + case['request']['id']
            cases.append(case)
    return cases


def literal_count_extrema_cases():
    primary = literal_count_extrema_performance_cases()
    old_timing_count = len(literal_rate_performance_cases())
    base = [c for c in literal_rate_cases()[old_timing_count:]
            if re.search(r'\brate\(', c['request']['query'])]
    controls = [transform(case, function) for function in FUNCTIONS for case in base]
    for function in FUNCTIONS:
        for profile in ('server-default', 'upstream-tests'):
            for operator in ('==', '!=', '>', '<', '>=', '<='):
                for left in (False, True):
                    child = function + '(m_total[4s])'
                    query = f'2.5 {operator} bool {child}' if left else f'{child} {operator} bool 2.5'
                    ident = f'summary-bool-{function}-{profile}-{operator}-{left}'
                    controls.append({'shape': 'edge', 'dataset': _records(4, 4),
                                     'request': _request(ident, query, 4, 'instant', profile)})
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
                    ident = f'summary-edge-{pattern}-{function}-{profile}-{suffix}'
                    controls.append({'shape': 'edge', 'dataset': data,
                                     'request': _request(ident, f'{function}(m_total[4s]) {suffix}',
                                                         4, 'instant', profile)})
    cases = primary + controls
    assert len({c['request']['id'] for c in cases}) == len(cases)
    return cases
