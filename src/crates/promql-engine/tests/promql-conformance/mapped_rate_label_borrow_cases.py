"""Ordinary mapped counter windows beside unchanged numeric-map controls."""
from copy import deepcopy
from comparison_map_label_borrow_cases import cases as previous
from info_cases import series

MAPS = [('div-right','{x}/3'),('div-left','3/{x}'),('abs','abs({x})'),
        ('ceil','ceil({x})'),('gt-right','{x}>0'),('bool-right','{x}>bool 0'),
        ('gt-left','0<{x}'),('bool-left','0<bool {x}')]

def cases():
    result = previous()
    for case in result:
        case['target'] = False
        case['timed'] = False
    def add(ident, expression, data, mode, delayed, target=False, timed=False, profile='server-default'):
        result.append({'request': {'id': ident+'-'+mode+'-d'+str(int(delayed)),
            'query':'sum by(job)('+expression+')','profile':profile,'mode':mode,
            'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,
            'lookback_ms':1000,'subquery_step_ms':1000,'delayed_name_removal':delayed},
            'dataset':data,'faults':{},'target':target,'timed':timed,
            'child':'mapped-rate','operator':'sum','keys':['job']})
    for count,width in ((4,2),(64,16)):
        data={'schema_version':1,'series':[series({'__name__':'m_total','__type__':'counter',
            '__unit__':'raw','id':f'{i:06}','job':f'g{i//2:06}','empty':'',
            **{f'payload{k:02}':'v'*32 for k in range(width)}},
            [(t*1000,float((i+1)*t),'float') for t in (1,2,3,4)]) for i in range(count)]}
        for function in ('rate','increase'):
            for name,template in MAPS:
                for mode in ('instant','range'):
                    for delayed in (False,True):
                        timed=count==64 and name not in ('gt-left','bool-left')
                        add(f'mapped-rate-n{count}-{function}-{name}',template.format(x=f'{function}(m_total[3s])'),
                            data,mode,delayed,timed and mode=='range',timed)
    states={'reset':[100,0,150,160],'nan':[100,'7ff8000000000042',0,10],
        'stale':[100,'7ff0000000000002',0,10],'native':[1,2,3,4],
        'mixed':[1,2,3,4],'start-reset':[1,2,3,4],'overlap':[1,2,3,4],
        'empty':[],'singleton':[1],'noncounter':[1,2,3,4]}
    for state,values in states.items():
        metric='m' if state=='noncounter' else 'm_total'
        records=[]
        for i in range(4):
            row=series({'__name__':metric,'id':str(i),'job':f'g{i//2}',
                '__type__':'counter','__unit__':'raw','empty':''},
                [((j+1)*1000,value,'float_histogram' if state=='native' or state=='mixed' and j==1 else 'float')
                 for j,value in enumerate(values)])
            if state in ('start-reset','overlap'):
                for point,start in zip(row['points'],[500,1500,1500,1500] if state=='start-reset' else [500,750,750,750],strict=True):
                    point['start_time_ms']=start
            records.append(row)
        data={'schema_version':1,'series':records}
        for function in ('rate','increase'):
            for name,template in (MAPS[0],MAPS[2],MAPS[4],MAPS[7]):
                for delayed in (False,True):
                    add(f'mapped-rate-{state}-{function}-{name}',template.format(x=f'{function}({metric}[3s])'),data,'range',delayed)
    data={'schema_version':1,'series':[series({'__name__':'m_total','id':str(i),'job':f'g{i//2}',
        '__type__':'counter','__unit__':'raw','empty':''},
        [(t*1000,float((i+1)*t),'float') for t in (1,2,3,4)]) for i in range(4)]}
    controls=[('delta','abs(delta(m_total[3s]))'),('irate','abs(irate(m_total[3s]))'),
        ('derived','abs(rate((m_total*1)[3s:1s]))'),('unary','-rate(m_total[3s])'),
        ('wrapped','abs(abs(rate(m_total[3s])))'),('offset','abs(rate(m_total[3s] offset 1s))'),
        ('fractional','abs(rate(m_total[2.999999999] offset 0.000000001))'),
        ('fixed','abs(rate(m_total[3s] @ 3))'),
        ('anchored','abs(rate(m_total[3s] anchored))'),('smoothed','abs(rate(m_total[3s] smoothed))')]
    for name,expression in controls:
        for delayed in (False,True):
            add('mapped-rate-control-'+name,expression,data,'range',delayed,profile='upstream-tests')
    assert len(result)==1698 and len({c['request']['id'] for c in result})==len(result)
    return result
