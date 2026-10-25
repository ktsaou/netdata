"""Remaining mapped rate functions, selected endpoint kinds and unchanged consumers."""
from mapped_rate_label_borrow_cases import cases as previous
from info_cases import series

MAPS=[('div-right','{x}/3'),('div-left','3/{x}'),('abs','abs({x})'),('ceil','ceil({x})'),
      ('gt-right','{x}>0'),('bool-right','{x}>bool 0'),('gt-left','0<{x}'),('bool-left','0<bool {x}')]
FUNCTIONS=('delta','irate','idelta')

def cases():
    result=previous()
    for case in result:case['target']=False;case['timed']=False
    def add(ident,expression,data,mode,delayed,target=False,timed=False,profile='server-default'):
        result.append({'request':{'id':ident+'-'+mode+'-d'+str(int(delayed)),
            'query':'sum by(job)('+expression+')','profile':profile,'mode':mode,'time_ms':4000,
            'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,
            'subquery_step_ms':1000,'delayed_name_removal':delayed},
            'dataset':data,'faults':{},'target':target,'timed':timed,'child':'mapped-rate-family','operator':'sum','keys':['job']})
    def record(index,points,width=0):
        return series({'__name__':'m','__type__':'gauge','__unit__':'raw','id':f'{index:06}',
            'job':f'g{index//2:06}','empty':'',**{f'payload{k:02}':'v'*32 for k in range(width)}},points)
    for count,width in ((4,2),(64,16)):
        data={'schema_version':1,'series':[record(i,[(t*1000,float((i+1)*t),'float') for t in (1,2,3,4)],width) for i in range(count)]}
        for function in FUNCTIONS:
            for name,template in MAPS:
                for mode in ('instant','range'):
                    for delayed in (False,True):
                        timed=count==64 and name not in ('gt-left','bool-left')
                        add(f'rate-family-n{count}-{function}-{name}',template.format(x=f'{function}(m[3s])'),data,mode,delayed,timed and mode=='range',timed)
    states={'reset':[100,0,150,160],'nan':[100,'7ff8000000000042',0,10],
        'stale':[100,'7ff0000000000002',0,10],'native':[1,2,3,4],
        'mixed-first':[1,2,3,4],'mixed-tail':[1,2,3,4],
        'native-then-float':[1,2,3,4],'float-then-native':[1,2,3,4],
        'start-reset':[1,2,3,4],'overlap':[1,2,3,4],'empty':[],'singleton':[1]}
    for state,values in states.items():
        records=[]
        for i in range(4):
            def kind(j):
                native=state=='native' or state=='mixed-first' and j==0 or state=='mixed-tail' and j==3
                native |= state=='native-then-float' and j<2 or state=='float-then-native' and j>=2
                return 'float_histogram' if native else 'float'
            row=record(i,[((j+1)*1000,value,kind(j)) for j,value in enumerate(values)])
            if state in ('start-reset','overlap'):
                for point,start in zip(row['points'],[500,1500,1500,1500] if state=='start-reset' else [500,750,750,750],strict=True):point['start_time_ms']=start
            records.append(row)
        data={'schema_version':1,'series':records}
        for function in FUNCTIONS:
            for name,template in (MAPS[0],MAPS[2],MAPS[4],MAPS[7]):
                for delayed in (False,True):
                    add(f'rate-family-{state}-{function}-{name}',template.format(x=f'{function}(m[3s])'),data,'range',delayed,
                        profile='upstream-tests' if state in ('start-reset','overlap') else 'server-default')
    for state,values in [('spacing',[2,5,11,17]),('reset-spacing',[10,20,2,4])]:
        data={'schema_version':1,'series':[record(i,[(time,float(value*(i+1)),'float') for time,value in zip((500,1500,2800,4000),values,strict=True)]) for i in range(4)]}
        for function in FUNCTIONS:
            for name,template in MAPS:
                for mode in ('instant','range'):
                    for delayed in (False,True):
                        add(f'rate-family-{state}-{function}-{name}',template.format(x=f'{function}(m[3s])'),data,mode,delayed)
    data={'schema_version':1,'series':[record(i,[(t*1000,float((i+1)*t),'float') for t in (1,2,3,4)]) for i in range(4)]}
    for function in FUNCTIONS:
        child=f'{function}(m[3s])'
        for name,expression in [('nested',f'abs(abs({child}))'),('unary','-'+child),
            ('derived',f'abs({function}((m*1)[3s:1s]))'),('offset',f'abs({function}(m[3s] offset 1s))'),
            ('fractional',f'abs({function}(m[2.999999999] offset 0.000000001))'),('fixed',f'abs({function}(m[3s] @ 3))')]:
            for delayed in (False,True):add(f'rate-family-control-{function}-{name}',expression,data,'range',delayed)
    assert len(result)==2406 and len({c['request']['id'] for c in result})==len(result)
    return result
