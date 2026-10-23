# SPDX-License-Identifier: GPL-3.0-or-later
"""Literal Deriv preparation, exact regression values and empty mixed warnings."""
from copy import deepcopy
from pathlib import Path
import json
import re
from info_cases import series
from literal_rate_cases import literal_rate_cases, literal_rate_performance_cases
from literal_count_extrema_cases import transform
from literal_predecessor_cases import literal_predecessor_performance_cases
from single_reset_cases import _records, _request
from native_endpoint_cases import _records as native_records


def _supplement():
    packed = json.loads(Path(__file__).with_name('literal_deriv_inputs.json').read_text())
    def expand(case):
        case = deepcopy(case)
        case['dataset'] = packed['datasets'][case.pop('dataset_index')]
        return case
    return {key: [expand(c) for c in packed[key]] for key in ('controls', 'timing')}


def literal_deriv_performance_cases():
    base = [c for c in literal_rate_performance_cases() if re.search(r'\brate\(', c['request']['query'])]
    cases = [transform(c, 'deriv') for c in base]
    for source in literal_predecessor_performance_cases():
        if source['request']['id'].startswith('predecessor-retained-') or (
            source['shape'] in ('changes', 'resets') and source['request']['query'].startswith('2 *')
        ):
            case = deepcopy(source)
            case['shape'] = 'control'
            case['request']['id'] = 'deriv-retained-' + source['request']['id']
            cases.append(case)
    for source in _supplement()['timing']:
        case = deepcopy(source)
        case['shape'] = 'control'
        case['request']['id'] = 'deriv-history-timing-' + source['request']['id']
        cases.append(case)
        if source['shape'] == 'deriv' and source['request']['mode'] == 'instant':
            case = deepcopy(source)
            case['shape'] = 'deriv'
            case['request']['id'] = 'deriv-history-literal-' + source['request']['id']
            case['request']['query'] = '3600 * ' + source['request']['query']
            cases.append(case)
    native_states = ('histogram', 'float_histogram', 'mixed', 'mixed-reverse', 'stale-histogram')
    for source in _supplement()['controls']:
        if source['request']['id'] in {state + '-deriv-0-instant' for state in native_states}:
            case = deepcopy(source)
            case['shape'] = 'control'
            case['request']['id'] = 'deriv-native-bare-' + source['request']['id']
            cases.append(case)
            case = deepcopy(source)
            case['shape'] = 'deriv'
            case['request']['id'] = 'deriv-native-literal-' + source['request']['id']
            case['request']['query'] += ' + 2'
            cases.append(case)
    return cases


def literal_deriv_C_cases():
    dataset = {'schema_version': 1, 'series': [series(
        {'__name__': 'm_total', 'zone': b'value\x00\xff'},
        [(index * 1000, index, 'float') for index in (1, 2, 3, 4)])]}
    cases = []
    for profile in ('server-default', 'upstream-tests'):
        child = 'deriv(m_total[4s])'
        expressions = [('bare', child)]
        for operator in ('+', '-', '*', '/', '^', '%', 'atan2'):
            for left in (False, True):
                expressions.append((f'{operator}-{left}', f'2 {operator} {child}' if left else f'{child} {operator} 2'))
        for suffix, query in expressions:
            cases.append({'shape': 'C-expectation', 'dataset': dataset,
                'request': _request(f'deriv-C-{profile}-{suffix}', query, 4, 'instant', profile)})
    return cases


def literal_deriv_cases():
    primary = literal_deriv_performance_cases()
    old_timing_count = len(literal_rate_performance_cases())
    base = [c for c in literal_rate_cases()[old_timing_count:] if re.search(r'\brate\(', c['request']['query'])]
    controls = [transform(c, 'deriv') for c in base]
    for profile in ('server-default', 'upstream-tests'):
        for operator in ('==', '!=', '>', '<', '>=', '<='):
            for left in (False, True):
                child = 'deriv(m_total[4s])'
                query = f'1 {operator} bool {child}' if left else f'{child} {operator} bool 1'
                controls.append({'shape': 'edge', 'dataset': _records(4, 4),
                    'request': _request(f'deriv-bool-{profile}-{operator}-{left}', query, 4, 'instant', profile)})
        for pattern in ('singleton', 'empty-series', 'incompatible-native', 'one-float-mixed'):
            data = _records(4, 4)
            if pattern == 'singleton':
                for row in data['series']:
                    row['points'] = row['points'][:1]
            elif pattern == 'empty-series':
                for row in data['series']:
                    row['points'] = []
            elif pattern == 'incompatible-native':
                data = native_records(4, 2)
                custom = native_records(4, 2, 'custom')
                for row, other in zip(data['series'], custom['series'], strict=True):
                    row['points'][1] = other['points'][1]
            else:
                native = native_records(4, 2)
                for row, other in zip(data['series'], native['series'], strict=True):
                    row['points'] = [row['points'][0], other['points'][1]]
            for suffix in ('+ 2', '* 2', '> bool 0'):
                controls.append({'shape': 'edge', 'dataset': data,
                    'request': _request(f'deriv-edge-{pattern}-{profile}-{suffix}',
                                        f'deriv(m_total[4s]) {suffix}', 4, 'instant', profile)})
        for query in ('deriv((m_total[4s])) + 2', '2 * deriv((m_total[4s]))',
                      'deriv((m_total[4s])) > bool 0', 'deriv((m_total[4s] anchored)) + 2',
                      'deriv((m_total[4s] smoothed)) + 2'):
            controls.append({'shape': 'edge', 'dataset': _records(4, 4),
                'request': _request(f'deriv-parenthesized-{profile}-{query}', query, 4, 'instant', profile)})
    for source in _supplement()['controls']:
        case = deepcopy(source)
        case['shape'] = 'historical-control'
        case['request']['id'] = 'deriv-history-control-' + source['request']['id']
        controls.append(case)
        if source['request']['query'].startswith('deriv('):
            for suffix in ('+ 2', '> bool 0'):
                case = deepcopy(source)
                case['shape'] = 'historical-composition'
                case['request']['id'] = 'deriv-history-composed-' + source['request']['id'] + '-' + suffix
                # Keep the whole source expression as the left operand, including any comparison.
                case['request']['query'] = '(' + source['request']['query'] + ') ' + suffix
                controls.append(case)
    cases = primary + controls + literal_deriv_C_cases()
    assert len({c['request']['id'] for c in cases}) == len(cases)
    return cases
