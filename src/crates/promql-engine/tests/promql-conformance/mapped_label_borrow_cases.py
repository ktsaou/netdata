"""Grouped abs with complete member identity, beside accepted runtime controls."""
from checked_label_view_cases import cases as previous
from info_cases import series

def cases():
    result=previous()
    for c in result:c['target']=False
    for groups in (4,16,64):
        for width in (0,8,32):
            data={'schema_version':1,'series':[series({'__name__':'m_total','id':f'{i:06}','job':f'g{i//2:06}',**{f'payload{k:02}':'v'*32 for k in range(width)}},[(t*1000,i+t+1,'float') for t in (1,2,3,4)]) for i in range(groups*2)]}
            for operator in ('sum','max'):
                for mode in ('instant','range'):
                    for delayed in (False,True):
                        result.append({'request':{'id':f'mapped-g{groups}-w{width}-{operator}-{mode}-d{int(delayed)}','query':f'{operator} by(job)(abs(m_total))','profile':'server-default','mode':mode,'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,'delayed_name_removal':delayed},'dataset':data,'faults':{},'count':groups*2,'groups':groups,'width':width,'child':'abs','operator':operator,'keys':['job'],'target':mode=='range','timed':(groups>=16 and width>=8) or (groups==4 and width==0)})
    assert len(result)==624 and len({c['request']['id'] for c in result})==len(result)
    return result
