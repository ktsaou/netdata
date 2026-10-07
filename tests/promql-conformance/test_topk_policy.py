"""Policy controls alter only declared map boundaries and preserve exact child points."""
import base64
import unittest
from copy import deepcopy

from topk_policy import declarations, expected_policy, injected_inputs


def label(value):
    return {"name": "eg==", "value": base64.b64encode(value.encode()).decode()}


def result(ident, rows):
    return {"id": ident, "kind": "matrix", "rows": rows, "warnings": [], "infos": []}


def row(value, bits="8000000000000000"):
    return {"labels": [label(value)], "points": [{"time_ms": 1000, "kind": "float", "float": bits}]}


def inputs():
    output = []
    for operator in ("topk", "bottomk"):
        for profile in ("server-default", "upstream-tests"):
            queries = [(f"parents-{operator}", 4, 3, f"{operator}(1,{operator}(3,m))")]
            queries += [(f"body-producers-{operator}", 3, window, f'{operator}(2,count_values("v",m))')
                        for window in range(4)]
            queries += [(f"body-producers-{operator}", 5, 3, f"{operator}(2,abs(m))")]
            for family, query_index, window, query in queries:
                output.append({"request": {"id": f"topk:{family}:{profile}:{query_index}:{window}",
                    "query": query, "profile": profile, "mode": "range", "start_ms": 1000,
                    "end_ms": 2000, "step_ms": 1000, "lookback_ms": 1},
                    "dataset": {"schema_version": 1, "id": "policy-control", "series": [row("z")]}})
    return output


class SelectionPolicy(unittest.TestCase):
    def test_controls_cover_both_operators_and_preserve_the_entire_child_scope(self):
        original = inputs()
        retained = deepcopy(original)
        declared, children = declarations(original)
        self.assertEqual(len(declared), 24)
        self.assertEqual(original, retained)
        self.assertEqual({spec[1] for spec in declared},
                         {"topk(3,m)", "bottomk(3,m)", 'count_values("v",m)', "abs(m)"})
        for spec, child in zip(declared, children, strict=True):
            source = next(item for item in original if item["request"]["id"] == spec[0])
            self.assertEqual(child["request"], source["request"] | {"id": "inner:" + spec[0], "query": spec[1]})
            self.assertEqual(child["dataset"], source["dataset"])
        original[0]["request"]["query"] = "topk(2,m)"
        with self.assertRaisesRegex(ValueError, "map boundary"):
            declarations(original)

    def test_injection_orders_only_rows_and_retains_zero_payload_and_time(self):
        declared, children = declarations(inputs())
        inner = {"results": [result(child["request"]["id"], [row("z"), row("a", "7ff8000000000023")])
                             for child in children]}
        retained = deepcopy(inner)
        injected = injected_inputs(declared, children, inner)
        self.assertEqual(inner, retained)
        self.assertEqual(injected[0]["dataset"]["series"], [row("a", "7ff8000000000023"), row("z")])
        self.assertEqual(injected[0]["dataset"]["schema_version"], 1)
        self.assertEqual(injected[0]["dataset"]["id"], "policy-control")
        for mutation in ("histogram", "repeat", "warning"):
            changed = deepcopy(inner)
            point = changed["results"][0]["rows"][0]["points"][0]
            if mutation == "histogram":
                point["kind"] = "float_histogram"
            elif mutation == "repeat":
                changed["results"][0]["rows"][0]["points"].append(deepcopy(point))
            else:
                changed["results"][0]["warnings"] = ["aWdub3JlZA=="]
            with self.assertRaises(ValueError):
                injected_inputs(declared, children, changed)

    def test_policy_results_are_confined_and_keep_the_raw_reference_unchanged(self):
        declared, _ = declarations(inputs())
        count_ids = [f"topk:parents-{operator}:{profile}:5:{window}"
                     for operator in ("topk", "bottomk")
                     for profile in ("server-default", "upstream-tests") for window in (0, 1)]
        original = {"schema_version": 1, "results": [result(spec[0], [row("z")]) for spec in declared]
                    + [result(ident, [row("z"), row("a")]) for ident in count_ids]
                    + [result("outside-boundary", [row("z"), row("a")])]}
        retained = deepcopy(original)
        outer = {"results": [result("outer:" + spec[0], [row("a")]) for spec in declared]}
        policy, changed = expected_policy(original, declared, outer)
        self.assertEqual(original, retained)
        self.assertEqual(len(changed), 32)
        self.assertEqual(policy["results"][-1], original["results"][-1])
        self.assertEqual(policy["results"][0], result(declared[0][0], [row("a")]))
        self.assertEqual(policy["results"][24]["rows"], [row("a"), row("z")])
        original["results"].pop(24)
        with self.assertRaisesRegex(ValueError, "incomplete"):
            expected_policy(original, declared, outer)


if __name__ == "__main__":
    unittest.main()
