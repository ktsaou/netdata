"""Fixed rate-acquisition scaling, fallback and empty-input controls."""
import re

from fold_join_cases import cases as window_cases
from info_cases import series
from mapped_rate_cases import _case
from pair_frontier_cases import performance_cases as pair_cases
from wrapped_group_cases import cases as wrapped_cases


def performance_cases():
    scaling = pair_cases()
    wrapped = wrapped_cases()
    window = window_cases()
    selected = [case for case in scaling if "-rate-abs-" in case["request"]["id"]]
    index = {case["request"]["id"]: case for case in scaling}
    for ident in [
        "pair-frontier-raw-root-g4-m1",
        "pair-frontier-raw-root-g64-m1",
        "pair-frontier-raw-root-g1024-m1",
        "pair-frontier-raw-abs-g1024-m1",
        "pair-frontier-leaf-abs-g1024-m1",
    ]:
        selected.append(index[ident])
    for needle in [
        "-histogram-server-default-0",
        "-float_histogram-server-default-0",
        "-sum-mixed-left-abs",
        "-sum-mixed-right-abs",
        "-sum-native-rate-abs",
        "-sum-all-native-rate-abs",
        "-sum-stale-rate-abs",
        "-sparse-server-default-0",
        "-instant-server-default-0",
    ]:
        selected.append(next(case for case in wrapped if needle in case["request"]["id"]))
    for name in ["increase", "delta", "avg_over_time"]:
        selected.append(next(case for case in window
                             if re.search(r"\b" + name + r"\s*\(", case["request"]["query"])))
    for name in ["increase", "delta"]:
        for count in [256, 1024]:
            original = index[f"pair-frontier-rate-abs-g{count}-m1"]
            request = {**original["request"],
                       "id": f"rate-acquisition-{name}-g{count}",
                       "query": original["request"]["query"].replace("rate(", name + "(")}
            selected.append({**original, "request": request})
    return selected


def empty_cases():
    output = []
    for state in ["empty", "float", "histogram", "float_histogram"]:
        points = [] if state == "empty" else [
            (time, "7ff0000000000002", state) for time in [1000, 2000]
        ]
        records = [series({"__name__": "m_total", "job": "worker"}, points)]
        for name in ["rate", "increase", "delta"]:
            for absent in [False, True]:
                expression = f"{name}(m_total[3s])"
                if absent:
                    expression = f"absent({expression})"
                for instant in [False, True]:
                    ident = f"rate-acquisition-{state}-{name}-{int(absent)}-{int(instant)}"
                    output.append(_case(ident, expression, records, instant=instant, end=3000))
    return output


def no_reset_cases():
    states = {
        'monotone': [1, 2, 3, 4], 'constant': [7, 7, 7, 7], 'negative': [-4, -3, -2, -1],
        'interior-reset': [100, 0, 150, 160], 'nan-bridge': [100, '7ff8000000000042', 0, 10],
        'stale-bridge': [100, '7ff0000000000002', 0, 10], 'zeros': [-0.0, 0.0, -0.0, 0.0],
        'infinity': ['7ff0000000000000'] * 4, 'start-reset': [1, 2, 3, 4],
        'overlap': [1, 2, 3, 4], 'native': [1, 2, 3, 4],
        'stale-histogram': [1, 2, 3, 4], 'mixed': [1, 2, 3, 4], 'empty': [], 'singleton': [1],
    }
    functional = []
    for state, values in states.items():
        points = [((index + 1) * 1000, value,
            'float_histogram' if state == 'native' or state in ('mixed', 'stale-histogram') and index == 1 else 'float')
            for index, value in enumerate(values)]
        record = series({'__name__': 'm_total', 'id': '0', 'job': 'worker'}, points)
        if state in ('start-reset', 'overlap'):
            for point, start in zip(record['points'], [500, 1500, 1500, 1500] if state == 'start-reset' else [500, 750, 750, 750], strict=True):
                point['start_time_ms'] = start
        if state == 'stale-histogram':
            record['points'][1]['float_histogram']['sum'] = '7ff0000000000002'
        for function in ('rate', 'increase', 'delta'):
            for profile in ('server-default', 'upstream-tests'):
                for delayed in (False, True):
                    for mode in ('instant', 'range'):
                        request = {'id': f'no-reset:{state}:{function}:{profile}:d{int(delayed)}:{mode}',
                            'query': function + '(m_total[3s])', 'profile': profile,
                            'delayed_name_removal': delayed, 'mode': mode, 'time_ms': 3000,
                            'start_ms': 3000, 'end_ms': 5000, 'step_ms': 1000,
                            'lookback_ms': 300000, 'subquery_step_ms': 1000}
                        functional.append({'state': state, 'function': function,
                            'dataset': {'schema_version': 1, 'series': [record]}, 'request': request})
    assert len(functional) == 360
    return functional


def no_reset_performance_cases():
    timing = []
    for count, points in ((1, 64), (8, 128), (16, 256), (64, 64)):
        for family in ('rate', 'increase', 'sum-rate', 'reset-rate', 'delta'):
            records = []
            for index in range(count):
                values = [0 if family == 'reset-rate' and tick == points - 1 else tick + 1 for tick in range(points + 2)]
                records.append(series({'__name__': 'm_total', 'id': str(index), 'job': 'worker'},
                    [((tick + 1) * 1000, value, 'float') for tick, value in enumerate(values)]))
            function = 'increase' if family == 'increase' else 'delta' if family == 'delta' else 'rate'
            expression = f'{function}(m_total[{points}s])'
            if family == 'sum-rate':
                expression = 'sum by(job)(' + expression + ')'
            for delayed in (False, True):
                for mode in ('instant', 'range'):
                    request = {'id': f'no-reset-timing:{family}:n{count}:w{points}:d{int(delayed)}:{mode}',
                        'query': expression, 'profile': 'server-default', 'delayed_name_removal': delayed,
                        'mode': mode, 'time_ms': points * 1000, 'start_ms': points * 1000,
                        'end_ms': (points + 2) * 1000, 'step_ms': 1000,
                        'lookback_ms': 300000, 'subquery_step_ms': 1000}
                    timing.append({'family': family, 'count': count, 'points': points,
                        'dataset': {'schema_version': 1, 'series': records}, 'request': request})
    return timing
