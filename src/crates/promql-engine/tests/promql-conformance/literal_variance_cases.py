# SPDX-License-Identifier: GPL-3.0-or-later
"""Literal variance preparation with independent numerical and C-fixture discriminators."""
from copy import deepcopy
from pathlib import Path
import json
import re
from info_cases import series
from literal_rate_cases import literal_rate_cases, literal_rate_performance_cases
from literal_count_extrema_cases import transform, literal_count_extrema_performance_cases
from single_reset_cases import _records, _request
from native_endpoint_cases import _records as native_records

FUNCTIONS = ('stdvar_over_time', 'stddev_over_time')


def literal_variance_performance_cases():
    base = [c for c in literal_rate_performance_cases() if re.search(r'\brate\(', c['request']['query'])]
    cases = [transform(case, function) for function in FUNCTIONS for case in base]
    for source in literal_count_extrema_performance_cases():
        if source['request']['query'].startswith('2 *') and (
            source['shape'] in ('count_over_time', 'min_over_time', 'max_over_time')
            or source['request']['id'].startswith('summary-retained-')
        ):
            case = deepcopy(source)
            case['shape'] = 'control'
            case['request']['id'] = 'variance-retained-' + case['request']['id']
            cases.append(case)
    return cases


def literal_variance_C_cases():
    dataset = {'schema_version': 1, 'series': [series(
        {'__name__': 'm_total', 'zone': b'value\x00\xff'},
        [(index * 1000, index, 'float') for index in (1, 2, 3, 4)])]}
    cases = []
    for function in FUNCTIONS:
        for profile in ('server-default', 'upstream-tests'):
            child = function + '(m_total[4s])'
            expressions = [('bare', child)]
            for operator in ('+', '-', '*', '/', '^', '%', 'atan2'):
                for left in (False, True):
                    expressions.append((f'{operator}-{left}', f'2 {operator} {child}' if left else f'{child} {operator} 2'))
            for suffix, query in expressions:
                cases.append({'shape': 'C-expectation', 'dataset': dataset,
                              'request': _request(f'variance-C-{function}-{profile}-{suffix}',
                                                  query, 4, 'instant', profile)})
    return cases


def literal_variance_cases():
    primary = literal_variance_performance_cases()
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
                    controls.append({'shape': 'edge', 'dataset': _records(4, 4),
                                     'request': _request(f'variance-bool-{function}-{profile}-{operator}-{left}',
                                                         query, 4, 'instant', profile)})
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
                    controls.append({'shape': 'edge', 'dataset': data,
                                     'request': _request(f'variance-edge-{pattern}-{function}-{profile}-{suffix}',
                                                         f'{function}(m_total[4s]) {suffix}', 4, 'instant', profile)})
    inputs = json.loads(Path(__file__).with_name('literal_variance_inputs.json').read_text())
    for item in inputs:
        for function in FUNCTIONS:
            for profile in ('server-default', 'upstream-tests'):
                child = f'{function}(m[{item["range_seconds"]}s])'
                for index, query in enumerate((child + ' + 0', '2 / ' + child, child + ' >= bool 0')):
                    controls.append({'shape': 'numerical-edge', 'dataset': item['dataset'],
                                     'request': _request(f'variance-numerical-{item["name"]}-{function}-{profile}-{index}',
                                                         query, item['time_ms'] // 1000, 'instant', profile)})
    cases = primary + controls + literal_variance_C_cases()
    assert len({c['request']['id'] for c in cases}) == len(cases)
    return cases
