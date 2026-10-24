"""Finite empty merge metadata experiment, including zero-row controls."""
from info_cases import series


def empty_merge_guard_cases():
    cases = []
    for count in (0, 1, 8, 32, 128):
        data = {'schema_version': 1, 'series': [series(
            {'__name__': 'm', 'id': f'{identity:06}', 'job': 'worker', 'source': 'x' * 16},
            [(tick * 1000, identity + tick + 1, 'float') for tick in (1, 2)])
            for identity in range(count)]}
        for shape, expression in [('plain', 'm'), ('unary', '-m'),
                                  ('projection', 'label_join(m,"dst","/","id")'),
                                  ('duplicates', 'label_join(m,"id","","missing")')]:
            for mode in ('instant', 'range'):
                for delayed in (False, True):
                    cases.append({'request': {'id': f'n{count}-{shape}-{mode}-d{int(delayed)}',
                        'query': expression, 'profile': 'server-default', 'mode': mode,
                        'time_ms': 2000, 'start_ms': 1000, 'end_ms': 2000, 'step_ms': 1000,
                        'lookback_ms': 1000, 'delayed_name_removal': delayed},
                        'dataset': data, 'faults': {}, 'count': count, 'shape': shape})
    assert len(cases) == 80
    return cases
