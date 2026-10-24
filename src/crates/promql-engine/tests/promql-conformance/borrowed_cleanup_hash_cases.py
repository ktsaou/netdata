"""Short and wide labels at final delayed cleanup; early paths are controls."""
from info_cases import series


def borrowed_cleanup_hash_cases():
    cases = []
    for count in (0, 1, 32, 128):
        for width in ((16,) if count == 0 else (16, 4096)):
            data = {'schema_version': 1, 'series': [series(
                {'__name__': 'm_total', '__type__': 'counter', '__unit__': 'bytes',
                 'id': f'{identity:06}', 'job': 'worker', 'detail': 'x' * width},
                [(tick * 1000, identity + tick + 1, 'float') for tick in (1, 2, 3)])
                for identity in range(count)]}
            for shape, expression in [('plain', 'm_total'), ('unary', '-m_total'),
                                      ('arithmetic', 'm_total + 0'), ('rate', 'rate(m_total[2s])')]:
                for mode in ('instant', 'range'):
                    for delayed in (False, True):
                        cases.append({'request': {
                            'id': f'n{count}-w{width}-{shape}-{mode}-d{int(delayed)}',
                            'query': expression, 'profile': 'server-default', 'mode': mode,
                            'time_ms': 3000, 'start_ms': 2000, 'end_ms': 3000, 'step_ms': 1000,
                            'lookback_ms': 1000, 'delayed_name_removal': delayed},
                            'dataset': data, 'faults': {}, 'count': count, 'width': width,
                            'shape': shape, 'target': delayed and count >= 32 and width == 4096})
    assert len(cases) == 112 and sum(case['target'] for case in cases) == 16
    return cases
