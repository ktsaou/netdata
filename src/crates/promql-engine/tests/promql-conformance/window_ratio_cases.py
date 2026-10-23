"""Small finite resource discriminator for completed direct Sum/Count reuse."""
from copy import deepcopy
from info_cases import series
from single_reset_cases import _records, _request
from native_endpoint_cases import _records as native_records


def window_ratio_cases():
    cases = []
    for count, width, label_width in [(4, 4, 0), (16, 256, 0), (256, 4, 0), (16, 4, 1024)]:
        data = _records(count, width)
        if label_width:
            data = {'schema_version': 1, 'series': [series(
                {'__name__': 'm_total', 'id': str(i), 'job': 'worker', 'zone': 'x' * label_width},
                [(tick * 1000, tick * (i + 1), 'float') for tick in range(1, width + 1)]) for i in range(count)]}
        left, right = f'sum_over_time(m_total[{width}s])', f'count_over_time(m_total[{width}s])'
        for matching in ('', 'ignoring(window)', 'on(id)'):
            cases.append({'shape': 'ratio', 'dataset': data,
                'request': _request(f'ratio-{count}-{width}-{label_width}-{matching}', f'{left} / {matching} {right}', width, 'instant')})
        for label, query, mode in [('plain-sum', left, 'instant'), ('plain-count', right, 'instant'),
            ('paren', f'({left}) / {right}', 'instant'), ('range', f'{left} / {right}', 'range'),
            ('reverse', f'{right} / {left}', 'instant')]:
            cases.append({'shape': 'control', 'dataset': data,
                'request': _request(f'ratio-control-{label}-{count}-{width}-{label_width}', query, width, mode)})
    for pattern in ('native', 'integer', 'mixed', 'custom', 'varying', 'stale'):
        for buckets in (8, 256):
            data = native_records(4, buckets, pattern)
            cases.append({'shape': 'native', 'dataset': data,
                'request': _request(f'ratio-native-{pattern}-{buckets}', 'sum_over_time(m_total[4s]) / ignoring(window) count_over_time(m_total[4s])', 4, 'instant')})
    for pattern in ('sparse', 'empty', 'missing', 'collision'):
        left = _records(4, 4)
        right = deepcopy(left)
        for record in right['series']:
            # Series fixture labels use base64 transport; change the metric via the established builder.
            import base64
            for label in record['labels']:
                if base64.b64decode(label['name']) == b'__name__':
                    label['value'] = base64.b64encode(b'n_total').decode()
        if pattern == 'sparse':
            left['series'][0]['points'] = []; right['series'][2]['points'] = []
        elif pattern == 'empty':
            left['series'] = []
        elif pattern == 'missing':
            right['series'] = right['series'][:2]
        data = {'schema_version': 1, 'series': left['series'] + right['series']}
        matching = 'on(job)' if pattern == 'collision' else 'on(id)'
        cases.append({'shape': 'edge', 'dataset': data,
            'request': _request(f'ratio-edge-{pattern}', f'sum_over_time(m_total[4s]) / {matching} count_over_time(n_total[4s])', 4, 'instant')})
    for width in (4, 32):
        data = {'schema_version': 1, 'series': [series(
            {'__name__': metric, 'id': str(i), 'z': str(i), 'job': 'worker'},
            [(tick * 1000, tick * (i + 1), 'float') for tick in range(1, width + 1)])
            for metric in ('m_total', 'n_total') for i in range(16)]}
        for metric in ('m_total', 'n_total'):
            cases.append({'shape': 'provider-fault', 'dataset': data,
                'faults': {metric + '/15': {'iterator_error': 'finite ratio read failure', 'fail_at': width * 1000}},
                'request': _request(f'ratio-fault-{metric}-{width}',
                    f'sum_over_time(m_total[{width}s]) / on(id) count_over_time(n_total[{width}s])', width, 'instant')})
    assert len({row['request']['id'] for row in cases}) == len(cases)
    return cases


def window_ratio_fault_scale_cases():
    cases = []
    for count in (4, 64, 256):
        data = {'schema_version': 1, 'series': [series(
            {'__name__': metric, 'id': str(i), 'z': str(i), 'job': 'worker'},
            [(tick * 1000, tick * (i + 1), 'float') for tick in range(1, 5)])
            for metric in ('m_total', 'n_total') for i in range(count)]}
        cases.append({'dataset': data,
            'faults': {'n_total/' + str(count - 1): {'iterator_error': 'finite ratio read failure', 'fail_at': 4000}},
            'request': _request(f'ratio-fault-scale-{count}',
                'sum_over_time(m_total[4s]) / on(id) count_over_time(n_total[4s])', 4, 'instant')})
    return cases
