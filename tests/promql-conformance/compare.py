#!/usr/bin/env python3
"""Compare complete typed results and report every paired binary64 difference."""

import argparse
import base64
import json
import math
import re
import struct
import sys

STALE = 0x7FF0000000000002
MIN_NORMAL = float.fromhex("0x1.0p-1022")
EPSILON = 1e-6


def bits(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-fA-F]{16}", value):
        raise ValueError("invalid binary64 encoding")
    raw = int(value, 16)
    return raw, struct.unpack(">d", raw.to_bytes(8, "big"))[0]


def same_number(left, right):
    """prometheus/prometheus @ 5241a27fe3c6983549fccc32f6e65917408c63cd:
    util/almost/almost.go:26-50; epsilon from promql/promqltest/test.go:61.
    """
    a_bits, a = bits(left)
    b_bits, b = bits(right)
    if a_bits == STALE or b_bits == STALE:
        return a_bits == b_bits
    if math.isnan(a) and math.isnan(b):
        return True
    if a == b:
        return True
    magnitude = abs(a) + abs(b)
    difference = abs(a - b)
    if a == 0 or b == 0 or magnitude < MIN_NORMAL:
        return difference < EPSILON * MIN_NORMAL
    return difference / min(magnitude, sys.float_info.max) < EPSILON


def byte_value(value):
    if not isinstance(value, str):
        raise ValueError("expected a base64 byte string")  # noqa: TRY004 - Unified wire-validation error contract.
    return base64.b64decode(value, validate=True)


def record(value, name):
    if not isinstance(value, dict):
        raise ValueError(name + " must be an object")  # noqa: TRY004 - Unified wire-validation error contract.
    return value


def array(value, name, nullable=False):
    if nullable and value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(name + " must be an array")  # noqa: TRY004 - Unified wire-validation error contract.
    return value


def labels(row):
    ls = []
    for label in array(record(row, "row")["labels"], "labels"):
        record(label, "label")
        ls.append((byte_value(label["name"]), byte_value(label["value"])))
    if any(not name for name, _ in ls) or len({name for name, _ in ls}) != len(ls):
        raise ValueError("empty or duplicate label name")
    return tuple(sorted(ls))


def histogram_buckets(histogram, side):
    """Bucket indices implement upstream Compact(0) equivalence without losing raw values."""
    index, cursor, out = 0, 0, {}
    buckets = histogram[side + "_buckets"] or []
    for span in histogram[side + "_spans"] or []:
        index += span["Offset"]
        for value in buckets[cursor : cursor + span["Length"]]:
            if index in out:
                raise ValueError("overlapping histogram buckets")
            out[index] = value
            index += 1
        cursor += span["Length"]
    return out


def checked_results(document):
    record(document, "result document")
    if (
        type(document.get("schema_version")) is not int
        or document["schema_version"] != 1
    ):
        raise ValueError("unsupported result schema")
    if not isinstance(document.get("results"), list):
        raise ValueError("results must be a list")  # noqa: TRY004 - Unified wire-validation error contract.
    out = {}
    for result in document["results"]:
        record(result, "result")
        ident = result["id"]
        if not isinstance(ident, str) or not ident or ident in out:
            raise ValueError("empty or duplicate result id")
        if result["kind"] not in {"scalar", "string", "vector", "matrix", "error"}:
            raise ValueError("unknown result kind")
        for field in ["warnings", "infos"]:
            messages = [byte_value(value) for value in array(result[field], field)]
            if len(messages) != len(set(messages)):
                raise ValueError("duplicate diagnostic")
        keys = [labels(row) for row in array(result["rows"], "rows")]
        if result["kind"] == "error":
            if result["rows"] or not result.get("error"):
                raise ValueError("malformed error result")
            error = record(result["error"], "error")
            if (
                error["phase"] not in {"request", "construction", "execution"}
                or not isinstance(error["type"], str)
                or not error["type"]
            ):
                raise ValueError("invalid error attribution")
            byte_value(error["message"])
            for parsed in array(error.get("parse_errors", []), "parse errors"):
                record(parsed, "parse error")
                for key in ["start", "end", "line_offset"]:
                    if type(parsed[key]) is not int:
                        raise ValueError("invalid parser position")
                byte_value(parsed["message"])
        elif result.get("error") is not None:
            raise ValueError("successful result contains error")
        elif result["kind"] in {"scalar", "string"}:
            if (
                len(result["rows"]) != 1
                or keys[0]
                or len(result["rows"][0]["points"]) != 1
            ):
                raise ValueError("malformed scalar/string result")
        for row in result["rows"]:
            array(row["points"], "points")
            if not row["points"] or (
                result["kind"] == "vector" and len(row["points"]) != 1
            ):
                raise ValueError("empty row or invalid vector point count")
            previous = None
            for point in row["points"]:
                record(point, "point")
                timestamp = point["time_ms"]
                if type(timestamp) is not int or not -(1 << 63) <= timestamp < (
                    1 << 63
                ):
                    raise ValueError("invalid sample timestamp")
                if previous is not None and timestamp < previous:
                    raise ValueError("timestamps are not increasing")
                previous = timestamp
                start = point.get("start_time_ms", 0)
                if type(start) is not int or not -(1 << 63) <= start < (1 << 63):
                    raise ValueError("invalid sample start timestamp")
                kind = point["kind"]
                allowed = {
                    "float": "float",
                    "string": "string",
                    "float_histogram": "float_histogram",
                }
                if any(
                    name in point and name != allowed.get(kind)
                    for name in ["float", "string", "histogram", "float_histogram"]
                ):
                    raise ValueError("ambiguous sample payload")
                if kind == "float":
                    bits(point["float"])
                elif kind == "string":
                    byte_value(point.get("string", ""))
                elif kind == "float_histogram":
                    histogram = record(point["float_histogram"], "float histogram")
                    if type(histogram["schema"]) is not int or not -(
                        1 << 31
                    ) <= histogram["schema"] < (1 << 31):
                        raise ValueError("invalid histogram schema")
                    if (
                        type(histogram["reset_hint"]) is not int
                        or not 0 <= histogram["reset_hint"] <= 3
                    ):
                        raise ValueError("invalid histogram reset hint")
                    for side in ["positive", "negative"]:
                        spans = array(
                            histogram[side + "_spans"], side + " spans", nullable=True
                        )
                        buckets = array(
                            histogram[side + "_buckets"],
                            side + " buckets",
                            nullable=True,
                        )
                        for span in spans:
                            record(span, "span")
                            if (
                                type(span["Offset"]) is not int
                                or not -(1 << 31) <= span["Offset"] < (1 << 31)
                                or type(span["Length"]) is not int
                                or not 0 <= span["Length"] < (1 << 32)
                            ):
                                raise ValueError("invalid histogram span")
                        if sum(span["Length"] for span in spans) != len(buckets):
                            raise ValueError("histogram spans and buckets disagree")
                    for name in ["zero_threshold", "zero_count", "count", "sum"]:
                        bits(histogram[name])
                    for name in [
                        "positive_buckets",
                        "negative_buckets",
                        "custom_values",
                    ]:
                        for value in array(histogram[name], name, nullable=True):
                            bits(value)
                else:
                    raise ValueError("invalid result sample kind")
                if result["kind"] == "string" and kind != "string":
                    raise ValueError("wrong string sample kind")
                if result["kind"] == "scalar" and kind != "float":
                    raise ValueError("wrong scalar sample kind")
                if result["kind"] in {"vector", "matrix"} and kind == "string":
                    raise ValueError("string in a numeric result")
        out[ident] = result
    return out


def difference(expected, actual, ordered=False, exact_numeric=False):
    failures, bit_differences = [], []

    def row_occurrences(rows):
        counts = {}
        for row in rows:
            identity = labels(row)
            occurrence = counts.get(identity, 0)
            counts[identity] = occurrence + 1
            yield (identity, occurrence), row

    def point_occurrences(points):
        counts = {}
        for point in points:
            identity = (point["time_ms"], point["kind"])
            occurrence = counts.get(identity, 0)
            counts[identity] = occurrence + 1
            yield (*identity, occurrence), point

    def exact(path, left, right):
        if left != right:
            failures.append(path)

    def numeric(path, left, right):
        left_bits, a = bits(left)
        right_bits, b = bits(right)
        if left_bits != right_bits:
            bit_differences.append(
                {"path": path, "reference": left, "candidate": right}
            )
        unequal_exact = a != b and not (math.isnan(a) and math.isnan(b))
        if (exact_numeric and unequal_exact) or not same_number(left, right):
            failures.append(path)

    def boundary(path, left, right):
        # Native bucket boundaries use exact equality in upstream tests.
        if bits(left)[0] != bits(right)[0]:
            bit_differences.append(
                {"path": path, "reference": left, "candidate": right}
            )
        if bits(left)[1] != bits(right)[1]:
            failures.append(path)

    exact("kind", expected["kind"], actual["kind"])
    for name in ["warnings", "infos"]:
        exact(
            name,
            sorted(map(byte_value, expected[name])),
            sorted(map(byte_value, actual[name])),
        )
    if expected["kind"] != actual["kind"]:
        return failures, bit_differences
    if expected["kind"] == "error":
        exact("error", expected["error"], actual["error"])
        return failures, bit_differences
    e_rows, a_rows = expected["rows"], actual["rows"]
    if not ordered:
        e_rows, a_rows = sorted(e_rows, key=labels), sorted(a_rows, key=labels)
    exact("series membership", [labels(r) for r in e_rows], [labels(r) for r in a_rows])
    # Original ordinary groups can project distinct raw hashes to identical labels.
    # Preserve each occurrence while reporting common rows when another is missing.
    actual_by_row_occurrence = dict(row_occurrences(a_rows))
    for row_index, (occurrence, er) in enumerate(row_occurrences(e_rows)):
        ar = actual_by_row_occurrence.get(occurrence)
        if ar is None:
            continue
        exact(f"row[{row_index}].point count", len(er["points"]), len(ar["points"]))
        identity = lambda point: (point["time_ms"], point["kind"])
        # Go sorts by timestamp alone; equal-time float/histogram tie order is unspecified.
        exact(
            f"row[{row_index}].typed timestamps",
            sorted(map(identity, er["points"])),
            sorted(map(identity, ar["points"])),
        )
        actual_by_occurrence = dict(point_occurrences(ar["points"]))
        for index, (occurrence, ep) in enumerate(point_occurrences(er["points"])):
            ap = actual_by_occurrence.get(occurrence)
            if ap is None:
                continue
            path = f"row[{row_index}].point[{index}]"
            exact(path + ".time_ms", ep["time_ms"], ap["time_ms"])
            exact(
                path + ".start_time_ms",
                ep.get("start_time_ms", 0),
                ap.get("start_time_ms", 0),
            )
            exact(path + ".kind", ep["kind"], ap["kind"])
            if ep["kind"] != ap["kind"]:
                continue
            if ep["kind"] == "float":
                numeric(path + ".float", ep["float"], ap["float"])
            elif ep["kind"] == "string":
                exact(
                    path + ".string",
                    byte_value(ep.get("string", "")),
                    byte_value(ap.get("string", "")),
                )
            else:
                eh, ah = ep["float_histogram"], ap["float_histogram"]
                for key in ["reset_hint", "schema"]:
                    exact(path + ".histogram." + key, eh[key], ah[key])
                boundary(
                    path + ".histogram.zero_threshold",
                    eh["zero_threshold"],
                    ah["zero_threshold"],
                )
                for key in ["zero_count", "count", "sum"]:
                    numeric(path + ".histogram." + key, eh[key], ah[key])
                e, a = eh["custom_values"] or [], ah["custom_values"] or []
                exact(path + ".histogram.custom_values.length", len(e), len(a))
                for j, (eb, ab) in enumerate(zip(e, a)):
                    boundary(f"{path}.histogram.custom_values[{j}]", eb, ab)
                for side in ["positive", "negative"]:
                    e, a = histogram_buckets(eh, side), histogram_buckets(ah, side)
                    active = lambda buckets: [
                        i for i, value in buckets.items() if bits(value)[1] != 0
                    ]
                    exact(
                        path + ".histogram." + side + ".indices", active(e), active(a)
                    )
                    for j in sorted(e.keys() & a.keys()):
                        numeric(f"{path}.histogram.{side}_bucket[{j}]", e[j], a[j])
    return failures, bit_differences


def compare(reference, candidate, cases):
    expected, actual = checked_results(reference), checked_results(candidate)
    if not isinstance(cases, list) or not cases:
        raise ValueError("required case list must be nonempty")
    for case in cases:
        record(case, "required case")
        if not isinstance(case.get("id"), str) or not case["id"]:
            raise ValueError("invalid required case id")
        for flag in ["ordered", "exact_numeric"]:
            if flag in case and type(case[flag]) is not bool:
                raise ValueError("invalid case comparison flag")
    identities = [case["id"] for case in cases]
    if len(set(identities)) != len(identities):
        raise ValueError("duplicate required case id")
    missing = sorted(set(identities) - (expected.keys() & actual.keys()))
    extra = sorted((expected.keys() | actual.keys()) - set(identities))
    failures, mismatches = {}, []
    for case in cases:
        ident = case["id"]
        if ident in missing:
            failures[ident] = ["missing required result"]
            continue
        differences, raw = difference(
            expected[ident],
            actual[ident],
            case.get("ordered", False),
            case.get("exact_numeric", False),
        )
        if differences:
            failures[ident] = differences
        mismatches.extend({"id": ident, **value} for value in raw)
    return {
        "required": len(cases),
        "evaluated": len(cases) - len(missing),
        "semantic_failures": failures,
        "unexpected_results": extra,
        "bitwise_differences": mismatches,
        "semantic_pass": not failures and not extra,
        "bitwise_pass": not failures and not extra and not mismatches,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference")
    parser.add_argument("candidate")
    parser.add_argument("cases")
    parser.add_argument("--strict-bits", action="store_true")
    args = parser.parse_args()
    try:
        with open(args.reference) as f:
            reference = json.load(f)
        with open(args.candidate) as f:
            candidate = json.load(f)
        with open(args.cases) as f:
            cases = json.load(f)
        report = compare(reference, candidate, cases)
        print(json.dumps(report, indent=2))
        sys.exit(not report["bitwise_pass" if args.strict_bits else "semantic_pass"])
    except (ValueError, KeyError, TypeError) as error:
        print(json.dumps({"harness_error": str(error)}))
        sys.exit(2)
