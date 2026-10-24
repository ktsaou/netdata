"""Completed Float rates beside accepted direct-selector and full-label controls."""
from copy import deepcopy
from info_cases import series
from selector_group_label_cases import selector_group_label_cases


def rate_group_label_cases():
    result = selector_group_label_cases()
    for case in result:
        case['target'] = case['timed'] and case['operator'] is not None and case['child'] == 'rate'
    for case in list(result):
        if case['child'] == 'rate':
            added = deepcopy(case)
            added['request']['id'] = added['request']['id'].replace('-rate-', '-increase-')
            added['request']['query'] = added['request']['query'].replace('rate(', 'increase(')
            added['child'] = 'increase'
            result.append(added)
        if case['request']['id'].startswith('labels-') and '-w32-' in case['request']['id'] and ('-global-' in case['request']['id'] or '-byjob-' in case['request']['id']):
            for function in ('rate', 'increase'):
                added = deepcopy(case)
                added['request']['id'] += '-' + function
                added['request']['query'] = added['request']['query'].replace('(m_total)', '(' + function + '(m_total[2s]))')
                added['child'] = function
                added['target'] = True
                result.append(added)
    assert len({c['request']['id'] for c in result}) == len(result)
    return result


def metadata_cases():
    cases = []
    datasets = {
        'metadata': [
            {'__name__': 'm_total', '__type__': 'counter', '__unit__': 'bytes', 'job': 'x', 'id': '0'},
            {'__name__': 'm_total', '__type__': 'gauge', '__unit__': 'packets', 'job': 'x', 'id': '1'},
        ],
        'metadata-alias': [
            {'__name__': 'm_total', '__type__': 'counter', '__unit__': 'bytes', 'job': 'x'},
            {'__name__': 'm_total', '__type__': 'gauge', '__unit__': 'packets', 'job': 'x'},
        ],
        'raw-empty': [
            {'__name__': 'm_total', 'id': '0'},
            {'__name__': 'm_total', 'id': '1', 'job': ''},
            {'__name__': 'm_total', 'id': '2', 'job': 'x'},
        ],
        'name-warnings': [
            {'__name__': 'first', 'id': '0', 'job': 'x'},
            {'__name__': 'second', 'id': '1', 'job': 'x'},
        ],
        'name-alias': [
            {'__name__': 'first', 'job': 'x'},
            {'__name__': 'second', 'job': 'x'},
        ],
    }
    for shape, labels in datasets.items():
        data = {'schema_version': 1, 'series': [
            series(label, [(t * 1000, (i + 1) * (t + 1), 'float') for t in (1, 2, 3, 4)])
            for i, label in enumerate(labels)]}
        selector = '{__name__=~"first|second"}' if shape.startswith('name-') else 'm_total'
        for function in ('rate', 'increase'):
            for grouping in ('', ' by(job)', ' by(__name__)', ' by(__type__)', ' by(__unit__)', ' by(job,id)', ' without(id)'):
                for delayed in (False, True):
                    for mode in ('instant', 'range'):
                        query = 'sum' + grouping + '(' + function + '(' + selector + '[2s]))'
                        cases.append({'request': {
                            'id': f'rate-metadata:{shape}:{function}:{grouping}:{int(delayed)}:{mode}',
                            'query': query, 'profile': 'server-default', 'mode': mode,
                            'time_ms': 4000, 'start_ms': 2000, 'end_ms': 4000, 'step_ms': 1000,
                            'lookback_ms': 1000, 'delayed_name_removal': delayed},
                            'dataset': data, 'faults': {}})
    assert len(cases) == 280
    return cases
