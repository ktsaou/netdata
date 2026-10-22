# SPDX-License-Identifier: GPL-3.0-or-later
"""Finite rejected terminal native predecessor experiment inputs."""
from copy import deepcopy
from native_second_cases import _case as _endpoint_case
from native_endpoint_cases import _records
from info_cases import bits

def _case(n,width,function,mode,pattern='native',reset=False):
    c=_endpoint_case(n,width,function,mode,pattern,reset)
    if pattern=='mixed-kinds':
        c['dataset']=_records(n,width,pattern)
        for row in c['dataset']['series']:
            for h in row['points']:
                value=h[h['kind']];value['schema']=4;value['reset_hint']=0
    if pattern in ['narrow-last','absent-last','trailing-stale']:
        for row in c['dataset']['series']:
            for index,p in enumerate(row['points']):
                h=p[p['kind']]
                if index>=n-1:
                    if pattern=='trailing-stale':h['sum']='7ff0000000000002'
                    else:
                        width_now=2 if pattern=='narrow-last' else 0
                        h['positive_spans']=[{'offset':0,'length':width_now}] if width_now else None
                        h['positive_buckets']=[bits(1)]*width_now if width_now else None
                        h['count']=bits(width_now);h['sum']=bits(width_now);h['reset_hint']=1
    c['shape']=function if pattern=='native' and n>=2 and function!='delta' else 'control'
    return c

def native_terminal_performance_cases():
    pilot=[_case(n,w,f,m,reset=r)for n in [2,4]for w in [2,256,4096]for f in ['rate','increase']for m in ['instant','range']for r in [False,True]]
    pilot += [_case(n,256,f,m,p)for p in ['integer','mixed-kinds']for n in [2,4]for f in ['rate','increase']for m in ['instant','range']]
    pilot += [_case(4,32,f,m,p)for p in ['float','mixed','stale','custom','empty','extra-custom','schemas','start','derived']for f in ['rate','increase']for m in ['instant','range']]
    pilot += [_case(4,4096,f,m,p)for p in ['narrow-last','absent-last','trailing-stale']for f in ['rate','increase']for m in ['instant','range']]
    pilot += [_case(n,256,'delta',m)for n in [1,2,4]for m in ['instant','range']]
    pilot += [_case(1,32,f,m)for f in ['rate','increase']for m in ['instant','range']]
    return pilot

def native_terminal_cases():
    pilot=native_terminal_performance_cases()
    controls=[]
    for c in pilot:
        c=deepcopy(c);r=c['request'];r['id']='opposite-'+r['id'];r['profile']='server-default' if r['profile']=='upstream-tests' else 'upstream-tests';controls.append(c)
    return pilot+controls
