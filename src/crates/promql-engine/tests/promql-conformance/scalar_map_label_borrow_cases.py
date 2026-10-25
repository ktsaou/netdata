"""Scalar arithmetic on both operand sides, beside accepted mapped controls."""
from float_map_label_borrow_cases import cases as previous
from info_cases import series
OPERATORS=[('add','+'),('sub','-'),('mul','*'),('div','/'),('mod','%'),('pow','^'),('atan2','atan2')]
TIMED={'add','sub','mul','div','mod'}
def cases():
 result=previous()
 for c in result:c['target']=False;c['timed']=False
 def add(ident,expression,data,mode,delayed,target=False,timed=False):
  result.append({'request':{'id':ident+'-'+mode+'-d'+str(int(delayed)),'query':'sum by(job)('+expression+')','profile':'server-default','mode':mode,'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,'delayed_name_removal':delayed},'dataset':data,'faults':{},'target':target,'timed':timed,'child':'scalar-map','operator':'sum','keys':['job']})
 for count,width in ((4,2),(64,16)):
  data={'schema_version':1,'series':[series({'__name__':'m_total','id':f'{i:06}','job':f'g{i//2:06}',**{f'payload{k:02}':'v'*32 for k in range(width)}},[(t*1000,float(i+t+1),'float') for t in (1,2,3,4)]) for i in range(count)]}
  for name,operator in OPERATORS:
   if count==64 and name not in TIMED:continue
   for side in ('left','right'):
    expression=f'3 {operator} m_total' if side=='left' else f'm_total {operator} 3'
    for mode in ('instant','range'):
     for delayed in (False,True):add(f'scalar-n{count}-{name}-{side}',expression,data,mode,delayed,count==64 and mode=='range',count==64)
 edge={'schema_version':1,'series':[series({'__name__':'m_total','id':str(i),'job':f'g{i//2}','empty':'','__type__':'gauge','__unit__':'raw'},[(t*1000,v,'float') for t,v in enumerate(values,1)]) for i,values in enumerate([[-4.0,-0.0,0.0,.25],[1.0,4.0,float('inf'),float('-inf')],['7ff8000000000001','7ff0000000000002',1.0,-1.0],[2.0,3.0,4.0,5.0]])]}
 native={'schema_version':1,'series':[series({'__name__':'m_total','id':str(i),'job':f'g{i//2}','empty':''},[(t*1000,float(i+t+1),'float_histogram' if i in (2,3) or (i==5 and t%2==0) else 'float') for t in (1,2,3,4)]) for i in range(6)]}
 for family,data in [('edge',edge),('native',native)]:
  for name,operator in OPERATORS:
   for side in ('left','right'):
    expression=f'0 {operator} m_total' if side=='left' else f'm_total {operator} 0'
    for delayed in (False,True):add(f'scalar-{family}-{name}-{side}',expression,data,'range',delayed)
 edge['series'] += [series({'__name__':'p','id':'a'},[(t*1000,2.0,'float') for t in (1,2,3,4)]),series({'__name__':'p','id':'b'},[(t*1000,1.0,'histogram') for t in (1,2,3,4)])]
 extras=[('dynamic-right','m_total/(time()-3)'),('dynamic-left','(time()-3)/m_total'),('parameter-info','m_total*scalar(min(p))'),('parameter-warning','scalar(sum(p))*m_total'),('nan-right','m_total+NaN'),('nan-left','NaN+m_total'),('infinite','m_total*Inf'),('comparison','m_total>3'),('comparison-left','3<m_total'),('bool','m_total>bool 3'),('bool-left','3<bool m_total'),('unary-minus','-m_total'),('unary-plus','+m_total'),('nested','(m_total+1)*2'),('range-child','rate(m_total[2s])/3')]
 for name,expression in extras:
  for delayed in (False,True):add('scalar-'+name,expression,edge,'range',delayed)
 assert len(result)==1086 and len({c['request']['id'] for c in result})==len(result)
 return result
