"""Fixed scaling inputs for completed pair-metadata traversal."""
from group_contributor_cases import _records
from mapped_rate_cases import _case


def performance_cases():
    inputs = []
    sizes = [(count, 1) for count in (4, 16, 64, 256, 1024)] + [(32, 4)]
    for count, members in sizes:
        records = _records(groups=count, members=members)
        left = 'sum by(cluster,zone)(m_total)'
        right = left.replace('m_total', 'n_total')
        join = left + ' / on(cluster,zone) ' + right
        rate_join = join.replace('m_total)', 'rate(m_total[3s]))').replace(
            'n_total)', 'rate(n_total[3s]))')
        shapes = [
            ('raw-root', join),
            ('raw-abs', f'abs({join})'),
            ('raw-filter', f'({join}) > .25'),
            ('rate-abs', f'abs({rate_join})'),
            ('leaf-abs', 'abs(m_total / on(cluster,zone,member) n_total)'),
        ]
        for shape, expression in shapes:
            ident = f'pair-frontier-{shape}-g{count}-m{members}'
            inputs.append(_case(ident, expression, records, end=3000))
    return inputs
