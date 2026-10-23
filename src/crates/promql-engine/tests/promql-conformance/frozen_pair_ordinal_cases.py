# SPDX-License-Identifier: GPL-3.0-or-later
"""Fixed complete-query targets and controls for certified pair label lookup."""
from copy import deepcopy
from info_cases import series


def frozen_pair_ordinal_cases():
    output = []
    for count in (8, 32):
        for width in (16, 256, 1024):
            records = []
            for metric in ('m', 'n'):
                for identity in range(count):
                    scale = (2 if identity % 2 == 0 else 8) if metric == 'm' else 4
                    records.append(series(
                        {'__name__': metric, 'detail': 'x' * width,
                         'id': f'{identity:04}', 'job': 'worker'},
                        [(tick * 1000, scale * tick, 'float') for tick in range(1, 9)]))
            for expression in ('sum by(job)(m+n)', 'm+n'):
                for delayed in (False, True):
                    target = expression.startswith('sum') and not delayed and width >= 256
                    output.append({
                        'request': {'id': f'n{count}-w{width}-{expression}-{int(delayed)}',
                                    'query': expression, 'profile': 'server-default',
                                    'mode': 'range', 'start_ms': 1000, 'end_ms': 8000,
                                    'step_ms': 1000, 'delayed_name_removal': delayed},
                        'dataset': {'schema_version': 1, 'series': deepcopy(records)},
                        'faults': {}, 'shape': 'target' if target else 'control',
                        'count': count, 'width': width})
    return output
