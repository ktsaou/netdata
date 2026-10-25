"""Complete mapped reducer labels and retained consumer/metadata boundary controls."""
from copy import deepcopy
import base64
from mapped_fold_label_cases import cases as previous
from info_cases import series

FUNCTIONS = ('min_over_time', 'max_over_time', 'count_over_time', 'present_over_time',
             'stdvar_over_time', 'stddev_over_time')

def cases():
    seed=previous();result=deepcopy(seed)
    for c in result:
        ident=c['request']['id'];c['target']=False
        c['timed']=bool(c.get('retained_rejected_control')) or (
            ident.startswith(('mapped-rate-n64-','rate-family-n64-')) and
            any('-'+m+'-range-' in ident for m in ('div-right','bool-right'))) or (
            ident.startswith('mapped-fold-n64-') and
            any('-'+m+'-range-' in ident for m in ('div-right','bool-right')))
    for template in seed:
        ident=template['request']['id']
        if not ident.startswith('mapped-fold-') or 'sum_over_time(' not in template['request']['query']:continue
        for function in FUNCTIONS:
            c=deepcopy(template);c['request']['id']=ident.replace('mapped-fold-','mapped-reducer-',1).replace('sum_over_time',function)
            c['request']['query']=c['request']['query'].replace('sum_over_time(',function+'(')
            selected=ident.startswith('mapped-fold-n64-') and any('-'+m+'-range-' in ident for m in ('div-right','bool-right'))
            c['target']=selected and (ident.endswith('-aggregate') or '-d0-root' in ident)
            c['timed']=selected or (ident.startswith('mapped-fold-n4-') and '-div-right-instant-d0-root' in ident)
            result.append(c)
    # Include the previously failed metadata-free boundary before any candidate is built.
    for template in list(result):
        ident=template['request']['id']
        if not ident.startswith('mapped-reducer-n64-') or '-div-right-range-' not in ident:continue
        for state in ('absent','leading-custom','type-only'):
            c=deepcopy(template);c['request']['id']='metadata-'+state+'-'+ident
            c['request']['query']=c['request']['query'].replace('m[','{job=~".+"}[')
            for row in c['dataset']['series']:
                if state!='leading-custom':
                    removed=(b'__name__',b'__unit__') if state=='type-only' else (b'__name__',b'__type__',b'__unit__')
                    row['labels']=[label for label in row['labels'] if base64.b64decode(label['name']) not in removed]
                else:row['labels'].insert(0,{'name':base64.b64encode(b'A').decode(),'value':base64.b64encode(b'first').decode()})
            c['target']=False;c['timed']=False;result.append(c)
        assert template['request']['query'].count('div-right')==0
        for name,expression in (('clamp','clamp({x},0,(time()-3)^2-.5)'),
                                ('clamp-nan','clamp({x},NaN,1)'),('comparison','{x}>time()*16-32')):
            c=deepcopy(template);c['request']['id']='filter-'+name+'-'+ident
            child=next(f for f in FUNCTIONS if f+'(' in c['request']['query'])+'(m[3s])'
            c['request']['query']=c['request']['query'].replace(child+'/3',expression.format(x=child))
            assert c['request']['query']!=template['request']['query']
            c['target']=False;c['timed']=False;result.append(c)
    data={'schema_version':1,'series':[series({'__name__':'m','job':'j','id':str(i)},[(t*1000,float(i+t),'float') for t in (1,2,3,4)]) for i in range(2)]}
    for function in FUNCTIONS+('absent_over_time',):
        for state,values in [('zero',['8000000000000000','0000000000000000','8000000000000000','0000000000000000']),('stale',['7ff0000000000002']*4),('empty',[])]:
            dataset=deepcopy(data)
            for row in dataset['series']:row['points']=series({},[(t*1000,v,'float') for t,v in enumerate(values,1)])['points']
            for mode in ('instant','range'):
                for delayed in (False,True):
                    for aggregate in (False,True):
                        expression=f'{function}(m{{job="j"}}[3s])/3'
                        result.append({'request':{'id':f'mapped-reducer-extra-{function}-{state}-{mode}-d{int(delayed)}-g{int(aggregate)}','query':f'sum by(job)({expression})' if aggregate else expression,'profile':'server-default','mode':mode,'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,'subquery_step_ms':1000,'delayed_name_removal':delayed},'dataset':dataset,'faults':{},'target':False,'timed':False})
    assert len(result)==len({c['request']['id'] for c in result})
    assert sum(c['target'] for c in result)==36
    return result
