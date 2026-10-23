# SPDX-License-Identifier: GPL-3.0-or-later
"""Direct delayed groups, deferred names and unchanged composition controls."""
from copy import deepcopy
from info_cases import series
from group_contributor_cases import OPERATORS, _records

def quadratic(groups):
    records=[]
    for position in range(groups*4):
        right=position>=groups*2;row=position%(groups*2)
        identity=row//2 if right else groups-1-row//2
        member=1-row%2 if right else row%2
        scale=(1 if member==0 else 3) if right else ((2 if member==0 else 4) if identity%2==0 else (8 if member==0 else 16))
        records.append(series({'__name__':'n_total' if right else 'm_total','id':f'{identity:06}','job':'worker','member':str(member)},
            [(t*1000,scale*(100+(t if right else t*t)),'float')for t in range(1,17)]))
    return records

def case(ident,expression,records,delayed=True,mode='instant',time=5000,shape='control',groups=3):
    return {'request':{'id':ident,'query':expression,'profile':'upstream-tests' if delayed else 'server-default','mode':mode,
            'time_ms':time,'start_ms':time-2000,'end_ms':time,'step_ms':1000,'lookback_ms':300000,'subquery_step_ms':1000},
        'dataset':{'schema_version':1,'series':deepcopy(records)},'faults':{},'shape':shape,'groups':groups}

def delayed_group_performance_cases():
    output=[]
    for groups in (4,16):
        records=quadratic(groups)
        for operator in ('sum','avg'):
            for leaf,child in (('selector','m_total'),('rate','rate(m_total[3s])')):
                for mode in ('instant','range'):
                    expression=f'max_over_time(({operator} by(id)({child}))[8s:1s])'
                    for delayed in (False,True):
                        output.append(case(f'delayed-perf:{groups}:{operator}:{leaf}:{mode}:{delayed}',expression,records,delayed,mode,16000,'target-'+leaf if delayed else 'control',groups))
    records=quadratic(4)
    group='sum by(id)(rate(m_total[3s]))'
    for index,child in enumerate((f'sum by(id)({group})',f'label_join({group},"site","/","id")',f'{group}/sum by(id)(rate(n_total[3s]))',f'abs({group})',f'{group}*2','sum by(__name__,id)(rate({__name__=~"[mn]_total"}[3s]))','sum by(id)(sum_over_time(m_total[3s]))','sum by(id)(rate(m_total[3s] @ 12))')):
        output.append(case(f'delayed-perf:fallback:{index}',f'max_over_time(({child})[8s:1s])',records,True,'instant',16000,groups=4))
    return output

def initial_delayed_group_cases():
    output=[];dense=_records(groups=3,members=2)
    for operator in OPERATORS:
        for leaf in ('m_total','rate(m_total[3s])','increase(m_total[3s])','delta(m_total[3s])'):
            for grouping in ('by(cluster,zone)','without(member)'):
                for delayed in (False,True):
                    for mode in ('instant','range'):
                        expression=f'max_over_time(({operator} {grouping}({leaf}))[3s:1s])'
                        output.append(case(f'delayed-core:{operator}:{leaf}:{grouping}:{delayed}:{mode}',expression,dense,delayed,mode))
    for state in ('stale','late','empty','native','mixed','all-native','nan','infinity','zero'):
        for leaf in ('m_total','rate(m_total[3s])'):
            for operator in ('sum','count'):
                for mode in ('instant','range'):
                    output.append(case(f'delayed-state:{state}:{leaf}:{operator}:{mode}',f'sum_over_time(({operator} by(cluster,zone)({leaf}))[3s:1s])',_records(state,groups=3,members=2),True,mode))
    aliases=[]
    for index in range(6):
        aliases.append(series({'__name__':f'm{index%2}','id':str(index//2),'empty':''},[(t*1000,(index+1)*(t+1)**2,'float')for t in range(6)]))
    selector='{__name__=~"m[01]"}'
    for leaf in (selector,f'rate({selector}[3s])'):
        for grouping in ('by(id)','without(__name__)','without()','by(__name__,id)','by()'):
            for delayed in (False,True):
                output.append(case(f'delayed-alias:{leaf}:{grouping}:{delayed}',f'max_over_time((sum {grouping}({leaf}))[3s:1s])',aliases,delayed))
    output.extend(delayed_group_performance_cases())
    assert len(output)==len({c['request']['id']for c in output})
    return output


def metadata_cases():
    records=[]
    for group in range(3):
        for member in range(2):
            records.append(series({'__name__':f'm{member}','id':str(group),'__type__':'counter' if member else 'gauge','__unit__':'bytes' if member else 'seconds'},
                [(t*1000,(group+1)*(member+1)*(t+1)**2,'float')for t in range(6)]))
    output=[]
    for leaf in ('{__name__=~"m[01]"}','rate({__name__=~"m[01]"}[3s])'):
        for grouping in ('by(id)','by(id,__name__)','by(id,__type__)','by(id,__unit__)','without(__type__,__unit__)','without()'):
            for delayed in (False,True):
                for mode in ('instant','range'):
                    output.append(case(f'delayed-metadata:{leaf}:{grouping}:{delayed}:{mode}',f'max_over_time((sum {grouping}({leaf}))[3s:1s])',records,delayed,mode))
    return output

def delayed_group_cases():
    return initial_delayed_group_cases()+metadata_cases()
