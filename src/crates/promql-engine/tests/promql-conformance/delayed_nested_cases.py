# SPDX-License-Identifier: GPL-3.0-or-later
"""Two-level delayed group completion and unchanged composition boundaries."""
from base64 import b64decode, b64encode
from copy import deepcopy
from delayed_ratio_cases import case
from group_contributor_cases import OPERATORS, _records
from info_cases import series


def nested(outer='sum', inner='max', leaf='rate(m_total[3s])',
           outer_group='by(cluster)', inner_group='by(cluster,zone)'):
    return f'{outer} {outer_group}({inner} {inner_group}({leaf}))'


def performance_records(groups):
    return [series({'__name__': 'm_total', 'job': f'{job:06}',
                    'instance': str(instance), 'replica': str(replica)},
                   [(tick * 1000, (job + 1) * (instance * 2 + replica + 1)
                     * (100 + tick * tick), 'float') for tick in range(1, 9)])
            for job in range(groups) for instance in range(2) for replica in range(2)]


def delayed_nested_performance_cases():
    output = []
    for groups in (4, 8):
        records = performance_records(groups)
        for outer in ('sum', 'avg'):
            for family, leaf in (('selector', 'm_total'), ('rate', 'rate(m_total[3s])')):
                child = nested(outer, 'max', leaf, 'by(job)', 'by(job,instance)')
                for mode in ('instant', 'range'):
                    for delayed in (False, True):
                        output.append(case(
                            f'delayed-nested-perf:{groups}:{outer}:{family}:{mode}:{delayed}',
                            f'max_over_time(({child})[4s:1s])', records, delayed, mode, 8000,
                            groups, 'target-' + family if delayed else 'control'))
    records = performance_records(4)
    child = nested(outer_group='by(job)', inner_group='by(job,instance)')
    fallbacks = (
        f'abs({child})', f'({child})*2', f'sum by(job)({child})',
        f'label_join({child},"site","/","job")',
        nested(outer_group='by(job,__type__)', inner_group='by(job,instance)'),
        nested(outer_group='by(job)', inner_group='by(job,instance,__unit__)'),
        nested(leaf='rate(m_total[3s] @ 6)', outer_group='by(job)', inner_group='by(job,instance)'),
        f'{child}/sum by(job)(rate(m_total[3s]))',
    )
    for index, body in enumerate(fallbacks):
        output.append(case(f'delayed-nested-perf:fallback:{index}',
                           f'max_over_time(({body})[4s:1s])', records,
                           time=8000, groups=4, shape='fallback'))
    return output


def delayed_nested_cases():
    output = []
    dense = _records(groups=3, members=3)
    for index, outer in enumerate(OPERATORS):
        for inner in OPERATORS:
            for delayed in (False, True):
                leaf = 'm_total' if index % 2 else 'rate(m_total[3s])'
                body = nested(outer, inner, leaf)
                output.append(case(f'delayed-nested-operators:{outer}:{inner}:{delayed}',
                                   f'max_over_time(({body})[3s:1s])', dense, delayed))
    for leaf in ('m_total', 'rate(m_total[3s])', 'increase(m_total[3s])', 'delta(m_total[3s])'):
        for outer, inner in (('sum', 'max'), ('avg', 'sum')):
            for mode in ('instant', 'range'):
                for delayed in (False, True):
                    output.append(case(f'delayed-nested-core:{outer}:{inner}:{leaf}:{mode}:{delayed}',
                                       f'sum_over_time(({nested(outer, inner, leaf)})[3s:1s])',
                                       dense, delayed, mode))
    for state in ('stale', 'late', 'empty', 'native', 'mixed', 'all-native', 'nan', 'infinity', 'zero'):
        records = _records(state, groups=3, members=2)
        for leaf in ('m_total', 'rate(m_total[3s])'):
            for outer, inner in (('sum', 'max'), ('count', 'sum')):
                for mode in ('instant', 'range'):
                    output.append(case(f'delayed-nested-state:{state}:{leaf}:{outer}:{mode}',
                                       f'max_over_time(({nested(outer, inner, leaf)})[3s:1s])',
                                       records, mode=mode))
    metadata = deepcopy(dense)
    for record in metadata:
        labels = {b64decode(x['name']).decode(): b64decode(x['value']).decode()
                  for x in record['labels']}
        for key, value in (('__type__', 'counter' if labels['member'] == '0' else 'gauge'),
                           ('__unit__', 'bytes' if labels['member'] == '0' else 'seconds')):
            record['labels'].append({'name': b64encode(key.encode()).decode(),
                                     'value': b64encode(value.encode()).decode()})
    for level in ('inner', 'outer'):
        for grouping in ('by(cluster)', 'by(cluster,__name__)', 'by(cluster,__type__)',
                         'by(cluster,__unit__)', 'without(member,__type__,__unit__)', 'without(member)'):
            for leaf in ('{__name__=~"[mn]_total"}', 'rate({__name__=~"[mn]_total"}[3s])'):
                body = nested(leaf=leaf, **{level + '_group': grouping})
                for delayed in (False, True):
                    output.append(case(f'delayed-nested-metadata:{level}:{grouping}:{leaf}:{delayed}',
                                       f'max_over_time(({body})[3s:1s])', metadata, delayed))
    for consumer in ('min_over_time', 'max_over_time', 'sum_over_time', 'avg_over_time',
                     'count_over_time', 'present_over_time', 'last_over_time', 'stdvar_over_time',
                     'stddev_over_time', 'changes', 'resets', 'quantile_over_time'):
        prefix = '.5,' if consumer == 'quantile_over_time' else ''
        for delayed in (False, True):
            output.append(case(f'delayed-nested-consumer:{consumer}:{delayed}',
                               f'{consumer}({prefix}({nested()})[3s:1s])', dense, delayed))
    for index, body in enumerate((
        '((sum by(cluster)(((max by(cluster,zone)(m_total))))))',
        nested(leaf='m_total offset 1s'), nested(leaf='rate(m_total[3s] offset 1s)'),
        'sum by(cluster)(topk by(cluster,zone)(1,m_total))',
    )):
        for delayed in (False, True):
            output.append(case(f'delayed-nested-shape:{index}:{delayed}',
                               f'max_over_time(({body})[3s:1s])', dense, delayed))
    sparse = deepcopy(dense)
    for record in sparse:
        labels = {b64decode(x['name']): b64decode(x['value']) for x in record['labels']}
        if labels[b'zone'] == b'1':
            for point in record['points']:
                if point['time_ms'] >= 3000:
                    point['float'] = '7ff0000000000002'
    for records, state in ((list(reversed(dense)), 'reverse'), (sparse, 'disappearing')):
        for delayed in (False, True):
            for profile in ('server-default', 'upstream-tests'):
                output.append(case(f'delayed-nested-membership:{state}:{delayed}:{profile}',
                                   f'max_over_time(({nested()})[3s:1s])', records, delayed,
                                   'range', profile=profile))
    output.extend(delayed_nested_performance_cases())
    assert len(output) == len({item['request']['id'] for item in output})
    return output
