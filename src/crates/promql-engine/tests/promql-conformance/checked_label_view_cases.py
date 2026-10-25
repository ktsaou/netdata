"""Output-label widths beside previously qualified aggregate controls."""
from rate_group_label_cases import rate_group_label_cases
from info_cases import series

def cases():
    result=rate_group_label_cases()
    for c in result:c['target']=False
    for width in (2,4,5,8,9,16,17,33):
        for count in (1,32):
            data={'schema_version':1,'series':[series({'__name__':'m_total','id':f'{i:06}',**{f'key{k:02}':'v'*32 for k in range(width-2)}},[(t*1000,i+t+1,'float') for t in (1,2,3,4)]) for i in range(count)]}
            for child in ('raw','abs'):
                for mode in ('instant','range'):
                    for delayed in (False,True):
                        output_width=width-(child=='abs')
                        result.append({'request':{'id':f'views-w{width}-n{count}-{child}-{mode}-d{int(delayed)}',
                            'query':'m_total' if child=='raw' else 'abs(m_total)','profile':'server-default','mode':mode,
                            'time_ms':4000,'start_ms':2000,'end_ms':4000,'step_ms':1000,'lookback_ms':1000,'delayed_name_removal':delayed},
                            'dataset':data,'faults':{},'count':count,'child':child,'operator':None,'keys':None,'timed':count==32,
                            'target':count==32 and (output_width<4 or output_width&(output_width-1)!=0)})
    assert len({c['request']['id'] for c in result})==len(result)
    return result
