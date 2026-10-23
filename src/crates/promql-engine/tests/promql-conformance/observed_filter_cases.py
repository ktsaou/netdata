# SPDX-License-Identifier: GPL-3.0-or-later
"""Existing literal-filter panel with a native-middle fallback discriminator."""
from literal_filter_cases import literal_filter_cases, literal_filter_performance_cases
from single_reset_cases import _records
from literal_filter_cases import case


def new_observed_filter_cases():
    result = []
    for function in ('rate', 'increase'):
        for left in (False, True):
            data = _records(4, 32, [3])
            native = _records(4, 32, [3], 'native')
            data['series'][2] = native['series'][2]
            child = f'{function}(m_total[32s])'
            query = f'0 < {child}' if left else f'{child} > 0'
            result.append(case(f'filter-observed-native-middle-{function}-{left}', query, data, 32))
    return result


def observed_filter_cases():
    return literal_filter_cases() + new_observed_filter_cases()


def observed_filter_performance_cases():
    return literal_filter_performance_cases() + new_observed_filter_cases()
