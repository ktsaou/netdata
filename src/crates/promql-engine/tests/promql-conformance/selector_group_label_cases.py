"""Direct selector grouping beside mapped, rate and full-key controls."""
from prepared_root_index_cases import prepared_root_index_cases
from info_cases import series


def selector_group_label_cases():
    result = prepared_root_index_cases()
    for c in result:
        c['keys'] = ['job'] if c['operator'] else None
        c['target'] = c['timed'] and c['operator'] is not None and c['child'] == 'raw'
    for count in (32, 128):
        for width in (0, 8, 32):
            for shape in ('global', 'byjob', 'all', 'withoutid'):
                labels = [{'__name__': 'm_total', 'id': f'{i:06}', 'job': f'g{i//2:06}',
                           **{f'payload{k:02}': 'v' * 32 for k in range(width)}} for i in range(count)]
                keys = ([] if shape == 'global' else ['job'] if shape == 'byjob' else
                        list(labels[0]) if shape == 'all' else [k for k in labels[0] if k not in ('id', '__name__')])
                group = ('' if shape == 'global' else ' without(id)' if shape == 'withoutid' else
                         ' by(' + ','.join(keys) + ')')
                data = {'schema_version': 1, 'series': [series(x, [(t*1000, i+t+1, 'float') for t in (1,2,3,4)])
                                                      for i, x in enumerate(labels)]}
                for mode in ('instant', 'range'):
                    for delayed in (False, True):
                        result.append({'request': {'id': f'labels-n{count}-w{width}-{shape}-{mode}-d{int(delayed)}',
                            'query': 'sum' + group + '(m_total)', 'profile': 'server-default', 'mode': mode,
                            'time_ms': 4000, 'start_ms': 2000, 'end_ms': 4000, 'step_ms': 1000,
                            'lookback_ms': 1000, 'delayed_name_removal': delayed},
                            'dataset': data, 'faults': {}, 'count': count, 'child': 'raw', 'operator': 'sum',
                            'keys': keys, 'timed': True, 'target': shape in ('global', 'byjob')})
    assert len(result) == 328 and sum(c['timed'] for c in result) == 280
    return result
