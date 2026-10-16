"""Fixed direct-set histogram, ordering and single-tick controls."""
from copy import deepcopy

from info_cases import series
from mapped_rate_cases import _case


def _records(count=2):
    return [series({"__name__": "fixture_http", "netdata_dimension_id": "gauge",
                    "netdata_host_id": f"h{index}"},
                   [(4000, float(5 + 2 * index), "float")])
            for index in reversed(range(count))]


def performance_cases():
    metric = 'fixture_http{netdata_dimension_id="gauge"}'
    buckets = (f'label_replace({metric},"le","1","netdata_dimension_id","gauge") or '
               f'label_replace({metric},"le","+Inf","netdata_dimension_id","gauge")')
    grouped = " or ".join(
        f'label_replace(({buckets}),"fixture_group","{group}","netdata_dimension_id","gauge")'
        for group in ("a", "b", "c"))
    quantile = f'histogram_quantile(0.5,({grouped}))'
    left = f'label_replace({metric},"branch","a","netdata_dimension_id","gauge")'
    right = left.replace('"a"', '"b"')
    expressions = [
        ("histogram", f'scalar(sum({quantile}))', _records()),
        ("unions", f'scalar(sum(({grouped})))', _records()),
        ("or", f'sum({left} or {right})', _records()),
        ("and", f'sum({left} and on(netdata_host_id) {right})', _records()),
        ("unless", f'sum({left} unless {right})', _records()),
        ("raw", f'scalar(sum({metric}))', _records()),
        *[(f"tiny{count}", f'sum({left} or {right})', _records(count))
          for count in (0, 1, 4)],
    ]
    output = []
    for name, expression, records in expressions:
        case = _case(f'instant-set-{name}', expression, records, instant=True)
        case["request"].update(time_ms=4000, start_ms=4000, end_ms=4000)
        output.append(case)
    return output


def cases():
    output = []
    controls = performance_cases()
    histogram = controls[0]["request"]["query"]
    controls.append(_case("instant-set-vector-histogram", histogram[len("scalar(sum("):-2], _records(), instant=True))
    controls.append(_case("instant-set-vector-or", 'label_replace(fixture_http,"branch","a","netdata_dimension_id","gauge") or label_replace(fixture_http,"branch","b","netdata_dimension_id","gauge")', _records(), instant=True))
    for original in controls:
        for delayed in (False, True):
            for mode, end in (("instant", 4000), ("equal", 4000), ("one", 4001), ("range", 6000)):
                case = deepcopy(original)
                request = case["request"]
                request.update(id=f'{request["id"]}-{mode}-d{int(delayed)}',
                               mode="instant" if mode == "instant" else "range",
                               time_ms=4000, start_ms=4000, end_ms=end, step_ms=1000,
                               delayed_name_removal=delayed)
                output.append(case)
    return output
