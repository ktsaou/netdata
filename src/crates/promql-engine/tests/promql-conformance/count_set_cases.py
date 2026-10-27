#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the grouped-count disappearance workload without retaining raw inputs."""

import argparse
import json
import struct

STALE = "7ff0000000000002"
OPERATIONS = ("left", "right", "unless", "and", "or")
LEFT = "count by(netdata_host_id,netdata_host_name,app_group)(count_over_time(app_processes[1h]))"
RIGHT = "count by(netdata_host_id,netdata_host_name,app_group)(app_processes)"


def bits(value):
    return struct.pack(">d", value).hex()


def labels(values):
    return [{"name": key.encode().hex(), "value": value.encode().hex()}
            for key, value in sorted(values.items())]


def dataset(groups):
    rows = []
    for group in range(groups):
        for instance in range(2):
            points = []
            for timestamp in range(4501):
                # Gone, stale then resumed, always present, and never collected.
                shape = group % 4
                if (shape == 3 or (shape == 0 and timestamp > 3000)
                        or (shape == 1 and 3600 < timestamp < 4200)):
                    continue
                value = STALE if shape == 1 and timestamp == 3600 else bits(float(instance + 1))
                if shape == 2 and timestamp % 97 == 0:
                    value = "7ff8000000000001"
                points.append({"time_ms": timestamp * 1000, "kind": "float", "float": value})
            rows.append({"labels": labels({
                "__name__": "app_processes", "netdata_host_id": str(group // 4),
                "netdata_host_name": "host" + str(group // 4), "app_group": "app" + str(group % 4),
                "instance": str(instance),
            }), "points": points})
    return {"schema_version": 1, "series": rows}


def expression(operation):
    if operation == "left":
        return LEFT
    if operation == "right":
        return RIGHT
    if operation not in OPERATIONS:
        raise ValueError(operation)
    return LEFT + " " + operation + " on(netdata_host_id,app_group) " + RIGHT


def requests(groups, page):
    identity = f"count-set-{groups}-{page}"
    yield {"op": "dataset", "dataset_id": identity, "dataset": dataset(groups), "page_size": page}
    for operation in OPERATIONS:
        yield {"id": operation, "dataset_id": identity, "expression": expression(operation).encode().hex(),
               "profile": "server-default", "mode": "range", "start_ms": 3600000,
               "end_ms": 4500000, "step_ms": 30000}


def normalized(rows):
    return sorted(rows, key=lambda row: tuple((label["name"], label["value"]) for label in row["labels"]))


def expected(data, operation):
    """Independent membership/count calculation for this synthetic fixture."""
    if operation not in OPERATIONS:
        raise ValueError(operation)
    output = {}
    for timestamp in range(3600000, 4500001, 30000):
        left, right = {}, {}
        for row in data["series"]:
            group = tuple((label["name"], label["value"]) for label in row["labels"]
                          if bytes.fromhex(label["name"]) not in (b"__name__", b"instance"))
            if any(timestamp - 3600000 < point["time_ms"] <= timestamp and point["float"] != STALE
                   for point in row["points"]):
                left[group] = left.get(group, 0) + 1
            latest = next((point for point in reversed(row["points"]) if point["time_ms"] <= timestamp), None)
            if latest and latest["time_ms"] >= timestamp - 300000 and latest["float"] != STALE:
                right[group] = right.get(group, 0) + 1
        if operation == "left":
            selected = left
        elif operation == "right":
            selected = right
        elif operation == "or":
            selected = {**right, **left}
        else:
            selected = {key: value for key, value in left.items() if (key in right) == (operation == "and")}
        for group, value in selected.items():
            output.setdefault(group, []).append({"time_ms": timestamp, "kind": "float", "float": bits(float(value))})
    return [{"labels": [{"name": key, "value": value} for key, value in group], "points": points}
            for group, points in sorted(output.items())]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--groups", type=int, nargs="+", default=[8, 32])
    parser.add_argument("--pages", type=int, nargs="+", default=[17, 1024])
    args = parser.parse_args()
    if any(value <= 0 for value in args.groups + args.pages):
        parser.error("groups and pages must be positive")
    for groups in args.groups:
        for page in args.pages:
            for request in requests(groups, page):
                print(json.dumps(request, separators=(",", ":")))


if __name__ == "__main__":
    main()
