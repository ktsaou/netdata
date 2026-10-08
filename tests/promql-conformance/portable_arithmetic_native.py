"""Exact native/semantic-width checks against independent Go observations."""
import hashlib
import json
import subprocess

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def histogram(state):
    return {name: state[name] for name in ('count', 'sum', 'schema', 'zero_count', 'zero_threshold',
                                          'positive_spans', 'negative_spans')} | {
        'reset_hint': state['hint'], 'positive_buckets': state['positive'],
        'negative_buckets': state['negative'] or None, 'custom_values': state['custom'] or None,
    }


def state(hist):
    if hist is None:
        return None
    return {name: hist[name] for name in ('count', 'sum', 'schema', 'zero_count', 'zero_threshold',
                                         'positive_spans', 'negative_spans')} | {
        'hint': hist['reset_hint'], 'positive': hist['positive_buckets'] or [],
        'negative': hist['negative_buckets'] or [], 'custom': hist['custom_values'] or [],
    }


def initial(value):
    return {'reset_hint': 0, 'schema': 0, 'zero_threshold': '3f50624dd2f1a9fc',
            'zero_count': value, 'count': value, 'sum': value,
            'positive_spans': [{'Offset': 0, 'Length': 2}],
            'negative_spans': [{'Offset': 0, 'Length': 1}],
            'positive_buckets': [value, value], 'negative_buckets': [value], 'custom_values': None}


def native_inputs(case):
    left, right = initial(case['A']), initial(case['B'])
    shape = case['Shape']
    if shape == 'new':
        left['positive_spans'][0]['Offset'] = 2
        left['negative_spans'][0]['Offset'] = 2
    elif shape == 'reduce':
        left['schema'] = 1
        left['positive_spans'][0]['Offset'] = 1
    elif shape == 'threshold':
        right['zero_threshold'] = '3ff199999999999a'
    elif shape in ('custom-match', 'custom-reconcile'):
        for value in (left, right):
            value.update(schema=-53, zero_threshold='0000000000000000', zero_count='0000000000000000',
                         negative_spans=None, negative_buckets=None,
                         positive_spans=[{'Offset': 0, 'Length': 3}], positive_buckets=[value['count']] * 3,
                         custom_values=['3fe0000000000000', '3ff0000000000000'])
        if shape == 'custom-reconcile':
            right['custom_values'] = ['3ff0000000000000', '4000000000000000']
    else:
        assert shape == 'matched'
    return left, right


def qualify_native(panels, kernel, task):
    all_rows = [(1, row) for row in panels['v1']] + [(2, row) for row in panels['v2']]
    expected, requests = {}, []
    for version, row in all_rows:
        assert 'panic' not in row and 'error' not in row
        case = row['input']
        identity = f"v{version}-{row['id']}"
        mode, request = None, None
        if 'Op' in case and 'Shape' in case:
            left, right = native_inputs(case)
            assert state(left) == row['before'] and state(right) == row['other']
            operation = {'operation': 'kahan' if case['Op'] == 'kahan-add-final' else case['Op'],
                         'left': left, 'right': right,
                         'compensation': None, 'compact': False}
            mode, request = 'float-arithmetic-set', {'operations': [operation]}
            expected[identity] = ('native', row)
        elif 'Op' in case and 'Scope' not in case and 'Mode' not in case:
            mode = 'float-copy-to-schema' if case['Op'] == 'copy' else 'float-reduce-resolution'
            before = initial('3ff0000000000000')
            before.update(zero_count='0000000000000000',
                          positive_spans=[{'Offset': 1, 'Length': 2}], negative_spans=[{'Offset': 1, 'Length': 2}],
                          positive_buckets=[case['A'], case['B']], negative_buckets=[case['B'], case['A']])
            assert state(before) == row['before']
            request = {'input': before, 'target_schema': -1}
            expected[identity] = ('reduction', row)
        elif 'Mode' in case and case['Mode'] in ('ldexp', 'compact', 'cumulative'):
            if case['Mode'] == 'ldexp':
                assert row['int_width'] == 64 and row['int_exponent'] == case['Exponent']
                mode, request = 'math-ldexp', {'input': case['A'], 'exponent': case['Exponent']}
                expected[identity] = ('ldexp', row)
            elif case['Mode'] == 'compact':
                before = initial('3ff0000000000000')
                before.update(zero_count='0000000000000000',
                              positive_spans=[{'Offset': 0, 'Length': 1}, {'Offset': 4, 'Length': 1}],
                              positive_buckets=['3ff0000000000000'] * 2, negative_spans=None, negative_buckets=None)
                assert state(before) == row['before']
                assert row['int_width'] == 64 and row['int_max_empty'] == case['Exponent']
                mode, request = 'float-compact', {'input': before, 'max_empty': case['Exponent']}
                expected[identity] = ('compact', row)
            else:
                left, right = initial('3ff0000000000000'), initial('3ff0000000000000')
                for value in (left, right):
                    value.update(schema=-53, zero_threshold='0000000000000000', zero_count='0000000000000000',
                                 count='4000000000000000', sum='4000000000000000',
                                 negative_spans=None, negative_buckets=None,
                                 positive_spans=[{'Offset': case['Offset'], 'Length': 1}] * 2,
                                 positive_buckets=['3ff0000000000000'] * 2)
                left['custom_values'] = ['3fe0000000000000', '3ff0000000000000']
                right['custom_values'] = ['3ff0000000000000', '4000000000000000']
                assert state(left) == row['before'] and state(right) == row['other']
                mode, request = 'float-arithmetic-set', {'operations': [{
                    'operation': case['Op'], 'left': left, 'right': right,
                    'compensation': None, 'compact': False,
                }]}
                expected[identity] = ('cumulative', row)
        elif 'Function' in case:
            continue
        elif 'Mode' in case and case['Mode'] == 'decimal':
            continue
        else:
            raise ValueError(case)
        if mode is not None:
            requests.append({'id': identity, 'mode': mode, 'request': request, 'borrowed': False})


    def execute(binary, requests, kind):
        before = sha(binary)
        raw = ''.join(json.dumps(row, separators=(',', ':')) + '\n' for row in requests).encode()
        (task / f'native-{kind}-requests.jsonl').write_bytes(raw)
        with (task / f'native-{kind}.jsonl').open('xb') as out, (task / f'native-{kind}.stderr').open('xb') as err:
            result = subprocess.run([str(binary)], input=raw, stdout=out, stderr=err, timeout=600, check=False)
        assert result.returncode == 0 and sha(binary) == before
        return [json.loads(line) for line in (task / f'native-{kind}.jsonl').read_bytes().splitlines()]


    failures = []


    def equal(identity, field, actual, reference):
        if actual != reference:
            failures.append({'id': identity, 'field': field, 'expected': reference, 'actual': actual})


    responses = execute(kernel, requests, 'kernel')
    assert [row['id'] for row in responses] == [row['id'] for row in requests]
    for response in responses:
        identity = response['id']
        kind, row = expected[identity]
        actual = response['actual']
        if kind in ('native', 'cumulative'):
            actual = actual['operations'][0]
            equal(identity, 'raw.error', actual['raw']['error'], '')
            equal(identity, 'raw.reset', actual['raw']['counter_reset_collision'], row.get('reset', False))
            equal(identity, 'raw.reconciled', actual['raw']['nhcb_bounds_reconciled'], row.get('reconciled', True))
            before = row.get('before_final', row['raw'])
            equal(identity, 'raw.receiver', state(actual['raw']['receiver_after']), before)
            equal(identity, 'raw.other', state(actual['raw']['argument_after']), row.get('other_after', row['other']))
            if 'before_final' in row:
                equal(identity, 'raw.compensation', state(actual['raw']['result']), row['compensation'])
                equal(identity, 'fold.error', actual['fold']['error'], '')
                equal(identity, 'fold.receiver', state(actual['fold']['receiver_after']), row['raw'])
            else:
                equal(identity, 'raw.result', state(actual['raw']['result']), row['raw'])
        elif kind == 'reduction':
            equal(identity, 'result', state(actual['result']), row['raw'])
            if 'input_after' in actual:
                equal(identity, 'input_after', state(actual['input_after']), row['input_after'])
            if 'error' in actual:
                equal(identity, 'error', actual['error'], '')
        elif kind == 'compact':
            equal(identity, 'result', state(actual), row['raw'])
        else:
            equal(identity, 'result', actual, row['result'])

    # The source probe calls a.Copy().Compact(0). Reproduce each step from candidate state.
    copy_requests = []
    for response in responses:
        identity = response['id']
        kind, row = expected[identity]
        if kind == 'native':
            actual = response['actual']['operations'][0]
            final = actual['fold'] if actual['fold'] is not None else actual['raw']
            copy_requests.append({'id': identity, 'mode': 'float-copy', 'borrowed': False,
                                  'request': {'input': final['receiver_after'], 'destination': None}})
    copies = execute(kernel, copy_requests, 'copy')
    assert [row['id'] for row in copies] == [row['id'] for row in copy_requests]
    compact_requests = [{'id': row['id'], 'mode': 'float-compact', 'borrowed': False,
                         'request': {'input': row['actual'], 'max_empty': 0}} for row in copies]
    compacted = execute(kernel, compact_requests, 'compact')
    assert [row['id'] for row in compacted] == [row['id'] for row in copy_requests]
    for response in compacted:
        equal(response['id'], 'copy_compact', state(response['actual']), expected[response['id']][1]['compact'])

    return {'kernel_cases': len(requests), 'source_rows': len(all_rows),
            'failures': failures, 'failed_cases': len({row['id'] for row in failures}),
            'kernel_sha256': sha(kernel), 'passed': not failures,
            'evidence_limits': ['native helper snapshots normalize nil bucket/custom payloads to empty lists',
                               'FunctionCalls results are retained observations; full engine families qualify caller diagnostics',
                               'decimal cases use the separate frontend transport',
                               'source Copy then Compact uses candidate-owned intermediate states']}
