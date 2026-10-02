#!/usr/bin/env python3
"""Frozen synthetic common-use workload; values are independent of candidate output."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
regular = list(range(0, 601, 60))
series = []

def add(name, labels, points):
    series.append({"labels": {"__name__": name, **labels}, "points": [[t * 1000, v] for t, v in points]})

for instance, zone, mode, value in [("a", "east", "user", 2), ("a", "east", "system", 4), ("b", "west", "user", 6), ("b", "west", "system", 8)]:
    labels = dict(instance=instance, job="web", mode=mode, zone=zone)
    for name in ["system.cpu", "system_cpu"]:
        add(name, labels, [(t, value) for t in regular])
    add("peer", {**labels, "zone": "other"}, [(t, 1) for t in regular])
for instance, zone, tier, value in [("a", "east", "gold", 10), ("b", "west", "silver", 20)]:
    add("limits", dict(instance=instance, job="web", zone=zone, tier=tier), [(t, value) for t in regular])
add("trend", {"case": "ramp"}, [(t, t / 60) for t in regular])
add("counter_total", {"case": "linear"}, [(t, 100 + t) for t in regular])
for case, tail in [("reset", [100, 140, 20, 60, 100]), ("last_reset", [100, 160, 220, 280, 20])]:
    add("counter_total", {"case": case}, [(t, 100 + t) for t in regular if t <= 300] + list(zip(regular[6:], tail)))
for case, points in [("gap", [(480, 100), (600, 220)]), ("zero", [(480, 0), (600, 120)]), ("single", [(600, 42)])]:
    add("counter_total", {"case": case}, points)
for le, value in [("0.1", 2), ("0.5", 6), ("1", 8), ("+Inf", 10)]:
    add("latency_bucket", dict(job="web", le=le), [(t, value) for t in regular])
for item, value in [("a", "NaN"), ("b", 2), ("c", "-Inf"), ("d", "+Inf")]:
    add("special", {"item": item}, [(t, value) for t in regular])

queries = [
    '{"system.cpu",instance="a",mode="user"}', 'system_cpu',
    'system_cpu{mode="user"}', 'system_cpu{mode!="user"}',
    'system_cpu{mode=~"user|system"}', 'system_cpu{mode!~"system"}',
    'system_cpu{mode=~"sys"}', 'system_cpu{missing=""}', '{__name__=~"system[._]cpu"}',
    '2 + 3 * 4', '(7 - 3) / 2', '(7 % 4) ^ 2', 'system_cpu + 2',
    '10 - system_cpu', 'system_cpu + system_cpu', 'system_cpu > 4',
    'system_cpu <= bool 4', '(system_cpu >= bool 4) + (system_cpu < bool 8)',
    '(system_cpu == bool 4) + (system_cpu != bool 4)',
    'system_cpu{mode="user"} and system_cpu{instance="a"}',
    'system_cpu{mode="user"} or system_cpu{instance="a"}', 'system_cpu unless system_cpu{mode="system"}',
    'sum by(instance)(system_cpu)', 'avg without(mode)(system_cpu)', 'min(system_cpu)',
    'max(system_cpu)', 'count(system_cpu)', 'topk(2,system_cpu)', 'bottomk(2,system_cpu)',
    'sum by(__name__)({__name__=~"system[._]cpu"})',
    'system_cpu / on(instance) group_left(tier) limits',
    'limits / on(instance) group_right(tier) system_cpu', 'system_cpu + ignoring(zone) peer',
    'system_cpu + on(instance) system_cpu', 'rate(counter_total{case="linear"}[5m])',
    'increase(counter_total{case="reset"}[5m])', 'rate(counter_total{case="reset"}[5m])',
    'irate(counter_total{case="last_reset"}[5m])', 'delta(trend[5m])', 'idelta(trend[5m])',
    'avg_over_time(trend[5m])', 'sum_over_time(trend[5m])', 'min_over_time(trend[5m])',
    'max_over_time(trend[5m])', 'count_over_time(trend[5m])', 'last_over_time(trend[5m])',
    'changes(trend[5m])', 'resets(counter_total{case="reset"}[5m])',
    'rate(counter_total{case="gap"}[5m])', 'rate(counter_total{case="zero"}[5m])',
    'rate(counter_total{case="single"}[5m])',
    'absent(missing_metric{job="ghost",instance=~".*"})', 'absent(system_cpu)',
    'absent_over_time(counter_total{case="single"}[1m] offset 2m)',
    'histogram_quantile(0.5,latency_bucket)',
    'label_replace(system_cpu{mode="user"},"node","$1","instance","(.*)")',
    'label_join(system_cpu{instance="a"},"key","/","instance","mode")',
    'abs({__name__=~"system[._]cpu"})', 'clamp_max(clamp_min(system_cpu,3),7)',
    'abs(-system_cpu)', 'round(vector(-0.5))', 'scalar(system_cpu{instance="a",mode="user"})',
    'vector(3)', 'sort(system_cpu)', 'sort_desc(system_cpu)', 'trend offset 2m',
]
cases = [{"id": f"B{i:03}", "query": q, "time_ms": 600000, "ordered": i in [28, 29, 64, 65], "expect_error": i in [34, 58]} for i, q in enumerate(queries, 1)]
cases.append({"id": "B067", "query": "sum(trend)", "time_ms": 600000, "start_ms": 480000, "end_ms": 600000, "step_ms": 60000})
risk = [{"id": "R068", "query": "trend @ 480", "time_ms": 600000},
        {"id": "R069", "query": "sum_over_time((sum(trend))[2m:1m])", "time_ms": 600000},
        {"id": "R-cancel", "query": "sum(system_cpu)", "time_ms": 600000, "cancel_at": 30},
        {"id": "R-budget", "query": "sum(system_cpu)", "time_ms": 600000, "work_limit": 1},
        {"id": "R-fault", "query": "sum(system_cpu)", "time_ms": 600000, "inject_failure": 1},
        {"id": "R-unwind", "query": "sum(system_cpu)", "time_ms": 600000, "inject_failure": 2},
        {"id": "R-invalid", "query": "sum(", "time_ms": 600000},
        {"id": "R-deep", "query": "1+" * 200 + "1", "time_ms": 600000},
        {"id": "R-long", "query": "1+" * 2000 + "1", "time_ms": 600000},
        {"id": "R-after", "query": "sum(system_cpu)", "time_ms": 600000}]
regression_queries = [
    'label_join(system_cpu,"key","-","instance","mode")',
    'label_replace(vector(1),"dst","$","missing",".*")',
    'label_replace(system_cpu{mode="user"},"node","$01","instance","(.*)")',
    'label_replace(system_cpu{mode="user"},"node","${1}","instance","(.*)")',
    'label_replace(missing_metric,"node","x","mode","[")',
    'missing_metric + on(instance) system_cpu',
    'system_cpu > on(instance) limits',
    'limits > on(instance) group_right system_cpu',
    'clamp_min(vector(0/0),1)', 'clamp_max(vector(1),0/0)',
    'sort(special)', 'sort_desc(special)', 'topk(2,special)', 'bottomk(2,special)',
    'trend[2m]', 'abs(vector(0/0))', 'vector(1/0)',
    'sum_over_time((vector(1))[2m:1m])', 'trend[1m:0.1ms]',
    'label_replace(vector(1),"dst","$$","missing",".*")',
    'label_replace(vector(1),"dst","${}","missing",".*")',
]
regression = [{"id": f"G{i:03}", "query": q, "time_ms": -30000 if i == 18 else 600000,
               "ordered": i in [11, 12, 13, 14]} for i, q in enumerate(regression_queries, 1)]
regression.append({"id": "G-lookback", "query": "trend", "time_ms": 660000, "lookback_ms": 60000})
regression.extend([
    {"id": "G-sort-empty", "query": "sort(missing_metric)", "time_ms": 600000, "ordered": True},
    {"id": "G-sort-desc-empty", "query": "sort_desc(missing_metric)", "time_ms": 600000, "ordered": True},
    {"id": "G-sort-window-empty", "query": "sort(system_cpu)", "time_ms": -1, "ordered": True},
    {"id": "G-sort-desc-window-empty", "query": "sort_desc(system_cpu)", "time_ms": -1, "ordered": True},
    {"id": "G-sort-after", "query": "sum(system_cpu)", "time_ms": 600000},
])

if __name__ == "__main__":
    for name, obj in [("dataset.json", {"series": series}), ("cases.json", cases), ("risk.json", risk), ("regression.json", regression)]:
        (ROOT / name).write_text(json.dumps(obj, indent=2) + "\n")
