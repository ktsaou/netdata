"""Inputs distinguish plural label identity, mutation order, and cleanup phases."""
import base64
import struct


def cases():
    def encoded(value):
        return base64.b64encode(value.encode()).decode()

    def number(value):
        return struct.pack(">d", value).hex()

    dataset = {"schema_version": 1, "series": []}

    def series(metric, extra, points):
        labels = {"__name__": metric, **extra}
        dataset["series"].append({"labels": [{"name": encoded(k), "value": encoded(v)}
                                            for k, v in sorted(labels.items())], "points": points})

    def floats(values):
        return [{"time_ms": i * 1000, "kind": "float", "float": number(value)}
                for i, value in enumerate(values)]

    for bound, values in [("1", [3] * 6), ("2", [2, 3, 2, 3, 2, 3]), ("+Inf", [3] * 6)]:
        series("m", {"le": bound, "empty": "", "role": "a", "q": "old",
                     "__type__": "counter", "__unit__": "seconds"}, floats(values))
    series("q", {}, [{"time_ms": i * 1000, "kind": "float", "float": value}
                     for i, value in enumerate([number(.25), number(.75), number(0.), number(-0.),
                                                "7ff8000000000042", "fff8000000000043"])])
    series("bad", {"le": "invalid"}, floats([1] * 6))
    histogram = {"reset_hint": 0, "schema": -53, "zero_threshold": number(0),
                 "zero_count": number(0), "count": number(3), "sum": number(4),
                 "positive_spans": [{"offset": 0, "length": 3}], "negative_spans": None,
                 "positive_buckets": [number(1)] * 3, "negative_buckets": None,
                 "custom_values": [number(1), number(2)]}
    native = [{"time_ms": i * 1000, "kind": "float_histogram", "float_histogram": histogram}
              for i in range(6)]
    for role in ["a", "b"]:
        series("h", {"role": role, "empty": "", "__type__": "counter", "__unit__": "seconds"}, native)
    series("mixed", {}, native)
    for bound in ["1", "+Inf"]:
        series("mixed", {"le": bound}, floats([3] * 6))

    queries = [
        'histogram_quantiles(m,"q",.25,.75)',
        'histogram_quantiles(m,"q",scalar(q),1-scalar(q))',
        'histogram_quantiles(m,"q",.25,.25)',
        'histogram_quantiles(m,"q",0/0,0/0)',
        'histogram_quantiles(m,"q",0,-0)',
        'histogram_quantiles(m,"q",-0,0)',
        'histogram_quantiles(m,"q",-1,.25,.75,2,0/0)',
        'histogram_quantiles(m,"q",.1,.2,.3,.4,.5,.6,.7,.8,.9,1)',
        'histogram_quantiles(m,"q",.25,.75,.25,.75,.25,.75,.25,.75,.25,.75)',
        'histogram_quantiles(missing,"q",-1,2)',
        'histogram_quantiles(bad,"q",-1,2)',
        'histogram_quantiles(mixed,"q",.25,.75)',
        'histogram_quantiles(h,"q",.25,.75)',
        'histogram_quantiles(h,"role",.25,.75)',
        'histogram_quantiles(h,"role",.25,.25)',
        'histogram_quantiles(h{role="a"},"q",scalar(q),1-scalar(q))',
        'histogram_quantiles(m or h,"q",.25,.75)',
        'histogram_quantiles(bad,("q"),-1)',
        'quantile(scalar(histogram_quantiles(bad,("q"),-1)),vector(1))',
    ]
    for destination in ['', 'le', 'role', '__name__', '__type__', '__unit__', 'a b', 'é', r'a\xff']:
        queries.append(f'histogram_quantiles(m,"{destination}",.25,.75)')
        queries.append(f'histogram_quantiles(h{{role="a"}},"{destination}",.25,.75)')
    for child in ['histogram_quantiles(m,"q",.25,.25)',
                  'histogram_quantiles(m,"q",scalar(q),1-scalar(q))',
                  'histogram_quantiles(h,"role",.25,.75)']:
        queries.extend([f'sum by(q)({child})', f'count by(q)({child})', f'topk(1,{child})',
                        f'sum({child})', f'sort_desc({child})', f'{child}>.5'])
    for child in ['histogram_quantiles(m,"",.25,.75)',
                  'histogram_quantiles(h{role="a"},"",.25,.75)']:
        queries.extend([f'sum({child})', f'sum(-({child}))', f'sum(abs({child}))',
                        f'sum({child}*1)', f'sum({child}>.5)', f'scalar({child})',
                        f'sum without(q)({child})', f'count({child})'])
    requests = []
    for delayed in [False, True]:
        for index, text in enumerate(queries):
            for mode in ['instant', 'range', 'one-step-range']:
                request = {'id': f'plural:{int(delayed)}:{index}:{mode}', 'query': text,
                           'profile': 'upstream-tests', 'delayed_name_removal': delayed,
                           'mode': 'instant' if mode == 'instant' else 'range'}
                if mode == 'instant':
                    request['time_ms'] = 1000
                else:
                    request.update(start_ms=0, end_ms=5000 if mode == 'range' else 0, step_ms=1000)
                requests.append(request)
    return dataset, requests
