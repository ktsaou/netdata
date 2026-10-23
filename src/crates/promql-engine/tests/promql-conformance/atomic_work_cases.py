# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite work-heavy targets with ordinary, empty, clock and provider-error controls."""
from copy import deepcopy
from fold_position_cases import performance_cases
from fold_geometry_cases import geometry_cases
from native_group_owner_cases import grouped_case


def atomic_work_performance_cases():
    cases = performance_cases()
    for case in cases:
        if case['request']['id'].startswith('position-16-256-'):
            case['shape'] = 'target'
    cases += [grouped_case(8, 1024, op, child, mode)
              for op in ('sum', 'avg') for child in ('selector', 'rate')
              for mode in ('instant', 'range')]
    source = {c['request']['id']: c for c in geometry_cases()}
    for count in (1025, 2049):
        case = deepcopy(source[f'geometry-backed-{count}-quantile_over_time'])
        case['shape'] = 'target'
        cases.append(case)
    cases += [deepcopy(source[f'geometry-empty-{fn}-server-default-instant'])
              for fn in ('rate', 'sum_over_time', 'count_over_time', 'deriv')]
    assert len(cases) == 70 and sum(c['shape'] == 'target' for c in cases) == 34
    return cases


def atomic_work_cases():
    cases = atomic_work_performance_cases()
    source = {c['request']['id']: c for c in geometry_cases()}
    cases += [deepcopy(source[f'ratio-fault-{metric}-{count}'])
              for metric in ('m_total', 'n_total') for count in (4, 32)]
    cases += [deepcopy(source[f'geometry-modified-offset -1s-{fn}-{mode}'])
              for fn in ('rate', 'deriv') for mode in ('instant', 'range')]
    assert len(cases) == len({c['request']['id'] for c in cases}) == 78
    return cases
