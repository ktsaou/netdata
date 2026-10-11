"""Declare missing default-profile controls using pinned builtin datasets."""

import hashlib
import json
from copy import deepcopy

import query

SELECTED = {
    "acos": "testdata/trig_functions.test:28:variant:0000",
    "acosh": "testdata/trig_functions.test:58:variant:0000",
    "asin": "testdata/trig_functions.test:23:variant:0000",
    "asinh": "testdata/trig_functions.test:53:variant:0000",
    "atan": "testdata/trig_functions.test:33:variant:0000",
    "atanh": "testdata/trig_functions.test:63:variant:0000",
    "ceil": "testdata/functions.test:1723:variant:0000",
    "clamp_max": "testdata/functions.test:617:variant:0000",
    "clamp_min": "testdata/functions.test:622:variant:0000",
    "cos": "testdata/trig_functions.test:13:variant:0000",
    "cosh": "testdata/trig_functions.test:43:variant:0000",
    "day_of_month": "testdata/functions.test:1506:variant:0000",
    "day_of_week": "testdata/functions.test:1519:variant:0000",
    "day_of_year": "testdata/functions.test:1512:variant:0000",
    "days_in_month": "testdata/functions.test:1562:variant:0000",
    "deg": "testdata/trig_functions.test:83:variant:0000",
    "exp": "testdata/functions.test:1959:variant:0000",
    "floor": "testdata/functions.test:1734:variant:0000",
    "group": "testdata/aggregators.test:579:variant:0000",
    "histogram_fraction": "testdata/histograms.test:115:variant:0000",
    "ln": "testdata/functions.test:1975:variant:0000",
    "log10": "testdata/functions.test:2023:variant:0000",
    "log2": "testdata/functions.test:2007:variant:0000",
    "minute": "testdata/functions.test:1531:variant:0000",
    "month": "testdata/functions.test:1500:variant:0000",
    "pi": "testdata/trig_functions.test:100:variant:0000",
    "rad": "testdata/trig_functions.test:68:variant:0000",
    "sgn": "testdata/functions.test:715:variant:0000",
    "sin": "testdata/trig_functions.test:8:variant:0000",
    "sinh": "testdata/trig_functions.test:38:variant:0000",
    "sqrt": "testdata/functions.test:1999:variant:0000",
    "stddev": "testdata/aggregators.test:805:variant:0000",
    "stdvar": "testdata/aggregators.test:809:variant:0000",
    "tan": "testdata/trig_functions.test:18:variant:0000",
    "tanh": "testdata/trig_functions.test:48:variant:0000",
    "year": "testdata/functions.test:1488:variant:0000",
}

DISABLED = {
    "histogram_quantiles": 'histogram_quantiles(m, "quantile", 0.5, 0.9)',
    "max_of": "max_of(1, 2)",
    "min_of": "min_of(1, 2)",
    "limitk": "limitk(1, m)",
    "limit_ratio": "limit_ratio(0.5, m)",
}

INPUT_SHA256 = "207a3f6b1ffed77bc0c6424dc2c043cde22a4ce6d0eeca8f1731730559c08758"


def cases(builtin_capture, upstream):
    capture, _, _, entries, _ = query.load(builtin_capture, upstream)
    indexed = {row["request"]["id"]: row for row in entries}
    if len(indexed) != len(entries):
        raise ValueError("builtin source request identities changed")
    inputs = []
    for name, ident in sorted(SELECTED.items()):
        row = indexed[ident]
        request = deepcopy(row["request"])
        if (
            request["profile"] != "upstream-tests"
            or request["mode"] != "instant"
            or row["result"]["kind"] not in ("vector", "scalar")
            or not any(
                point for result in row["result"]["rows"] for point in result["points"]
            )
        ):
            raise ValueError("default-profile representative source outcome changed")
        request.update(
            id="default-profile:" + name + ":" + ident, profile="server-default"
        )
        dataset = capture / "datasets" / (row["dataset_id"] + ".json")
        inputs.append(
            {
                "request": request,
                "dataset": json.loads(dataset.read_bytes()),
                "faults": {},
            }
        )
    for name, expression in sorted(DISABLED.items()):
        inputs.append(
            {
                "request": {
                    "id": "default-profile:disabled:" + name,
                    "query": expression,
                    "profile": "server-default",
                    "mode": "instant",
                    "time_ms": 0,
                },
                "dataset": {"schema_version": 1, "series": []},
                "faults": {},
            }
        )
    encoded = json.dumps(
        inputs, sort_keys=True, separators=(",", ":"), allow_nan=False
    ).encode()
    if len(inputs) != 41 or hashlib.sha256(encoded).hexdigest() != INPUT_SHA256:
        raise ValueError("default-profile query/dataset/fault roster changed")
    return inputs
