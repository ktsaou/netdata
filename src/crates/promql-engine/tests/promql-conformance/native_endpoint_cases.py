# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite native instant-rate ownership targets and generic-copy controls."""
from copy import deepcopy
from info_cases import series, bits
from single_reset_cases import _request


def _records(n,buckets,pattern='native'):
    out=[]
    for identity in range(4):
        row=series({'__name__':'m_total','id':str(identity),'job':'worker'},[])
        for tick in range(1,n+3):
            integer=pattern=='integer' or pattern=='mixed-kinds' and tick%2==0
            native=pattern!='float' and not(pattern=='mixed' and tick%3==0)
            count=(tick+identity+1)
            point=series({},[(tick*1000,count,'histogram'if integer else 'float_histogram'if native else 'float')])['points'][0]
            if native:
                key='histogram'if integer else 'float_histogram';h=point[key]
                size=8 if pattern=='varying' and (tick//2)%2 else buckets
                # Two equal-width samples followed by two narrow ones exercise expired-tail fallback.
                h['positive_spans']=[{'offset':0,'length':size}]
                h['positive_buckets']=[count]+[0]*(size-1) if integer else [bits(count)]*size
                h['count']=count*size if integer else bits(count*size)
                h['sum']=bits(count*size)
                if pattern=='custom':
                    h['schema']=-53;h['custom_values']=[bits(x+1)for x in range(size-1)]
                if pattern=='stale' and tick%5==0:h['sum']='7ff0000000000002'
            row['points'].append(point)
        out.append(row)
    return {'schema_version':1,'series':out}


def native_endpoint_performance_cases():
    pilot=[]
    for n in [32,256]:
        for buckets in [8,256]:
            for function in ['irate','idelta']:
                ident=f'target-{function}-{n}-{buckets}'
                pilot.append({'shape':function,'dataset':_records(n,buckets),'request':_request(ident,f'{function}(m_total[{n}s])',n,'instant')})
    for pattern in ['float','integer','mixed-kinds','mixed','custom','varying','stale']:
        for function in ['irate','idelta']:
            n,buckets=32,256
            pilot.append({'shape':'control','dataset':_records(n,buckets,pattern),'request':_request(f'control-{function}-{pattern}',f'{function}(m_total[{n}s])',n,'instant')})
    return pilot


def native_endpoint_cases():
    pilot = native_endpoint_performance_cases()
    controls=[]
    for c in pilot:
        for profile in ['server-default','upstream-tests']:
            d=deepcopy(c);d['request']['id']='range-'+d['request']['id']+'-'+profile;d['request']['mode']='range';d['request']['profile']=profile;controls.append(d)
    return pilot + controls
