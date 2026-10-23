# SPDX-License-Identifier: GPL-3.0-or-later
"""Fractional physical endpoint controls and retained complete cache workloads."""
from fold_range_key_cases import range_key_cases, range_key_performance_cases
from native_endpoint_cases import _records
from single_reset_cases import _request


def endpoint_cases():
    cases = []
    for pattern in ('float', 'native'):
        data = _records(14, 8, pattern)
        for series in data['series']:
            for point in series['points']:
                point['time_ms'] //= 10
        for function in ('rate', 'increase', 'delta'):
            for modifier in ('', 'anchored', 'smoothed'):
                for offset in ('0.0005', '-0.0005'):
                    for mode in ('instant', 'range'):
                        profiles = ('upstream-tests',) if modifier else ('server-default', 'upstream-tests')
                        for profile in profiles:
                            ident = f'endpoint-key-{pattern}-{function}-{modifier or "ordinary"}-{offset}-{mode}-{profile}'
                            expression = f'{function}(m_total[0.3335] {modifier} offset {offset})'
                            request = _request(ident, expression, 1, mode, profile)
                            request.update(time_ms=1000, start_ms=1000, end_ms=1200, step_ms=100)
                            cases.append({'shape': 'control', 'dataset': data, 'request': request})
    return cases


def endpoint_key_cases():
    return range_key_cases() + endpoint_cases()


def endpoint_key_performance_cases():
    return range_key_performance_cases()
