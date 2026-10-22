# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite literal scalar rate arithmetic targets and generic-path controls."""
from single_reset_cases import _records, _request, single_reset_cases
from info_cases import series


def _case(ident, expression, data, n=4, shape='edge', mode='instant', profile='server-default'):
    return {'shape': shape, 'dataset': data,
            'request': _request(ident, expression, n, mode, profile)}


def literal_rate_performance_cases():
    pilot = []
    for count, n in [(4, 32), (16, 256)]:
        for function in ['rate', 'increase']:
            child = f'{function}(m_total[{n}s])'
            data = _records(count, n, [n // 2])
            for operator in ['+', '-', '*', '/', '^', '%', 'atan2']:
                for left in [False, True]:
                    expr = f'2 {operator} {child}' if left else f'{child} {operator} 2'
                    pilot.append(_case(f'target-{function}-{operator}-{left}-{count}-{n}', expr,
                                      data, n, function))
            for label, expr, mode in [
                ('plain', child, 'instant'), ('bool', child + ' > bool 0', 'instant'),
                ('filter', child + ' > 0', 'instant'), ('dynamic', child + ' * time()', 'instant'),
                ('nested', f'abs(2 * {child})', 'instant'), ('range', child + ' / 2', 'range'),
                ('equal-range', child + ' / 2', 'range'),
                ('offset', f'2 * {function}(m_total[{n}s] offset 1ms)', 'instant')]:
                row = _case(f'control-{function}-{label}-{count}-{n}', expr, data, n, 'control', mode)
                if label == 'equal-range': row['request']['end_ms'] = row['request']['start_ms']
                pilot.append(row)
    for pattern in ['native', 'native-first', 'native-last', 'alternating', 'stale-first', 'stale-last']:
        for function in ['rate', 'increase']:
            pilot.append(_case(f'control-{function}-{pattern}', f'{function}(m_total[32s]) / 2',
                              _records(4, 32, [3], pattern), 32, 'control'))
    return pilot


def literal_rate_cases():
    selected = [c for c in single_reset_cases() if c['request']['id'].startswith('special-')
                and c.get('state') and c['request']['query'].startswith('rate(')
                and c['request']['mode'] == 'instant' and c['request']['profile'] == 'server-default'
                and 'bool' not in c['request']['query']]
    data_sets = [(c['state'], c['dataset']) for c in selected]
    sparse = _records(4, 4); sparse['series'][0]['points'] = sparse['series'][0]['points'][:1]
    sparse['series'][2]['points'] = []
    data_sets += [('sparse', sparse), ('empty', {'schema_version': 1, 'series': []}),
                  ('mixed', _records(2, 4, [2], 'native-first'))]
    controls = []
    for label, data in data_sets:
        for function in ['rate', 'increase']:
            child = f'{function}(m_total[4s])'
            for operator in ['+', '-', '*', '/', '^', '%', 'atan2']:
                for left in [False, True]:
                    expr = f'0 {operator} {child}' if left else f'{child} {operator} 0'
                    for profile in ['server-default', 'upstream-tests']:
                        controls.append(_case(f'edge-{label}-{function}-{operator}-{left}-{profile}',
                                             expr, data, profile=profile))
    for function in ['rate', 'increase']:
        for profile in ['server-default', 'upstream-tests']:
            for modifier in ['@ 4', '@ start()', '@ end()', 'offset 1s', 'offset -1s', 'anchored', 'smoothed']:
                controls.append(_case(f'modified-{function}-{profile}-{modifier}',
                    f'2 * {function}(m_total[4s] {modifier})', _records(2, 8, [3]), profile=profile))
            for scalar in ['NaN', 'Inf', '-Inf', '-0', '0.5', '1024']:
                for operator in ['/', '%', '^', 'atan2']:
                    controls.append(_case(f'literal-{function}-{profile}-{operator}-{scalar}',
                        f'{scalar} {operator} {function}(m_total[4s])', _records(2, 4), profile=profile))
            collision = {'schema_version': 1, 'series': [series({'__name__': name, 'job': 'worker'},
                         [(i * 1000, i, 'float') for i in range(1, 5)]) for name in ['a', 'b']]}
            controls.append(_case(f'collision-{function}-{profile}',
                f'2 * {function}({{job="worker"}}[4s])', collision, profile=profile))
    return literal_rate_performance_cases() + controls
