# SPDX-License-Identifier: GPL-3.0-or-later
"""Fixed projection identity targets and ordinary comparison controls."""
from copy import deepcopy
from info_cases import series


def ordered_label_identity_cases():
    output = []
    expressions = [
        ('binary', 'm+n', False),
        ('projected-binary', 'label_join(m+n,"row","/","id","job")', True),
        ('aggregate', 'sum by(id,detail)(m)', False),
        ('projected-aggregate', 'label_join(sum by(id,detail)(m),"row","/","id")', True),
    ]
    for count in (8, 32):
        for width in (16, 4096):
            records = []
            for metric in ('m', 'n'):
                for identity in range(count):
                    scale = 4 if metric == 'n' else (2 if identity % 2 == 0 else 8)
                    records.append(series(
                        {'__name__': metric, 'detail': 'x' * width,
                         'id': f'{identity:06}', 'job': 'worker'},
                        [(tick * 1000, scale * tick, 'float') for tick in range(1, 4)]))
            for shape, expression, projected in expressions:
                for mode in ('instant', 'range'):
                    for delayed in (False, True):
                        output.append({
                            'request': {
                                'id': f'n{count}-w{width}-{shape}-{mode}-d{int(delayed)}',
                                'query': expression, 'profile': 'server-default', 'mode': mode,
                                'time_ms': 3000, 'start_ms': 1000, 'end_ms': 3000,
                                'step_ms': 1000, 'lookback_ms': 1000,
                                'delayed_name_removal': delayed},
                            'dataset': {'schema_version': 1, 'series': deepcopy(records)},
                            'faults': {}, 'count': count, 'width': width,
                            'shape': 'target' if projected and width == 4096 else 'control'})
    return output
