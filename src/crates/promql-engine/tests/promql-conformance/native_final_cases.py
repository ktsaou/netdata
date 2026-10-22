# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite final native endpoint ownership and normalization cases."""
from copy import deepcopy
from native_endpoint_cases import _records
from info_cases import bits
from single_reset_cases import _request

def _case(n,width,function,mode,pattern='native',reset=False):
    data=_records(n,width,pattern if pattern in ['float','integer','mixed','stale','custom'] else 'native')
    for record in data['series']:
        for index,point in enumerate(record['points']):
            key=point['kind']
            if key=='float':continue
            h=point[key];integer=key=='histogram'
            h['reset_hint']=3 if function=='idelta' else 0
            if pattern!='custom':h['schema']=4
            if reset and index%2:
                count=1
                h['positive_buckets']=[count]+[0]*(width-1) if integer else [bits(count)]*width
                h['count']=count*width if integer else bits(count*width)
                h['sum']=bits(count*width)
            if pattern=='empty':h['negative_spans']=[];h['negative_buckets']=[]
            if pattern=='extra-custom':h['custom_values']=[bits(1)]
            if pattern=='schemas':h['schema']=3+index%2
            if pattern=='start':point['start_time_ms']=point['time_ms']-100
    ident=f'{pattern}-{function}-{mode}-{n}-{width}-{reset}'
    request=_request(ident,f'{function}(m_total[{n}s])',n,mode,
                     'upstream-tests' if pattern=='start' else 'server-default')
    return {'shape':function if pattern=='native' else 'control','dataset':data,'request':request}

def native_final_performance_cases():
    pilot=[_case(n,width,function,mode,reset=reset) for n in [2,4] for width in [2,32,256,4096]
           for function in ['irate','idelta'] for mode in ['instant','range'] for reset in [False,True]]
    pilot += [_case(4,32,function,mode,pattern) for pattern in
              ['float','integer','mixed','stale','custom','empty','extra-custom','schemas','start']
              for function in ['irate','idelta'] for mode in ['instant','range']]
    return pilot

def native_final_cases():
    pilot=native_final_performance_cases()
    controls=[]
    for original in pilot:
        opposite=deepcopy(original);r=opposite['request']
        r['profile']='server-default' if r['profile']=='upstream-tests' else 'upstream-tests'
        r['id']='opposite-'+r['id'];controls.append(opposite)
    return pilot+controls
