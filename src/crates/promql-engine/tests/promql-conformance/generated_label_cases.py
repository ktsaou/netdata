# SPDX-License-Identifier: GPL-3.0-or-later
"""Generated label buffers, capacity boundaries and ordinary mutation controls."""
from copy import deepcopy
from info_cases import series
from owned_label_cases import cases as ordinary_label_cases
from single_reset_cases import _request


def generated_label_cases():
    output = []
    operations = {
        "join-new": 'label_join(m,"dst","","source")',
        "replace-new": 'label_replace(m,"dst","$1","source","(.*)")',
        "join-existing": 'label_join(m,"source","","source")',
        "replace-existing": 'label_replace(m,"source","$1","source","(.*)")',
    }
    for width in (1, 3, 4, 16, 1024, 1025, 4096, 5000, 6000):
        for count in ((1, 8, 32) if width in (16, 4096) else (1,)):
            records = [series({"__name__": "m", "id": f"{i:04d}", "source": "x" * width},
                [(t * 1000, i + t + 1, "float") for t in range(4)]) for i in range(count)]
            for name, expression in operations.items():
                for mode in ("instant", "range"):
                    request = _request(f"generated-{name}-{width}-{count}-{mode}", expression, 1, mode)
                    output.append({"shape": "target" if count in (8, 32) else "control",
                        "dataset": {"schema_version": 1, "series": records}, "request": request})
    controls = {
        "empty": 'label_join(m,"source","","missing")',
        "no-match": 'label_replace(m,"dst","$1","source","(z+)")',
        "join-pieces": 'label_join(m,"dst",":","source","id","source")',
        "replace-pieces": 'label_replace(m,"dst","${1}x$1","source","(.*)")',
        "grow": 'label_join(label_join(m,"dst","","source"),"dst","","other")',
        "name": 'label_replace(m,"__name__","renamed","source",".*")',
        "delete-nested": 'label_join(label_join(m,"dst","","source"),"dst","","missing")',
        "no-source": 'label_join(m,"dst","")',
        "filtered": 'label_join(m,"dst","","source") > 0',
        "summed": 'sum(label_join(m,"dst","","source"))',
        "invalid-destination": 'label_join(m,"bad.name","","source")',
    }
    for kind in ("float", "float_histogram"):
        records = [series({"__name__": "m", "id": f"{i:04d}", "source": "x" * 5000, "other": "y" * 7000},
            [(t * 1000, i + t + 1, kind) for t in range(4)]) for i in range(4)]
        for name, expression in controls.items():
            for mode in ("instant", "range"):
                request = _request(f"generated-control-{kind}-{name}-{mode}", expression, 1, mode)
                output.append({"shape": "control", "dataset": {"schema_version": 1, "series": records}, "request": request})
    return output


def generated_label_functional_cases():
    ordinary = deepcopy(ordinary_label_cases())
    for case in ordinary:
        case["shape"] = "control"
    return generated_label_cases() + ordinary


def generated_label_performance_cases():
    return [case for case in generated_label_cases()
        if case["shape"] == "target" or case["request"]["id"] in {
            "generated-join-new-3-1-instant", "generated-replace-new-1025-1-instant",
            "generated-join-existing-5000-1-range", "generated-replace-existing-6000-1-range",
            "generated-control-float-empty-instant", "generated-control-float-no-match-instant",
            "generated-control-float-grow-range", "generated-control-float-join-pieces-instant",
            "generated-control-float-replace-pieces-range", "generated-control-float_histogram-grow-range",
            "generated-control-float-name-instant", "generated-control-float-delete-nested-range",
        }]
