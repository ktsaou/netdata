# SPDX-License-Identifier: GPL-3.0-or-later
"""First native SUM/AVG ownership and independent aggregate/endpoint controls."""
from copy import deepcopy
import base64
from native_endpoint_cases import _records
from native_final_cases import native_final_cases, native_final_performance_cases
from observed_filter_cases import observed_filter_performance_cases
from single_reset_cases import _request
from info_cases import bits


def grouped_case(groups, width, operator, child, mode, pattern='native'):
    template = _records(4, width, pattern if pattern in ('float', 'integer', 'custom', 'stale') else 'native')
    data = {'schema_version': 1, 'series': []}
    for group in range(groups):
        for member in range(2):
            row = deepcopy(template['series'][member])
            labels = {'__name__': 'm_total', 'id': f'{group:03d}-{member}', 'job': f'group{group:03d}'}
            row['labels'] = [{'name': base64.b64encode(k.encode()).decode(), 'value': base64.b64encode(v.encode()).decode()} for k,v in labels.items()]
            for point in row['points']:
                kind = point['kind']
                if kind == 'float':
                    continue
                h = point[kind]
                if pattern != 'custom':
                    h['schema'] = 4
                if pattern == 'schemas':
                    h['schema'] = 4 - member
                if pattern == 'present-empty':
                    h['negative_spans'] = []
                    h['negative_buckets'] = []
                if pattern == 'extra-custom':
                    h['custom_values'] = [bits(1)]
                if pattern == 'overflow':
                    h['sum'] = bits(1.7e308)
                if (pattern == 'float-first' and member == 0) or (pattern == 'native-first' and member == 1):
                    point.pop(kind)
                    point.update(kind='float', float=bits(point['time_ms'] / 1000))
            data['series'].append(row)
    expression = 'm_total' if child == 'selector' else 'rate(m_total[4s])'
    ident = f'group-owner-{pattern}-{groups}-{width}-{operator}-{child}-{mode}'
    request = _request(ident, f'{operator} by(job)({expression})', 4, mode)
    return {'shape': 'target' if pattern == 'native' else 'control', 'dataset': data, 'request': request}


def new_performance_cases():
    targets = [grouped_case(groups, width, op, child, mode)
               for groups in (8,32) for width in (32,1024) for op in ('sum','avg')
               for child in ('selector','rate') for mode in ('instant','range')]
    controls = [grouped_case(8,32,op,'selector',mode,pattern)
                for pattern in ('integer','custom','schemas','present-empty','extra-custom','stale','float-first','native-first','overflow')
                for op in ('sum','avg') for mode in ('instant','range')]
    controls += [grouped_case(8,32,op,'selector',mode,'float')
                 for op in ('sum','avg','min','max','stdvar','stddev','count','group') for mode in ('instant','range')]
    return targets + controls


def inherited_controls(cases):
    result = deepcopy(cases)
    for case in result:
        case['shape'] = 'control'
    return result


def native_group_performance_cases():
    return new_performance_cases() + inherited_controls(native_final_performance_cases()) + inherited_controls(observed_filter_performance_cases())


def native_group_cases():
    cases = new_performance_cases()
    opposite = deepcopy(cases)
    for case in opposite:
        case['request']['id'] += '-upstream-tests'
        case['request']['profile'] = 'upstream-tests'
    return cases + opposite + inherited_controls(native_final_cases()) + inherited_controls(observed_filter_performance_cases())
