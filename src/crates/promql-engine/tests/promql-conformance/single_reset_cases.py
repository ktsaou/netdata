# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite Float one-reset targets and original-profile controls."""
from copy import deepcopy
from info_cases import series
from rate_acquisition_cases import no_reset_cases

def _request(ident, expression, n, mode, profile='server-default'):
    return {'id': ident, 'query': expression, 'profile': profile, 'mode': mode, 'time_ms': n * 1000, 'start_ms': n * 1000, 'end_ms': (n + 2) * 1000, 'step_ms': 1000, 'lookback_ms': 300000, 'subquery_step_ms': 1000}

def _records(count, n, positions=(), pattern=None):
    values = []
    value = 100
    for tick in range(n + 2):
        value = 0 if tick in positions else value + 1
        values.append(value)
    result = []
    for identity in range(count):
        points = []
        for tick, value in enumerate(values):
            native = pattern == 'native' or (pattern in ('native-first', 'stale-first') and tick == 0) or (pattern in ('native-last', 'stale-last') and tick == n - 1) or (pattern == 'alternating' and tick % 2 == 1)
            v = '7ff0000000000002' if native and pattern.startswith('stale') else value * (identity + 1)
            points.append(((tick + 1) * 1000, v, 'float_histogram' if native else 'float'))
        result.append(series({'__name__': 'm_total', 'id': str(identity), 'job': 'worker'}, points))
    return {'schema_version': 1, 'series': result}

def _expr(shape, n):
    value = ('increase' if shape == 'increase' else 'delta' if shape == 'delta' else 'rate') + f'(m_total[{n}s])'
    return 'sum by(job)(' + value + ')' if shape == 'grouped-rate' else value

def single_reset_performance_cases():
    pilot = []
    for count, n in [(4, 32), (16, 256)]:
        for position, at in [('early', 3), ('middle', n // 2), ('late', n - 2)]:
            for shape in ['rate', 'increase', 'grouped-rate']:
                for mode in ['instant', 'range']:
                    pilot.append({'shape': shape, 'count': count, 'extent': n, 'position': position, 'dataset': _records(count, n, [at]), 'request': _request(f'one-{shape}-{position}-{count}-{n}-{mode}', _expr(shape, n), n, mode)})
        for pattern, positions in [('zero', []), ('two', [n // 3, 2 * n // 3])]:
            for shape in ['rate', 'increase', 'grouped-rate']:
                for mode in ['instant', 'range']:
                    pilot.append({'shape': pattern + '-control', 'count': count, 'extent': n, 'dataset': _records(count, n, positions), 'request': _request(f'{pattern}-{shape}-{count}-{n}-{mode}', _expr(shape, n), n, mode)})
        for mode in ['instant', 'range']:
            pilot.append({'shape': 'delta-control', 'count': count, 'extent': n, 'dataset': _records(count, n, [n // 2]), 'request': _request(f'delta-{count}-{n}-{mode}', _expr('delta', n), n, mode)})
        for shape in ['rate', 'increase', 'grouped-rate']:
            for mode in ['instant', 'range']:
                data = _records(count, n, [n // 2])
                for record in data['series']:
                    for tick, point in enumerate(record['points']):
                        point['start_time_ms'] = 500 if tick < n // 2 else n // 2 * 1000 + 500
                pilot.append({'shape': 'start-control', 'count': count, 'extent': n, 'dataset': data, 'request': _request(f'start-{shape}-{count}-{n}-{mode}', _expr(shape, n), n, mode, 'upstream-tests')})
    for pattern in ['native', 'native-first', 'native-last', 'alternating', 'stale-first', 'stale-last']:
        for shape in ['rate', 'increase']:
            for mode in ['instant', 'range']:
                pilot.append({'shape': 'native-control', 'pattern': pattern, 'count': 4, 'extent': 32, 'dataset': _records(4, 32, [3], pattern), 'request': _request(f'{pattern}-{shape}-{mode}', _expr(shape, 32), 32, mode)})
    assert len(pilot) == 100
    return pilot

def single_reset_cases():
    controls = []
    for case in no_reset_cases():
        r = case['request']
        if r['delayed_name_removal'] != (r['profile'] == 'upstream-tests'):
            continue
        c = deepcopy(case)
        del c['request']['delayed_name_removal']
        controls.append(c)
    assert len(controls) == 180
    special = {'one-order': [2 ** 54, 0, 1, 1], 'two-order': [2 ** 54, 0, 1, 0], 'nan-one': [100, '7ff8000000000042', 10, 0], 'positive-infinity': ['7ff0000000000000', 0, 1, 2], 'negative-infinity': ['fff0000000000000', '7ff0000000000000', 'fff0000000000000', 'fff0000000000000'], 'stale-one': [100, '7ff0000000000002', 0, 1], 'signed-zero': [1, -0.0, 0.0, -0.0], 'negative-reset': [-1, -2, -1, 0]}
    for state, values in special.items():
        data = {'schema_version': 1, 'series': [series({'__name__': 'm_total', 'id': '0', 'job': 'worker'}, [((i + 1) * 1000, v, 'float') for i, v in enumerate(values)])]}
        for function in ['rate', 'increase', 'delta', 'rate-bool']:
            expression = _expr('rate' if function == 'rate-bool' else function, 4) + (' > bool 0' if function == 'rate-bool' else '')
            for profile in ['server-default', 'upstream-tests']:
                for mode in ['instant', 'range']:
                    controls.append({'state': state, 'dataset': data, 'request': _request(f'special-{state}-{function}-{profile}-{mode}', expression, 4, mode, profile)})
    assert len(controls) == 308
    return single_reset_performance_cases() + controls
