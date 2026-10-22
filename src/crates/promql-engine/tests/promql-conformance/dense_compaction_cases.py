# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite rejected dense compaction targets and unchanged-layout controls."""
from copy import deepcopy
from native_second_cases import _case as endpoint_case
from info_cases import bits

def _case(n,width,function,mode,pattern='native'):
    c=endpoint_case(n,width,function,mode,pattern)
    for row in c['dataset']['series']:
        for point in row['points']:
            if point['kind']=='float':continue
            h=point[point['kind']]
            h['reset_hint']=3 if function in ['delta','idelta'] else 0
            if pattern=='positive-offset':h['positive_spans'][0]['offset']=3
            if pattern=='zero-output':
                h['positive_buckets']=[bits(1)]*width;h['count']=bits(width);h['sum']=bits(width)
            if pattern=='sparse':
                h['positive_buckets']=[bits(0)if i%4==0 else value for i,value in enumerate(h['positive_buckets'])]
            if pattern=='multiple':
                h['positive_spans']=[{'offset':0,'length':width//2},{'offset':0,'length':width-width//2}]
    c['shape']=function if pattern=='native' and width>=256 else 'control'
    return c

def dense_compaction_performance_cases():
    functions=['rate','increase','delta','irate','idelta']
    pilot=[_case(n,w,f,m)for n in [2,4]for w in [256,4096]for f in functions for m in ['instant','range']]
    pilot += [_case(2,2,f,m)for f in functions for m in ['instant','range']]
    pilot += [_case(4,256,f,m,p)for p in ['positive-offset','zero-output','sparse','multiple','integer','custom','mixed','float','start','derived']for f in functions for m in ['instant','range']]
    for width in [256,4096]:
        for function,expression in [('sum','sum(m_total)'),('avg','avg(m_total)'),('add','m_total+m_total'),('subtract','m_total-m_total')]:
            for mode in ['instant','range']:
                c=_case(2,width,'rate',mode);c['shape']='control';c['request']['query']=expression;c['request']['id']=f'aggregate-{function}-{mode}-{width}';pilot.append(c)
    return pilot

def dense_compaction_cases():
    pilot=dense_compaction_performance_cases()
    controls=[]
    for c in pilot:
        c=deepcopy(c);r=c['request'];r['id']='opposite-'+r['id'];r['profile']='server-default' if r['profile']=='upstream-tests' else 'upstream-tests';controls.append(c)
    return pilot+controls
