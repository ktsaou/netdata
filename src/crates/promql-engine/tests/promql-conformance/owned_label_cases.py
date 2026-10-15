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
