"""Pure alert wrappers around ordinary grouped joins and retained fallbacks."""

from copy import deepcopy
import base64

import group_contributor_cases as contributors
import mapped_rate_cases
from mapped_rate_cases import _case


def cases():
    output = []
    for original in contributors.cases():
        source = original["request"]
        if "-standalone-" in source["id"]:
            continue
        expression = source["query"]
        wrappers = {"abs": f"abs({expression})"}
        dense = any(source["id"].endswith(f"-{operator}-raw") for operator in contributors.OPERATORS)
        if dense:
            wrappers |= {
                "paren": f"({expression})",
                "positive": f"+({expression})",
                "negative": f"-({expression})",
                "keep": f"({expression}) > .25",
                "drop": f"({expression}) > 1e30",
                "bool": f"({expression}) > bool .75",
                "scalar-left": f".25 < ({expression})",
                "clamp": f"clamp_min(({expression}),.25)",
                "time": f"({expression}) > (time()-3)",
                "scalar-math": f"({expression}) * (time()+1)^2",
                "round": f"round(({expression}),.25)",
                "scalar-fallback": f"clamp_min(({expression}),scalar(vector(time())))",
            }
        elif source["id"].endswith("-raw"):
            wrappers |= {
                "filter": f"({expression}) > .75",
                "bool": f"({expression}) > bool .75",
            }
        for name, expression in wrappers.items():
            case = deepcopy(original)
            case["request"] |= {"id": f"wrapped-{source['id']}-{name}", "query": expression}
            output.append(case)
    for profile in ("server-default", "upstream-tests"):
        for operator in contributors.OPERATORS:
            left = f"{operator} by(cluster,zone)(m_total)"
            right = f"{operator} by(cluster,zone)(n_total)"
            for wrapper in ("abs", "paren"):
                comparison = f"{left} > on(cluster,zone) ({right}-10)"
                expression = f"abs({comparison})" if wrapper == "abs" else f"({comparison})"
                output.append(_case(f"wrapped-inner-filter-{profile}-{operator}-{wrapper}",
                                    expression, contributors._records(), profile))
            records = contributors._records()
            for record in records:
                labels = {base64.b64decode(label["name"]): base64.b64decode(label["value"])
                          for label in record["labels"]}
                if labels[b"__name__"] == b"n_total" and labels[b"zone"] == b"2":
                    for label in record["labels"]:
                        if base64.b64decode(label["name"]) == b"zone":
                            label["value"] = base64.b64encode(b"unmatched").decode()
            output.append(_case(f"wrapped-unmatched-{profile}-{operator}",
                                f"abs({left} / on(cluster,zone) {right})", records, profile))
            derived = f"{operator} by(cluster,zone)(last_over_time((n_total*1)[3s:1s]))"
            output.append(_case(f"wrapped-derived-fallback-{profile}-{operator}",
                                f"abs({left} / on(cluster,zone) {derived})",
                                contributors._records(), profile))
    for original in mapped_rate_cases.cases():
        if original["request"]["id"].startswith("wrapped-rate-"):
            continue
        case = deepcopy(original)
        request = original["request"]
        case["request"] |= {"id": f"wrapped-leaf-{request['id']}", "query": f"abs({request['query']})"}
        output.append(case)
    return output


def performance_cases():
    output = []
    for original in contributors.performance_cases():
        request = original["request"]
        wrappers = {"control": request["query"]}
        if "-join-" in request["id"]:
            wrappers |= {"filter": f"({request['query']}) > .25", "abs": f"abs({request['query']})"}
        for name, expression in wrappers.items():
            case = deepcopy(original)
            case["request"] |= {"id": f"wrapped-{request['id']}-{name}", "query": expression}
            output.append(case)
    return output
