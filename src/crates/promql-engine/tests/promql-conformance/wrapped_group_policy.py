"""Exact Decision9 qualification for declared dense Float child-map boundaries."""

import base64
from copy import deepcopy

from compare import labels
from group_contributor_cases import OPERATORS


def declarations(inputs):
    by_id = {case["request"]["id"]: case for case in inputs}
    if len(by_id) != len(inputs):
        raise ValueError("policy input identities must be unique")
    declared, children = [], []
    selector = '{cluster!="",__name__=""}'
    for profile in ("server-default", "upstream-tests"):
        for operator in OPERATORS:
            for shape in ("mapped-inner", "derived-fallback"):
                mapped = shape == "mapped-inner"
                ident = (f"wrapped-contributors-{profile}-{operator}-mapped-inner-abs" if mapped
                         else f"wrapped-derived-fallback-{profile}-{operator}")
                child = "abs(m_total*2)" if mapped else "last_over_time((n_total*1)[3s:1s])"
                left, right = (child, "n_total") if mapped else ("m_total", child)
                def parent(first, second):
                    return (f"abs({operator} by(cluster,zone)({first}) / on(cluster,zone) "
                            f"{operator} by(cluster,zone)({second}))")
                original = by_id[ident]
                request = original["request"]
                if (request["query"] != parent(left, right) or request["profile"] != profile
                        or request["mode"] != "range" or original.get("faults")):
                    raise ValueError("declared dense child boundary no longer matches its parent")
                case = deepcopy(original)
                case["request"].update(id="inner:" + ident, query=child)
                children.append(case)
                declared.append({"id": ident, "child": child,
                    "retained_metric": "n_total" if mapped else "m_total",
                    "outer": parent(selector, right) if mapped else parent(left, selector)})
    return declared, children


def injected_inputs(declared, children, results):
    output = []
    for declaration, original, result in zip(declared, children, results["results"], strict=True):
        ident = declaration["id"]
        request = original["request"]
        if (result["id"] != "inner:" + ident or result["kind"] != "matrix"
                or result["warnings"] or result["infos"] or result.get("error")):
            raise ValueError("policy injection child has unsupported semantics")
        if request["step_ms"] <= 0 or request["end_ms"] < request["start_ms"]:
            raise ValueError("policy injection requires a dense increasing output clock")
        ticks = list(range(request["start_ms"], request["end_ms"] + 1, request["step_ms"]))
        rows = sorted(deepcopy(result["rows"]), key=labels)
        identities = [labels(row) for row in rows]
        if not rows or len(set(identities)) != len(rows):
            raise ValueError("policy child labels must be unique and nonempty")
        for row, identity in zip(rows, identities, strict=True):
            decoded = dict(identity)
            if (b"__name__" in decoded or not decoded.get(b"cluster")
                    or any(point["kind"] != "float" for point in row["points"])
                    or [point["time_ms"] for point in row["points"]] != ticks
                    or any(set(point) != {"time_ms", "kind", "float"} for point in row["points"])):
                raise ValueError("policy injection requires ordinary dense Float points and dropped names")
        retained = []
        wanted = declaration["retained_metric"].encode()
        for row in original["dataset"]["series"]:
            decoded = {base64.b64decode(label["name"]): base64.b64decode(label["value"])
                       for label in row["labels"]}
            if decoded.get(b"__name__") == wanted:
                retained.append(deepcopy(row))
        if not retained:
            raise ValueError("policy original opposite metric is missing")
        output.append({"request": request | {"id": "outer:" + ident, "query": declaration["outer"]},
                       "dataset": original["dataset"] | {"series": rows + retained}})
    return output


def expected_policy(original, declared, outer):
    expected = deepcopy(original)
    replacements = {item["id"]: result | {"id": item["id"]}
                    for item, result in zip(declared, outer["results"], strict=True)}
    if any(result["id"] != "outer:" + item["id"]
           for item, result in zip(declared, outer["results"], strict=True)):
        raise ValueError("policy outer original identities changed")
    found = set()
    for index, result in enumerate(expected["results"]):
        if result["id"] in replacements:
            found.add(result["id"])
            expected["results"][index] = replacements[result["id"]]
    if found != replacements.keys():
        raise ValueError("policy original result roster is incomplete")
    return expected
