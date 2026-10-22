# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite inputs retained for the rejected descriptor-hash experiment."""
from copy import deepcopy
import re
from info_cases import series

def _cases(count, ticks, width):
    data = {'schema_version': 1, 'series': [series(
        {'__name__': 'm', 'id': str(i), 'job': 'worker', 'detail': 'a' * width},
        [(t * 1000, float(10 + t + i), 'float') for t in range(ticks + 1)])
        for i in range(count)]}
    out = []
    for shape, expression in [('rate', 'rate(m[2s])'), ('fold', 'sum_over_time(m[2s])'),
                              ('scheduled', 'abs(m)'), ('generic', 'count_values("value",m)'),
                              ('selector', 'm')]:
        for mode in ['range', 'instant']:
            ident = f'{shape}-{mode}-{count}-{ticks}-{width}'
            request = {'id': ident, 'query': expression, 'profile': 'server-default', 'mode': mode,
                'time_ms': ticks * 1000, 'start_ms': 1000, 'end_ms': ticks * 1000,
                'step_ms': 1000, 'lookback_ms': 300000, 'subquery_step_ms': 1000}
            out.append({'shape': shape if mode == 'range' and shape != 'selector' else 'control',
                        'dataset': data, 'request': request})
    return out

def descriptor_hash_performance_cases():
    panel = [c for count, ticks in [(4,128),(16,256)] for width in [24,1024]
             for c in _cases(count,ticks,width)]
    added=[]
    for case in panel:
        if case['request']['query'].startswith('count_values('):
            repeated=deepcopy(case)
            repeated['request']['id']='repeated-'+case['request']['id']
            repeated['request']['query']='count_values by(job,detail)("value",m % 2)'
            added.append(repeated)
            case['shape']='control'
    panel+=added
    assert len(panel)==48
    return panel

def descriptor_hash_cases():
    panel=descriptor_hash_performance_cases()
    controls=[]
    queries=['m','abs(m)','rate(m[2s])','sum_over_time(m[2s])','last_over_time(m[2s])',
        'count_values by(job)("value",m)','sum by(job)(m)','sort(m)','topk(1,m)',
        'm or m','abs(rate(m[2s]))','count_values by(job)("value",m % 2)']
    for pattern in ['sparse','mixed','native','names','empty-label','bytes']:
        data=[]
        for i in range(2):
            labels={'__name__':'m','id':str(i),'job':'worker'}
            if pattern=='names':labels['__name__']='m' if i==0 else 'other'
            if pattern=='empty-label':labels['optional']=''
            if pattern=='bytes':labels['detail']=b'\xff\x00\n'
            points=[(t*1000,float(t+i+1),'float_histogram' if pattern=='native' or
                     (pattern=='mixed' and t%2) else 'float') for t in range(9)
                    if pattern!='sparse' or (t+i)%3]
            data.append(series(labels,points))
        for expression in queries:
            if pattern=='names':expression=re.sub(r'\bm\b','{job="worker"}',expression)
            for profile in ['server-default','upstream-tests']:
                ident=f'edge-{pattern}-{len(controls)}-{profile}'
                controls.append({'shape':'edge','dataset':{'schema_version':1,'series':data},
                    'request':{'id':ident,'query':expression,'profile':profile,'mode':'range',
                        'time_ms':8000,'start_ms':1000,'end_ms':8000,'step_ms':1000,
                        'lookback_ms':1500,'subquery_step_ms':1000}})
    assert len(panel)==48 and len(controls)==144
    return panel+controls
