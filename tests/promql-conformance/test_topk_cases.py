"""Declared inputs retain selection boundaries, scope controls and alias parents."""
import unittest
from itertools import pairwise

from topk_cases import cases


class SelectionCases(unittest.TestCase):
    def test_roster_is_repeatable_and_reaches_source_boundaries(self):
        first = cases()
        self.assertEqual(first, cases())
        self.assertEqual(len(first), len({row["request"]["id"] for row in first}))
        queries = {row["request"]["query"] for row in first}
        self.assertTrue({"topk(0,m)", "bottomk(NaN,m)", "topk(time()+1,m @0)",
            "topk(scalar(p),m @0)", 'label_join(topk(2,{a=~".+"}),"new","","a")'} <= queries)
        lengths = {len(row["dataset"]["series"]) for row in first
                   if row["request"]["id"].startswith("topk:threshold-")}
        self.assertTrue({12, 13, 49, 50, 51, 1023, 1024, 1025} <= lengths)
        contextual = [row["request"] for row in first
                      if row["request"]["id"].startswith("topk:contextual-parameter-")]
        self.assertEqual({row["profile"] for row in contextual}, {"upstream-tests"})
        self.assertEqual(len(contextual), 32)
        self.assertTrue(any(row["request"]["start_ms"] == 1000
                            and "predict_linear(p[2s] @0,1)" in row["request"]["query"]
                            for row in first))

    def test_inputs_keep_strict_timelines_both_profiles_and_independent_faults(self):
        rows = cases()
        self.assertEqual({row["request"]["profile"] for row in rows},
                         {"server-default", "upstream-tests"})
        checked = set()
        for row in rows:
            ident = row["request"]["id"].split(":")[1]
            if ident not in checked:
                for record in row["dataset"]["series"]:
                    times = [point["time_ms"] for point in record["points"]]
                    self.assertTrue(all(left < right for left, right in pairwise(times)))
                    for point in record["points"]:
                        self.assertEqual(point.keys(), {"time_ms", "kind", point["kind"]})
                checked.add(ident)
        selected = next(row for row in rows if "set-fault-topk-scalar(p)-m" in row["request"]["id"])
        original = next(row for row in rows if "set-fault-topk-scalar(p)-p" in row["request"]["id"])
        selected["faults"]["m"]["set_error"] = "mutated"
        self.assertEqual(original["faults"], {"p": {"set_error": "selection fault"}})


if __name__ == "__main__":
    unittest.main()
