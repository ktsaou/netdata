#!/usr/bin/env python3
"""Compare typed results without hiding duplicate labelsets or point/order errors."""
import json
import math
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

if __name__ == "__main__":
    reference, candidate, cases = (json.load(open(p)) for p in sys.argv[1:])
    failures = compare(reference, candidate, cases)
    print(json.dumps({"passed": len(cases) - len(failures), "total": len(cases), "failures": failures}, indent=2))
    sys.exit(bool(failures))
