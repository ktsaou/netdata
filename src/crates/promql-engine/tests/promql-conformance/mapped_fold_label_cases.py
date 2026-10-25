"""Mapped Sum/Avg windows with native/metadata and prior rejected control shapes."""
from copy import deepcopy
from rate_family_label_borrow_cases import cases as previous, MAPS
from direct_rate_family_label_cases import cases as direct_cases
from fold_group_label_cases import fold_group_label_cases
from rate_group_label_cases import metadata_cases
from info_cases import series

FUNCTIONS=('sum_over_time','avg_over_time')
REJECTED_CONTROLS=(
    'n128-none-rate-plural-range-d0-sum_over_time',
    'n128-none-rate-plural-range-d0-avg_over_time',
    'n128-none-rate-plural-range-d1-sum_over_time',
    'n128-none-rate-plural-range-d1-avg_over_time',
    'labels-n128-w32-withoutid-instant-d0',
)

def cases():
    seed=previous();result=deepcopy(seed)
    for c in result:c['target']=False;c['timed']=False
    prefix='sum by(job)('
    for template in seed:
        ident=template['request']['id']
        if not ident.startswith('rate-family-') or '-delta-' not in ident:continue
        assert template['request']['query'].startswith(prefix)
        for function in FUNCTIONS:
            for context in ('aggregate','root'):
                c=deepcopy(template)
                c['request']['id']=ident.replace('rate-family-','mapped-fold-',1).replace('-delta-','-'+function+'-',1)+'-'+context
                query=c['request']['query'].replace('delta(',function+'(')
                c['request']['query']=query if context=='aggregate' else query[len(prefix):-1]
                c['child']='mapped-fold';c['operator']='sum' if context=='aggregate' else None
                c['timed']=bool(template.get('timed')) or (ident.startswith('rate-family-n4-') and any('-'+m+'-' in ident for m in ('div-right','bool-right')))
                c['target']=bool(template.get('target'));result.append(c)
    for state,values in [('negative',[-10,-4,3,-5]),('infinities',[1,'7ff0000000000000','fff0000000000000',4])]:
        data={'schema_version':1,'series':[series({'__name__':'m','__type__':'gauge','__unit__':'KiB','id':str(i),'job':str(i//2),'empty':''},[(time,value,'float') for time,value in zip((500,1500,2500,4000),values,strict=True)]) for i in range(4)]}
        for function in FUNCTIONS:
            for name,pattern in MAPS:
                for context in ('aggregate','root'):
                    for mode in ('instant','range'):
                        for delayed in (False,True):
                            expression=pattern.format(x=f'{function}(m[4500ms])')
                            result.append({'request':{'id':f'mapped-fold-{state}-{function}-{name}-{context}-{mode}-d{int(delayed)}','query':prefix+expression+')' if context=='aggregate' else expression,'profile':'server-default','mode':mode,'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,'subquery_step_ms':1000,'delayed_name_removal':delayed},'dataset':data,'faults':{},'target':False,'timed':False})
    for template in metadata_cases():
        if ':rate:' not in template['request']['id']:continue
        query=template['request']['query'];start=query.index('rate(');child=query[start:-1]
        assert query.endswith('))') and child.startswith('rate(')
        contexts=('aggregate','root') if query.startswith('sum(rate(') else ('aggregate',)
        for function in FUNCTIONS:
            for name,pattern in (MAPS[0],MAPS[4],MAPS[5]):
                for context in contexts:
                    c=deepcopy(template);c['request']['id']='mapped-fold-'+c['request']['id'].replace(':rate:',':'+function+':')+'-'+name+'-'+context
                    expression=pattern.format(x=child.replace('rate(',function+'(',1))
                    c['request']['query']=query[:start]+expression+')' if context=='aggregate' else expression
                    c['target']=False;c['timed']=False;result.append(c)
    for template in direct_cases():
        if not template['target']:continue
        c=deepcopy(template);c['target']=False;c['timed']='-sum-' in c['request']['id'] and c['request']['mode']=='range';result.append(c)
    byid={c['request']['id']:c for c in result}
    for function in ('rate','increase','delta','irate','idelta'):
        for mapping in ('div-right','bool-right'):
            for delayed in (0,1):
                ident=f'{"mapped-rate" if function in ("rate","increase") else "rate-family"}-n64-{function}-{mapping}-range-d{delayed}'
                byid[ident]['timed']=True
    historical={c['request']['id']:c for c in fold_group_label_cases()}
    for ident in REJECTED_CONTROLS:
        if ident not in byid:
            c=deepcopy(historical[ident]);result.append(c);byid[ident]=c
        c=byid[ident];assert c['request']==historical[ident]['request'] and c['dataset']==historical[ident]['dataset']
        c['target']=False;c['timed']=True;c['retained_rejected_control']=True
    assert len(result)==len({c['request']['id'] for c in result})
    return result
