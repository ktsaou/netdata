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
- **Owned results and concurrent C callers:** four threads, 50 iterations each, hold two independent results, then
  destroy/scrub input before inspecting returned labels and points. Selector, matrix, generated-label, aggregation
  and mixed-label-count outputs are checked; releasing one result must leave the other valid.

All candidates matched the pinned Go reference on types, labels, timestamps and finite values within absolute or
relative `1e-6`. Nonfinite values are compared by class/sign. Expected errors require the correct error category;
an unsupported or parsing error cannot satisfy the cardinality/duplicate-label cases. Comparison preserves row
multiplicity and checks order for `topk`, `bottomk`, `sort` and `sort_desc`.

Raw IEEE-754 output bits are also exported before string/NaN normalization. All **133 finite common-profile values**
and **23 finite regression values** match Go bit-for-bit for every candidate. The 12 nonfinite regression values
include four NaN-payload differences per candidate: Go emits `7ff8000000000001`, while the prototypes emit
`7ff8000000000000`. These remain numeric-class passes; exact payload/stale-marker semantics are unqualified.
Per-case bit differences are preserved in `evidence.json`, independently of the tolerance gate.
The retained replay has two one-bit finite differences per candidate, in the gap/zero rate probes (`B049`/`B050`),
among 134 finite values. Both remain within the numeric tolerance; they are recorded alongside the NaN differences.

The “80/20” threshold is a practical scope estimate. No monthly query logs were used, and no percentage of real
traffic or the complete language is claimed. `cases.json` is the concrete scope, not a statistical denominator.

`language-matrix.json` inventories all **90 functions, 14 aggregations, 21 lexer operators and 13 modifier keywords**
from the pinned upstream catalog, plus value/grammar/runtime/integration surfaces. All candidates have the same
tested profile: 26 function slices and seven aggregations. Rows identify the tested float slice, bounded probes,
unsupported or unqualified work; these counts are not language or traffic coverage percentages.

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

## Original prototype measurements

`evidence.json` is the unchanged step-00 snapshot. It preserves the original source digests, candidate commits,
toolchains, fixture hashes, checks and measurements. The subsequent Rust experiment is recorded separately below.
The following representative results use 512 synthetic nodes, 4,096 series and 249,856 samples. Values are median
milliseconds per parse/evaluate/release over three sequential trials of 20 repetitions. Input loading and JSON
serialization are excluded; ownership and cleanup are included. These compare these implementations, not abstract
language speed or production performance. Label matching precedes label copying in all three candidates.

| Query shape | C | C++ | Rust |
|---|---:|---:|---:|
| Gauge total | 1.19 | 1.20 | 1.53 |
| Grouped gauge total | 1.43 | 1.60 | 2.37 |
| Grouped counter rate | 1.06 | 1.08 | 1.28 |
| Label join for division | 3.38 | 3.54 | 4.30 |
| Label construction | 2.49 | 2.21 | 3.01 |
| Classic histogram percentile | 4.64 | 4.83 | 5.90 |

The result view records parsing and evaluation phases through a shared monotonic clock. Parsing includes query
preparation, lexer priming and AST construction. Evaluation includes final owned C result views. Owner setup,
AST destruction and release contribute to residual wall time; phase totals are not the full wall interval.
Wall time also includes phase-counter and loop bookkeeping.
For grouped counter rate at the same scale:

| Phase or invocation | C | C++ | Rust |
|---|---:|---:|---:|
| Parsing, microseconds | 1.5 | 2.4 | 2.5 |
| Evaluation, milliseconds | 0.961 | 1.074 | 1.272 |
| First invocation, milliseconds | 1.236 | 1.444 | 1.678 |
| Subsequent invocation, milliseconds | 1.035 | 1.057 | 1.254 |

These are medians of three trials. “First” is the first invocation of each query after input loading in the running
process; it excludes process startup. Every invocation reparses; no AST cache is used. The phase/first/repeated
measurements cover the six instant-query benchmark shapes, with complete data for both scales in `evidence.json`.

Observed process peak RSS was roughly 93–97 MiB at that size. It includes the common JSON host/input and launcher
overhead, so it is not an engine-only allocation measurement. C/C++ were built with GCC 13.3; Rust 1.97.1 was used.
One compilation with ccache disabled took approximately 0.78s / 4.44s / 1.53s; stripped-debug caller binaries were
64 / 166 / 1,603 KiB. Compiler settings and archive sizes are in the evidence; these are developer artifacts,
without LTO or a production packaging policy.

The result is **three viable candidates for this slice**. C led most measured shapes; C++ stayed close after the
selector-copy difference was removed; Rust also remained practical and used a working C boundary. These results
do not select a production language. Full conformance maintenance, memory/failure behavior and supported-platform
integration still determine that choice.

## Rust performance experiment

`rust-performance.json` records the bounded follow-on experiment: frozen source pins, current source digests,
build-setting comparisons, five interleaved timing trials, allocation counters, instruction profiles and checks.
The final comparison remeasures all candidates together on one CPU, at both 32 and 512 synthetic nodes. Each trial
uses a fresh process containing one workload. Input loading and JSON construction are outside the engine interval;
parsing, evaluation, owned-result construction and release are inside it. Process CPU time uses the same interval.

The original Rust profile uses `opt-level=2`, `debug-assertions=yes`, `overflow-checks=yes` and `panic=unwind`.
The selected experimental profile uses `opt-level=3`, `debug-assertions=no`, `overflow-checks=yes`, `codegen-units=1`,
`lto=thin` and `panic=unwind`. It targets the portable CPU default, without fast-math or host-specific instructions.
Intermediate profiles isolate disabling debug assertions, increasing optimization, reducing codegen units and
enabling ThinLTO. These measurements do not select production build flags.

Build-only comparisons initially used CPU 0. All before/after results below instead come from the final comparison
on CPU 28, including fresh measurements of the frozen Rust O2 and ThinLTO builds. Cross-CPU timing changes are not
credited as optimization gains. Small timing differences can reflect noise; raw trials and dispersion are retained.

### Confirmed costs and bounded changes

Callgrind instruction attribution identified allocation/free and ordered label-comparison work as substantial
costs. Profiling is restricted to `pp_eval` and `pp_free`, and runs separately from timing. Before/after instruction
events with the same original Rust build settings decreased by 15.0% for gauge total, 17.2% for grouped gauge and
10.6% for the division join. Instruction events are not hardware cycles or elapsed-time percentages.

The Rust changes replace four unnecessary materializations:

- Instant selectors retain the last eligible sample while scanning and ticking every input point.
- Projection consumes already-owned label maps when their original contents are no longer needed.
- Ordinary aggregation groups retain values instead of complete label/sample rows, preserving group and sum order.
- Output labels use flat owned backing vectors, with point buffers moved into the result owner.

Top/bottom-k buffering, BTree ordering, UTF-8/CString validation, cancellation checkpoints, checked arithmetic and
panic containment remain intact. The C ABI is unchanged; no external Rust dependency or new unsafe block is added.
The Rust evaluator grows from 1,552 to 1,575 physical lines: 73 insertions and 50 deletions.

### Engine time

These are median milliseconds per parse/evaluate/release on the larger fixture: 4,096 series and 249,856 samples.
Instant-query trials contain 40 evaluations each. The CPU medians closely track the wall medians.

| Query shape | C O2 | C++ O2 | Frozen Rust O2 | Changed Rust ThinLTO | Change versus C |
|---|---:|---:|---:|---:|---:|
| Gauge total | 1.112 | 1.144 | 1.506 | 1.037 | -6.7% |
| Grouped gauge total | 1.421 | 1.661 | 2.449 | 1.502 | +5.7% |
| Grouped counter rate | 1.040 | 1.086 | 1.307 | 0.980 | -5.8% |
| Label join for division | 3.376 | 3.668 | 4.589 | 2.996 | -11.3% |
| Label construction | 2.470 | 2.205 | 2.933 | 1.975 | -20.0% |
| Classic histogram percentile | 4.765 | 4.645 | 5.063 | 3.702 | -22.3% |

Equal weighting of these six shapes gives diagnostic geometric means, not a production traffic mix:

| Rust comparison with frozen Rust O2 | 32 nodes | 512 nodes |
|---|---:|---:|
| Build settings only, frozen source | 22.2% less time | 19.4% less time |
| Evaluator changes only, original settings | 23.1% less time | 12.7% less time |
| Evaluator changes plus ThinLTO settings | 39.7% less time | 31.7% less time |

Changed Rust ThinLTO is faster than the current C++ prototype on all six shapes. On the larger fixture, its slowest
comparison with C is grouped gauge at +5.7%; other shapes are 5.8–22.3% faster. On the smaller fixture, all six are
15.9–29.5% faster than C. **C/C++ remain unchanged O2 prototypes and were not subjected to equivalent optimization**;
these results qualify these implementations rather than establish a language speed ceiling.

### Advancing evaluations and range requests

Alert-style sequences evaluate at 21 advancing timestamps. Range sequences evaluate at 11 timestamps. Each step
reparses and immediately releases its result; inputs are immutable, without a scheduler, prepared-query cache,
concurrent ingestion or persisted alert state. The following medians cover one complete sequence, excluding JSON:

| Larger-fixture sequence | C O2, ms | C++ O2, ms | Frozen Rust O2, ms | Changed Rust ThinLTO, ms |
|---|---:|---:|---:|---:|
| Grouped gauge threshold, 21 evaluations | 33.825 | 38.692 | 57.438 | 35.748 |
| Grouped rate threshold, 21 evaluations | 22.117 | 22.667 | 27.388 | 22.578 |
| Grouped rate, 11 range steps | 12.548 | 13.895 | 18.469 | 13.086 |
| Division join, 11 range steps | 37.528 | 40.729 | 51.908 | 35.660 |

Separate `range_host_*` measurements include evaluation and retained JSON matrix assembly for one request. They
exclude final serialization and matrix release. The shared host searches existing output rows linearly by labels;
its cost can dominate this standalone experiment. For the larger division range, C core time is 37.528 ms and full
host time is 584.672 ms; changed Rust core time is 35.660 ms and host time is 620.608 ms. Faster engine execution
therefore does not establish a faster complete response. No range-host optimization is included in this stage.

### Allocation and memory evidence

Heap measurements use a separate, single-threaded Linux/glibc allocation-interposed caller. A fixed nonallocating
table records successful allocation/reallocation events, requested byte traffic and peak live requested bytes
between evaluation and release. A focused counter check covers failed realloc preserving the original allocation.
All measured core calls finish with zero live tracked bytes, zero table overflows and zero untracked frees.

These are larger-fixture peak live requested bytes. They exclude the input fixture, stacks, allocator metadata and
fragmentation, executable pages and libc's transient internal realloc storage. **They are not engine RSS.**

| Query shape | C O2 | C++ O2 | Frozen Rust | Changed Rust |
|---|---:|---:|---:|---:|
| Gauge total | 1,994,259 | 1,201,048 | 736,822 | 687,702 |
| Grouped gauge total | 2,400,715 | 1,319,264 | 1,120,271 | 687,762 |
| Grouped counter rate | 1,298,994 | 708,296 | 596,251 | 596,251 |
| Label join for division | 4,288,632 | 2,467,584 | 1,391,061 | 1,391,061 |
| Label construction | 2,835,781 | 1,403,948 | 727,029 | 960,281 |
| Classic histogram percentile | 5,624,076 | 3,223,426 | 2,390,796 | 2,390,796 |

Grouped gauge allocation events decrease from 23,688 to 14,458, with a 38.6% reduction in peak live requested bytes.
All six Rust shapes use fewer allocation events, though Rust still performs more than C on some shapes. Flat output
backing arrays increase label-construction peak by 32.1% versus frozen Rust because they overlap live evaluation
labels; the resulting peak remains below C/C++ in these fixtures. This is a measured trade-off, not a memory win on
every query. Range-core peaks are the maximum of separately released steps; `range_host_heap` also includes the
retained JSON matrix and must be considered separately.

### Compatibility and integration implications

- Six compared variants pass 67 common cases, 27 existing regressions, 15 focused selector/group/nonfinite cases,
  68 cached real Tier 0 replay cases, the expanded concurrent ownership test and all ten controlled probes.
- Changed Rust preserves frozen Rust's output bits, row order, work counts and controlled-failure messages on the
  checked corpora. Go comparison retains the original typed/label/timestamp/numeric tolerance gate; existing rate
  rounding and NaN-payload differences are reported rather than hidden or called bit-exact Go compatibility.
- Release and existing sanitizer-mode gates pass. C callers and C/C++ cores are ASan/UBSan instrumented; Rust core
  uses debug assertions rather than sanitizer instrumentation. Valgrind Memcheck reports zero errors for changed
  Rust under both original and ThinLTO settings, covering ownership, focused cases and controlled probes.
- The changed Rust caller shrinks from 1,659,864 bytes with original settings to 660,624 bytes with ThinLTO after
  debug stripping. On one pinned CPU the corresponding compilation observations are 4.22s and 7.27s. Rust's
  standard runtime is statically included, while the C++ standard library is dynamically linked; executable bytes
  alone are not a fair comparison of total deployment footprint.

The measured slowdown is substantially removable without changing the supported semantics. Rust is a credible
candidate for the user's gradual migration objective; production selection still requires the conformance,
allocator/platform and live-storage qualification described below. This stage does not implement full PromQL,
production query APIs, the independent meta health engine or a Cloud execution split.

## Reproduce

Keep these sibling Git worktrees: `netdata`, `netdata-c`, `netdata-cpp`, `netdata-rust`. The three candidate branches
are `experiment/promql-c`, `experiment/promql-cpp`, `experiment/promql-rust`; the harness branch is
`feature/promql-meta-health`. Run from the harness worktree:

```sh
python3 tests/promql-prototype/build.py --oracle
python3 tests/promql-prototype/run.py
python3 tests/promql-prototype/build.py --sanitize
python3 tests/promql-prototype/run.py --sanitize
python3 tests/promql-prototype/test_compare.py
python3 tests/promql-prototype/qualification.py
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

For the Rust performance experiment, reuse the oracle and retained fixture above. Run both timing phases with the
same CPU affinity; the driver chooses the first CPU in its inherited permitted set. It prints commands, installs
nothing, and writes temporary output under `build/performance/`. Long runs may use the development workstation's
established transient-job runner with an explicit CPU affinity. Profiling additionally needs existing Valgrind and
`callgrind_annotate`; heap instrumentation requires Linux/glibc, and affinity requires `taskset`.

```sh
python3 tests/promql-prototype/performance.py baseline --trials 5
python3 tests/promql-prototype/performance.py profile --variant rust-baseline
python3 tests/promql-prototype/performance.py final --trials 5 --selected-profile thin-lto
python3 tests/promql-prototype/performance.py profile --variant rust-optimized-baseline
python3 tests/promql-prototype/performance.py verify
python3 tests/promql-prototype/performance.py memcheck
python3 tests/promql-prototype/performance.py capture
```

The driver materializes original candidate source from the pinned commits and reads changed Rust from its sibling
worktree. `capture` writes `rust-performance.json` independently; `capture_evidence.py` belongs to the original
prototype pipeline and is not used to overwrite the frozen `evidence.json` during this follow-on experiment.

## Native histogram risk and completion costs

`native-histogram-risk.json` contains a separate typed upstream fixture: one gauge histogram with schema 0, count 2,
sum 1.5 and one positive bucket. The pinned Go engine returns `histogram_count = 2`, `histogram_sum = 1.5` and
`histogram_avg = 0.75`, with the metric name removed. Its sample/bucket ownership probe also passes.

**Native histogram values and operations remain unsupported in all three candidates.** The C `PPPoint` interface
has no histogram type. Object-valued sample input is rejected by the shared C host before entering an evaluator.
Separate legal `histogram_count/sum/avg(vector(1))` probes show explicit unsupported-function errors in each engine,
followed by a successful ordinary query; Go returns empty vectors for those float inputs. This reports the bounded
risk slice's blocker under the experiment stop rule. It is not a native histogram operation or ownership PASS,
and no retained Netdata histogram mapping is selected.

| Remaining work | Shared consequence | Candidate considerations |
|---|---|---|
| Typed and mixed histogram samples | Extend value/input/result ownership, schema/bucket arithmetic, mixed-sample behavior and annotations | C needs explicit arena/array cleanup; C++ needs owned variant/container paths; Rust needs owned values plus an expanded unsafe C adapter |
| Full grammar, types and functions | Complete the catalog/matrix, type checks, duration/string/identifier grammar and upstream differential corpus | Each independent parser/evaluator needs maintenance; this experiment measures no full-completion effort |
| Regex and numerical conformance | RE2 equivalence, compensated reductions, special values and exact histogram algorithms | PCRE2 is shared; numerical kernels remain independent in all three languages |
| Runtime and supported platforms | Memory/sample/deadline accounting, allocator failure, Netdata allocation hooks and platform/package qualification | C retains arena intermediates; C++ integration must account for allocator/exception/build policy; Rust must qualify allocator aborts, toolchains and static runtime packaging |
| Retained/API/rule/Cloud integration | Live NIDL identities, tier ownership, replicas, query APIs, rule lifecycle and distributed semantics | Shared Netdata adapters and Go Cloud work remain separate from this native language comparison |

These are concrete cost drivers, not person-month estimates. The complete remaining inventory is in
`language-matrix.json`; the current experiment supports a language discussion without selecting its outcome.

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
