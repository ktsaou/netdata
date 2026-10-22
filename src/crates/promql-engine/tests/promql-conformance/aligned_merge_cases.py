# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite aligned histogram arithmetic targets and general-layout controls."""
from copy import deepcopy
from dense_compaction_cases import dense_compaction_performance_cases


def aligned_merge_performance_cases():
    cases = dense_compaction_performance_cases()
    for case in cases:
        if case['request']['id'].startswith('aggregate-'):
            case['shape'] = case['request']['id'].split('-')[1]
    return cases


def aligned_merge_cases():
    cases = aligned_merge_performance_cases()
    controls = []
    for case in cases:
        case = deepcopy(case)
        request = case['request']
        request['id'] = 'opposite-' + request['id']
        request['profile'] = ('server-default' if request['profile'] == 'upstream-tests'
                              else 'upstream-tests')
        controls.append(case)
    return cases + controls
