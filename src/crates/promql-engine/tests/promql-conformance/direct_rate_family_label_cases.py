"""Direct Rate-family groups, typed histories, metadata aliases and unchanged consumers."""
from copy import deepcopy
from rate_family_label_borrow_cases import cases as previous
from rate_group_label_cases import metadata_cases
from info_cases import series

FUNCTIONS=('delta','irate','idelta')
OPERATORS=('sum','avg','count','min','max','group','stdvar','stddev')

def cases():
    result=previous()
    for c in result:c['target']=False;c['timed']=False
    old=list(result)
    def dataset(count,width):
        return {'schema_version':1,'series':[series({'__name__':'m_total','__type__':'gauge','__unit__':'raw','id':f'{i:06}','job':f'g{i//2:06}','empty':'',**{f'payload{k:02}':'v'*32 for k in range(width)}},[(t*1000,float((i+1)*t),'float') for t in (1,2,3,4)]) for i in range(count)]}
    def add(ident,operator,group,child,data,mode,delayed,target=False,timed=False,profile='server-default'):
        result.append({'request':{'id':ident+'-'+mode+'-d'+str(int(delayed)),'query':operator+group+'('+child+')','profile':profile,'mode':mode,'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,'subquery_step_ms':1000,'delayed_name_removal':delayed},'dataset':data,'faults':{},'target':target,'timed':timed,'child':'direct-rate-family','operator':operator})
    for count,width in ((4,2),(64,16)):
        data=dataset(count,width)
        for function in FUNCTIONS:
            for operator in OPERATORS:
                for shape,group in [('global',''),('byjob',' by(job)')]:
                    for mode in ('instant','range'):
                        for delayed in (False,True):
                            target=count==64 and operator in ('sum','max')
                            timed=target or count==4 and operator=='sum'
                            add(f'direct-family-n{count}-{operator}-{shape}-{function}',operator,group,f'{function}(m_total[3s])',data,mode,delayed,target,timed)
    data=dataset(64,16)
    for function in ('raw','rate','increase'):
        child='m_total' if function=='raw' else f'{function}(m_total[3s])'
        for operator in OPERATORS:
            for shape,group in [('global',''),('byjob',' by(job)')]:
                for mode in ('instant','range'):
                    for delayed in (False,True):
                        add(f'direct-family-control-{operator}-{shape}-{function}',operator,group,child,data,mode,delayed,timed=operator in ('sum','max'))
    for function in FUNCTIONS:
        base=f'{function}(m_total[3s])'
        for shape,group,child in [('wide',' by(job,id)',base),('without',' without(id)',base),('mapped',' by(job)',f'abs({base})'),('derived',' by(job)',f'{function}((m_total*1)[3s:1s])'),('unary',' by(job)','-'+base),('metadata',' by(__name__)',base)]:
            for delayed in (False,True):
                add(f'direct-family-control-{function}-{shape}','sum',group,child,data if shape in ('wide','without') else dataset(4,2),'range',delayed,timed=shape in ('wide','without'))
    for state in ('reset','nan','stale','native','mixed-first','mixed-tail','native-then-float','float-then-native','start-reset','overlap','empty','singleton'):
        template=next(c for c in old if c['request']['id']==f'rate-family-{state}-delta-div-right-range-d0')
        for function in FUNCTIONS:
            for operator in ('sum','max','count'):
                for shape,group in [('global',''),('byjob',' by(job)')]:
                    for delayed in (False,True):
                        add(f'direct-family-state-{state}-{function}-{operator}-{shape}',operator,group,f'{function}(m[3s])',template['dataset'],'range',delayed,profile=template['request']['profile'])
    for template in metadata_cases():
        if ':rate:' not in template['request']['id']:continue
        for function in FUNCTIONS:
            c=deepcopy(template);c['request']['id']='direct-family-'+c['request']['id'].replace(':rate:',':'+function+':');c['request']['query']=c['request']['query'].replace('rate(',function+'(');c['target']=False;c['timed']=False;result.append(c)
    assert len(result)==3870 and len({c['request']['id'] for c in result})==len(result)
    assert sum(c['target'] for c in result)==48 and sum(c['timed'] for c in result)==132
    return result
