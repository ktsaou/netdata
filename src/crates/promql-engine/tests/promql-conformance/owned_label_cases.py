"""Complete-query inputs for copied label capacity and subsequent mutations."""

from info_cases import series


def _records(count, fields):
    records = []
    for row in range(count):
        labels = {"__name__": "m"}
        if fields > 1:
            labels["series"] = f"{row:04d}"
        for field in range(fields - 2):
            labels[f"label{field:02d}"] = f"value{field:02d}"
        records.append(
            series(
                dict(sorted(labels.items())),
                [(tick * 1000, row + tick + 1, "float") for tick in range(3)],
            )
        )
    return records


def _case(ident, expression, records):
    return {
        "request": {
            "id": ident,
            "query": expression,
            "profile": "server-default",
            "mode": "range",
            "time_ms": 0,
            "start_ms": 0,
            "end_ms": 2000,
            "step_ms": 1000,
            "lookback_ms": 1000,
            "subquery_step_ms": 1000,
        },
        "dataset": {"schema_version": 1, "series": records},
        "faults": {},
    }


def cases():
    inputs = []
    shapes = [("full", "m"), ("map", "m+0"), ("projected", "sum by(series) (m)")]
    for count, fields in [
        (1, 1),
        (1, 2),
        (1, 9),
        (1, 17),
        (1, 32),
        (64, 2),
        (64, 9),
        (64, 17),
        (64, 32),
    ]:
        records = _records(count, fields)
        for shape, expression in shapes:
            inputs.append(
                _case(f"labels-{fields}-rows-{count}-{shape}", expression, records)
            )
    mutations = [
        ("replace-add", 'label_replace(m,"added","north","series",".*")'),
        ("replace-existing", 'label_replace(m,"series","${1}x","series","(.*)")'),
        ("replace-delete", 'label_replace(m,"label00","","series",".*")'),
        ("join-add", 'label_join(m,"added",":","series","label00")'),
        ("join-existing", 'label_join(m,"series",":","series","label00")'),
        (
            "nested-add",
            'label_join(label_join(m,"added1",":","series","label00"),"added2",":","series","label00")',
        ),
        ("filter-after-add", 'label_replace(m,"added","north","series",".*") > 0'),
        ("include-one", "m / on() group_left(added) n"),
        ("include-two", "m / on() group_left(added,added2) n"),
    ]
    for count in [1, 64]:
        for fields in [2, 3, 4, 8, 9, 16, 17, 32]:
            records = _records(count, fields)
            right = series(
                {"__name__": "n", "added": "north", "added2": "east"},
                [(tick * 1000, 4, "float") for tick in range(3)],
            )
            for shape, expression in mutations:
                inputs.append(
                    _case(
                        f"mutation-labels-{fields}-rows-{count}-{shape}",
                        expression,
                        records + ([right] if shape.startswith("include-") else []),
                    )
                )
    inputs.append(_case("labels-empty-selector", 'm{missing="none"}', _records(1, 1)))
    return inputs


def binary_result_cases():
    """Immutable binary/set label outputs and their real ownership consumers."""
    from copy import deepcopy
    import grouped_join_cases
    import instant_set_cases
    import mapped_rate_cases

    output = []
    for family, inputs in (
        ("grouped", grouped_join_cases.cases()),
        ("rate", mapped_rate_cases.cases()),
        ("set", instant_set_cases.cases()),
    ):
        for original in inputs:
            supplied = original["request"]
            profiles = (supplied["profile"],) if family != "set" else ("server-default", "upstream-tests")
            delayed_options = (supplied["delayed_name_removal"],) if "delayed_name_removal" in supplied else (False, True)
            for profile in profiles:
                for delayed in delayed_options:
                    case = deepcopy(original)
                    case["request"].update(
                        id=f'binary-labels:{family}:{supplied["id"]}:{profile}:d{int(delayed)}',
                        profile=profile, delayed_name_removal=delayed)
                    output.append(case)

    expressions = (
        "m + n",
        "m / on(id) n",
        "m > bool on(id) n",
        "m < on(id) n",
        "m and on(id) n",
        "m or on(id) n",
        "m unless on(id) n",
        "sum by(job)(m / on(id) n)",
        "abs(m or m) / on(id) n",
        "abs(m + n)",
        'label_join((m+n),"row","/","id","job")',
        'count_values("result",m+n)',
    )
    for state in ("dense", "long", "empty-values", "native", "mixed", "stale", "empty"):
        records = []
        if state != "empty":
            for metric in ("m", "n"):
                for index in (3, 1, 0, 2):
                    labels = {"__name__": metric, "id": str(index), "job": "worker",
                              "__type__": "gauge", "__unit__": "KiB"}
                    if state == "long":
                        labels["long"] = "v" * 512
                    if state == "empty-values":
                        labels.update(empty="", opaque=b"\xff\0")
                    points = []
                    for tick in range(3):
                        value = float((index + 1) * (tick + 1) + (metric == "n"))
                        kind = "float_histogram" if state == "native" or state == "mixed" and index % 2 else "float"
                        if state == "stale" and index != tick:
                            value, kind = "7ff0000000000002", "float"
                        points.append((1000 * tick, value, kind))
                    records.append(series(labels, points))
        for profile in ("server-default", "upstream-tests"):
            for delayed in (False, True):
                for instant in (False, True):
                    for index, expression in enumerate(expressions):
                        case = _case(f"binary-labels:owners:{state}:{profile}:d{int(delayed)}:i{int(instant)}:{index}",
                                     expression, records)
                        case["request"].update(profile=profile, delayed_name_removal=delayed,
                            mode="instant" if instant else "range", time_ms=2000)
                        output.append(case)
    assert len({case["request"]["id"] for case in output}) == len(output)
    return output
