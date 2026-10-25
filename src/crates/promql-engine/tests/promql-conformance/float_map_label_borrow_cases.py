"""Finite Float-map ownership cases and unchanged accepted controls."""
from mapped_label_borrow_cases import cases as previous
from info_cases import series
UNARY='abs ceil floor exp sqrt ln log2 log10 sin cos tan asin acos atan sinh cosh tanh asinh acosh atanh rad deg sgn days_in_month day_of_month day_of_week day_of_year hour minute month year'.split()
EXPRESSIONS=[(name,f'{name}(m_total)') for name in UNARY]+[('round','round(m_total)'),('round-half','round(m_total,0.5)'),('clamp','clamp(m_total,0,100)'),('clamp_min','clamp_min(m_total,0)'),('clamp_max','clamp_max(m_total,100)')]
TIMED=['ceil','floor','round','clamp','clamp_min','clamp_max','sqrt','ln','hour','sgn']
def cases():
    result=previous()
    for c in result:c['target']=False;c['timed']=False
    def add(ident,expression,data,mode,delayed,target=False,timed=False):
        result.append({'request':{'id':ident+'-'+mode+'-d'+str(int(delayed)),'query':'sum by(job)('+expression+')','profile':'server-default','mode':mode,'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,'delayed_name_removal':delayed},'dataset':data,'faults':{},'target':target,'timed':timed,'child':'float-map','operator':'sum','keys':['job']})
    for count,width in ((4,2),(64,16)):
        data={'schema_version':1,'series':[series({'__name__':'m_total','id':f'{i:06}','job':f'g{i//2:06}',**{f'payload{k:02}':'v'*32 for k in range(width)}},[(t*1000,float(i+t+1),'float') for t in (1,2,3,4)]) for i in range(count)]}
        for name,expression in EXPRESSIONS:
            if count==64 and name not in TIMED:continue
            for mode in ('instant','range'):
                for delayed in (False,True):add(f'float-n{count}-{name}',expression,data,mode,delayed,mode=='range' and count==64,count==64)
    edge={'schema_version':1,'series':[series({'__name__':'m_total','id':str(i),'job':f'g{i//2}','empty':'','__type__':'gauge','__unit__':'raw'},[(t*1000,v,'float') for t,v in enumerate(values,1)]) for i,values in enumerate([[-4.0,-0.0,0.0,0.25],[1.0,4.0,float('inf'),float('-inf')],['7ff8000000000001','7ff0000000000002',1.0,-1.0],[2.0,3.0,4.0,5.0]])]}
    edge_expressions=EXPRESSIONS+[('clamp-empty','clamp(m_total,2,1)'),('clamp-changing','clamp(m_total,0,(time()-3)^2-.5)'),('clamp-nan','clamp_min(m_total,NaN)'),('clamp-scalar','clamp_min(m_total,scalar(min(p)))'),('clamp-warning','clamp_min(m_total,scalar(sum(p)))'),('round-zero','round(m_total,0)'),('round-negative','round(m_total,-0.5)'),('round-nan','round(m_total,NaN)'),('nested','ceil(floor(m_total))'),('range-child','ceil(rate(m_total[2s]))'),('calendar-default','hour()'),('timestamp','timestamp(m_total)')]
    edge['series'] += [series({'__name__':'p','id':'a'},[(t*1000,0.0,'float') for t in (1,2,3,4)]),series({'__name__':'p','id':'b'},[(t*1000,1.0,'histogram') for t in (1,2,3,4)])]
    for name,expression in edge_expressions:
        for delayed in (False,True):add('edge-'+name,expression,edge,'range',delayed)
    assert len({c['request']['id'] for c in result})==len(result)
    return result
