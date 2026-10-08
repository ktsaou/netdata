"""Literal portable arithmetic inputs, independent of candidate results."""
from __future__ import annotations

import base64
import copy

VALUES = [
    "7ff8000000000001", "7ff8000000000042", "fff8000000000043",
    "7ff0000000000044", "fff0000000000045", "0000000000000000",
    "8000000000000000", "3ff0000000000000", "bff0000000000000",
    "7ff0000000000000", "fff0000000000000",
]
ZERO, ONE, HALF, THREE, FOUR = (
    "0000000000000000", "3ff0000000000000", "3fe0000000000000",
    "4008000000000000", "4010000000000000",
)
MAX = "7fefffffffffffff"
FUNCTIONS = [
    "sum_over_time", "avg_over_time", "stdvar_over_time", "stddev_over_time",
    "delta", "rate", "increase", "idelta", "irate", "double_exponential_smoothing",
]


def function_cases(functions, scope=None):
    output = []
    for function in functions:
        for i, a in enumerate(VALUES):
            for j, b in enumerate(VALUES):
                row = {"id": f"function-{scope + '-' if scope else ''}{function}-{i:02}-{j:02}"}
                if scope is not None:
                    row["scope"] = scope
                row.update(function=function, a=a, b=b)
                output.append(row)
    return output


def v1():
    native = []
    for shape in ["matched", "new", "reduce", "threshold", "custom-match", "custom-reconcile"]:
        for op in ["add", "sub", "kahan-add-final"]:
            for i, a in enumerate(VALUES):
                for j, b in enumerate(VALUES):
                    native.append({"id": f"native-{shape}-{op}-{i:02}-{j:02}",
                                   "shape": shape, "op": op, "a": a, "b": b})
    reduction = [
        {"id": f"reduction-{op}-{i:02}-{j:02}", "op": op, "a": a, "b": b}
        for op in ["copy", "reduce"]
        for i, a in enumerate(VALUES) for j, b in enumerate(VALUES)
    ]
    return {"values": VALUES, "native": native, "reduction": reduction,
            "functions": function_cases(FUNCTIONS)}


def v2():
    functions = function_cases(FUNCTIONS, "endpoints")
    functions += function_cases(FUNCTIONS[:4], "overflow")
    functions += function_cases(["rate", "increase", "irate"], "start-reset")
    functions += function_cases(["rate", "increase"], "start-first")
    for i, a in enumerate(VALUES[:5]):
        for j, b in enumerate(VALUES[:5]):
            functions.append({"id": f"function-nan-factor-smoothing-{i:02}-{j:02}",
                              "scope": "nan-factor", "function": "double_exponential_smoothing",
                              "a": a, "b": b})
    exponents = [
        -9223372036854775808, -9223372036854775756, -4294967297, -4294967296,
        -2147483649, -2147483648, -1075, -1074, -1023, -1022, -1, 0, 1,
        1023, 1024, 2147483647, 2147483648, 4294967297,
        9223372036854775755, 9223372036854775807,
    ]
    widths = [
        {"id": f"ldexp-{i:02}-{j:02}", "mode": "ldexp", "a": a, "exponent": exponent}
        for i, a in enumerate(VALUES + ["0000000000000001", "0010000000000000", HALF])
        for j, exponent in enumerate(exponents)
    ]
    compact = [-9223372036854775808, -9223372036854775807, -4294967293,
               -2147483649, -1, 0, 1, 3, 4, 4294967295, 4294967299, 9223372036854775807]
    widths += [{"id": f"compact-{i:02}", "mode": "compact", "exponent": e}
               for i, e in enumerate(compact)]
    widths += [{"id": f"cumulative-{op}-{i:02}", "mode": "cumulative", "op": op, "offset": offset}
               for op in ["add", "sub"] for i, offset in enumerate([0, 1, 2147483646, 2147483647])]
    decimal = [
        ("go-atof-0214", "2." + "2" * 4000 + "e+1", "src/internal/strconv/atof_test.go:283"),
        ("go-atof-0217", "0x2." + "2" * 4000 + "p221", "src/internal/strconv/atof_test.go:286"),
    ]
    for first_id, prefix in [(4, "0x1.00000000000008"), (12, "0x1.fffffffffffff7fff"),
                             (20, "0x0.00000000000008"), (28, "-0x1.00000000000008")]:
        for sticky in [0, 1]:
            decimal.append((f"hex-sticky-{first_id + sticky:05}",
                            prefix + "0" * 1024 + ("1" if sticky else "") + "p0", "hex-sticky"))
    ordinal = 0
    for zeros, exponents in [(799, [799, 800, 10000, 10001, 100000, 999990]),
                             (800, [800, 801, 10000, 10001, 100000, 999990]),
                             (801, [801, 802, 10000])]:
        for exponent in exponents:
            for sign in ["", "-"]:
                decimal.append((f"decimal-zero-tail-cancel-{ordinal:05}",
                                sign + "1" + "0" * zeros + f"e-{exponent}", "decimal-zero-tail-cancel"))
                ordinal += 1
    widths += [{"id": f"decimal-{id_}", "mode": "decimal", "text": text, "origin": origin}
               for id_, text, origin in decimal]
    return {"functions": functions, "widths": widths}


def histogram(a=ZERO, b=ZERO, shape="empty"):
    h = {"reset_hint": 3, "schema": 0, "zero_threshold": "3f50624dd2f1a9fc",
         "zero_count": ZERO, "count": ONE, "sum": ONE,
         "positive_spans": None, "negative_spans": None,
         "positive_buckets": None, "negative_buckets": None, "custom_values": None}
    if shape in ["positive", "both", "zero-nan-positive"]:
        h.update(positive_spans=[{"offset": 0, "length": 2}], positive_buckets=[a, b])
    if shape in ["negative", "both"]:
        h.update(negative_spans=[{"offset": 0, "length": 2}], negative_buckets=[b, a])
    if shape in ["custom", "custom-nan-bound"]:
        h.update(schema=-53, zero_threshold=ZERO,
                 positive_spans=[{"offset": 0, "length": 3}], positive_buckets=[b, b, b],
                 custom_values=[HALF, a if shape == "custom-nan-bound" else ONE])
    if shape in ["zero-nan", "zero-nan-positive"]:
        h.update(zero_threshold=a, zero_count=ONE)
    return h


def histogram_inputs():
    output = []
    for i, lower in enumerate(VALUES):
        for j, upper in enumerate(VALUES):
            for k, value in enumerate(VALUES):
                for linear in [False, True]:
                    output.append({"ID": f"fraction-{int(linear)}-{i:02}-{j:02}-{k:02}",
                                   "Mode": "fraction", "Lower": lower, "Upper": upper,
                                   "Value": value, "Linear": linear})
    rhs = VALUES[:5] + [ZERO, "3fe8000000000000", "bfe8000000000000",
                        "3ff8000000000000", "bff8000000000000", VALUES[-2], VALUES[-1]]
    for i, a in enumerate(VALUES[:5] + [HALF]):
        for j, b in enumerate(VALUES[:5] + [ONE]):
            for shape in ["positive", "negative", "both", "custom", "custom-nan-bound",
                          "zero-nan", "zero-nan-positive"]:
                for k, value in enumerate(rhs):
                    for upper in [False, True]:
                        output.append({"ID": f"trim-{shape}-{int(upper)}-{i:02}-{j:02}-{k:02}",
                                       "Mode": "trim", "Histogram": histogram(a, b, shape),
                                       "Value": value, "UpperTrim": upper})
    return output


def label_bytes(value):
    return base64.b64encode(value).decode("ascii")


def series(name, values, times=None, id_="0", starts=None):
    points = []
    if times is None:
        times = [0] * len(values)
    for i, (value, time) in enumerate(zip(values, times)):
        point = {"time_ms": time}
        if starts is not None:
            point["start_time_ms"] = starts[i]
        point.update(kind="float", float=value)
        points.append(point)
    return {"labels": [{"name": label_bytes(b"__name__"), "value": label_bytes(name.encode())},
                       {"name": label_bytes(b"id"), "value": label_bytes(id_.encode())}], "points": points}


def query_case(id_, owner, query, series_):
    return {"id": id_, "owner": owner, "query": query,
            "data": {"schema_version": 1, "series": series_}}


def engine_inputs():
    output = []
    for i, a in enumerate(VALUES):
        for j, b in enumerate(VALUES):
            for scope, values in [("pair", [a, b]), ("interior", [ONE, a, b, THREE]),
                                  ("overflow", [MAX, MAX, a, b])]:
                data = [series("m", [value], id_=str(k)) for k, value in enumerate(values)]
                for function in ["sum", "avg", "stdvar", "stddev"]:
                    output.append(query_case(f"aggregation-{scope}-{function}-{i:02}-{j:02}",
                                             f"aggregation:{function}", f"{function}(m)", copy.deepcopy(data)))
            for operator, symbol in [("add", "+"), ("sub", "-"), ("mul", "*"), ("div", "/")]:
                data = [series("a", [a]), series("b", [b])]
                output.append(query_case(f"scalar-{operator}-{i:02}-{j:02}", f"scalar:{operator}",
                                         f"scalar(a){symbol}scalar(b)", copy.deepcopy(data)))
                output.append(query_case(f"vector-{operator}-{i:02}-{j:02}", f"vector:{operator}",
                                         f"a{symbol}b", copy.deepcopy(data)))
            for modifier in ["anchored", "smoothed"]:
                for function in ["delta", "rate", "increase"]:
                    output.append(query_case(f"extended-{modifier}-{function}-{i:02}-{j:02}",
                                             f"extended:{modifier}:{function}", f"{function}(m[3s] {modifier})",
                                             [series("m", [a, ONE, THREE, b], [-4000, -2000, -1000, 1000])]))
    return output


def range_inputs():
    output = []
    for prefix, rows in [("v1", v1()["functions"]), ("v2", v2()["functions"])]:
        for row in rows:
            a, b, scope, function = row["a"], row["b"], row.get("scope", ""), row["function"]
            values, starts = [ONE, a, b, THREE], [0, 0, 0, 0]
            if scope == "endpoints":
                values = [a, ONE, THREE, b]
            elif scope == "overflow":
                values = [MAX, MAX, a, b]
            elif scope == "start-reset":
                starts = [-4000, -2500, -1500, -500]
            elif scope == "start-first":
                starts = [-3500, 0, 0, 0]
            elif scope == "nan-factor":
                values = [ONE, "4000000000000000", THREE, FOUR]
            data = [series("m", values, [-3000, -2000, -1000, 0], starts=starts)]
            query = f"{function}(m[10s])"
            if function == "double_exponential_smoothing":
                if scope == "nan-factor":
                    data += [series("sf", [a], starts=[0]), series("tf", [b], starts=[0])]
                    query = f"{function}(m[10s],scalar(sf),scalar(tf))"
                else:
                    query = f"{function}(m[10s],0.4,0.6)"
            output.append(query_case(prefix + "-" + row["id"], f"range:{scope}:{function}", query, data))
    for i, a in enumerate(VALUES[:5]):
        for j, b in enumerate(VALUES[:5]):
            for side in ["left", "right"]:
                values = [a, b, THREE, FOUR] if side == "left" else [ONE, "4000000000000000", a, b]
                for function in ["delta", "rate", "increase"]:
                    output.append(query_case(f"interpolate-{side}-{function}-{i:02}-{j:02}",
                                             f"extended:interpolate:{side}:{function}", f"{function}(m[3s] smoothed)",
                                             [series("m", values, [-4000, -2000, -1000, 1000], starts=[0] * 4)]))
    return output


def extra_engine_inputs():
    output = []
    for i, a in enumerate(VALUES):
        for j, b in enumerate(VALUES):
            h = histogram()
            h.update(count=b, sum=a)
            h_series = series("h", [ONE])
            h_series["points"] = [{"time_ms": 0, "kind": "float_histogram", "float_histogram": h}]
            output.append(query_case(f"histogram-avg-{i:02}-{j:02}", "histogram:avg",
                                     "histogram_avg(h)", [h_series]))
            for function in ["irate", "idelta"]:
                output.append(query_case(f"instant-{function}-{i:02}-{j:02}", f"instant:{function}",
                                         f"{function}(m[10s])", [series("m", [a, b], [-2000, -1000])]))
    for i, a in enumerate(VALUES):
        for name, query in [("neg-vector", "-m"), ("plus-vector", "+m"),
                            ("neg-scalar", "-scalar(m)"), ("plus-scalar", "+scalar(m)")]:
            output.append(query_case(f"unary-{name}-{i:02}", f"unary:{name}", query, [series("m", [a])]))
    return output


def decimal_inputs():
    shifts = [-1075, -1074, -120, -61, -60, -53, -29, -28, -27, -1, 0, 1,
              27, 28, 29, 53, 60, 61, 120]
    patterns = []
    for n in [50, 780, 790, 795, 799, 800, 810]:
        for name, digits in [("nines", "9" * n), ("ones", "1" * n),
                             ("ascending", ("1234567890" * ((n + 9) // 10))[:n]),
                             ("fiveplus", "5" + "0" * (n - 2) + "1")]:
            patterns.append((f"{name}-{n}", "0." + digits))
    shift_cases = [{"id": f"shift-{name}-{shift}", "decimal": text, "shift": shift}
                   for name, text in patterns for shift in shifts]
    # Preserve this retained literal as an ordinary parser control, not an exact binary tie.
    long_control = '2.47032822920623272088284396434110686182529901307162382212792841250337753635104375932649918180817996189898282347722858865463328355177969898199387398005390939063150356595155702263922908583924491051844359318028499365361525003193704576782492193656236698636584807570015857692699037063119282795585513329278334338409351978015531246597263579574552766465272827220056374006485499977096599470454020828166226237857393450736339007967761930577506740176324673600968951340535537458516661134223766678604162159680461914467291840300530057530849048765391711386591646239524912623653881879636239373280423891018672348497668235089863388587925628302755995657524455507255189313690836254779186948667994968324049705821028513185451396213837722826145437693412532098591327667236328125e-324'
    parse_values = ["0", "-0", "1", "-1", "0.5", "0.1", "1e-324", long_control,
                    "4.9406564584124654e-324", "2.2250738585072011e-308",
                    "2.2250738585072012e-308", "1.7976931348623157e308",
                    "1.7976931348623158e308", "1.7976931348623159e308"]
    parse_values += [text for _, text in patterns]
    parse_values += ["1." + "0" * n + "1e-300" for n in [780, 799, 800, 810]]
    parse_cases = [{"id": f"parse-{i:03}", "decimal": text} for i, text in enumerate(parse_values)]
    half = "1.00000000000000011102230246251565404236316680908203125"
    boundary = [("exact", half), ("above-retained", half + "0" * 780 + "1"),
                ("below-retained", half[:-1] + "4" + "9" * 781), ("exact-padded", half + "0" * 781)]
    for sign in ["", "negative"]:
        for name, text in boundary:
            parse_cases.append({"id": f"fallback-boundary-{sign or 'positive'}-{name}",
                                "decimal": ("-" if sign else "") + text})
    return {"shift_cases": shift_cases, "parse_cases": parse_cases}


def rad_deg_inputs():
    return [query_case(f"float-{function}-{i:02}", f"float:{function}", f"{function}(m)",
                       [series("m", [value])])
            for function in ["rad", "deg"] for i, value in enumerate(VALUES)]



def pointwise_inputs():
    functions = ["abs", "ceil", "floor", "exp", "sqrt", "ln", "log2", "log10",
                 "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh",
                 "asinh", "acosh", "atanh", "rad", "deg", "sgn"]
    queries = [(function, f"{function}(m)") for function in functions]
    queries += [("round", "round(m)"), ("round-step", "round(m,0.1)"),
                ("clamp", "clamp(m,-1,1)"), ("clamp_min", "clamp_min(m,-1)"),
                ("clamp_max", "clamp_max(m,1)")]
    queries += [(function, f"{function}(m)") for function in
                ["days_in_month", "day_of_month", "day_of_week", "day_of_year",
                 "hour", "minute", "month", "year"]]
    return [query_case(f"pointwise-{name}-{i:02}", f"float:{name}", query,
                       [series("m", [value])])
            for name, query in queries for i, value in enumerate(VALUES)]


def literal_unary_inputs():
    queries = ["NaN", "+NaN", "-NaN", "--NaN", "-(-NaN)", "-(NaN)", "-(-(-NaN))",
               "+(-NaN)", "-Inf", "-0", "+0", "-1", "(-(NaN))", "(-NaN)",
               "-nan", "+nan", "nan", "NAN", "-NAN"]
    output = [query_case(f"literal-unary-{i:02}", "parser:unary:number", query, [])
              for i, query in enumerate(queries)]
    h = histogram()
    # A valid one-observation zero-bucket gauge; equality compares raw histogram fields.
    h.update(zero_count=ONE, sum=ZERO)
    h_series = series("h", [ONE])
    h_series["points"] = [{"time_ms": 0, "kind": "float_histogram", "float_histogram": h}]
    comparisons = [
        "(h * -NaN) == bool (h * NaN)", "(h * -(NaN)) == bool (h * NaN)",
        "(h * --NaN) == bool (h * NaN)", "(h * -(-NaN)) == bool (h * NaN)",
        "(h * -NaN) == bool (h * -(NaN))", "(h * -(-(-NaN))) == bool (h * -NaN)",
    ]
    output += [query_case(f"literal-histogram-equality-{i:02}", "parser:unary:histogram-equality",
                          query, [copy.deepcopy(h_series)])
               for i, query in enumerate(comparisons)]
    return output



def parser_coupled_inputs():
    h = histogram()
    h.update(zero_count=ONE, sum=ZERO)
    h_series = series("h", [ONE])
    h_series["points"] = [{"time_ms": 0, "kind": "float_histogram", "float_histogram": h}]
    data = [series("m", [ONE]), h_series]
    queries = [
        "missing + fill_left(-NaN) m", "m + fill_right(-NaN) missing",
        "missing + fill(-NaN) m",
        "(h * fill_right(-NaN) missing) == bool (h * -NaN)",
        "(missing * fill_left(-NaN) h) == bool (h * -NaN)",
        "(missing * fill(-NaN) h) == bool (h * -NaN)",
    ]
    output = [query_case(f"parser-fill-{i:02}", "parser:fill:negative-nan", query, copy.deepcopy(data))
              for i, query in enumerate(queries)]
    negative = [series("m", ["bff0000000000000"]), copy.deepcopy(h_series)]
    output += [query_case(f"pow-generated-invalid-{i:02}", "math:pow:generated-invalid", query,
                          copy.deepcopy(negative))
               for i, query in enumerate(["scalar(m)^-0.5", "scalar(m)^0.5",
                                          "(h * (scalar(m)^-0.5)) == bool (h * (scalar(m)^0.5))"])]
    return output


def decimal_parse_inputs():
    return {"widths": [{"id": row["id"], "mode": "decimal", "text": row["decimal"],
                        "origin": "Go1.27.1:internal/strconv/atof.go"}
                       for row in decimal_inputs()["parse_cases"]]}


def arithmetic_route_inputs():
    return [query_case("route-fma", "runtime:FMA-on", "0.5^1.5", [])]


PANELS = {"v1": v1, "v2": v2, "histogram": histogram_inputs,
          "engine": engine_inputs, "range": range_inputs, "extra": extra_engine_inputs,
          "raddeg": rad_deg_inputs, "pointwise": pointwise_inputs, "literal": literal_unary_inputs,
          "coupled": parser_coupled_inputs, "decimal": decimal_parse_inputs, "route": arithmetic_route_inputs}
