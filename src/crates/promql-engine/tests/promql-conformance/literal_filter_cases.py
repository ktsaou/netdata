# SPDX-License-Identifier: GPL-3.0-or-later
"""Ordinary instant rate comparison reuse and finite fallback controls."""
from copy import deepcopy
from info_cases import series
from literal_rate_cases import literal_rate_performance_cases
from single_reset_cases import _records, _request, single_reset_cases


def case(ident, query, data, n=4, shape='control', mode='instant', profile='server-default'):
    return {'shape': shape, 'dataset': data, 'request': _request(ident, query, n, mode, profile)}


def literal_filter_performance_cases():
    cases = []
    for count, n in ((4, 32), (16, 256)):
        for function in ('rate', 'increase'):
            data = _records(count, n, [n // 2])
            child = f'{function}(m_total[{n}s])'
            for select, threshold in (('all', '0'), ('some', str(count * (n if function == 'increase' else 1) / 2)), ('none', 'Inf')):
                for left in (False, True):
                    query = f'{threshold} < {child}' if left else f'{child} > {threshold}'
                    cases.append(case(f'filter-{function}-{select}-{left}-{count}-{n}', query, data, n, function))
            for label, query in (('paren-call', f'({child}) > 0'), ('paren-matrix', f'{function}((m_total[{n}s])) > 0'),
                                 ('nested', f'abs({child} > 0)'), ('offset', f'{function}(m_total[{n}s] offset 1ms) > 0')):
                cases.append(case(f'filter-control-{function}-{label}-{count}-{n}', query, data, n))
    for source in literal_rate_performance_cases():
        row = deepcopy(source)
        row['shape'] = 'control'
        row['request']['id'] = 'filter-retained-' + row['request']['id']
        cases.append(row)
    for function in ('rate', 'increase'):
        for native in ('all', 'first', 'last'):
            data = _records(4, 32, [3])
            hist = _records(4, 32, [3], 'native')
            indices = range(4) if native == 'all' else (0,) if native == 'first' else (3,)
            for i in indices:
                data['series'][i] = hist['series'][i]
            for left in (False, True):
                child = f'{function}(m_total[32s])'
                cases.append(case(f'filter-native-{function}-{native}-{left}', f'0 < {child}' if left else f'{child} > 0', data, 32))
    return cases


def literal_filter_cases():
    cases = literal_filter_performance_cases()
    states = [('ordinary', _records(4, 4)), ('singleton', _records(4, 1)),
              ('empty', {'schema_version': 1, 'series': []}),
              ('mixed', _records(4, 4, (), 'native-first'))]
    sparse = _records(4, 4)
    sparse['series'][0]['points'] = sparse['series'][0]['points'][:1]
    sparse['series'][2]['points'] = []
    states.append(('sparse', sparse))
    for source in single_reset_cases():
        if source.get('state') and source['request']['id'].endswith('-rate-server-default-instant'):
            states.append((source['state'], source['dataset']))
    for state, data in states:
        for function in ('rate', 'increase'):
            child = f'{function}(m_total[4s])'
            for profile in ('server-default', 'upstream-tests'):
                for operator in ('==', '!=', '<', '>', '<=', '>='):
                    for left in (False, True):
                        query = f'1 {operator} {child}' if left else f'{child} {operator} 1'
                        cases.append(case(f'filter-edge-{state}-{function}-{profile}-{operator}-{left}', query, data, profile=profile))
    for function in ('rate', 'increase'):
        for profile in ('server-default', 'upstream-tests'):
            for literal in ('NaN', 'Inf', '-Inf', '-0', '0', '1.25'):
                for operator in ('==', '!=', '<', '>'):
                    for left in (False, True):
                        child = f'{function}(m_total[4s])'
                        query = f'{literal} {operator} {child}' if left else f'{child} {operator} {literal}'
                        cases.append(case(f'filter-literal-{function}-{profile}-{operator}-{left}-{literal}', query, _records(2, 4), profile=profile))
            for modifier in ('@ 4', '@ start()', '@ end()', 'offset 1s', 'offset -1s', 'anchored', 'smoothed'):
                for left in (False, True):
                    child = f'{function}(m_total[4s] {modifier})'
                    cases.append(case(f'filter-modified-{function}-{profile}-{modifier}-{left}', f'0 < {child}' if left else f'{child} > 0', _records(2, 8, [3]), profile=profile))
            for mode in ('range', 'instant'):
                for child in (f'{function}(m_total[4s])', f'{function}((m_total[4s]))', f'{function}((m_total+1)[4s:1s])'):
                    cases.append(case(f'filter-shape-{function}-{profile}-{mode}-{child}', child+' > 0', _records(2, 8), mode=mode, profile=profile))
            for threshold in ('0', '1.5', 'Inf'):
                data = {'schema_version': 1, 'series': [series({'__name__': name, 'job': 'worker'},
                    [(i * 1000, i * scale, 'float') for i in range(1, 5)]) for name, scale in [('a', 1), ('b', 2)]]}
                cases.append(case(f'filter-collision-{function}-{profile}-{threshold}', f'{function}({{job="worker"}}[4s]) > {threshold}', data, profile=profile))
    cases.extend(literal_filter_supplement_cases())
    assert len({c['request']['id'] for c in cases}) == len(cases)
    return cases


def literal_filter_supplement_cases():
    cases=[]
    for function,value in [('rate',1),('increase',4)]:
        for duration in ('2s+2s','1s*4'):
            for left in (False,True):
                child=f'{function}(m_total[{duration}])'
                query=f'0 < {child}' if left else f'{child} > 0'
                cases.append({'shape':'dynamic-control','dataset':_records(4,4),
                    'request':_request(f'filter-duration-{function}-{duration}-{left}',query,4,'instant','upstream-tests')})
        data={'schema_version':1,'series':[series({'__name__':'m_total','zone':b'value\x00\xff'},[(i*1000,i,'float')for i in (1,2,3,4)])]}
        for profile in ('server-default','upstream-tests'):
            for operator in ('==','!=','>','<','>=','<='):
                for left in (False,True):
                    scalar=value if operator in ('==','>=','<=') else value+1 if operator=='!=' or operator==('>'if left else'<') else 0
                    child=f'{function}(m_total[4s])'
                    query=f'{scalar} {operator} {child}'if left else f'{child} {operator} {scalar}'
                    cases.append({'shape':'C-expectation','dataset':data,'request':_request(f'filter-C-{function}-{operator}-{left}-{profile}',query,4,'instant',profile)})
    return cases
