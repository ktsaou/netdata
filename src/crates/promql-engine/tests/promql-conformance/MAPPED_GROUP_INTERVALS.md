# Completed intervals through mapped groups

Common CPU-history queries can evaluate several inner timestamps for each output point:

```promql
max_over_time((100 - avg by(netdata_host_id, netdata_host_name)(system_cpu{netdata_dimension_id="idle"}))[5m:30s])
```

The ordinary aggregate replay remembered only one completed timestamp. Two inner timestamps displaced each other
as the engine visited successive output groups. An 18-host, six-hour fixture recorded zero reuse hits from
13,870 attempts; the same fixture with one inner timestamp per output step recorded 6,120 hits. Reader checkpoints
were accepted in both cases, so the repeated work was not a restart of raw history.

The existing fixed-map recognizer now admits ordinary aggregates over direct instant selectors. Qualified scalar
arithmetic and comparisons, unary expressions, fixed Float functions and the supported unary/Float composition use
the same interval metadata as direct grouped subqueries. Existing mapped rate-family expressions also retain the
aggregate's actual terminal completion.

## Completion and ownership

- The aggregate must have drained its input and finalized its selected groups.
- Only transparent passes, running ordered rows and qualified maps without a pending diagnostic can carry this
  completion to the range reducer. The existing path proof checks resolved children, clocks and wrapper limits.
- The receiver still requires a single Float descriptor. Mapping, duplicate checks and the numerical visitor run
  before real aggregate completion. Emitting a value alone never grants interval credit.
- Filtered and absent results finish through the ordinary path. Existing native, mixed, delayed-name, fixed-time and
  unsupported-composition restrictions remain. Preparation does not satisfy fresh final reads.
- Existing bounded pages and provider-owned temporary backing remain in use. There is no additional timestamp cache,
  raw history buffer, metadata catalogue or C interface change.

## Regression checks

`computed_group_tests` contains a regression that fails on the previous implementation at the missing interval path.
It exercises inner steps smaller than, equal to and larger than the output step, multiple groups/contributors,
physical pages of 1/17/1024 samples, both scalar sides, filtering/Boolean comparisons, unary/Float maps and rate inputs.
It compares complete results and logical sample totals with ordinary retirement, requires actual terminal completion,
and bounds cold contributor scans by observed timestamps.

The mapped cancellation test interrupts the real aggregate terminal step and requires no published output. Existing
native phase-change tests verify that Float-looking COUNT/MAX results do not hide native input from progress checks.
Both optimized profiles pass 1,106 tests with two ignored, and strict lint passes. Focused ARM32/ARM64 checks execute
under QEMU; actual native GCC/Clang C/C++ callers pass. These are correctness checks, not foreign-platform timings.

The expanded original-reference panel includes 3,600 exact comparisons per new build across default/upstream-test
profiles, sparse/nonfinite/mixed data, ambiguous labels, fixed-time fallbacks and page sizes. The previous frozen
build independently passes the same 3,600 comparisons. Broader builtin, retained cursor and SRE results belong to the
source-bound qualification receipt.

## Resource measurement

For the six-hour fixture with 30-second inner steps:

| Observation | Previous | Candidate |
| --- | ---: | ---: |
| Accounted work | 153,194,742 | 47,533,119 |
| Cumulative workspace allocation, bytes | 367,878,544 | 55,198,076 |
| Peak workspace, bytes | 97,411 | 97,523 |
| Fetched sample entries | 789,930 | 789,930 |

The additional 112 bytes in this fixture belong to the completed interval of its one mapped aggregate. This is not a
universal per-query bound. Other observed memory-category peaks are unchanged. Lower allocation traffic and fewer
operations do not by themselves establish elapsed-time improvement; repeated timings and live tests are separate.


Six alternating measured runs after one excluded warmup reduce the six-hour, 30-second-inner-step median from
829.42 to 479.06 ms in `release` and 1,314.86 to 580.11 ms in `release-min`: approximately 42% and 56% faster.
The mixed-batch one-inner-clock release control instead measured 296.10 to 320.24 ms. A separate twelve-run
single-query repeat measured 288.17 to 287.79 ms, so that slowdown was not reproduced in the isolated control.
Both observations remain in the report. All measurements use the shared development workstation.

[The qualification receipt](mapped-group-intervals-qualification.json) retains 152 timing/resource rows, the
separate control, exact source/binary identities, 128,652 builtin comparisons, 18,192 retained cursor comparisons,
7,200 new mapped comparisons, and 231,540 SRE evaluations. The builtin panel retains its original per-request
numeric comparison policy; the mapped and SRE panels require exact paired results. This does not claim complete
language coverage or resolve live latency by inference. Installed behavior and remaining findings are recorded
separately under `tests/promql-integration/grafana/`.
