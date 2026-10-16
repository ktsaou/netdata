"""Fixed byte-boundary and whole-query controls for shared UTF-8 validation."""
import struct


def _quote(value):
    return '"' + ''.join(f'\\x{byte:02x}' for byte in value) + '"'


def _dataset(ident, values):
    series = []
    for index, value in enumerate(values):
        labels = [(b'__name__', b'm'), (b'host', value), (b'id', str(index).encode())]
        series.append({
            'labels': [{'name': name.hex(), 'value': value.hex()} for name, value in labels],
            'points': [{'time_ms': 1000, 'kind': 'float',
                        'float': struct.pack('>d', index + 1).hex()}],
        })
    return {'op': 'dataset', 'dataset_id': ident, 'page_size': 17,
            'dataset': {'schema_version': 1, 'series': series}}


def cases():
    packets, expected_counts, timing_ids = [], {}, []

    def query(dataset, suffix, expression, expected=None, timed=False):
        ident = dataset + '-' + suffix
        packets.append({'id': ident, 'dataset_id': dataset, 'expression': expression.encode().hex(),
                        'mode': 'instant', 'profile': 'server-default', 'time_ms': 1000})
        if expected is not None:
            expected_counts[ident] = expected
        if timed:
            timing_ids.append(ident)

    for count, width in [(16, 16), (256, 16), (256, 128), (256, 1024), (256, 4096)]:
        dataset = f'utf8-ascii-n{count}-w{width}'
        values = [b'a' + b'b' * (width - 3) + (b'b' if index % 2 == 0 else b'7') + b'z'
                  for index in range(count)]
        packets.append(_dataset(dataset, values))
        expressions = [
            ('fallback-pos', 'count(m{host=~"(a[^0-9]+z)"})', count // 2),
            ('fallback-neg', 'count(m{host!~"(a[^0-9]+z)"})', count // 2),
            ('replace', 'count(label_replace(m,"dst","x","host","(a[^0-9]+z)"))', count),
            ('prefix-control', 'count(m{host=~"a.*"})', count),
        ]
        for suffix, expression, expected in expressions:
            query(dataset, suffix, expression, expected, timed=True)

    dataset = 'utf8-subject-boundaries'
    values = []
    for rune in ['\u00e9', '\u20ac', '\U0001f600', '\ufffd']:
        for prefix in [1021, 1022, 1023, 1024, 1025, 2047]:
            values.append(b'a' + b'b' * (prefix - 1) + rune.encode() + b'z')
    for malformed in [b'\x80', b'\xc0\x80', b'\xe1\x80A', b'\xed\xa0\x80',
                      b'\xf4\x90\x80\x80', b'\xf0\x9f\x98', b'\xe2\x82']:
        for prefix in [1023, 1024, 2047]:
            values.append(b'a' + b'b' * (prefix - 1) + malformed + b'z')
    packets.append(_dataset(dataset, values))
    for suffix, expression in [
        ('fallback-pos', 'm{host=~"(a[^0-9]+z)"}'),
        ('fallback-neg', 'm{host!~"(a[^0-9]+z)"}'),
        ('replace', 'label_replace(m,"dst","$1","host","(a[^0-9]+z)")'),
        ('replace-short', 'label_replace(m,"dst","x","host","(a[^0-9]+z)")'),
    ]:
        query(dataset, suffix, expression)

    dataset = 'utf8-name-boundaries'
    packets.append(_dataset(dataset, [b'abz']))
    names = [b'', b'x', b'x' * 1023 + '\u00e9'.encode(), b'x' * 1023 + '\u20ac'.encode(),
             b'x' * 1023 + '\U0001f600'.encode(), b'x' * 1023 + '\ufffd'.encode(),
             b'x' * 1023 + b'\xe1\x80A', b'x' * 1024 + b'\xff', b'x' * 2047 + b'\xe2\x82']
    for index, name in enumerate(names):
        literal = _quote(name)
        for operation, expression in [
            ('join', f'label_join(m,{literal},"-","host")'),
            ('replace', f'label_replace(m,{literal},"x","host","a.+z")'),
            ('count-values', f'count_values({literal},m)'),
            ('grouping', f'sum by ({literal})(m)'),
            ('regex-first', f'label_replace(m,{literal},"x","host","(")'),
            ('source-first', f'label_join(m,{literal},"-","\\xff")'),
        ]:
            query(dataset, f'{operation}-{index}', expression)
    return packets, expected_counts, timing_ids
