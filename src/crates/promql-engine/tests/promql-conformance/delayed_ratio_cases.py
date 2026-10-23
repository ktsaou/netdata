# SPDX-License-Identifier: GPL-3.0-or-later
"""Direct grouped ratios with explicit delayed metadata removal and fallbacks."""
from copy import deepcopy
from base64 import b64decode, b64encode
from delayed_group_cases import quadratic
from group_contributor_cases import OPERATORS, _records
from info_cases import series


def case(ident, expression, records, delayed=True, mode='instant', time=5000,
         groups=3, shape='functional', profile='server-default'):
    return {'request': {'id': ident, 'query': expression, 'profile': profile,
        'delayed_name_removal': delayed, 'mode': mode, 'time_ms': time,
        'start_ms': time-1000, 'end_ms': time, 'step_ms': 1000,
        'lookback_ms': 300000, 'subquery_step_ms': 60000},
        'dataset': {'schema_version': 1, 'series': deepcopy(records)},
        'faults': {}, 'groups': groups, 'shape': shape}


def ratio(operator='sum', function='rate', grouping='by(cluster,zone)', modifier=''):
    return (f'{operator} {grouping}({function}(m_total[3s])) / {modifier} '
            f'{operator} {grouping}({function}(n_total[3s]))')


def delayed_ratio_performance_cases():
    output=[]
    for groups in (4, 8):
        records=quadratic(groups)
        for record in records:
            record['points']=record['points'][:8]
        for operator in ('sum', 'avg'):
            for function in ('rate', 'increase', 'delta'):
                for mode in ('instant', 'range'):
                    query=f'max_over_time(({ratio(operator,function,"by(id)")})[4s:1s])'
                    for delayed in (False, True):
                        output.append(case(f'delayed-ratio-perf:{groups}:{operator}:{function}:{mode}:{delayed}',
                            query, records, delayed, mode, 8000, groups,
                            'target-'+function if delayed else 'control'))
    records=quadratic(4)
    for record in records:
        record['points']=record['points'][:8]
    direct=ratio(grouping='by(id)')
    fallbacks=(f'abs({direct})', f'({direct})*2',
        'm_total/n_total',
        'sum by(id)(m_total)/sum by(id)(n_total)',
        ratio(grouping='by(__name__,id)', modifier='on(id)'),
        'sum by(id)(sum by(id)(rate(m_total[3s])))/sum by(id)(rate(n_total[3s]))',
        ratio(modifier='on() group_left', grouping='by(id)'),
        'sum by(id)(rate(m_total[3s] @ 6))/sum by(id)(rate(n_total[3s]))')
    for index, child in enumerate(fallbacks):
        output.append(case(f'delayed-ratio-perf:fallback:{index}',
            f'max_over_time(({child})[4s:1s])', records, time=8000, groups=4, shape='fallback'))
    return output


def delayed_ratio_cases():
    output=[]
    dense=_records(groups=3,members=2)
    for operator in OPERATORS:
        for function in ('rate','increase','delta'):
            for delayed in (False,True):
                for mode in ('instant','range'):
                    output.append(case(f'delayed-ratio-core:{operator}:{function}:{delayed}:{mode}',
                        f'max_over_time(({ratio(operator,function)})[3s:1s])',dense,delayed,mode))
    for state in ('stale','late','empty','native','mixed','all-native','nan','infinity','zero'):
        for operator in ('sum','count'):
            for mode in ('instant','range'):
                output.append(case(f'delayed-ratio-state:{state}:{operator}:{mode}',
                    f'sum_over_time(({ratio(operator)})[3s:1s])',
                    _records(state,groups=3,members=2),mode=mode))
    consumers=('min_over_time','max_over_time','sum_over_time','avg_over_time',
        'count_over_time','present_over_time','last_over_time','stdvar_over_time',
        'stddev_over_time','changes','resets','quantile_over_time')
    for consumer in consumers:
        prefix='.5,' if consumer=='quantile_over_time' else ''
        for delayed in (False,True):
            output.append(case(f'delayed-ratio-consumer:{consumer}:{delayed}',
                f'{consumer}({prefix}({ratio()})[3s:1s])',dense,delayed))
    for grouping in ('by(cluster,zone)','without(member,__type__,__unit__)',
                     'by(cluster,zone,__name__)','by(cluster,zone,__type__)',
                     'by(cluster,zone,__unit__)','without(member)'):
        records=deepcopy(dense)
        for record in records:
            labels={b64decode(x['name']).decode():b64decode(x['value']).decode() for x in record['labels']}
            for key,value in [('__type__','gauge' if labels['member']=='0' else 'counter'),
                              ('__unit__','bytes' if labels['member']=='0' else 'seconds')]:
                record['labels'].append({'name':b64encode(key.encode()).decode(),'value':b64encode(value.encode()).decode()})
        for delayed in (False,True):
            for mode in ('instant','range'):
                output.append(case(f'delayed-ratio-metadata:{grouping}:{delayed}:{mode}',
                    f'max_over_time(({ratio(grouping=grouping,modifier="ignoring(__name__)")})[3s:1s])',records,delayed,mode))
    for state in ('left-missing','right-missing','both-missing','aliases','disjoint'):
        records=deepcopy(dense)
        for record in records:
            labels={b64decode(x['name']).decode():b64decode(x['value']).decode() for x in record['labels']}
            right=labels['__name__']=='n_total'
            if labels['zone']=='1' and (state=='both-missing' or state=='right-missing' and right or state=='left-missing' and not right):
                for point in record['points']:
                    if point['time_ms']>=2000: point['float']='7ff0000000000002'
            if state=='aliases':
                record['labels'][0]['value']=b64encode((labels['__name__']+labels['member']).encode()).decode()
            if state=='disjoint' and right:
                next(x for x in record['labels'] if b64decode(x['name'])==b'zone')['value']=b64encode(('r'+labels['zone']).encode()).decode()
        child=ratio()
        if state=='aliases':
            child=child.replace('m_total[','{__name__=~"m_total[01]"}[').replace('n_total[','{__name__=~"n_total[01]"}[')
        for delayed in (False,True):
            for mode in ('instant','range'):
                output.append(case(f'delayed-ratio-membership:{state}:{delayed}:{mode}',
                    f'max_over_time(({child})[3s:1s])',records,delayed,mode))
    for modifier in ('on(cluster,zone)','ignoring(member)','on(cluster)','on()','on(cluster) group_left'):
        for delayed in (False,True):
            output.append(case(f'delayed-ratio-matching:{modifier}:{delayed}',
                f'max_over_time(({ratio(modifier=modifier)})[3s:1s])',dense,delayed))
    for child in (f'(({ratio()}))',f'abs({ratio()})',f'({ratio()})*2',
                  f'-({ratio()})','sum by(cluster,zone)(m_total)/sum by(cluster,zone)(n_total)'):
        for delayed in (False,True):
            output.append(case(f'delayed-ratio-composition:{child}:{delayed}',
                f'max_over_time(({child})[3s:1s])',dense,delayed))
    for delayed in (False,True):
        for mode in ('instant','range'):
            output.append(case(f'delayed-ratio-upstream:{delayed}:{mode}',
                f'max_over_time(({ratio()})[3s:1s])',dense,delayed,mode,profile='upstream-tests'))
    output.extend(delayed_ratio_performance_cases())
    assert len(output)==len({c['request']['id'] for c in output})
    return output
