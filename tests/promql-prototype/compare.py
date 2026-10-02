#!/usr/bin/env python3
"""Compare typed results without hiding duplicate labelsets or point/order errors."""
import json
import math
import struct
import sys

def same_number(a, b):
    a, b = float(a), float(b)
    if math.isnan(a) or math.isnan(b):
        return math.isnan(a) and math.isnan(b)
    return a == b or (math.isfinite(a) and math.isfinite(b) and math.isclose(a, b, rel_tol=1e-6, abs_tol=1e-6))

def row_key(row):
    return tuple(sorted(row["labels"].items()))

def difference(expected, actual, ordered=False):
    if expected["kind"] != actual["kind"]:
        return f'kind {actual["kind"]}, expected {expected["kind"]}: {actual.get("error", "")}'
    if expected["kind"] == 4:
        def category(error):
            if "many-to" in error or "duplicate series" in error:
                return "cardinality"
            if "same labelset" in error or "duplicate output labelset" in error:
                return "duplicate-labelset"
            return "other"
        e, a = category(expected.get("error", "")), category(actual.get("error", ""))
        return None if actual.get("error") and e == a else f'error class {a}, expected {e}: {actual.get("error")}'
    e, a = expected["rows"], actual["rows"]
    if not ordered:
        e, a = sorted(e, key=row_key), sorted(a, key=row_key)
    if len(e) != len(a):
        return f"row count {len(a)}, expected {len(e)}"
    for er, ar in zip(e, a):
        if er["labels"] != ar["labels"]:
            return f'labels {ar["labels"]}, expected {er["labels"]}'
        if len(er["points"]) != len(ar["points"]):
            return "point count"
        for ep, ap in zip(er["points"], ar["points"]):
            if ep[0] != ap[0] or not same_number(ep[1], ap[1]):
                return f"point {ap}, expected {ep}"
    return None

def compare(reference, candidate, cases):
    ref = {r["id"]: r for r in reference["results"]}
    got = {r["id"]: r for r in candidate["results"]}
    failures = {}
    for case in cases:
        ident = case["id"]
        if ident not in ref or ident not in got:
            failures[ident] = "missing result"
            continue
        error = difference(ref[ident], got[ident], case.get("ordered", False))
        if error:
            failures[ident] = error
    return failures

def numeric_difference_report(reference, candidate, cases):
    """Report actual binary64 bits after typed tolerance comparison, without changing its acceptance."""
    refs = {r["id"]: r for r in reference["results"]}
    got = {r["id"]: r for r in candidate["results"]}
    report = {"finite_values": 0, "bitwise_differences": [], "nonfinite_values": 0,
              "skipped_non_equivalent_cases": [],
              "method": "raw IEEE-754 binary64 output bits before NaN/string normalization; tolerance acceptance unchanged"}
    for case in cases:
        ident = case["id"]
        if ident not in refs or ident not in got or difference(refs[ident], got[ident], case.get("ordered", False)):
            report["skipped_non_equivalent_cases"].append(ident)
            continue
        if refs[ident]["kind"] == 4:
            continue
        expected, actual = refs[ident]["rows"], got[ident]["rows"]
        if not case.get("ordered", False):
            expected, actual = sorted(expected, key=row_key), sorted(actual, key=row_key)
        for er, ar in zip(expected, actual):
            if len(er.get("point_bits", [])) != len(er["points"]) or len(ar.get("point_bits", [])) != len(ar["points"]):
                raise ValueError(f"missing raw point bits: {ident}")
            for ep, ap, abits, bbits in zip(er["points"], ar["points"], er["point_bits"], ar["point_bits"]):
                a, b = float(ep[1]), float(ap[1])
                for number, raw in [(a, abits), (b, bbits)]:
                    if len(raw) != 16:
                        raise ValueError(f"invalid raw point bits: {ident}")
                    decoded = struct.unpack(">d", bytes.fromhex(raw))[0]
                    if math.isfinite(number):
                        valid = raw == struct.pack(">d", number).hex()
                    else:
                        valid = same_number(number, decoded)
                    if not valid:
                        raise ValueError(f"raw bits disagree with numeric output: {ident}")
                if not math.isfinite(a) or not math.isfinite(b):
                    report["nonfinite_values"] += 1
                else:
                    report["finite_values"] += 1
                if abits != bbits:
                    report["bitwise_differences"].append({"id": ident, "labels": er["labels"],
                        "time_ms": ep[0], "reference": ep[1], "candidate": ap[1],
                        "reference_bits": abits, "candidate_bits": bbits})
    return report

if __name__ == "__main__":
    reference, candidate, cases = (json.load(open(p)) for p in sys.argv[1:])
    failures = compare(reference, candidate, cases)
    print(json.dumps({"passed": len(cases) - len(failures), "total": len(cases), "failures": failures}, indent=2))
    sys.exit(bool(failures))
