"""Comparison filtering and Boolean results beside arithmetic controls."""
from scalar_map_label_borrow_cases import cases as previous
from info_cases import series
OPERATORS=[('eq','=='),('ne','!='),('gt','>'),('lt','<'),('ge','>='),('le','<=')]
TIMED={'eq','ne','gt'}
def cases():
 result=previous()
 for case in result:case['target']=False;case['timed']=False
 def add(ident,expression,data,mode,delayed,target=False,timed=False):
  result.append({'request':{'id':ident+'-'+mode+'-d'+str(int(delayed)),'query':'sum by(job)('+expression+')','profile':'server-default','mode':mode,'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,'delayed_name_removal':delayed},'dataset':data,'faults':{},'target':target,'timed':timed,'child':'comparison-map','operator':'sum','keys':['job']})
 for count,width in ((4,2),(64,16)):
  data={'schema_version':1,'series':[series({'__name__':'m_total','__type__':'gauge','__unit__':'raw','id':f'{i:06}','job':f'g{i//2:06}',**{f'payload{k:02}':'v'*32 for k in range(width)}},[(t*1000,float(i+t),'float') for t in (1,2,3,4)]) for i in range(count)]}
  for name,operator in OPERATORS:
   for side in ('left','right'):
    for boolean in (False,True):
     op=operator+(' bool' if boolean else '');threshold=count//2
     expression=f'{threshold} {op} m_total' if side=='left' else f'm_total {op} {threshold}'
     for mode in ('instant','range'):
      for delayed in (False,True):
       timed=count==64 and name in TIMED
       add(f'compare-n{count}-{name}-{side}-b{int(boolean)}',expression,data,mode,delayed,timed and mode=='range',timed)
 edge={'schema_version':1,'series':[series({'__name__':'m_total','id':str(i),'job':f'g{i//2}','empty':'','__type__':'gauge','__unit__':'raw'},[(t*1000,v,'float') for t,v in enumerate(values,1)]) for i,values in enumerate([[-4.0,-0.0,0.0,.25],[1.0,4.0,float('inf'),float('-inf')],['7ff8000000000001','7ff0000000000002',1.0,-1.0],[2.0,3.0,4.0,5.0]])]}
 native={'schema_version':1,'series':[series({'__name__':'m_total','id':str(i),'job':f'g{i//2}','empty':''},[(t*1000,float(i+t+1),'float_histogram' if i in (2,3) or (i==5 and t%2==0) else 'float') for t in (1,2,3,4)]) for i in range(6)]}
 for family,data in [('edge',edge),('native',native)]:
  for name,operator in OPERATORS:
   for side in ('left','right'):
    for boolean in (False,True):
     op=operator+(' bool' if boolean else '')
     expression=f'0 {op} m_total' if side=='left' else f'm_total {op} 0'
     for delayed in (False,True):add(f'compare-{family}-{name}-{side}-b{int(boolean)}',expression,data,'range',delayed)
 edge['series'] += [series({'__name__':'p','id':'a'},[(t*1000,2.0,'float') for t in (1,2,3,4)]),series({'__name__':'p','id':'b'},[(t*1000,1.0,'histogram') for t in (1,2,3,4)])]
 extras=[('dynamic-right','m_total>(time()-3)'),('dynamic-left','(time()-3)<m_total'),('dynamic-bool','m_total>bool(time()-3)'),('parameter-info','m_total>=scalar(min(p))'),('parameter-warning','scalar(sum(p))!=bool m_total'),('nan-right','m_total==NaN'),('nan-left','NaN!=bool m_total'),('infinite','m_total<Inf')]
 for name,expression in extras:
  for delayed in (False,True):add('compare-'+name,expression,edge,'range',delayed)
 assert len(result)==1390 and len({c['request']['id'] for c in result})==len(result)
 return result
