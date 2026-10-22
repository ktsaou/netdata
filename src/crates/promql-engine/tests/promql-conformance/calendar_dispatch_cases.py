# SPDX-License-Identifier: GPL-3.0-or-later
"""Bounded complete calendar queries and date-dependent dispatch controls."""
from copy import deepcopy
from info_cases import series

FUNCTIONS = ('hour', 'minute', 'day_of_week', 'days_in_month', 'day_of_month',
             'day_of_year', 'month', 'year')
TARGETS = FUNCTIONS[:3]


def calendar_performance_cases():
    epoch = 1704110400000
    cases = []

    def add(ident, expression, count, steps, shape):
        records = [series({'__name__': 'm', 'id': str(i)},
                          [(epoch + tick * 1000, epoch / 1000 + tick + i, 'float')
                           for tick in range(steps)]) for i in range(count)]
        cases.append({'shape': shape, 'dataset': {'schema_version': 1, 'series': records},
                      'request': {'id': ident, 'query': expression, 'profile': 'server-default',
                                  'mode': 'range', 'time_ms': 0, 'start_ms': epoch,
                                  'end_ms': epoch + (steps - 1) * 1000, 'step_ms': 1000,
                                  'lookback_ms': 300000, 'subquery_step_ms': 60000}})

    for function in FUNCTIONS:
        shape = function if function in TARGETS else 'control'
        for steps in (1, 1024):
            add(f'{function}-clock-{steps}', f'{function}()', 0, steps, shape)
        for steps in (1, 64):
            add(f'{function}-values-{steps}', f'{function}(m)', 32, steps, shape)
    for function in TARGETS:
        add(function + '-timestamps', f'{function}(timestamp(m))', 32, 64, function)
    for steps in (1, 64):
        add(f'abs-control-{steps}', 'abs(m)', 32, steps, 'control')
    add('hour-business', 'hour() >= 9 and hour() < 17', 0, 1024, 'hour')
    add('weekday-business', 'day_of_week() >= 1 and day_of_week() <= 5', 0, 1024, 'day_of_week')
    assert len(cases) == 39
    return cases


def calendar_cases():
    primary = calendar_performance_cases()
    other = deepcopy(primary)
    for case in other:
        case['request']['id'] = 'opposite-' + case['request']['id']
        case['request']['profile'] = 'upstream-tests'
    return primary + other
