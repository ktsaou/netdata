"""Plural aggregate construction with comparable singleton and nongrouped controls."""
from info_cases import series


def unique_group_index_cases():
    cases = []
    for count in (0, 1, 32, 128):
        for child, source in [('raw', 'm_total'), ('mapped', 'abs(m_total)'),
                              ('rate', 'rate(m_total[2s])')]:
            variants = [(operator, arrangement) for operator in ('sum', 'max')
                        for arrangement in (('single', 'plural') if count >= 32 else ('single',))]
            if count >= 32:
                variants.append((None, 'plural'))
            for operator, arrangement in variants:
                data = {'schema_version': 1, 'series': [series(
                    {'__name__': 'm_total', 'id': f'{identity:06}',
                     'job': f'g{identity // 2 if arrangement == "plural" else 0:06}'},
                    [(tick * 1000, identity + tick + 1, 'float') for tick in (1, 2, 3, 4)])
                    for identity in range(count)]}
                expression = f'{operator} by(job)({source})' if operator else source
                for mode in ('instant', 'range'):
                    for delayed in (False, True):
                        cases.append({'request': {
                            'id': f'n{count}-{operator or "none"}-{child}-{arrangement}-{mode}-d{int(delayed)}',
                            'query': expression, 'profile': 'server-default', 'mode': mode,
                            'time_ms': 4000, 'start_ms': 2000, 'end_ms': 4000, 'step_ms': 1000,
                            'lookback_ms': 1000, 'delayed_name_removal': delayed},
                            'dataset': data, 'faults': {}, 'count': count, 'child': child,
                            'operator': operator, 'arrangement': arrangement, 'timed': count >= 32,
                            'target': count >= 32 and operator is not None and arrangement == 'plural'})
    assert len(cases) == 168 and sum(c['timed'] for c in cases) == 120
    assert sum(c['target'] for c in cases) == 48
    return cases
