# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite instant Boolean Rate/Increase targets and exact fallback controls."""
from copy import deepcopy
from single_reset_cases import _records, _request, single_reset_cases
from info_cases import series


def _case(ident, expression, data, n, shape, mode='instant', profile='server-default'):
    return {'shape': shape, 'dataset': data,
            'request': _request(ident, expression, n, mode, profile)}


def boolean_rate_performance_cases():
    result = []
    for count, n in [(4, 32), (16, 256)]:
        for function in ['rate', 'increase']:
            child = f'{function}(m_total[{n}s])'
            for resets, positions in [('zero', []), ('one', [n // 2]), ('two', [n // 3, 2 * n // 3])]:
                data = _records(count, n, positions)
                for left in [False, True]:
                    expression = f'0 < bool {child}' if left else f'{child} > bool 0'
                    result.append(_case(f'target-{function}-{resets}-{count}-{n}-{left}',
                                       expression, data, n, function))
            data = _records(count, n, [n // 2])
            for label, expression, mode in [
                ('plain', child, 'instant'), ('filter', child + ' > 0', 'instant'),
                ('dynamic', child + ' > bool time()', 'instant'),
                ('nested', f'abs({child} > bool 0)', 'instant'),
                ('range', child + ' > bool 0', 'range'),
                ('equal-range', child + ' > bool 0', 'range'),
                ('offset', f'{function}(m_total[{n}s] offset 1ms) > bool 0', 'instant'),
            ]:
                row = _case(f'control-{function}-{label}-{count}-{n}', expression, data, n, 'control', mode)
                if label == 'equal-range':
                    row['request']['end_ms'] = row['request']['start_ms']
                result.append(row)
    for pattern in ['native', 'native-first', 'native-last', 'alternating', 'stale-first', 'stale-last']:
        for function in ['rate', 'increase']:
            result.append(_case(f'control-{function}-{pattern}', f'{function}(m_total[32s]) > bool 0',
                               _records(4, 32, [3], pattern), 32, 'native-control'))
    return result


def _functional_cases():
    result = []
    selected = [c for c in single_reset_cases() if c['request']['id'].startswith('special-') and c.get('state') and
                c['request']['query'].startswith('rate(') and c['request']['mode'] == 'instant'
                and c['request']['profile'] == 'server-default' and 'bool' not in c['request']['query']]
    datasets = [(c['state'], c['dataset']) for c in selected]
    sparse = _records(4, 4)
    sparse['series'][0]['points'] = sparse['series'][0]['points'][:1]
    sparse['series'][2]['points'] = []
    datasets += [('sparse', sparse), ('empty', {'schema_version': 1, 'series': []}),
                 ('mixed', _records(2, 4, [2], 'native-first'))]
    for label, data in datasets:
        for function in ['rate', 'increase']:
            child = f'{function}(m_total[4s])'
            for operator in ['==', '!=', '>', '<', '>=', '<=']:
                for left in [False, True]:
                    expression = f'0 {operator} bool {child}' if left else f'{child} {operator} bool 0'
                    for profile in ['server-default', 'upstream-tests']:
                        result.append(_case(f'edge-{label}-{function}-{operator}-{left}-{profile}',
                                           expression, deepcopy(data), 4, 'edge', profile=profile))
    for function in ['rate', 'increase']:
        for profile in ['server-default', 'upstream-tests']:
            for modifier in ['@ 4', '@ start()', '@ end()', 'offset 1s', 'offset -1s', 'anchored', 'smoothed']:
                result.append(_case(f'modified-{function}-{profile}-{modifier}',
                                   f'{function}(m_total[4s] {modifier}) > bool 0',
                                   _records(2, 8, [3]), 4, 'modified', profile=profile))
            collision = {'schema_version': 1, 'series': [
                series({'__name__': name, 'job': 'worker'},
                       [(i * 1000, i, 'float') for i in range(1, 5)]) for name in ['a', 'b']]}
            result.append(_case(f'collision-{function}-{profile}',
                               f'{function}({{job="worker"}}[4s]) > bool 0', collision, 4,
                               'collision', profile=profile))
    return result



def boolean_rate_cases():
    return boolean_rate_performance_cases() + _functional_cases()
