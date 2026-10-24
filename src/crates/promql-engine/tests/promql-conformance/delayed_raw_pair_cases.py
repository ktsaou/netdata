# SPDX-License-Identifier: GPL-3.0-or-later
"""Delayed raw arithmetic joins under ordinary metadata-removing groups."""
from delayed_ratio_cases import case
from info_cases import series


def delayed_raw_pair_cases():
    output=[]
    dense=[series({'__name__':metric,'id':str(i),'job':'worker'},
                  [(tick*1000,(i+1)*(2 if metric=='m' else 4)*tick,'float')
                   for tick in range(1,4)])
           for metric in ('m','n') for i in range(3)]
    for reducer in ('sum','avg','count','min','max','group','stdvar','stddev'):
        for operator in ('+','-','*','/','%','^','atan2'):
            for delayed in (False,True):
                output.append(case(f'delayed-raw-operator:{reducer}:{operator}:{delayed}',
                    f'{reducer} by(job)(m {operator} n)',dense,delayed,'range',3000,1))
    for key in ('__name__','__type__','__unit__'):
        for separate in (False,True):
            records=[]
            for side in range(2):
                for identity in range(2):
                    labels={'__name__':'m','id':str(identity),'job':'worker'}
                    labels[key]=('m','n')[side] if key=='__name__' else ('a','b')[side]
                    points=[(tick*1000,'7ff0000000000002' if separate and ((tick==2)!=(side==1))
                             else (identity+1)*(side+1)*tick,'float') for tick in range(1,4)]
                    records.append(series(labels,points))
            selector='{__name__=~"m|n"}' if key=='__name__' else 'm'
            names=','.join(sorted({'__name__','id','job',key}))
            body=f'{selector} + on({names}) {selector}'
            for grouping in ('by(job)',f'by(job,{key})',
                             'without(__name__,__type__,__unit__,id)','without(id)'):
                for delayed in (False,True):
                    for mode in ('instant','range'):
                        output.append(case(f'delayed-raw-alias:{key}:{separate}:{grouping}:{delayed}:{mode}',
                            f'sum {grouping}({body})',records,delayed,mode,3000,1))
    for value in output:
        value['request']['start_ms']=1000
    assert len(output)==208
    return output
