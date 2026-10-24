"""Ordinary Sum/Avg windows beside accepted selector/rate and full-label controls."""
from copy import deepcopy
from rate_group_label_cases import rate_group_label_cases

def fold_group_label_cases():
    result = rate_group_label_cases()
    for case in result:
        case['target'] = False
    for case in list(result):
        if case['child'] != 'rate':
            continue
        for function in ('sum_over_time', 'avg_over_time'):
            added = deepcopy(case)
            added['request']['id'] += '-' + function
            added['request']['query'] = added['request']['query'].replace('rate(', function + '(')
            added['child'] = function
            added['target'] = added['timed'] and added['operator'] is not None
            result.append(added)
    assert len({c['request']['id'] for c in result}) == len(result)
    return result
