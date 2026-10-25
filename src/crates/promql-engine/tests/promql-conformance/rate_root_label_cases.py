"""Top-level Rate maps beside existing aggregate, Fold and child-consumer controls."""
import base64
from copy import deepcopy
from mapped_fold_label_cases import cases as previous

FUNCTIONS=('rate','increase','delta','irate','idelta')

def cases():
    seed=previous();result=deepcopy(seed)
    for c in result:
        ident=c['request']['id']
        c['target']=False
        c['timed']=bool(c.get('retained_rejected_control')) or (
            ident.startswith(('mapped-rate-n64-','rate-family-n64-','mapped-fold-n64-'))
            and any('-'+m+'-' in ident for m in ('div-right','bool-right'))
            and c['request']['mode']=='range')
    prefix='sum by(job)('
    for template in seed:
        ident=template['request']['id']
        if not ident.startswith(('mapped-rate-','rate-family-')):continue
        query=template['request']['query']
        if not query.startswith(prefix):continue
        c=deepcopy(template);c['request']['id']='root-'+ident
        c['request']['query']=query[len(prefix):-1]
        c['target']=False;c['timed']=False;c['operator']=None;c['child']='rate-root'
        large=ident.startswith(('mapped-rate-n64-','rate-family-n64-'))
        small=ident.startswith(('mapped-rate-n4-','rate-family-n4-'))
        common=any('-'+m+'-' in ident for m in ('div-right','bool-right','abs'))
        c['timed']=large and common or small and '-div-right-' in ident and c['request']['mode']=='range'
        c['target']=large and common and c['request']['delayed_name_removal']
        result.append(c)
        if (large or small) and any('-'+m+'-' in ident for m in ('div-right','gt-right')):
            wrapped=deepcopy(c);wrapped['request']['id']='parenthesized-'+ident
            wrapped['request']['query']='('+c['request']['query']+')'
            wrapped['target']=False
            wrapped['timed']=large and '-div-right-' in ident and c['request']['mode']=='range'
            result.append(wrapped)
        if small and '-gt-right-' in ident:
            computed=deepcopy(c);computed['request']['id']='computed-'+ident
            assert computed['request']['query'].endswith('>0')
            computed['request']['query']=computed['request']['query'][:-2]+'>scalar(vector(0))'
            computed['target']=False;computed['timed']=False;result.append(computed)
    assert len(result)==len({c['request']['id'] for c in result})
    assert sum(bool(c.get('retained_rejected_control')) for c in result)==5
    return result


def filter_cases(seed=None):
    seed=cases() if seed is None else seed
    result=[]
    for c in seed:
        ident=c['request']['id']
        if not ident.startswith(('root-mapped-rate-n64-','root-rate-family-n64-')):continue
        if '-abs-range-' not in ident:continue
        query=c['request']['query'];assert query.startswith('abs(') and query.endswith(')')
        child=query[4:-1]
        for name,expression in [('clamp',f'clamp({child},0,(time()-3)^2-.5)'),
                                ('clamp-nan',f'clamp({child},NaN,1)'),
                                ('comparison',f'{child}>time()*16-32')]:
            new=deepcopy(c);new['request']['id']='filter-'+name+'-'+ident
            new['request']['query']=expression;new['target']=False;new['timed']=False;result.append(new)
    return result


def metadata_cases(seed=None):
    seed=cases() if seed is None else seed
    result=[]
    for template in seed:
        ident=template['request']['id']
        if not ident.startswith(('root-mapped-rate-n64-','root-rate-family-n64-')):continue
        if not template['target']:continue
        for state in ('absent','leading-custom','type-only'):
            c=deepcopy(template);c['request']['id']='metadata-'+state+'-'+ident
            c['request']['query']=c['request']['query'].replace('m_total[','{job=~".+"}[').replace('m[','{job=~".+"}[')
            for row in c['dataset']['series']:
                if state!='leading-custom':
                    row['labels']=[label for label in row['labels'] if base64.b64decode(label['name']) not in
                        ((b'__name__',b'__unit__') if state=='type-only' else (b'__name__',b'__type__',b'__unit__'))]
                else:
                    row['labels'].insert(0,{'name':base64.b64encode(b'A').decode(),'value':base64.b64encode(b'first').decode()})
            result.append(c)
    return result
