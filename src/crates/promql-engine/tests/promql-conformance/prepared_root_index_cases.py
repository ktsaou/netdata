"""Outer aggregate lookup reuse with cardinality, duration and wrapped controls."""
from unique_group_index_cases import unique_group_index_cases
from info_cases import series


def prepared_root_index_cases():
    cases = unique_group_index_cases()
    for c in cases:
        c['clocks'] = 1 if c['request']['mode'] == 'instant' else 3
        c['wrapped'] = False
    for count, extent in [(32, 8), (32, 32), (128, 8), (128, 32), (1024, 3)]:
        end = extent + 1
        for operator in ('sum', 'max'):
            for arrangement in ('single', 'plural'):
                data = {'schema_version': 1, 'series': [series(
                    {'__name__': 'm_total', 'id': f'{i:06}',
                     'job': f'g{i // 2 if arrangement == "plural" else 0:06}'},
                    [(tick * 1000, i + tick + 1, 'float') for tick in range(1, end + 1)])
                    for i in range(count)]}
                for mode in (('instant', 'range') if count == 1024 else ('range',)):
                    for delayed in (False, True):
                        cases.append({'request': {'id': f'n{count}-t{extent}-{operator}-raw-{arrangement}-{mode}-d{int(delayed)}',
                            'query': f'{operator} by(job)(m_total)', 'profile': 'server-default', 'mode': mode,
                            'time_ms': end * 1000, 'start_ms': 2000, 'end_ms': end * 1000, 'step_ms': 1000,
                            'lookback_ms': 1000, 'delayed_name_removal': delayed},
                            'dataset': data, 'faults': {}, 'count': count, 'child': 'raw', 'operator': operator,
                            'arrangement': arrangement, 'timed': True, 'target': arrangement == 'plural',
                            'clocks': 1 if mode == 'instant' else extent, 'wrapped': False})
    for c in list(cases):
        if c['count'] == 128 and c['clocks'] in (1, 3) and c['operator'] and c['arrangement'] == 'plural' and c['child'] in ('raw', 'rate'):
            cases.append(c | {'request': c['request'] | {'id': c['request']['id'] + '-wrapped', 'query': '(' + c['request']['query'] + ')'},
                              'target': False, 'wrapped': True})
    assert len(cases) == 232 and sum(c['timed'] for c in cases) == 184
    assert sum(c['target'] for c in cases) == 72
    return cases
