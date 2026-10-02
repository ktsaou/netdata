# Comparative PromQL prototypes

C, C++ and Rust each implement the same practical dashboard/alert query slice over decoded retained scalar samples.
The experiment supplies implementation evidence for a language decision. Full PromQL qualification, production
query APIs, the independent meta health lifecycle and the Cloud execution split remain later work.

## What works in the experiment

- **67 common-query cases per candidate:** 65 successful results and two expected cardinality/duplicate-label errors.
- **27 focused regressions:** string tokens, replacement captures, invalid regex, empty joins, comparison names/values,
  NaN/Inf handling, sorting including empty windows and following-query recovery, selector matrices, lookback
  boundaries and negative-time subquery alignment.
- **10 probes:** numeric `@`, a subquery, cancellation after temporary rows exist, a work budget, injected failure,
  C++ exception/Rust panic containment, malformed input, depth/token limits, and a successful following query.
- **68 real dbengine replay cases:** the common profile plus a fractional retained-value probe. The isolated fixture
  exports 26 logical series from Tier 0 acquired UUID handles and retained init/next/finalize cursors. It exposes
  `system.cpu` and `system_cpu` over the same stored points. The fractional value changes from `1.23456789` to
  retained `1.2345679`; every evaluator and Go receive that decoded value.
- **Owned results and concurrent C callers:** four threads, 50 evaluations each, destroy/scrub input before inspecting
  returned labels and points, then release results through the owning library.

All candidates matched the pinned Go reference on types, labels, timestamps and finite values within absolute or
relative `1e-6`. Nonfinite values are compared by class/sign. Expected errors require the correct error category;
an unsupported or parsing error cannot satisfy the cardinality/duplicate-label cases. Comparison preserves row
multiplicity and checks order for `topk`, `bottomk`, `sort` and `sort_desc`.

The “80/20” threshold is a practical scope estimate. No monthly query logs were used, and no percentage of real
traffic or the complete language is claimed. `cases.json` is the concrete scope, not a statistical denominator.

## Implementations and input contract

| Candidate worktree | Implementation | Main trade-off |
|---|---|---|
| `netdata-c` | C11 owned AST, request arena, native hash indexes, explicit error path | Small runtime; manual ownership/control-flow; intermediates live until release |
| `netdata-cpp` | C++17 owned AST, standard containers, RAII, exception containment | Familiar native integration; allocator accounting and platform behavior need qualification |
| `netdata-rust` | Owned Rust AST/values, standard collections, private `repr(C)` adapter | Safe internal ownership; unsafe boundary, allocator-abort behavior and packaging need qualification |

Each has its own lexer, parser and evaluator. None calls Go or another candidate to evaluate. Rust uses no external
parser crate, external Rust dependencies or async runtime. Shared components are the C input/result interface,
C caller, JSON transport, experimental regex facility and fixtures. Each static library is linked to a C-compiled
caller. The shared range frontend calls that candidate at each step and assembles a matrix; its result is compared
with upstream **`NewRangeQuery`**, not three unrelated instant-query expectations. It reparses at each range step.

`abi.h` borrows immutable label/sample spans during synchronous evaluation. Engines own returned labels and samples
until `pp_free`. Result views are read-only; C/C++ emergency allocation errors may use static storage, which their
release functions handle. Millisecond timestamps stay integer-valued. The host emits sample values as strings for
lossless binary64 round trips and nonfinite values.

The storage diagnostic exposes `(end_time_s * 1000, sum / count)`, skipping gap records and applying no rendering,
interpolation or retimestamping. It does not create original-exporter storage. Query names are ordinary logical
series names, so name-dropping collisions produce normal duplicate-label errors. This fixture uses one tier;
automatic endpoint ownership across retained tiers and live NIDL catalog projection are not implemented here.

The functions/operators exercised are selectors with equality/regex matchers; scalar/vector arithmetic and
comparisons with `bool`; `and/or/unless`; `on/ignoring/group_left/group_right`; grouping with `by/without`;
`sum/avg/min/max/count/topk/bottomk`; `rate/increase/irate/delta/idelta`, counter resets/extrapolation;
`avg/sum/min/max/count/last_over_time`, `changes/resets`; `absent/absent_over_time`; classic `histogram_quantile`;
`label_replace/label_join`; `abs/clamp_min/clamp_max/round/scalar/vector/sort/sort_desc`; positive offsets, numeric
`@`, subqueries and instant/range results.

## Measurements

`evidence.json` preserves source digests, candidate commits, toolchains, fixture hashes, checks and all measurements.
The following representative results use 512 synthetic nodes, 4,096 series and 249,856 samples. Values are median
milliseconds per parse/evaluate/release over three sequential trials of 20 repetitions. Input loading and JSON
serialization are excluded; ownership and cleanup are included. These compare these implementations, not abstract
language speed or production performance. Label matching precedes label copying in all three candidates.

| Query shape | C | C++ | Rust |
|---|---:|---:|---:|
| Gauge total | 1.15 | 1.36 | 1.51 |
| Grouped gauge total | 1.42 | 1.60 | 2.38 |
| Grouped counter rate | 1.03 | 1.08 | 1.29 |
| Label join for division | 3.37 | 3.65 | 4.50 |
| Label construction | 2.49 | 2.24 | 3.01 |
| Classic histogram percentile | 5.01 | 4.73 | 5.80 |

Observed process peak RSS was roughly 92–95 MiB at that size. It includes the common JSON host/input and launcher
overhead, so it is not an engine-only allocation measurement. C/C++ were built with GCC 13.3; Rust 1.97.1 was used.
One compilation with ccache disabled took approximately 0.78s / 4.60s / 1.52s; stripped-debug caller binaries were
64 / 166 / 1,603 KiB. Compiler settings and archive sizes are in the evidence; these are developer artifacts,
without LTO or a production packaging policy.

The result is **three viable candidates for this slice**. C led most measured shapes; C++ stayed close after the
selector-copy difference was removed; Rust also remained practical and used a working C boundary. These results
do not select a production language. Full conformance maintenance, memory/failure behavior and supported-platform
integration still determine that choice.

## Reproduce

Keep these sibling Git worktrees: `netdata`, `netdata-c`, `netdata-cpp`, `netdata-rust`. The three candidate branches
are `experiment/promql-c`, `experiment/promql-cpp`, `experiment/promql-rust`; the harness branch is
`feature/promql-meta-health`. Run from the harness worktree:

```sh
python3 tests/promql-prototype/build.py --oracle
python3 tests/promql-prototype/run.py
python3 tests/promql-prototype/build.py --sanitize
python3 tests/promql-prototype/run.py --sanitize
python3 tests/promql-prototype/dbengine.py --build
python3 tests/promql-prototype/benchmark.py
python3 tests/promql-prototype/capture_evidence.py
```

Existing tools required: GCC/G++, Rust with unwinding support, Go >=1.26, Python 3, pkg-config, json-c, PCRE2,
objcopy; CMake/Ninja and Netdata's build dependencies for the native diagnostic. The scripts install nothing.
The Go module is pinned to Prometheus **v0.315.0**, corresponding to **v3.15.0 / `5241a27fe3c6983549fccc32f6e65917408c63cd`**.
Dependencies/caches and native output remain under ignored project build/cache directories. A missing ACLK
submodule is brought in as a pinned Git worktree. Native CMake may acquire its pinned bundled SQLite source.

`fixtures.py` deterministically regenerates the fixture/request manifests. The oracle has a real forward-only
sample iterator, an iterator contract test, and explicit lookback/subquery settings. `run.py` checks independent
fixture arithmetic before comparison, including reset increase 175 and the actual range matrix timestamps.

`dbengine.py` builds a test binary and runs a **one-shot diagnostic**. It creates a fresh private directory, a marker
and a private configuration; the native hook verifies ownership, permissions and runtime paths before RRD setup.
The Linux fixture uses `/proc/self/cwd` paths to keep checkout paths out of its configuration. Default paths are
refused before RRD/storage initialization. Collection/cursors/handles are finalized before dbengine shutdown and host teardown.
No collector, listener or production daemon is launched. Fixture directories remain for inspection; scripts do
not delete them. The newest fixture is referenced by `build/latest-fixture.txt`.

## Qualification limits and next decision

- The grammar/function set is partial. Examples still unsupported include negative offsets, metric identifiers
  containing colons, `stddev`, `quantile` and the native-histogram function family. String escaping and parser/type
  validation are incomplete. Queries outside the frozen profile require qualification.
- The C value interface supports floats only. Native histogram sample types, annotations, stale-marker payloads
  and numerical extremes/compensated aggregation require further work. Upstream annotations are captured in the
  evidence; the common profile produced none, and the native interface has no annotation channel.
- PCRE2 is an experimental shared facility with anchored DOTALL matching and explicit failure returns. It supports
  wider syntax than Go RE2 and is **not equivalent**. Pattern/subject/output and PCRE2 match/depth limits are bounded.
- Work budgets count selected evaluation steps. Some copying/indexing/sorting is outside the counter; there is no
  hard deadline or upstream `MaxSamples` equivalence. Queries have byte/token/parse/evaluation-depth limits.
- C/C++ cores, C callers and regex code were ASan/UBSan instrumented. Rust used debug assertions with the same
  instrumented C caller; Rust core sanitizer instrumentation was not enabled. Panic containment is exercised, but
  standard-library allocator OOM can abort. The allocation injection is not an OOM-recovery proof.
- Only Linux x86_64 was tested. Netdata allocator accounting, other supported platforms, tier seams/restart,
  replicas, live metadata concurrency, API discovery/protocols, rule lifecycle and Cloud transport remain later work.
- Alertmanager remains excluded. The existing health engine and Cloud notification feature set are untouched by
  these isolated prototypes. The independent meta health implementation and distributed query boundary need their
  own approved design after the native direction is selected.

The next user decision can use this evidence to compare the native maintenance/runtime trade-offs. A production
choice should include the complete conformance plan and platform/failure qualification budget; this experiment
does not establish 100% compatibility.

Algorithm references: [upstream engine](https://github.com/prometheus/prometheus/blob/5241a27fe3c6983549fccc32f6e65917408c63cd/promql/engine.go),
[range functions](https://github.com/prometheus/prometheus/blob/5241a27fe3c6983549fccc32f6e65917408c63cd/promql/functions.go),
[classic quantiles](https://github.com/prometheus/prometheus/blob/5241a27fe3c6983549fccc32f6e65917408c63cd/promql/quantile.go).
