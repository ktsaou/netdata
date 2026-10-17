# PromQL conformance foundation

In the Netdata checkout, run this guide's commands from `src/crates/promql-engine`. This directory's
"repository root" means that independent Cargo workspace, which contains both the engine and this harness.

This directory defines the independent reference and comparison contract for completing the Rust engine.
It targets the complete pinned language, including all required SRE recording and alert expressions.
Candidate qualification uses separately retained complete results; reference self-tests and SRE parsing alone
do not establish engine compatibility.

The historical C/C++/Rust experiment and original evidence remain in the [Netdata experiment](https://github.com/ktsaou/netdata/blob/feature/promql-rust-core/tests/promql-prototype/README.md).
This harness uses a separate Go module and the same dependency locks. It does not run a daemon or change metrics.

## Standalone expression core

The [design decisions](DESIGN.md) summarize the approved architecture, rationale, trade-offs and integration boundaries.

The standalone Rust core evaluates derived expressions, nested subqueries, ordinary/extended range functions,
vector sorting and typed histogram operations through bounded storage replay. Subquery clocks, source preprocessing
visits, parameter phases and diagnostic accumulation have separate owners. Raw provider pages and decoder state
are released after each replay; completed folds and their scalar resume certificates may survive at the current
evaluation clock. The cache replaces an older fold instead of retaining a timeline. Raw histories remain paged;
general temporal quantiles use separate provider-owned temporary backing. No second raw-storage page cache or
enabled admission limit is introduced.

Retained qualification covers:

- Validated ordinary float rate endpoints reuse their initial acquisition. The
  [rate acquisition panel](#validated-float-rate-acquisition-reuse) records 691 native units/profile, 4,630 strict
  original comparisons and 12 actual C/C++ programs. Fixed `rate`/`increase` inputs fetch 25% fewer entries and
  `delta` inputs 50% fewer; all measured query peaks stay equal. Timing gains, slower controls and text costs remain explicit.
- Certified warm pair metadata avoids repeated reconstruction and processed-prefix walks. The
  [pair traversal panel](#completed-pair-metadata-traversal) records 8,088 native and 2,000 compiled ARM complete
  comparisons, 682 native units/profile and actual C/C++ consumers. Fixed 1,024-group thresholds improve
  6.5–6.9× with 8 KiB lower workspace peaks. All small/fallback costs, including the 8 B Step increase, remain visible.
- Scheduled selector points transfer their existing owned labels into sample completion. The
  [label ownership panel](#scheduled-selector-label-ownership) records 1,656 native and 1,104 compiled ARM
  comparisons, 654 native units/profile and actual C/C++ consumers. Workspace peaks fall in 864 cases; mapped
  64-series timings improve 1.28–1.34×, with all slower controls and binary costs retained.
- Eligible grouped float ranges reuse completed operand discovery. The
  [grouped-join panel](#completed-grouped-float-join-scans) records 1,692 native and 1,128 compiled ARM comparisons,
  647 native units/profile and 12 actual C/C++ callers. Fixed 64-series queries become 19× faster with unchanged
  scaling peaks; small rejected-proof, mapped-peak and timing costs remain explicit.
- Absence inference respects preprocessing of aggregation-parameter arguments. The
  [argument-edge panel](#absence-argument-preprocessing) records outcome-changing label inference, ordinary and
  generic matrix controls, byte/matcher history, and actual C/C++ calls without adding runtime state.
- Nested eligible sorts reuse delivered permutation prefixes. The [nested-sort panel](#nested-vector-sort-prefixes)
  records 1,644 complete native and 840 compiled ARM comparisons, unchanged workspace peaks, and 8.1–8.7× faster
  fixed 512-series `abs(sort(m))` ranges. It also records slower ordinary/tiny controls and exact source attribution.
- Selector backend capture tables are omitted for boolean-only consumers. The
  [capture-table panel](#selector-backend-capture-tables) records both native optimized profiles, compiled ARM
  selector originals, short actual C/C++ calls and source-distinguished test-only correction. Of 484 complete native
  query groups, 200 lower estimated regex peak/acquired memory and none increase; engine-owned observations match.
- Boolean selector fallback avoids unused capture results. The [selector panel](#boolean-selector-search) records
  484 complete native matches, 504 compiled ARM matches and 21,320 original regex cases per profile/target.
  Of 484 resource groups, 328 acquire less workspace and do less work; total query peaks mostly stay unchanged.
- Earned mapped operands pass 1,986 complete native and 1,196 compiled ARM matches, 632 native units/profile and
  12 actual C/C++ callers. The [operand panel](#completed-earned-operand-joins) records direct arithmetic, unary,
  label and native-to-float schedule reuse, 36–43× large-case timing gains, unchanged scaling peaks and small costs.
- Composed Float-map joins pass 4,126 complete native and 8,156 compiled ARM matches, 632 native units/profile and 12 actual
  C/C++ callers. The [composition panel](#completed-composed-float-map-joins) records existing schedule reuse, full
  value/label/time checks, large-query savings and the 312 B small-case peak cost.
- Single-lock received-page accounting passes 631 native units/profile, 246 complete queries with every exported
  nonelapsed observation unchanged, 28 focused compiled ARM unit executions and four actual native C/C++ callers.
  The [page accounting panel](#single-lock-received-page-accounting) records exact counter/error phases, unchanged
  measured memory and fixed warmed timings, including slower controls.
- Direct one-argument float-map joins pass 10,372 complete native and 3,656 compiled ARM matches, 627 units/profile
  and 12 actual C/C++ callers. The [float-map panel](#completed-direct-float-map-join-scans) records fewer repeated
  reads, large-query allocation savings, small 82–312 B peak increases and matched whole-query timings.
- Exact temporal rank accounting passes 626 units/profile and 936 complete native matches with every exported
  nonelapsed observation unchanged, plus 800 compiled ARM matches. The [accounting panel](#exact-rank-work-accounting)
  records saturation/control tests, retained reducer/temporal panels and paired whole-query measurements.
- Scheduled derived quantile ranks pass 624 final units/profile, 864 complete native and 776 compiled ARM matches,
  plus 56,124 retained reducer and 14,664 temporal outcomes. All 864 measured memory peaks remain unchanged; 408 cases
  fetch fewer entries. The [rank traversal panel](#scheduled-derived-quantile-rank-traversals) records the complete
  phase/control, source/binary and paired-timing evidence separately from earlier broad qualification cohorts.
- Compact group/join lookups pass 535 fixture units per optimized build, 128,652 builtin comparisons and all
  231,540 required SRE comparisons across both builds and pages 1/17/1024. Fresh C/C++ callers execute on AMD64,
  ARM64 and ARM32; 58 controlled live API/contexts checks pass. The [lookup panel](#compact-group-and-join-lookups)
  records 114 complete before/after comparisons and 56,124 ranking/quantile outcomes with separate ordering controls.
- Certified final vector range output passes 527 fixture units in each optimized build, 128,652 builtin comparisons
  and all 231,540 required SRE comparisons across both builds and pages 1/17/1024. Fresh native C/C++ and 58 controlled
  live Netdata API/contexts checks pass. ARM compilation and earlier foreign runtime receipts remain distinguished.
  The [final output panel](#certified-final-range-output) records the bounded-memory before/after measurements;
  [cursor fault qualification](#physical-cursor-fault-qualification) separates strict originals from page-reuse policy.
- The native diagnostic printability getter reuses the existing Go Unicode17 table without allocating. Its
  additive cohort passes522 fixture units in both profiles,520 production units, lint/format/MSRV checks and448
  rebuilt C/C++ records across AMD64/ARM64/ARM32. Twelve rune boundaries and invalid output-pointer controls run
  in actual callers. This qualifies the primitive; native form decoding and composed API messages remain caller work.
- Precise query/protocol primitives in both native profiles:230 original parser/selector/text cases plus528
  label conjunctions per build, and27 precise-clock comparisons per build at pages1/17/1024. Their scoped follow-up
  passes479 production release and480 fixtures release-min unit tests, Rust1.91 checks and18 actual C/C++
  executions/360 records across AMD64/ARM64/ARM32. This does not execute an HTTP router or live provider; historical
  full expression panels keep their original source and binary identities.
- All 21,442 builtin expression variants in release and release-min, with no unsupported outcome, mismatch or
  paired numerical bit difference in that panel.
- All 1,110 SRE source expressions through 231,540 controlled evaluations across both builds and page sizes
  1/17/1024. Recording chains consume the Rust candidate's own completed parent results. The source/input audit
  checks every stage and bridge; this evidence covers expression execution rather than rule scheduling or live data.
- 19,214 canonical arithmetic checks, bounded cursor/progress controls, actual C/C++ embedding and focused work-driven
  cancellation/panic cleanup. Precise engine/foreign ownership and estimated regex dependency memory retain their
  declared accounting contracts. Enforcement remains disabled.
- All 3,229 parenthesized-matrix and native-operation controls in both builds at page sizes 1/17/1024:
  19,374 complete outcomes, with no unsupported result, mismatch or paired numerical bit difference in that panel.
- The complete frontend panels on compiled AMD64, ARM64 and 32-bit ARM, in both builds: 135,936 comparisons across
  lexer/strings, 32 literals, 356 parser cases, 784 option controls and 21,320 regex cases per target/build.
- Successful expressions with nonempty original results for all 90 catalog functions and 14 aggregations in the
  explicit upstream-test profile. The default profile has corresponding witnesses for all 73 stable functions and
  12 stable aggregations. These are expression AST witnesses, not instrumented node visits or branch coverage.
- A 42-case default-profile panel, including 37 positive results and five optional-feature rejections, passes all
  756 comparisons across three targets, both builds and pages 1/17/1024. Existing 41-case inputs and results remain
  unchanged; the added `max(http_requests)` control retains its complete upstream dataset.
- The duration-width cohort passes 460 native unit tests in each build, 12 focused cross-target regressions,
  12 strict production/fixture lint checks and 459 production unit tests with Rust 1.91. Its actual binaries and
  source manifests remain attributed to that cohort. The subsequent metadata-control correction is qualified
  separately in `core-qualification.json`.
- The final control/accounting correction passes 471 native unit tests in each build and 69 compiled checks,
  including 12 strict production/fixture lint checks, Rust 1.91 and focused tests on ARM64/32-bit ARM. Fresh C/C++
  callers pass 18 executions/324 records on the same final source across all three GNU targets and both builds.
  Earlier complete expression panels retain their actual binary identity; the small follow-up changes only
  compilation control checks and diagnostic work observation, with its source and consequential checks recorded.

All 2,152 temporal outcomes pass in both native builds at page sizes 1/17/1024, including the 150 formerly open
mixed-history cases. They use provider-owned paged float-bit backing and the literal upstream sort. Storage creation,
configuration and cleanup belong to the provider. Missing or failed backing returns a specific storage error when
the general-sort path is reached. The production core creates no files and has no whole-history RAM fallback.
Parenthesized matrix arguments inside unvisited scalar aggregation parameters follow the source's generic
function path, including its successful results and distinct errors.

The standalone expression core is qualified against these pinned panels and independently assessed source scopes.
[Catalog coverage](catalog-coverage.json) identifies successful expression witnesses, and
[core qualification](core-qualification.json) records the original source/binary cohorts, comparison policy,
qualified scope and limitations. These panels do not establish equivalence for every possible input.
Controlled live dbengine/API integration is qualified separately in the current runtime cohorts and the
[Netdata integration guide](../../../../../tests/promql-integration/README.md). Alert scheduling and Cloud distribution
remain later stages. Alertmanager is outside this work.

The detailed counts below preserve individual historical checkpoints and their source attribution. Historical
OPEN counts and earlier prerequisite limitations describe those earlier cohorts. Current scope and final source
qualification are stated here; a finite corpus does not establish equivalence for every possible input.

Raw-input memory is bounded by pages and decoder/operator state. Series metadata, derived group state, histogram
payloads and an independently owned complete result can still grow with cardinality. All resource caps remain
disabled, so this stage does not guarantee bounded total memory or process RSS for every query. Streaming output
consumers may release provisional chunks immediately. General exact temporal quantiles need provider-owned
temporary backing; the core has no whole-history RAM fallback.

## Cooperative control and work observations

[Basic query statistics](DESIGN.md#basic-query-statistics) expose primary logical participation, useful source
reads, actual owned sample peaks and execution phases separately from fetched entries, bytes and work. The
streaming consumer and convenience collector have different output lifetimes; per-step arrays remain disabled.

Label descriptor walks, ordered validation, diagnostic formatting/merging, metadata rosters/filters and variable
query task walks use the query's existing cancellation/deadline control and work account. Long label comparisons
and diagnostic position scans poll in bounded byte chunks. Parenthesis projection and helper-shortcut walks
record each hop; their callers propagate interruption through preparation and evaluation.

Unary collision detection reuses the existing prepared merge guard and checks actual collision candidates with
an accounted metadata sort. Distinct series avoid an all-pairs scan. Identity-changing operators retain their
ordered-output path; ordinary ordinal output uses the existing interruptible metadata sort.

A completed provider failure keeps precedence over cancellation received while transporting its diagnostics.
Diagnostic position scans still charge their visited query bytes on that failure path; suppressing later
cancellation does not suppress work accounting. Compilation also polls while enqueuing variadic call arguments,
so pending cancellation is observed before growing the remaining task stack.
Successful/END callbacks observe cancellation before converting their diagnostic roster. Generated diagnostics
publish completed formatted messages; interrupted formatting retains the prior valid message.

Controls are cooperative. The core cannot interrupt a blocking foreign callback. Resource enforcement remains
disabled, and no implicit query timeout or admission policy is introduced.

## Absence argument preprocessing

Function arguments use the effective edge selected by source preprocessing. Prometheus removes parentheses from
visited call arguments, while aggregation-parameter subtrees keep their surviving parentheses. Inferring labels
through those parentheses can change a later vector match and the aggregation parameter's value.

For example, with no matching `missing` series and three distinct `m` values of 1, 2 and 3,
`topk(scalar(absent((missing{job="a"})) + on(job) vector(1)), m)` must return the largest two series. Previously,
Rust inferred `job="a"`, lost the vector match and returned `Parameter value is NaN`. A related quantile expression
returned NaN and a warning where Prometheus returned 3 without a warning.

The evaluator now resolves the argument through its existing preprocessing owner. The absence helper checks that
immediate selector or matrix node and leaves surviving parentheses intact. Ordinary parenthesized root calls still
infer labels. Generic unvisited matrix calls, subqueries, matcher equality history, byte labels and source spans
retain their existing owners. No new state, cache, dependency, ABI or provider behavior is introduced.

The permanent `absence_argument_cases.py` roster uses the existing series codec and includes missing/present
selectors, parenthesis depth, empty/duplicate/byte matchers, ordinary calls, top-k/quantile parameters, matrix and
subquery controls in both parser profiles and instant/range modes. The complete-query regression
`absent_label_inference_respects_visited_argument_edges` checks exact rows, clocks, bits, diagnostics and cleanup.
The short C/C++ caller mode `--absence-only` checks collected top-k/quantile results and provider-failure cleanup
at pages 1/17/1024 in both query modes.

[Absence argument qualification](absence-argument-qualification.json) preserves the original failing runtime,
fresh pinned-oracle evaluations, actual compiler identities, complete outcome comparisons and resource observations.
Both native optimized builds pass 644 library tests and 984 complete original comparisons. Compiled ARM64/ARM32
execution passes 656 complete comparisons and 24 selected unit executions across both profiles. Twelve actual
C/C++ executables each pass the short 24-arm caller. This is selected foreign library/caller execution, not full
foreign suites or a rebuilt Agent.

The expanded original panel has 144 cases; the previous runtime failed 32 per profile/page. Its 672 unchanged-result
native groups preserve all reads, with no higher measured workspace peak, acquired workspace or work. Peaks fall
in 204 groups and acquired workspace in 240. The 192 corrected-result groups require more evaluation and output
work than the previous wrong error/NaN paths; their resource costs are recorded separately. Native query-fixture
ELF text grows by 16 bytes per profile. These are engine counters, not process RSS or throughput measurements.
Historical broader language, SRE, live Agent and API cohorts retain their source identities.

## Parenthesized matrix and native-operation qualification

The compiler preserves surviving parentheses in unvisited aggregation parameters. Source dispatch recognizes
actual matrix/subquery nodes; a surviving parenthesized argument follows generic function evaluation. Replacing
that path with ordinary range evaluation changes results, labels, errors, annotations and acquisition order.
The internal matrix visitor scans and discards borrowed raw points one page at a time. It retains no queried
history or matrix output; parenthesized subqueries still visit their complete logical child grid. Multi-instant
raw matrix evaluation returns its source error before matrix expansion, with upstream selector preparation intact.

Reached native histogram failures use checked, accounted source diagnostics through the storage/evaluator layers.
Query recovery retains the original plain/wrapped error class and discards incomplete evaluator annotations.
The C boundary reports the existing evaluation status/error kind; unexpected Rust panics retain internal-error
containment. Standalone histogram helpers preserve their original raw panic messages. Native kernel probes retain
empty-versus-absent spans; only the query fixture codec applies the original query oracle's empty-span projection.
Checked reads include Kahan schema reduction, the first equal-offset receiving bucket and custom-bound
intersection. Guards run at the reached access, preserving preceding source operations and skipped buckets.
Custom-bound diagnostics retain signed 64-bit indices, including `[-4294967296]`, under the AMD64 contract.

Native acquisition also distinguishes buffered `CopyTo` from the exact current point's borrowed `At` result.
The buffered path clears custom-schema zero/negative fields or exponential-schema custom bounds at the source copy
boundary. An exact current point retains provider fields, including during later overlapping replay. Rate endpoints,
instant-rate tails and changes/reset previous values retain those acquired fields; subsequent source-defined output
copies keep their own normalization. Raw matrix output preserves acquired fields through the sink and owned collector.

The independently declared panels are:

| Panel | Cases | Covered boundaries |
| --- | ---: | --- |
| `unvisited-primary` | 1,512 | Generic matrix calls in aggregation parameters |
| `unvisited-supplement` | 1,044 | Parameters, labels, scalar outcomes and typed source errors |
| `unvisited-phases` | 336 | Acquisition, faults, query-wide statistics and precedence |
| `native-valid` | 24 | Valid extreme histogram layouts and error contracts |
| `native-checked` | 123 | Reached native guards, ignored operands and trim outputs |
| `native-notes` | 17 | COUNT/native error and diagnostic composition |
| `matrix-copy` | 39 | Buffered versus exact-point native acquisition |
| `instant-copy` | 8 | Instant-rate retained-pointer fields |
| `native-recovery` | 126 | Receiving/copied reduction, early merge, custom-bound errors and skipped reads |

`native_matrix_cases.py` checks each complete input roster against its canonical digest. The primary generator
remains in `unvisited_matrix_cases.py`; its CLI delegates to the shared multi-panel workflow. Reference capture and
candidate execution retain process identities, outputs and completion status without automatic signals.

Four original successful trim results contain inconsistent spans/bucket counts. Their complete original payloads
are retained in `native-trim-source-results.json`. The panel validator accepts exactly those four declared outcomes,
including labels, timestamps, notes and every raw field. All other results retain strict canonical validation;
inputs and comparison policy are unchanged. This is source behavior, not a general validation exemption.

Ordinary histogram outputs retain the existing `Compact(0)` packing equivalence in `contract.json`.
Upstream's reusable histogram targets also make absent-versus-empty buffer presentation depend on previous queries:
the same `h[2s]` input returns negative bucket arrays `[null, null]` in a fresh original process and `[null, []]`
after earlier queries. `FloatHistogram.CopyTo` retains cleared destination buffers, and the matrix selector reuses
those targets (`model/histogram/float_histogram.go:119-120`, `promql/engine.go:2876-2886,3083` at the pinned commit).
This panel does not pin that pool-dependent presentation. Rust/C output copies still preserve their own evaluated
arrays; the four malformed trim controls require complete literal payload equality.

Capture and qualify one panel using existing verified sources and a frozen candidate:

```sh
python3 tests/promql-conformance/native_matrix_calls.py --panel unvisited-primary --capture \
  --upstream "${PROMQL_UPSTREAM_SOURCE:?}"
python3 tests/promql-conformance/native_matrix_calls.py --panel unvisited-primary \
  --reference "${PROMQL_MATRIX_REFERENCE:?}" --binary "${PROMQL_QUERY_FIXTURE:?}" --page-size 1024
```

The same commands accept every panel in the table. Permanent engine regressions cover a 100,000-point discarded
history, late faults, cleanup/reuse, native acquisition and raw helper diagnostics. The native kernel, expression,
portable arithmetic and foreign embedding checks cover shared helpers beyond this panel. These finite gates do
qualify their declared source panels. The temporal backing panel above now passes; these gates do not qualify
a live storage/API integration.

## Reference and profiles

The reference is **Prometheus v3.15.0**, module `v0.315.0`, commit
`5241a27fe3c6983549fccc32f6e65917408c63cd`. Commit identifiers, module checksums and source digests in this
directory identify upstream source; they are not credentials or private identities.

- `server-default`: stable functions and syntax, duration expressions enabled, `@` and negative offsets enabled.
  Delayed name removal and start-timestamp evaluation are disabled.
- `upstream-tests`: the four parser options used by the upstream builtin tests are enabled, together with delayed
  name removal and start-timestamp evaluation. Optional syntax remains attributable to this profile.
- `language-catalog.json`: the pinned function signatures, 90 functions (73 stable, 17 experimental), 14 aggregations
  and profile options. The inventory is separate from the source-bound implementation and qualification evidence.
- `upstream-sources.json`: digests for all 21 evaluation DSL files (2,243 commands: 2,240 `eval` and three `eval_fail`;
  12,023 physical lines)
  and the parser/lexer fixture sources. `upstream_inventory.py` rejects any digest change.
- `contract.json`: the versioned target, required corpora, numerical contract and remaining stages.

The oracle records Go version, architecture, operating system and Unicode version. The checked catalog used
Go 1.27.1 on Linux amd64 with Unicode 17.0.0. A different toolchain must be attributed, especially for regex
Unicode behavior; a release pin alone does not pin the Go standard library.

The standalone oracle uses a five-minute lookback, one-minute implicit subquery step, ten-million-sample ceiling
and one-minute timeout. The complete builtin capture preserves the upstream test engine's 10,000-sample ceiling,
100-second timeout and nil query options. Requests can override lookback and subquery step in the standalone oracle.
These are harness settings; the approved Rust engine has resource enforcement disabled.

Authoritative source owners at the reference commit:

- `cmd/prometheus/main.go:405`: server duration-expression option.
- `promql/promqltest/test.go:92`: builtin-test parser options; `:99`: engine options.
- `util/almost/almost.go:26`: numerical tolerance algorithm; builtin-test epsilon is `1e-6`.
- `promql/promqltest/test.go:1397`: native histogram comparison; bucket packing is compared after `Compact(0)`.
- `model/histogram/generic.go:264`: histogram compaction.

## Build and validate offline

Use an existing Go module cache containing the pinned sources. Downloads and toolchain auto-installation are disabled.
The builder verifies cached module hashes, builds the oracle and runs its focused Go tests. The Python harness uses
the standard library and PyYAML from the existing development environment.

From the repository root:

```sh
python3 tests/promql-conformance/build.py --mod-cache "${PROMQL_GO_MOD_CACHE:?}"
python3 -m unittest discover -s tests/promql-conformance -p 'test_*.py' -v
python3 tests/promql-conformance/upstream_inventory.py \
  --upstream "${PROMQL_UPSTREAM_SOURCE:?}"
tests/promql-conformance/.tmp/oracle --catalog
```

These path arguments describe existing sources; the tools do not create environments or download dependencies.
All generated binaries, build caches and private fixtures remain in ignored `.tmp/`. Numeric and histogram validation
retains a uniquely named input directory for reproduction. The file-backed Rust fixture provider also requires
`tests/promql-conformance/.tmp/` in the tested checkout. `build.py` creates it; create this ignored directory before
running Cargo tests directly or testing a separate source copy. The production provider owns its backing policy
and creates no files through the Rust core.

The Go tests assert fixture-derived values for typed samples, staleness, strings, time, joins, absence, classic and
native histograms, offsets, subqueries and threshold/bool results. Mutation tests reject changed complete results.
The numerical comparator is checked against actual upstream `almost.Equal` on 4,177 pairs, including special values
and seeded arbitrary bit patterns. Histogram packing equivalence is checked against actual upstream `Compact(0)`.
These validate the harness; they do not substitute for running a Rust candidate.

## Typed inputs and results

The internal wire format has `schema_version: 1`.

- Dataset: `series` is ordered; each entry has `labels` and `points`.
- Labels: arrays of `{name, value}`, both base64 byte strings. Invalid UTF-8 and NUL remain lossless.
- Sample: `time_ms`, optional `start_time_ms`, `kind`, and one corresponding payload. Input kinds are `float`,
  `histogram` and `float_histogram`; timelines may mix them.
- Float: `float` contains exactly 16 hexadecimal digits representing the raw binary64 bits.
- Histograms: schema, reset hint, zero threshold/count, count/sum, positive/negative spans and buckets, custom
  boundaries. Integer buckets retain signed deltas; float buckets retain raw bits. `Span` keys are `Offset` and `Length`.
- Requests: nonempty unique `id`, `query` or lossless `query_bytes`, `profile`, `mode`, and the relevant instant
  `time_ms` or range `start_ms`/`end_ms`/`step_ms`. `lookback_ms` and `subquery_step_ms` are optional.
- Results: `kind`, complete `rows`, separate warning/info sets and an optional structured `error`. Errors retain
  request/construction/execution phase, upstream type, original message bytes and parser positions when available.
- Precise query fixtures additionally accept `precise.start`/`precise.end` timestamps with `unix_seconds` and
  normalized `nanoseconds`, and raw `interval_ns`/`lookback_ns`. Instant requests use `precise.start`.
  Request clocks retain nanoseconds; stored samples, evaluator grids and output points remain milliseconds.
- Strings use base64 `string`; a missing string field represents an empty string. Output histograms are float histograms.
- Matrix output may contain repeated points at one timestamp, including within one kind. Comparison pairs
  points by timestamp, kind and occurrence, preserving equal-time order within each kind. Equal-time cross-kind
  serialization order is unspecified. Input timelines remain strictly increasing across all kinds, and vector
  rows contain one point. Pinned `aggregationK` can merge full-hash aliases into one row with repeated floats;
  their sequence and multiplicity are required output behavior.

Run the oracle with dataset and request files:

```sh
tests/promql-conformance/.tmp/oracle dataset.json requests.json > reference.json
python3 tests/promql-conformance/compare.py reference.json candidate.json cases.json
```

`cases.json` is a nonempty list of required IDs with optional Boolean `ordered` and `exact_numeric` flags.
Missing, duplicate and unexpected results fail; malformed wire data is a harness error.

Numeric fields use the pinned upstream tolerance, including its much stricter near-zero rule. Labels, times, sample
types, membership, strings, warning/info sets and errors remain exact. Histogram schemas/reset hints and bucket
indices are exact after zero-bucket compaction; zero thresholds and custom boundaries use exact numeric equality.
Every paired numeric bit difference is reported, including signed zero and NaN payloads. `--strict-bits` makes those
differences a separate failure gate. It is not the ordinary numeric contract.

## Computed upstream frontend fixtures

Capture the actual upstream lexer and expression-table cases from the verified module cache:

```sh
python3 tests/promql-conformance/capture.py \
  --upstream "${PROMQL_UPSTREAM_SOURCE:?}" --mod-cache "${PROMQL_GO_MOD_CACHE:?}"
```

The capture runs the original upstream assertions for all 157 lexer cases and 356 expression cases. It includes
52 declared lexer failures, 148 parser failures, nine series-description lexer cases, invalid byte strings,
recoverable lexer errors and the complete emitted streams. Token numeric codes have a captured semantic-name table.
This captures the expression table and lexer table; it does not claim every upstream parser helper test is imported.

A private source facade preserves every original module file byte-for-byte, including embedded test data. Additional
test hooks and shared wire/codec packages are added; the module cache stays unchanged. Workspace discovery and downloads
are disabled. The manifest records every source, added-file and output digest, with actual counts.
Outputs remain in ignored `.tmp/`. Use `--surface frontend` or `--surface evaluation` to capture one surface.

ASTs use flat node tables with node references, preserving all fields and spans without imposing the JSON encoder's
nesting limit. Strings remain base64 bytes and floats retain raw bits. Repeated parser-error queries reference the
case input once; error order, messages, positions and line offsets remain explicit. Successful ASTs/token streams
and all ordered errors are also compared with the computed table's declared expectations.

These captures establish reference fixtures and original assertions. Separate candidate execution passes the
computed frontend panels on all six target/build variants; see `core-qualification.json` for source attribution.
Reference capture alone is not a Rust conformance result or coverage of every parser helper.

## Complete builtin evaluation fixtures

The default capture includes all 21 builtin files through the original loader and assertions. It captures **2,243
source commands and 21,442 actual query executions**, including generated `@` expressions and range checks. The earlier
inventory missed an indented `eval` at `promql/promqltest/testdata/start_timestamps.test:169`; source digests are unchanged.

- `evaluation.jsonl`: source command/file/line, generated ordinal, byte expression, instant/range request, nil query
  options, effective evaluation context, assertion scope, dataset identity and complete result.
- `evaluation-metadata.json`: original assertion/replay status, all source command identities, actual counts, Go/Unicode
  environment and every effective engine/parser option. Upstream test limits are attributed here.
- `datasets/<sha256>.json`: 296 content-addressed snapshots of actual committed storage. They include decoded start
  timestamps, native histograms, mixed/stale samples and NHCBs generated by the original loader. Snapshot order is retained.
- `oracle/wire`: shared byte/bit/histogram model and test storage adapter, without parser/engine imports.
- `oracle/codec`: shared result/error encoding, copied unchanged except for its wire import path into the test facade.

Every captured execution is replayed through the same Go engine and query options using its snapshot. Replay requires
exact numerical bits, labels, timestamps, sample types, reset hints and diagnostics. It normalizes unordered result rows
and nil versus empty histogram arrays. Results are deep-copied before upstream comparison/compaction and query `Close`.
The checked snapshots occupy 12,167,958 bytes; the result stream is 33,163,800 bytes. Runtime counts are checked on each run.

Generated range checks preserve their original assertion scope: some check only a middle step, and ordered instant
tests only require the generated range execution to succeed. The capture still retains the complete oracle result;
it does not claim upstream independently asserted every generated range point. A passing snapshot replay qualifies
this test adapter for these fixtures; arbitrary TSDB iterator behavior remains outside that evidence.

Materialized input snapshots and owned reference results are **test adapters**. They do not change the approved Rust
execution architecture: bounded raw pages, sufficient operator state, exact replay and incremental output.

## Owned Rust frontend

`the repository root` is a standalone Cargo workspace, edition 2024 with an MSRV declaration of 1.91.
It is outside the Agent/plugin default build. The library uses pinned `regex-automata` and `regex-syntax`;
the optional `fixtures` feature adds `serde_json` for the test binary's projections.
The checked toolchains are Rust 1.97.1 and the declared MSRV, Rust 1.91.0.

```sh
python3 tests/promql-conformance/build_frontend.py
python3 tests/promql-conformance/frontend.py --capture "${PROMQL_CAPTURE_DIR:?}"
python3 tests/promql-conformance/literals.py
python3 tests/promql-conformance/parser.py --capture "${PROMQL_CAPTURE_DIR:?}"
python3 tests/promql-conformance/parser_cases.py
```

The builder uses offline, locked Cargo operations and the ignored `.tmp/core-target` output directory.
The driver verifies the reference fixture digest and compares all lexer items, byte positions and diagnostic bytes.
The current candidate passes **157/157 lexer cases** and **7/7 captured string literal values**. Focused Rust
tests also cover invalid literals, non-UTF-8/NUL escapes, raw CR/newline preservation and Go `%#U` escape diagnostics.

`literal-cases.json` contains 32 independently captured pinned Go parser results. `literals.py` compares exact
number bits, error bytes and conversion to nanoseconds against the Rust helpers. Integer-first octal/hex parsing,
decimal underflow/overflow, NaN payloads and split integer/fractional duration seconds all pass. Out-of-range
float-to-int conversion matches the pinned Go/amd64 behavior; this is not a universal Go-language guarantee.
`capture/literal_cases.go` regenerates these results from JSON `{id,input}` requests on stdin when run from the
locked oracle module with its existing module cache and downloads disabled. These targeted helper cases do not
qualify complete parsing or evaluation.

Bucket labels use float-only conversion, while NUMBER keeps its integer-first conversion. Thus `010` is decimal 10
for a bucket bound and octal 8 for a NUMBER; float-only `-0` keeps its sign and `0x10` requires a binary exponent.
The shared frontend handles Go hexadecimal floats with sticky truncation, subnormal ties-to-even rounding
and range-error classification. Decimal conversion preserves Go's optimized 19-digit path, frozen exponent prefixes
and distinct 800-digit fallback. It scans borrowed input; fallback scratch is accounted only when needed. Input and
fallback digit walks support cancellation/deadlines.

`float_literals.py` captures all 264 original Go 1.27.1 float-test inputs and independently generated boundaries,
specials, raw bytes, long mantissas and round trips. It verifies the pinned runtime source hashes, retains the source,
probe, requests and results in ignored `.tmp/`, and compares value bits and error classes exactly. The 13,087-case
panel passes in both release profiles, including 933 added long-decimal cancellation/fallback cases. The earlier
12,154-case panel missed these source behaviors; both original captures and failed before binaries remain retained.
Eighteen additional scalar comparison requests now match Prometheus exactly across parser profiles, query modes
and fixture page sizes. All 264 core unit tests pass in both profiles and Rust 1.91; the fixtures feature
adds one transport test. The existing
157 lexer, 356 AST, 784 parser-option and 32 literal cases continue to match. The complete expression panel remains
17,330 passing variants and 4,112 unsupported variants, with no lost passes or new paired numerical differences.
This is a finite parser-helper check; the histogram expression qualification below covers its actual callers.

```sh
python3 tests/promql-conformance/float_literals.py
python3 tests/promql-conformance/float_literals.py --capture "${PROMQL_FLOAT_CAPTURE_DIR:?}" \
  --binary tests/promql-conformance/.tmp/core-target/release-min/promql-frontend-fixture
```

Numeric annotations and generated labels require Go-compatible float text. The Rust numerical helper supplies
shortest lowercase g, two-significant-digit g and Prometheus OpenMetrics projections. It uses a 32-byte inline
output buffer and fixed digit/scalar state, sharing the float parser's integer scaling table. Final text can be
appended into the caller's accounted destination. General text preserves negative zero; OpenMetrics renders both
zero signs as `0.0` and appends `.0` to integers without an exponent. NaN and infinity text, decimal carry,
ties-to-even rounding and exponent signs/padding follow the original sources. Distinct fixed-six error formatting
keeps its existing contract.

`float_text.py` first runs untouched Go `TestFtoa` and compiles the original test declarations, preserving all
148 binary64 FormatFloat/prefixed AppendFloat assertions and the 38 applicable g rows. Additional bit inputs cover
every finite binary exponent at four mantissas, decimal-decade neighbors, rounding ties, signed specials and seeded
values. The reference calls actual pinned `FormatOpenMetricsFloat` for its label projection. Source, profile,
module, probe, generator, input and result hashes bind the capture. All **80,942 text/round-trip cases match exactly**
in both release profiles; all 13,087 float parser cases and the existing frontend/expression outcomes remain exact.
These are finite numerical-helper checks; histogram expression execution is qualified separately in the later
expression panels.

```sh
python3 tests/promql-conformance/float_text.py
python3 tests/promql-conformance/float_text.py --capture "${PROMQL_FLOAT_TEXT_CAPTURE_DIR:?}" \
  --binary tests/promql-conformance/.tmp/core-target/release-min/promql-frontend-fixture
```

The annotation owner supports histogram invalid-quantile, malformed/missing bucket-label,
mixed-classic/native and native-NaN notes. Native kernel text projections share their static message owner
with the accounted annotation constructors. Forced-monotonicity notes retain their first source position,
merge occurrence counts and timestamp/bucket/difference extrema, and preserve signed-zero/NaN comparison
behavior. Opaque provider messages replace the previous payload. Empty query text returns the raw message.

Only monotonicity notes own an optional tracked box for the extra fields; regular notes retain one pointer
of additional state. Raw and formatted byte capacities, the box and shared result ownership are precisely
accounted. UTC timestamp text uses the existing calendar arithmetic, whole-second truncation and Go's signed
year padding, including years outside strict RFC3339 bounds.

`histogram_annotations.py --capture` first runs untouched upstream annotation and Go RFC3339 tests, then
captures the original constructors and merge operations. Its **1,583 cases match exactly in both release
profiles**, including negative/extreme times, invalid label bytes, optional names, repeated and child merges,
provider replacement, raw/final messages, severities, positions and occurrences. Hashes bind sources, runtime
profile, helpers, inputs and original results in ignored `.tmp/`. Ownership units and focused Miri checks
cover precise payload charges, retained result lifetime and replacement cleanup. This qualifies annotation
helpers; histogram expression dispatch and bounded count replay remain required.

```sh
python3 tests/promql-conformance/histogram_annotations.py --capture
python3 tests/promql-conformance/histogram_annotations.py --reference "${PROMQL_ANNOTATION_CAPTURE_DIR:?}"
python3 tests/promql-conformance/histogram_annotations.py --reference "${PROMQL_ANNOTATION_CAPTURE_DIR:?}" \
  --binary tests/promql-conformance/.tmp/core-target/release-min/promql-frontend-fixture
```

The typed AST uses flat arena references and cached structural types/spans. Its 90 function signatures come from
the independent language catalog. The owned Pratt parser and ordered typed validator use explicit continuation
stacks. `parser.py` executes all 356 captured expression cases and compares complete AST fields and ordered error
bytes/positions. **356/356 match exactly**, including the 41,001-node AST and 10,999-error case. Compiled selectors
use the owned regex frontend below. The parser driver fails on any difference and writes differences to ignored `.tmp/`.
The committed `parser-cases.json` adds **784/784 exact public regression/profile cases**, captured independently
from Go for all sixteen parser-option combinations. They cover trailing argument commas, malformed grouping,
fixed-arity duration calls, repeated commas, surviving parentheses after recovery, chained offset signs and
experimental gates. Regenerate with
`parser_cases.py --mod-cache "${PROMQL_MODULE_CACHE:?}"`; the capture verifies every source file before and after
execution and leaves the reference module cache unchanged. These finite cases do not prove arbitrary-input parity.

- Tokens borrow their original input spans; error items own their message bytes. UTF-8 failure consumes one byte,
  and quoted strings distinguish byte escapes from Unicode escapes.
- Go diagnostic printability uses a generated Unicode 17.0.0 table from Go 1.27.1 `strconv.IsPrint`.
  `capture/go_tables.go` regenerates the range data without dependency downloads. Source notices accompany the crate.
- Both release and release-min retain unwind support. The crate rejects `panic=abort` builds; this prevents inheriting
  the plugin workspace's abort profile. The synchronous C boundary and its finite embedding checks are described below.
- Expression consumers stop on the first lexer error. Raw-string EOF is terminal in Rust, avoiding the reference
  scanner's repeating error state; the first error and all captured finite streams match.

These are finite frontend checks. Remaining helper/source corpora, evaluation, the C interface and runtime accounting
still require qualification; captured parser matches cannot establish query compatibility.

Set `exact_numeric` for boolean/discrete result cases. Always test the complete filtered/bool expression around a
threshold, alongside its numerical intermediate. The preserved prototype rate regression demonstrates why a
one-bit numerical difference can change whether an alert returns a series. Numeric tolerance never excuses that
changed result. Require `ordered` for order-sensitive output; the oracle retains traversal order even when ignored
by a case comparison.

## Owned Go-compatible regex frontend

The Rust frontend owns byte parsing, canonical syntax, simplification, capture numbering, Go replacement expansion
and Prometheus selector optimizations. It constructs a flat Go metadata program with explicit work stacks, then
uses the public `regex-automata 0.4.16` Thompson Builder and PikeVM. `regex-syntax 0.8.11` supplies bounded UTF-8
range encoding. The backend does not parse the expression or recursively compile its syntax tree. Release and
release-min tests compile, execute and drop 999 nested captures and 1,500 noncapturing groups on a 128 KiB test stack.
The test creates a thread to control stack size; the library creates no runtime or execution thread.

Upstream has two syntax origins. Selectors first parse with `grafana/regexp/syntax` at
`f7b3be9d18538c56fb03caa6088db55404e784e8`, serialize, then compile with Go 1.27.1 standard syntax. Label replacement
compiles its wrapped original pattern with standard syntax. The owned frontend preserves their different Unicode
aliases and case-folding equality. Unicode properties, simple fold and lowercase use Go Unicode 17.0.0 tables;
map optimizations use pinned `golang.org/x/text v0.41.0` NFKD and stream-safe decomposition.

Capture and compare using the existing pinned cache:

```sh
python3 tests/promql-conformance/regex_capture.py --mod-cache "${PROMQL_GO_MOD_CACHE:?}"
python3 tests/promql-conformance/regex_cases.py --capture "${PROMQL_REGEX_CAPTURE_DIR:?}"
```

The checked capture contains **21,320 exact Rust comparisons**:

| Surface | Cases |
|---|---:|
| Fork syntax / standard syntax | 478 / 478 |
| Compilation / original program and OnePass metadata | 40 / 237 |
| Find, capture vectors and byte replacement | 4,505 |
| Anchored label-replacement kernel | 4,348 |
| Prometheus selector matching | 11,188 |
| Normalized and lowercase byte strings | 46 |

Original Go assertions run before capture. Private source facades and the immutable cache retain all 33 Grafana
and 1,654 Prometheus source files byte-for-byte. Original input bindings, targeted requests, source manifests,
added-hook whitelist and output hashes are checked before candidate execution. Mutation tests reject missing
cases/fields, changed syntax origins/profiles, malformed capture vectors and modified source/hook bytes.
Every expected field is compared, including error bytes, canonical syntax, duplicate capture names, offsets,
nil versus empty rune slices, branch links, OnePass eligibility and prefix fields. No unsupported case counts as passing.

Targeted inputs include invalid UTF-8, ASCII Perl classes, Unicode 17 properties, duplicate capture names, replacement
name grammar, surrogate-prefix behavior, 16-entry normalized maps, overlapping contains retries, nullable repeats,
and empty-class concatenations. They include all 37 original OnePass compilation patterns and its separate execution
regression. Five Rust unit tests pass in release and release-min; this includes the small-stack test above.

This finite corpus does not prove arbitrary-pattern compatibility. RE2/Fowler/exhaustive suites and remaining
optimizer helper suites are outside this evidence. The anchored replacement checks qualify its regex kernel, not
the PromQL `label_replace` evaluation and label mutation qualified separately by the expression gates. PikeVM construction currently omits meta-engine
accelerators. A finite [boolean selector panel](#boolean-selector-search) now measures complete-query performance;
it does not establish arbitrary-pattern throughput. Go syntax limits and optimizer strategy thresholds are
preserved as upstream behavior. No engine regex or query resource quota is enabled.

Opaque regex-backend allocations and selector hash-table backing memory are reported as explicit estimates.
Retained regex plans, keys, names and compilation scratch use observed buffer capacities. Invalid-byte subjects,
offsets, normalization, retry frames, captures and replacements use the current search owner; shared compiled
matchers retain one charge. Public backend memory reports are observed after construction/mutations and searches.
Their hidden compilation/search capacities and temporary peaks, and hash-table resizing peaks, remain unknown.
Observable engine-owned buffers and borrowed storage pages remain subject to precise accounting. A total containing
estimates cannot be a guaranteed bound. This accounting trade-off retains the qualified matcher and all 21,320
original regex cases pass unchanged in both build profiles. Engine-owned runtime accounting and foreign allocation
leases are described in the C embedding and allocation ownership sections below.

### Literal prefix selector examples

`selector-prefix-inputs.json` preserves the five entries in the pinned Prometheus test table
`model/labels/regexp_test.go::TestStringMatcherFromRegexp_LiteralPrefix`. The standalone Go AST extractor
`capture/selector_prefix_sources.go` reads the literal table that this function iterates. It uses `strconv.Unquote`
and retains source positions, file/function hashes, matcher counts and all 32 positive plus 20 negative declarations.
Computed, missing, duplicate and unexpected fields are rejected. Matcher counts describe the Go test's provenance;
they do not assert the Rust optimizer's internal shape.

`selector_prefix_cases.py` turns each declared pattern/subject pair into complete `=~` and `!~` expressions. Each
subject has a unique series ID and finite value. The 104 outcomes must have complementary singleton/empty membership,
exact labels, timestamp, value bits and empty diagnostics. Regex patterns and actual newline subjects retain their
original bytes through PromQL string quoting. The old 21,320-case capture and its hash-pinned hooks stay unchanged.

Run from this crate with the pinned Go toolchain. `PROMQL_UPSTREAM_DIR` must identify the verified original source
facade, and `PROMQL_PREFIX_DIR` an existing private directory for fresh files. First extract and check the source:

```sh
go run tests/promql-conformance/capture/selector_prefix_sources.go "${PROMQL_UPSTREAM_DIR:?}" \
  > "${PROMQL_PREFIX_DIR:?}/source.json"
python3 tests/promql-conformance/selector_prefix_cases.py \
  --check-source "${PROMQL_PREFIX_DIR:?}/source.json"
python3 tests/promql-conformance/selector_prefix_cases.py --write-dir "${PROMQL_PREFIX_DIR:?}"
```

Run the unchanged named original test with `-count=1` and the exact test-name filter before capturing expressions.
Authenticate the retained general query probe through `plural_histogram.load`, then run that probe with the new
`dataset.json` and `requests.json` as its two arguments. Its historical loader authenticates the probe; it does not
qualify the new request roster. Separately bind the new ledger, dataset, requests, ordered IDs, runtime header and
raw output before and after execution. Retain that output as `original.json`, then check a source-bound candidate:

```sh
python3 tests/promql-conformance/selector_prefix_cases.py \
  --check-original "${PROMQL_PREFIX_DIR:?}/original.json"
"${PROMQL_QUERY_BINARY:?}" < "${PROMQL_PREFIX_DIR:?}/inputs.jsonl" \
  > "${PROMQL_PREFIX_DIR:?}/candidate.jsonl"
python3 tests/promql-conformance/selector_prefix_cases.py \
  --compare "${PROMQL_PREFIX_DIR:?}/original.json" "${PROMQL_PREFIX_DIR:?}/candidate.jsonl"
```

The [qualification record](selector-prefix-qualification.json) contains 416 exact comparisons: all 104 expressions
on retained native release/release-min and ARM32/ARM64 release fixtures, with page size 17 and QEMU for ARM.
All 348 prior runtime inputs are byte-identical; no engine rebuild or fix was needed. Seven focused validator tests,
six actual extractor rejection controls and three CLI checks qualify the new capture workflow. This adds finite
matching evidence; it makes no new language-feature, optimizer-branch, performance, memory, C embedding or live API
claim. Historical qualification keeps its actual source and binary identities.

### Suffix and optional-character selector examples

`selector-suffix-inputs.json` preserves the pinned `TestStringMatcherFromRegexp_LiteralSuffix` and
`TestStringMatcherFromRegexp_Quest` tables from `model/labels/regexp_test.go`. Their ten patterns have 101 declarations:
60 positive and 41 negative. One positive `bbbddd` declaration repeats. Keep both occurrences, their array ordinals
and distinct literal byte spans, line numbers and columns; generate only 100 distinct pattern/subject pairs.
Contradictory expectations are an error. Internal matcher counts describe the original Go assertions, not Rust
optimizer requirements.

The tests use anchored dot-newline matching. Actual `test\n` subjects match `test.?`; a source comment suggesting
otherwise does not override its assertions. Scoped case folding accepts `bbbXXXCCC` while rejecting `BBBccc` in the
relevant suffix pattern. `selector_suffix_cases.py` validates the source and deduplicates generation;
`selector_source_cases.Panel` owns the shared prefix/suffix construction, exact comparison, packets and CLI wiring.
The prefix adapter retains its 104 generated queries, complete expected objects and valid CLI outputs. The Go
extractor's original one-argument output remains byte-identical.

Run from this crate with the pinned Go toolchain, verified `PROMQL_UPSTREAM_DIR` facade and existing private
`PROMQL_SUFFIX_DIR` containing no prior fixture files:

```sh
go run tests/promql-conformance/capture/selector_prefix_sources.go "${PROMQL_UPSTREAM_DIR:?}" \
  TestStringMatcherFromRegexp_LiteralSuffix TestStringMatcherFromRegexp_Quest \
  > "${PROMQL_SUFFIX_DIR:?}/source.json"
python3 tests/promql-conformance/selector_suffix_cases.py \
  --check-source "${PROMQL_SUFFIX_DIR:?}/source.json"
python3 tests/promql-conformance/selector_suffix_cases.py --write-dir "${PROMQL_SUFFIX_DIR:?}"
```

Run the three unchanged prefix/suffix/optional-character original test functions with an exact name filter and
`-count=1`. Authenticate the retained general query probe through `plural_histogram.load`, then supply the new
`dataset.json` and `requests.json`. Separately bind the new source ledger, request roster, runtime header and raw
results; authenticating an older probe does not authenticate new requests. Check the fresh `original.json` and
candidate with the suffix adapter's `--check-original` and `--compare` options, following the prefix command pattern.

The [qualification record](selector-suffix-qualification.json) retains 800 new complete comparisons and 416 unchanged
prefix comparisons across native release/release-min and ARM32/ARM64 release fixtures, page 17 with QEMU for ARM.
All 349 prior runtime inputs remain unchanged, so these are the exact already-qualified binaries. Eleven tooling
validators, ten actual extractor rejection controls, literal-position checks and CLI checks cover the shared tools.
The three original test functions and fifteen subtests pass without a cached test result. All 200 fresh original
expressions have complete complementary singleton/empty outcomes, exact labels, timestamp, value bits and diagnostics.

These are additional finite matching examples. Earlier source/capture/binary identities, the old 21,320 regex records
and all previous qualification objects remain intact. No new engine build, function count, branch coverage,
performance, memory, C embedding, live API or concurrent-throughput claim is introduced.

## Bounded UTF-8 validation

The shared validator used by regex subjects, quoted grouping labels and label-function names validates at most
1024 bytes per standard-library call. A valid scalar crossing that boundary is decoded against the original
remaining bytes. Invalid sequences consume one byte at their first malformed position, matching the existing Go
rune decoder; a valid U+FFFD is accepted. No normalization buffer, retained state or dependency is added.

The validator keeps its initial control check and checks before each batch charge. At most 1027 distinct input bytes
are examined between checks, including a crossing four-byte scalar. It retains the existing final-charge phase;
there is no added terminal check. Uninterrupted work remains exactly the valid byte length or first invalid offset
plus one. Intermediate work-triggered test checkpoints can move; caller cancellation/deadline control remains
cooperative, with cancellation taking precedence. Regex compilation/name validation and subject normalization stay
at their original call sites.

`frontend::work::tests` checks all 65,536 two-byte combinations, scalar crossings, malformed/truncated boundaries,
exact work, initial cancellation/deadlines, interrupted long validation and terminal behavior. The fixed
`utf8_work_cases.cases()` generator supplies 78 complete query cases: 20 small/large ASCII execution controls,
four byte-subject/capture queries and 54 name/grouping/error-order cases. It returns the dataset/query packets,
independent count expectations and fixed timing IDs. Combine them with the retained 242 selector controls for the
native comparison; the foreign subset uses all 58 boundary queries and the four 16-series controls.

Qualification preserves the actual 21,320 regex originals and 1,140 parser/profile originals without recapture.
Complete results include parse-only packets, which correctly omit execution observations. For executed queries,
compare nonelapsed observations separately: fewer accounting updates must not be reported as less work or memory.
Time fixed alternating before/candidate executions in both unwind profiles after owned qualification jobs close.
Engine elapsed excludes compilation and fixture transport; these shared-host synthetic timings do not establish
live API, concurrent-query, physical ARM or RSS performance. Actual source/binary identities, all timing samples,
resource equality and code-size costs belong in `utf8-work-qualification.json`.

The [source-bound qualification](utf8-work-qualification.json) passes 686 native units per profile,
127,920 original regex outcomes, 6,840 original parser outcomes, 640 native and 248 foreign complete query
comparisons, and 12 actual C/C++ callers. All 632 native executed-query observation groups remain equal; eight
parse-only packets remain exactly equal without observations. Foreign units focus on work and regex owners.

Fixed 256-series examples below are five-sample median execution milliseconds. The negative-selector row makes
the mixed size-optimized result visible; the report retains all 40 case/profile medians and 800 checked responses.

| Label bytes / selector | Release before → candidate | Release-min before → candidate |
|---|---:|---:|
| 128 / positive | 11.920 → 10.895 | 25.665 → 25.361 |
| 1024 / positive | 84.454 → 75.608 | 193.954 → 186.804 |
| 4096 / negative | 353.378 → 322.116 | 757.927 → 779.903 |

Three release medians are slower, all optimized controls, by at most 1.73%; nine release-min medians are slower,
by at most 2.90%. Fixture ELF text grows by 800 bytes in release and 96 bytes in release-min. This change reduces
validation overhead; it does not reduce the recorded work or memory footprint. The existing 73 ledger objects
retain their original identities. The initial comparison harness corrections and verified reuse of successful
commands are recorded separately from runtime results.

## Typed sample and initial histogram kernels

This is an earlier helper checkpoint. Its counts, bit differences and then-unfinished expression/runtime owners
remain historical; the standalone overview and source-bound core report state current qualification.

The standalone crate now has owned and borrowed float/integer histogram payloads and timestamped samples.
Borrowed views carry the provider's page lifetime. Retaining one copies mutable spans/buckets and custom bounds;
owned histogram copies share immutable custom bounds. These types do not retain a series history or implement
query evaluation. Nil and empty arrays, reset hints, integer deltas and raw floating-point bits remain distinct.

The kernels implement schema-aware Copy/CopyTo, multiplication/division, integer-to-float conversion, compensated
summation, sparse compaction, schema copy/reduction, bucket boundaries, borrowed bucket traversal, custom-bound
matching, counter reset detection, Add/Sub/KahanAdd, data equality and overflow scans. Reduction reuses the current payload's buffers; borrowed schema copying
constructs only the reduced result. Boundaries use all 511 compiled Go constants
and the Go scaling operation order, including subnormal rounding and the last finite bucket. Capture and compare
with the existing toolchains:

```sh
python3 tests/promql-conformance/histogram_capture.py --mod-cache "${PROMQL_GO_MOD_CACHE:?}"
python3 tests/promql-conformance/histogram.py --capture "${PROMQL_HISTOGRAM_CAPTURE_DIR:?}"
```

All twenty-five original histogram tests run first. Exact Go-AST declaration spans and procedural bodies supply 238 source fixtures,
including the dedicated reverse-iterator and paired-compaction setups. The 5,843 separately attributed supplemental cases cover special
factors, hints, high integer counts, compensation,
nil/empty layouts, partial reduction failures and every compiled bound fraction at ten critical exponents.
Map cases use source-bound keys rather than iteration order.
Iterator fixtures compare all returned buckets, indices, bound bits, inclusivity and panic messages across eight
routes. They cover both directions, schema merging, custom buckets, malformed counts/spans, zero clipping even
without a populated zero bucket, and start-value filtering after a whole target bucket is merged. The 104 iterator
fixture groups exercise 595 Go routes; both owned and borrowed Rust entry points are compared. Direct zero-bucket
fixtures also verify special values and the custom-schema panic guard, including its effect in both All directions.
A separate test
skips 100,000 buckets on a 128 KiB caller stack. Iterators retain constant-sized state and borrow the payload;
only test transport materializes bucket arrays.
Reset detection includes 42 original and 34 supplementary groups. It preserves hint shortcuts, schema reduction,
missing populated buckets and custom-bound rollups without mapped count arrays. Twenty-two direct zero-threshold
groups qualify ordered compensated scans, restarted scans, optional compensation, unchanged operands, special
values, threshold-error messages and short correction-array diagnostics. Operand snapshots observe post-call state,
including panic recovery. Custom-bound equality is numerical: signed zeros match, while NaNs do not.

Arithmetic capture executes all 114 original Add/Sub/Kahan invocations, including both operand orders where the
source tests do so and six supplied-correction fixtures. It freezes operands after upstream Copy, then records raw
results before compensation folding and final compaction. Declared assertions retain their original scope; final
expected layouts never substitute for raw results. Supplementary arithmetic adds 168 invocations for all 36 pairs
of normal/unknown reset hints, nil/empty buffers, signed zeros, special values, custom-bound intersections and
cancellation with nonzero corrections. Twenty-five paired-compaction cases check both count arrays; correction
span headers retain the state produced by the source helper.
These standalone paired-compaction fixtures use independent span arrays. Go's internal compensation spans can
share receiver backing storage; direct arithmetic followed by paired compaction does not qualify that alias lifecycle.
Normal KahanAdd resynchronizes compensation spans before returning; Rust mutable buffers remain independently owned.
The right operand stays borrowed. Schema conversion constructs reduced buffers, and its initial zero correction
is read virtually. Sparse insertion and paired compaction reuse accumulator capacity; custom reconciliation builds
target main/correction arrays. These are current payloads and operator state, not retained raw history. Buffer
tests verify reuse and lifetime after the borrowed operand is released. No total-memory or accounting claim follows.

Equality capture preserves the complete sequential bodies of both upstream equality tests, including integer-to-float
conversion and assertions before each observation. All 41 original pairs and 30 additional pairs run in both directions.
Counts, sums and bucket data use bit equality; thresholds and custom bounds use numeric equality. Empty spans fold with
wrapping offsets, while adjacent nonempty spans remain distinct. Thirty-five overflow witnesses distinguish infinities
in watched fields from NaNs and the ignored zero threshold. Both helpers borrow the payload and use constant auxiliary memory.
Their unchanged operand snapshots use exact bit comparison, including NaN payloads and signed zeros.

The complete 1,654-file source facade stays unchanged. The public Go-derived input ledger binds every request,
declared expectation, source prefix/position and the source-manifest digest independently of the capture directory.
Mutation tests reject changed inputs, nil/empty substitutions, duplicate payloads, missing cases and altered origins.
When intentionally extending source coverage, `histogram_capture.py --write-input-ledger` regenerates input bindings
only from verified Go execution. It preserves the pinned source-manifest digest; ordinary capture/comparison never
refreshes the ledger. Candidate results never supply input bindings or expected values.

The immutable Rust table can be regenerated with `histogram_bounds.py --capture <verified-capture>`; these are
shared mathematical constants, while the boundary algorithm is independently compared with executed Go outcomes.
Boundary results use exact bit comparison, so underflow-to-zero cannot pass the ordinary numeric tolerance.

The current comparison passes **6,440 semantic fixture comparisons** in both release profiles, including owned and borrowed
retention paths. Compensated subtraction reports **ten NaN-sign bit differences in release and one in release-min**
on the qualified Rust/Go amd64 toolchains;
`--strict-bits` fails this separate diagnostic gate. There are also three signed-zero differences between Go's
declared table literals and actual arithmetic. Candidate comparison uses the actual independently executed Go
results, rather than those declared literals. The report preserves both kinds of difference.

Rust ownership tests mutate source spans/buckets after copying and use retained values after all provider buffers
leave scope. Stale markers are checked separately from ordinary NaNs in every sample kind. These checks qualify
only the implemented value kernels. Copy-to-schema checks preserve the unchanged receiver; failed reduction checks
include the complete mutated receiver and upstream error classification. Remaining histogram helpers,
numerical/functions algorithms, expression evaluation,
the C boundary, runtime accounting and paged-provider qualification remain required.

## Numerical prerequisites

This is an earlier helper checkpoint. Its counts, bit differences and then-unfinished expression/runtime owners
remain historical; the standalone overview and source-bound core report state current qualification.

Frexp, Ldexp, Exp2, natural/binary logarithms and public square root are implemented for the pinned Go 1.27.1 linux/amd64
profile. Log preserves the actual assembly's subnormal extraction, negative-NaN handling and inclusive half-sqrt-two
branch. Log2 retains the exact-power shortcut and ordered multiplication/addition. Sqrt uses the same compiler
primitive as Go's public intrinsic. Exp2 preserves the public generic reduction and polynomial order; Ldexp
retains signed special values, subnormal scaling and machine-int wrapping. These operations use constant auxiliary memory.

Power, remainder, min/max and natural exponential follow the pinned source algorithms. Production Exp, Pow,
Sinh, Cosh and Tanh use the retained Go amd64 FMA-on arithmetic on every device. Fused operations retain one
rounding even when the target requires a software implementation. Device feature detection does not select the
query's numerical contract. Explicit fixture helpers preserve the separate FMA-off arithmetic for diagnostics.
The 547 additional math inputs retain their exact comparisons against native, FMA-disabled and AVX-disabled Go
captures through those explicit helper branches. The power threshold and duration cases demonstrate why the
route matters to series membership. Variable power/remainder loops poll cancellation and account work without
an iteration cap. Broader platform qualification remains required.

The capture adds one test file through a Go overlay; it never changes installed source files. All six untouched
original math tests run first, including the generic SqrtGo and Exp2Go assertions. Thirty-one exact AST source definitions
and all 113 math/compiler source hashes are retained and checked. The public input ledger binds 4,336 original and
22,110 supplemental groups, covering power-of-two neighbors, all 511 compiled histogram-bound neighbors, Exp2 reduction
and cutoff neighbors, Ldexp exponent extremes/subnormal ties, and special values. Those groups execute 49,963 public
primitive calls. Original declaration comparisons preserve exact, `alike`, `close` and `veryclose` assertion semantics;
candidate comparison uses independently executed public results. All twenty earlier definitions and 10,013 input
bindings remain unchanged. Exponents retain full signed integer precision through capture and candidate decoding.

```sh
python3 tests/promql-conformance/numerical_capture.py --histogram-capture "${PROMQL_HISTOGRAM_CAPTURE_DIR:?}"
python3 tests/promql-conformance/numerical.py --capture "${PROMQL_NUMERICAL_CAPTURE_DIR:?}" --strict-bits
```

Intentional source extensions use `--write-input-ledger`; ordinary capture and comparison never refresh input bindings.
Current release and release-min comparisons pass all 26,446 groups with zero candidate bit differences.
Three original declaration-versus-Go differences remain separate: two Sqrt NaN payloads and one Exp2 value differing
by one bit. They satisfy the original assertion rules. Other numerical functions, platform qualification and complete
expression evaluation remain required.

## Histogram fractions and in-place trimming

This is an earlier helper checkpoint. Its counts, bit differences and then-unfinished expression/runtime owners
remain historical; the standalone overview and source-bound core report state current qualification.

`Bucket.FractionBelow` and `FloatHistogram.TrimBuckets` follow the pinned source, including infinite-bound
rules, one-sided zero buckets and the unchanged-total fast path. Trimming shares forward traversal's constant
position state, mutates one existing bucket slot at a time, and compacts the same buffers after traversal.
A 100,000-bucket ownership test runs with a 128KiB caller stack and verifies retained buffer addresses/capacities.

Separate additive histogram/promql test hooks run all three untouched original tests before instrumented copies.
The complete AST bodies retain the seven fraction rows, 168 directional trims, zero-count witness, two known
divergences and two ownership checks. Original declarations preserve absolute `1e-12`, relative `1e-9`, exact
Count and lifecycle assertion scopes; complete actual layouts/sums/counts are independently captured.
The public input ledger also binds 1,481 boundary/composition observations, for 1,661 comparisons in each build profile.
All 1,654 reference sources and prior histogram/numerical input bindings remain unchanged.

```sh
python3 tests/promql-conformance/trim_capture.py --mod-cache "${PROMQL_GO_MOD_CACHE:?}"
python3 tests/promql-conformance/trim.py --capture "${PROMQL_TRIM_CAPTURE_DIR:?}"
```

Both profiles pass all 1,661 comparisons with exact candidate bits. A literal-peer Trim-to-Equals regression
first reproduced a boolean failure despite numerical tolerance passing. Local NaN destination guards now preserve
the actual pinned Go accumulation/midpoint behavior; the composed owned/borrowed equality gate passes.
Eleven declared-versus-Go numeric differences (one fraction and ten Counts) remain separate and satisfy
the original assertion scope.
Copy-source snapshots and immutable metadata use exact bits. Complete expressions, C/accounting and broader
platform qualification remain unfinished.

## Native histogram fraction and quantile helpers

This is an earlier helper checkpoint. Its counts, bit differences and then-unfinished expression/runtime owners
remain historical; the standalone overview and source-bound core report state current qualification.

`HistogramFraction` and `HistogramQuantile` follow pinned `promql/quantile.go:225–516`. Each numeric scan borrows
the histogram, retains one bucket plus iterator/rank state, and returns one scalar plus an optional typed info
identity. It does not copy or compact input buckets. This preserves custom infinite-bound early returns,
slice-length zero-side checks and the quantile NaN continuation's final-bucket overwrite. Diagnostic formatting
uses the qualified Go byte quoting and original query byte positions; numeric scans allocate no message strings.
Internal variants poll every visited bucket in the main and NaN-tail scans. Cancellation witnesses
cover both phases, preserve borrowed inputs and confirm unchanged allocation observations.
A 100,000-bucket custom histogram test exercises forward/reverse scans on a 128KiB caller stack and checks
the caller's retained buffers and values.

Two additive test hooks execute untouched original QT and complete native-histogram DSL assertions first.
The input ledger binds 171 QT fractions, 118 expanded DSL fractions and 69 expanded DSL quantiles, plus 7,253
supplemental calls: 7,611 comparisons per build profile. DSL arguments are their original AST byte slices,
evaluated by Go at the original query time; derived histogram inputs freeze before the argument query closes.
All 1,654 reference sources are verified unchanged. Prior histogram, trimming and numerical ledgers remain intact.
QT declarations retain their computed trim-count cross-check rather than inventing literal fractions.

Candidate values use the pinned numeric tolerance. Complete borrowed input snapshots, info identities,
severity, raw/formatted bytes and reciprocal infinity classifications compare exactly. The eight original literal
info declarations and two `no_info` commands are also checked against the captured helper info sets. Full
expression invalid-quantile warnings and global diagnostic deduplication remain caller behavior outside this gate.
Reference decoding also checks source-owned info messages, the five metric spellings exercised by this finite
capture and original byte-position suffixes; refreshing an output hash cannot legitimize arbitrary diagnostic bytes.

```sh
python3 tests/promql-conformance/histogram_functions_capture.py --mod-cache "${PROMQL_GO_MOD_CACHE:?}"
python3 tests/promql-conformance/histogram_functions.py --capture "${PROMQL_HISTOGRAM_FUNCTIONS_CAPTURE_DIR:?}"
```

Both release profiles pass all 7,611 comparisons bit-for-bit. The earlier 80 NaN result/reciprocal diagnostics
are corrected on the qualified AMD64 target. Optimized original Go instructions determine each accumulation's
destination: the main scans preserve the arriving bucket's NaN, while quantile's tail preserves the old count.
The correction also preserves interpolation operand order. A separate 14,400-case original Go capture includes
positive, negative, custom and zero buckets, signaling NaNs and infinities; retained before results identify
1,344 bit differences, and the corrected helpers match exactly. Other-target numerical policy remains unresolved.
Twenty-four original DSL declaration-versus-Go differences remain separately reported and satisfy the original
`almost.Equal` assertions. Expression dispatch for these two helpers remains unfinished; these gates do not
establish complete Rust expression evaluation.

## Native histogram statistics

This is an earlier helper checkpoint. Its counts, bit differences and then-unfinished expression/runtime owners
remain historical; the standalone overview and source-bound core report state current qualification.

Count, Sum, Avg, StdVar and StdDev follow pinned `promql/functions.go:2044–2125`. Count and Sum return the
unchanged fields; Avg divides Sum by Count without a zero-count guard. Variance borrows the ascending bucket
iterator and retains only scalar state. It skips empty buckets, preserves custom midpoint overflow and
left-associated Count-times-delta-times-delta, and finalizes compensated addition before dividing by Count.
StdDev uses the qualified square root. These kernels allocate no bucket or result vector and emit no diagnostic.
The existing 100,000-bucket, 128KiB-stack test also checks both variance scans and retained caller buffers.

An additive test hook executes the three complete original DSL files unchanged, then observes the registered
function inputs while replaying each complete original expression. It preserves ancestor-dependent stats decoding
and selector modifiers; AST argument spans can omit `smoothed` and are retained only as diagnostic source slices.
The hook freezes payloads before query closure and restores function dispatch after each command. Eight source
owners and all 1,654 original files are bound independently. The ledger contains 135 root commands, 145 expanded
native inputs and 1,125 supplemental calls. Four float arguments are ignored by the original caller; two original
commands remain empty. Nested Count calls retain their outer expression assertions in the later evaluator gate.

```sh
python3 tests/promql-conformance/histogram_statistics_capture.py --mod-cache "${PROMQL_GO_MOD_CACHE:?}"
python3 tests/promql-conformance/histogram_statistics.py --capture "${PROMQL_HISTOGRAM_STATISTICS_CAPTURE_DIR:?}"
```

Both release profiles pass all 1,270 comparisons bit-for-bit, including exact input snapshots, nil helper diagnostics
and reciprocal/threshold outcomes. The earlier eight NaN differences are corrected on the qualified AMD64 target.
Fourteen original declaration-versus-Go differences are reported separately and satisfy the pinned
assertions. Overflow and compensation consumers distinguish reordered arithmetic that changes a comparison.
Composite expression coverage is measured separately by the evaluator gates below.

## Classic histogram bucket kernels

This is an earlier helper checkpoint. Its counts, bit differences and then-unfinished expression/runtime owners
remain historical; the standalone overview and source-bound core report state current qualification.

`BucketQuantile` and `BucketFraction` follow pinned `promql/quantile.go` and mutate one borrowed bucket-aggregate
slice in place. The kernels preserve full backing tails after coalescing, quantile monotonicity/precision metadata,
and Fraction's original cumulative counts. Go 1.27.1's comparator sort and binary-search visit order are ported
explicitly: accepted NaN bounds and counts, duplicate buckets and zero signs make these choices observable.
The pinned Go coalescing operand's NaN payload is retained locally without changing finite addition order.
A 100,000-bucket test checks descending and permuted sorts, interpolation and retained caller storage on a
128KiB stack. A group's aggregate slice may grow; this gate does not establish whole-query memory bounds.

The additive capture executes untouched quantile and `almost.Equal` assertions before observing copies with
the same assertions. It binds 29 source owners, all 1,654 Prometheus files and four Go support-source hashes.
Twenty-four original quantiles and ten near-equality assertions retain their exact source scopes. Supplemental
cases exercise sorting thresholds, correction extrema, repeated full-slice mutation, empty-helper panics,
signed-zero reciprocals and quiet/signaling NaNs. Valid quantiles and fractions require a nonempty aggregate
group; these pure-helper panic probes do not define complete expression errors.

```sh
python3 tests/promql-conformance/classic_capture.py --mod-cache "${PROMQL_GO_MOD_CACHE:?}"
python3 tests/promql-conformance/classic.py --capture "${PROMQL_CLASSIC_CAPTURE_DIR:?}"
```

The gate contains 4,176 cases: 1,574 bucket sequences and 2,602 near-equality outcomes. The bucket sequences
execute 1,676 operation steps, giving 4,278 operation/outcome checks in total. Candidate
snapshots, panic classifications, all five quantile metadata fields and composed booleans compare exactly;
scalar values use the pinned numerical contract, with every bit difference reported separately. Original
quantile values also satisfy their narrower relative-epsilon assertions. Both release profiles pass the
4,176 cases bit for bit, with three finite declaration-versus-Go differences reported separately.
Classic interpolation and coalescing use the actual pinned Go AMD64 operand destinations, including the
different addition destinations at fraction's lower and upper interpolation sites. Earlier NaN-only
candidate diagnostics are cleared; original declarations remain unchanged.
Actual grouped expression, diagnostic and storage replay qualification is described below. Foreign C allocation
accounting remains a separate required surface.

Single-call count replay retains sorted bound/ordinal metadata and constant scalar scratch. It suspends for
one original count at a time; the evaluator can supply that count through its existing frame loop. Adjacent
equal bounds coalesce from their first count, preserving signed zero and NaN payloads. Quantile makes one
full correction sweep, then uses direct probes when raw/coalesced positions coincide and no count was repaired.
Other quantiles retain literal Go binary-search prefix probes and an endpoint sweep; fraction makes a
terminal-count sweep and a rank sweep. For N inputs and M coalesced runs, generic count reads are bounded by
N*(2+ceil(log2 M)) and 2N respectively. Eligible direct quantiles use at most N+ceil(log2 N)+2 reads.
Child-expression replay adds its own work. No raw or corrected count
array, history buffer or additional resource cap is introduced. The canonical numerical contract below
now supplies the retained AMD64 operand behavior on every device.

```sh
python3 tests/promql-conformance/classic_replay.py --capture \
  --mod-cache "${PROMQL_GO_MOD_CACHE:?}" --reference "${PROMQL_CLASSIC_CAPTURE_DIR:?}" \
  --binary "${PROMQL_KERNEL_BINARY:?}"
python3 tests/promql-conformance/classic_replay.py --supplement "${PROMQL_CLASSIC_OPERAND_CAPTURE_DIR:?}" \
  --reference "${PROMQL_CLASSIC_CAPTURE_DIR:?}" --binary "${PROMQL_KERNEL_BINARY:?}"
```

### Dense owned-result collection

The convenience Collector can use a direct lookup while every stored row has `series[index].series_id == index`.
This proof covers both existing IDs and first-insertion absence. A successful append with another ID invalidates
it; the original identity scan then handles sparse, permuted and arbitrary IDs. There is one private Boolean and
no per-series index or additional heap allocation. The [design](DESIGN.md#dense-owned-result-identity-lookup)
records the proof and its ownership boundaries.

Check complete labels, value bits, timestamps, series identity and point arrival order before work bounds. The
focused `result::tests::collector_lookup` tests cover dense and arbitrary IDs, first-label copies, equal-label
distinct rows, successful sorting, actual interruption after sorting swaps, post-sort chunks, take, abort, rebegin
and cancellation/deadline entry. Sorting must invalidate the proof before its first fallible operation. The existing
long-lookup interruption test deliberately establishes generic lookup through an actual nonpositional chunk.

Measure the actual checked output owner with a private delegating sink created from `begin_checked`'s query account.
Keep the production engine and fixture codec unchanged; bind the diagnostic overlay and actual linked libraries.
Validate complete results and all nonelapsed observations before attributing callback work. Three ticks over N dense
rows previously required `(3*N*N+N)/2` identity comparisons; direct lookup requires `3*N` checked units. Tiny cases
can charge more work. Callback timers and charged work do not establish whole-query speed.

Qualify both optimized native unit suites, affected complete original expressions at pages 1, 17 and 1024, installed
ARM64/ARM32 release execution at page 17, and actual default-feature C callers. Preserve histogram output ownership,
statistics, generic fallback and pinned final-order controls. Use raw-selector and bounded filtered-aggregate controls
beside grouped rates and quantiles. Keep larger generic-replay witnesses separate from passing bounded panels.

For whole-query timing, use fixed alternating ordinary-fixture before/candidate processes, complete outcomes first,
and stable observations per side. Include tiny and sparse controls; retain all slower rows, all eight memory
categories, native Collector layout and optimized linked-code costs. The fixture retains the complete owned answer.
This improves Rust collection and C convenience collection; HTTP streaming uses a separate output lookup.
Shared-host synthetic measurements do not establish live HTTP, concurrency, maximum throughput, RSS, cold-disk
or physical-ARM performance. Raw input paging and disabled implicit caps retain their existing contracts.

The [qualification record](dense-collection-qualification.json) retains the complete finite panel and costs.
At 256 jobs, 30 buckets, two workers and three result ticks, owned raw-selector collection improves 13.74–19.28 times
and the grouped-rate control improves 1.63–1.72 times across the two native profiles. All eight measured memory
categories are unchanged, and native Collector layout remains 200 bytes. The primary panel retains 20 slower
medians; a fixed focused follow-up does not reproduce its two larger quantile slowdowns. Linked fixture and C sizes,
including the release fixture's 1,720-byte BSS increase, remain explicit in the record; default-feature C BSS is
unchanged. These are owned-collection results under recorded shared-host activity.

### Direct classic quantile probes

The first quantile sweep still reads every count, applies the pinned correction rules and records all diagnostic
fields. Only a completed, nonterminal `QuantileTotal` transition with `groups == bounds.len()` and both copied result
repair flags false may reuse the existing prepared traversal. This proves raw/coalesced positional identity and
unchanged count bits. It does not require finite counts or globally unique numerical bounds: NaN equality has its
original behavior. Duplicate/coalesced bounds, precision repairs, monotonicity repairs and fractions keep generic
prefix replay. Constructor and zero-total exits remain unchanged.

Keep `finish_run`, its search and its reset before admission. The reset releases obsolete correction/count/previous
sample owners while preserving total ownership, result metadata, rank and search state. Redispatch at the generic
loop top before another Count index. Simply setting the prepared bit inside the generic loop can index past the
bounds. Credit belongs to this Replay at this instant; no Plan cache, new field, count array or raw history is added.

The original binary-search predicate and interpolation operands remain unchanged. Remaining probes and endpoints
read fresh child values. Prepared interpolation may read the target before its predecessor. The complete first
sweep still exposes fixed input failures; later callback failures follow actual physical operations. Cancellation
must be checked after completed scratch release and before the first direct request. Primary logical participation
stays distinct from useful/fetched reads, work, owning peaks and allocation traffic.

The existing classic replay command above reuses 16,676 original/helper supplement inputs; the classic mutation gate
also qualifies the shared prepared dispatcher with 724 original operation steps. Focused units check complete
numeric/diagnostic/sample outcomes before request bounds, late repairs/coalescing fallbacks and cancellation at the
promotion boundary. With 30 ordinary increasing buckets and q=.95, the source-bound helper witness changes from
181 Count requests to 37, preserving the complete value and metadata.

Whole-query measurements use `histogram_quantile(.95,sum by(job,le)(rate(request_duration_seconds_bucket[3s])))`,
its `scalar(sum(...))` wrapper and the grouped-rate control, at 16/256 groups and 3/30 buckets. Each group has two
workers; each bucket has five one-second counter samples `tick * bucket_index`, with `+Inf` last. Range evaluation
uses seconds 3, 4 and 5 and page size 17. Compare complete original results before examining resource observations.
Fixed alternating before/candidate processes include small and unchanged controls; retain all slower rows and all
eight memory categories. The fixture includes owned result collection; the grouped control emits one row per
bucket, while the quantile emits one per group. Subtracting the control does not isolate quantile evaluation.
The [qualification record](classic-quantile-probes-qualification.json) retains actual
source/build identities and finite scope. These synthetic observations do not establish RSS, concurrency, live API,
cold-disk or physical-ARM performance.

The gate compares 16,676 single-call outcomes bit for bit in both profiles: the original 1,676 steps and
15,000 independent count permutations, including quiet/signaling NaNs, infinities, signed zeros and
duplicate bounds. Each original step uses its actual Go pre-call backing state, including earlier mutations.
The supplemental capture validates the complete original module digest, runtime profile, capture source,
input identities and frozen file hashes. Read counts, panic behavior, reciprocal/boolean results and every
quantile diagnostic are checked. Tests reject altered results, read bounds and identities. Cancellation
covers every replay sweep; 100,000 buckets run on a 128KiB stack with unchanged accounted memory during
count reconstruction. This qualifies the private Q=1 continuation. Repeated plural calls use the separate
mutation continuation described below, preserving the complete original-length backing between quantiles.

`histogram_quantile` and `histogram_fraction` now use that continuation through the evaluator's explicit frames.
Arguments run in source order. Classification retains labels, ordinals, kinds and bounds while discarding values;
count reconstruction reads one requested bucket value at a time. Native calculations retain one owned histogram.
Neither path retains raw history or a cross-metric count array. Metadata and classic group/output bindings take
O(N) space; nested child replay contributes additional work. Chronological preparation keeps the classic label
cache and freezes output bindings for later value replay. It retains no presence or cache-event timeline.

`histogram_quantiles` evaluates its vector before its scalar arguments, then processes quantiles in argument
order. Its classic continuation retains bound/source-reference metadata and completed coalescing results;
singletons remain references to paged input. Monotonicity repair aliases the previous reference. Every later
quantile sorts the full backing, including the tail left by earlier prefix overwrites. A prepared-prefix reader
then probes the total, literal binary search, and interpolation endpoints without repeating repairs. At most
Q*floor(N/2) completed fold values are retained for Q quantiles and N backing slots; these are aggregations,
not a raw count vector, and release between evaluation instants. The typed language permits at most ten quantiles.

Output bindings distinguish source, actual generated quantile text, and occurrence rank. Repeated values,
signed zeros and NaNs can produce duplicate labels; delayed instant parents may consume each occurrence before
final cleanup. Generated destinations accept arbitrary bytes, including an empty name, and Builder reset removes
empty-valued source labels. Immediate metadata removal runs before setting the destination; delayed cleanup runs
at its existing source boundary. Pinned reserved-label removal fails on an empty first name; the engine returns
the same wrapped runtime error at that removal phase. Parents that aggregate away the label can succeed before
delayed cleanup. Result comparison permits empty generated names and still requires exact byte
identity and unique names within each label set. Storage input validation remains independent.

```sh
python3 tests/promql-conformance/plural_histogram.py --capture \
  --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/plural_histogram.py --reference "${PROMQL_PLURAL_CAPTURE_DIR:?}" \
  --binary "${PROMQL_QUERY_BINARY:?}" --page-size 17
python3 tests/promql-conformance/classic_mutation.py --reference "${PROMQL_CLASSIC_CAPTURE_DIR:?}" \
  --binary "${PROMQL_KERNEL_BINARY:?}"
```

The standalone Go probe invokes the unchanged engine with experimental parsing and independently chosen name
removal. Its 426 cases cover changing and swapped values, repeated finite/NaN/zero values, native/classic conflicts,
empty/opaque/reserved destinations, parent aggregation/filtering, and warning/error phases in instant and range
queries. The Rust fixture accepts an optional `delayed_name_removal` boolean for this panel; omitted values retain
the existing parser-profile default. Capture binds every original module file, runtime profile, input and probe
artifact. Qualification compares complete labels, membership, timestamps, diagnostics, errors and numeric bits.
The mutation gate compares 724 original quantile steps in 690 contiguous runs, including complete post-call
backing and panic state. Fraction boundaries supply their captured Go state to a fresh quantile run; candidate
output never becomes an expected input. Completed producer and raw-read bounds are checked separately.

Grouping excludes only `le`. Native/classic conflicts, malformed bucket labels, delayed name removal, timestamps,
duplicate results and diagnostic positions follow the pinned source. At source map boundaries, approved 9.A orders
the actual stored label bytes. The original call closure's warning map persists across steps and can share a special
diagnostic with its outer evaluator; repeated merges can double occurrences even on a step without a new warning.
A precisely accounted callback accumulator reproduces that behavior without mutable shared ownership.

```sh
python3 tests/promql-conformance/histogram_calls.py --capture \
  --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}" --oracle "${PROMQL_ORIGINAL_QUERY_BINARY:?}"
python3 tests/promql-conformance/histogram_calls.py --capture --provider \
  --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/histogram_calls.py --reference "${PROMQL_HISTOGRAM_CALL_CAPTURE_DIR:?}" \
  --binary "${PROMQL_QUERY_BINARY:?}" --page-size 17
```

The independent original-query panel contains 72 complete requests covering silent steps, nested same-key warnings,
ordinary parents, both name-removal modes and integer wrapping. Both profiles match all results and numeric bits at
pages 1/17/1024. The retained pre-correction binary fails 48 of those requests; its original expectations remain
unchanged. Actual expression tests keep retained Workspace/ProviderHeld/BorrowedView peaks unchanged from 2,048 to
100,000 input samples at those page sizes, with one active storage cursor. Storage failure and cancellation during
count reconstruction abort output and release owners. These finite witnesses do not establish a total memory bound.

The provider panel adds 12 complete original engine queries with an ordinary storage warning sharing the
monotonicity warning's raw text. A special callback replacing that different error type starts sharing its object
with the outer warning map. The retained before-fix binary fails six requests (including six occurrences versus
four across warning/silent/warning steps). Capture uses an offline Go wrapper over the unchanged pinned engine;
its source, module locks, compiler profile, binary and diagnostic inputs remain hash-bound. The same comparison
command accepts either original capture kind. Both corrected build profiles pass all 12 requests at page sizes
1/17/1024 with zero numerical-bit differences. These checks qualify warning merging, not arbitrary provider behavior.

The shared Go comparator sort has a fallible polling entry and an inert wrapper for existing numeric helpers.
It checks before comparisons, swaps and PDQ loop entries, including comparison-free reverse and pattern-breaking
work. Cancellation returns the original error; already owned metadata may be partly permuted and must be discarded.
The common implementation uses resumable comparison/swap instructions and a fixed inline control stack bounded
by the machine word size plus helper depth. It allocates no buffer and retains Go's smaller-side evaluation order.
NaN bounds violate the usual
strict weak ordering requirement, so this path preserves the original permutation rather than assuming the
usual quicksort complexity bound.

The same instruction program provides boolean comparison requests for selection reducers. A caller can suspend
at each comparison, read two values sequentially, release them and resume using the reply. Numeric values are never
stored in the program. Physical swaps and comparison argument order remain separate operations; reversed sorting
exchanges comparison arguments. Container heap Push/Fix use their original child/parent comparisons, which differ
from sort's heap fallback. The heap entries are currently qualified through fixtures and unit tests; their query
reducer integration remains required work.

`selection_order.py` binds the pinned Go sort/heap sources and copies the unchanged Prometheus comparator methods.
Its 13,828 declared cases compare complete permutations, values, comparison/swap counts and an ordered operation
digest, including private original helper return values. Original `sort` and `container/heap` tests also run.
Fixture vectors belong to the qualification harness; the production program retains only control state.

```sh
python3 tests/promql-conformance/selection_order.py --upstream "${PROMQL_UPSTREAM_DIR:?}"
python3 tests/promql-conformance/selection_order.py --reference "${PROMQL_SELECTION_ORDER_CAPTURE_DIR:?}" \
  --binary "${PROMQL_KERNEL_BINARY:?}"
```

```sh
python3 tests/promql-conformance/go_sort.py
python3 tests/promql-conformance/go_sort.py --capture "${PROMQL_SORT_CAPTURE_DIR:?}" \
  --binary "${PROMQL_KERNEL_BINARY:?}"
```

This additional gate runs original Go 1.27.1 `slices` assertions and captures 9,539 bound/ordinal metadata sorts:
small special-value products, insertion/PDQ/Tukey boundaries, duplicates, signed zeros, quiet/signaling NaNs,
pivot positions and four groups of 100,000 references. Both profiles match every complete permutation,
comparator count and comparator trace digest. The trace is a constant-state checksum; complete bound bits and
ordinals are compared directly. Seven interrupted sort phases preserve every item and memory observation;
an expired deadline precedes comparisons, and a 100,000-reference pollable sort passes on a 128KiB stack.
The fixtures-only transport requires the common `borrowed` Boolean even though this mode holds no sample values.
This gate qualifies metadata sorting; grouped histogram expression dispatch remains required.

The shared sort/heap controller uses `u64` logical addresses and canonical 64-bit pattern masks with 67 fixed
frames. Physical metadata ordinals remain `usize`; each controller comparison and swap checks conversion and
the physical buffer boundary before access. Boxed reducer state is charged at its actual allocation size.
The controller can therefore address a paged logical window without constructing its whole-length array.
ABI 4 provider-owned temporary storage backs exact quantile windows through this controller and two bounded
pages. Providers choose their backing policy; unavailable or full backing returns a specific query error.

Nine virtual controls captured from unchanged Go 1.27.1 AMD64 sort/heap helpers cover `Fix` at four lengths,
`choosePivot`, three `breakPatterns` masks and `Push`. Exact captured comparisons, swaps and helper returns
are retained in `go_sort_program` unit tests. Their declared lengths include `2^32 + 5`, `2^40 + 5` and
`2^60 + 5`; they retain at most 33 value exceptions and perform at most 64 comparisons/swaps. These controls
qualify logical addressing and pattern masks. They do not measure a complete sort of billions of samples or
qualify a foreign Rust build. Original Go owner hashes remain the same as the interface/heap capture above.

## Required SRE expression sources

The source-pinned extractor reads Git objects without changing the repository mirrors:

```sh
python3 tests/promql-conformance/sre_inventory.py \
  --infra "${NETDATA_REPOS_DIR:?}/infra" \
  --eks-module "${NETDATA_REPOS_DIR:?}/infra-terraform-module-eks-resources"
```

It produces a private `.tmp/sre-<unique>/manifest.json` and `requests.json`. The manifest preserves expression slots,
source digests/lines, template bindings, rendered environment variants, recording outputs/labels and rule group order.
Private query bodies, selectors and endpoints must remain in ignored output; public tests use synthetic identities.
The exact source pins are recorded in `sre_inventory.py`. It never runs Terraform, Sloth or generator installation.

The checked source closure contains:

| Source | Slots / variants |
|---|---:|
| Common HCL alert definitions | 107 |
| HCL expansion: production / staging / testing | 147 / 140 / 140 |
| Generated SLO rules | 680: 608 recordings, 72 alerts |
| Explicit analytics rules | 3 |
| Total source slots / rendered requests | 790 / 1,110 |

This counts source definitions and their rendered variants, not live deployments or traffic frequency. All 1,110
rendered expressions parse as vectors with the pinned server profile. They use 14 functions and five aggregations
(`avg`, `count`, `max`, `min`, `sum`). The issue 6028 synthetic cases additionally cover `unless`.

The extractor is bounded to the inspected HCL shapes, including six comprehensions and nested environment bindings.
It fails on unknown interpolation/directives or changed closure counts. Loader/generator source digests are retained;
the selected module disables default chart rules. External chart contents and manually installed live rules are not
certified by this source snapshot.

## Paged Rust expression checks

`query.py` executes every variant in a verified builtin evaluation capture. It verifies the complete reference
source and hook roster, original assertion/replay status, toolchain profile, expression identities, dataset hashes
and effective query settings before running the candidate. It reuses `compare.py` for complete result comparison.

```sh
python3 tests/promql-conformance/query.py \
  --capture "${PROMQL_EVALUATION_CAPTURE_DIR:?}" \
  --upstream "${PROMQL_UPSTREAM_DIR:?}" \
  --oracle tests/promql-conformance/.tmp/oracle \
  --binary tests/promql-conformance/.tmp/core-target/release/promql-query-fixture \
  --page-size 1024
```

The binary's test adapter retains one immutable fixture snapshot at a time, outside query-owned state. Cursors
borrow bounded slices; the evaluator retains descriptors and sufficient operator state, then replays pages.
This materialized test input does not represent the future dbengine adapter. The fixture collector independently
owns the complete output. Byte fields cross the Rust transport as hex and the Go wire as base64; numeric fields
retain binary64 hex, native histogram layout and start timestamps. Shared histogram codecs have one test owner.

Every captured identity receives a pass, fail or open disposition. An explicit unsupported engine path is open;
it cannot match an expected upstream error or satisfy the full gate. The command exits unsuccessfully while any
required expression remains open or mismatched. Both semantic and strict bitwise status are retained, with every
paired bit difference reported. Intermediate partial expression counts do not certify complete language support.

Each variant also receives a numeric contract from the pinned Go AST, independently of candidate parsing and
results. Boolean, count, integer-valued functions and value-preserving discrete expressions receive exact numeric
comparison; general numeric results retain the agreed upstream tolerance. Fractional fill values are included in
the classification of arithmetic and returned filter operands. Missing classification fails the comparison. The
retained corpus has 2,788 variants classified for exact numeric comparison. Near-integer boolean/function mutations
are rejected, while fractional fill outputs retain their permitted tolerance.

The earlier full run after native histogram projections, timestamp support, intermediate ordering, native scalar/vector
maps, native vector aggregation, request-context preparation, ordinary math/calendar calls, native/instant
rates, regression/smoothing, ordinary absence and direct start timestamps matched 9,684 of 21,442 variants, with zero completed semantic failures and 11,758 open variants at page sizes
17 and 1024. Its 24 recorded paired bit differences were
NaN payload/sign differences from one original aggregation case. Further implementation and replay remain
required. Separate fresh Go checks match
192 typed reducer, start-timestamp and fixed-`@` executions exactly across page sizes 1, 17 and 1024.
Another 320-case numerical range capture matches all cases semantically in release at page size 1024 and
release-min at page size 17, including its 24 native sum/average cases. Release matches all bits; release-min reports twelve ordinary
NaN-payload differences. It covers ordinary, overflow, nonfinite, signed-zero, compensated, subnormal, stale, mixed
and empty inputs. The test runner checks observed-owner cleanup for unsupported executions too.
An additional 684-case capture qualifies plain and nested parenthesized arguments for all nineteen implemented
ordinary range functions, including mixed diagnostics and fixed-time calls. It matches semantically at all three
page sizes in both builds. Release is bit-exact; release-min reports 48 ordinary NaN payload differences in the
mixed variance/stddev cases. Argument lookup preserves the frontend AST while using the inner source position
for diagnostics and recognizing fixed-time calls through parentheses.
An additional 352-case native sum/average capture matches exactly at page sizes 1/17/1024 in both builds. It covers
overflow trial rollback, compensated arithmetic, changing exponential resolution, custom-bound reconciliation,
original counter-reset hints, schema-warning precedence and late-float suppression. Native evaluation classifies
the complete window before replaying histograms; it retains an accumulator, compensation and temporary trial or
scaled input, without retaining the raw history.

## Duplicate projected output rows

`output_label_calls.py` declares 200 complete SUM/MIN/COUNT/GROUP/ABS controls, independently of candidate
results. Present-empty and absent grouping labels produce distinct raw grouping hashes, then the source label
builder removes empty values. At Prometheus `5241a27fe3c6983549fccc32f6e65917408c63cd`, ordinary aggregate
output can consequently contain separate rows with identical projected labels (`promql/engine.go:1630–1668,1754–1767`;
`model/labels/labels_stringlabels.go:471–480`). Delayed cleanup and ordinary parent operations can instead return
the source duplicate-label error. The capture retains all of these complete, profile-specific outcomes.

The decoder preserves each output row occurrence, its point occurrences and series multiplicity. Individual
label-name uniqueness, vector single-point shape, diagnostics and error attribution remain required. Mutations
of missing, extra, swapped, first and last duplicate rows fail; common occurrences retain every paired bit report
even when another series is absent. Input labelset uniqueness remains required.

Owned matrix collection uses the qualified Go interface sorter because equal projected labels make its exact
row provenance observable (`promql/engine.go:919`, `promql/value.go:338`). Checked comparison retains byte-level
polling; both collector entry points account sorting work. Provisional chunks retain distinct query-local series
identities. Controls cover 2/12/13/50/51/129 raw groups, both profiles, instant and single/multiple-step range queries.
Capture with `output_label_calls.py --capture --upstream "$PROMETHEUS_SOURCE"`; compare using `--reference`,
`--binary` and `--page-size`. The gate requires exact numbers and output order at every boundary.

## Aggregation quantile

`quantile_aggregate_cases.py` independently declares 3,558 complete queries in 166 families, with 1,561
server-default and 1,997 upstream-test calls. `quantile_aggregate_calls.py` uses the source-bound cursor facade
for original capture and candidate qualification. The original reference was captured before any aggregation
QUANTILE runtime implementation. The Rust reducer retains series identities and grouping state, reads samples
sequentially, sorts identities through the qualified Go continuation, and rereads two interpolation endpoints.
It retains no raw numeric vector across metrics or complete time history. Its one-key snapshot uses the entered
evaluation scope and discards unused previous identity state before preparing another step.

Controls cover global and moving parameters, invalid-quantile warnings, empty/stale/native-only bodies, ignored
histogram positions, first-writer metric-name flags, grouping/hash aliases, duplicate projected output labels,
fixed/context clocks, independent parameter/body faults and nested parents. Signed zero, distinct quiet and
signalling NaN payloads, infinities, singleton/integer-rank/zero-weight interpolation and Go sort thresholds up
to 1,025 identities retain the original arithmetic and branch evidence. Source owners are pinned Prometheus
`5241a27fe3c6983549fccc32f6e65917408c63cd`, `promql/engine.go:1630–1767,3675–3740,3939–3944,4004–4005` and
`promql/quantile.go:717–738`; the captured unmodified Go AMD64 assembly resolves interpolation operand order.
An additional 360 controls retain Infinity/NaN parameter permutations across float, native-only and empty bodies:
Go `math.Min/Max` can preserve terminal infinities alongside NaN, producing several quantile warnings. The first
3,198 declarations remain an exact prefix of their independently captured pre-runtime inputs.

Capture with `quantile_aggregate_calls.py --capture --upstream "$PROMETHEUS_SOURCE"`; compare using `--reference`,
`--binary` and `--page-size`. This focused edge gate requires exact numerical outcomes, series order and paired
bits; the general corpus retains its independently classified upstream numerical tolerance. Input-witness tests
check raw grouping identities, IEEE payload provenance, changing native kinds and independent fault declarations.
Temporal quantile/MAD and general nested-work qualification remain required separately.

`quantile_policy.py` separately qualifies eight declared multistep `or` boundaries under decision 9.A. Original
map iteration can change whole-matrix deferred name removal when a float and ignored histogram share a group.
The addition-only private source facade evaluates the original parameter and body, orders the complete child
Matrix by raw label bytes with its original `DropName` flags, and invokes unchanged aggregation and cleanup.
Captured child observations are copied before original pooling. Production source owners remain byte-identical;
scope, native values, flags, diagnostics, output membership, order and numbers remain checked.

Capture this control with `quantile_policy.py --capture --base "$QUANTILE_REFERENCE" --upstream "$PROMETHEUS_SOURCE"`.
Apply it with `--base`, `--reference "$POLICY_REFERENCE"` and `--candidate "$COMPARISON_DIRECTORY"` after the ordinary
comparison. Raw reports remain unchanged; the separate policy report preserves their failures and verifies the
original input/output hashes. Policy tests reject changed flags, scopes, identities, numbers and undeclared controls.

## Median absolute deviation source gate

This is an earlier bounded-reducer checkpoint. Its then-open expression integration is historical; current
standalone expression qualification is listed above. The original fault and throughput limits below remain
specific to this panel.

`mad_cases.py` independently declares 2,248 complete ordinary `mad_over_time` requests in 209 families, with
1,124 calls per feature profile. The inputs preserve 176 declared IEEE arrays and their permutations, including
signed zeros, subnormals, infinities, raw NaN payloads, arithmetic overflow and a 1,030-sample page boundary.
Additional controls cover moving windows, fixed timestamps, offsets, scalar consumers, parent aggregations,
native-only and mixed ranges, stale points, duplicate output labels and provider failures after a raw NaN.

`mad_calls.py` captures original preparation and results through an explicit construction-error mode of the
cursor facade. This mode records disabled-feature parser errors and query-construction errors as complete error
results. Successful evaluation phases and default-mode generated helpers remain byte-identical; the additional
helper and its two exact failure sites are bound and checked. Production Prometheus source remains unchanged.
Capture with `mad_calls.py --capture --upstream "$PROMETHEUS_SOURCE"`; compare with `--reference`, `--binary`
and `--page-size`. The focused gate requires exact numeric outcomes, diagnostics and paired bits. It preserves
exact range and ordinary instant ordering; only instant root `count_values` output uses its existing unordered
source contract because the original constructs those rows by map iteration. All values and labels remain checked.

The Rust reducer uses full initial classification
and simultaneous two-rank refinement of each median, with two fixed 16-counter tables and at most 33 complete
page scans. Raw NaNs return the source's explicit NaN after complete decoding; generated NaNs retain the qualified
arithmetic operands and both weighted terms, including zero weights. The original excluded-left seek target is
carried separately from clipped cursor bounds through numeric MAD, regression and smoothing. The public inclusive
scan retains its own contract. Mixed ranges report the source info; native-only and empty ranges return no sample.
Runtime checks compare workspace, provider and borrowed-view peaks over 2,048 and 100,000 samples, keep one open
cursor, and verify cancellation/error release in both median phases, unclipped seeks and failures after raw NaN.
Complete buffered-window fault qualification for other range paths remains required; physical replay is not an
original iterator trace. Repeated scans can increase storage work, and this gate makes no throughput claim.
Extended selectors, subquery inputs and general temporal quantile sorting retain their separate open scope.

## Historical productive-expression checkpoint

This section records an earlier implementation and its remaining work at that checkpoint. The current expression
checkpoint above supersedes its OPEN counts and implementation status; original evidence remains attributed below.

The productive Rust path connects the owned frontend to typed page cursors, float/native rate/increase/delta
and irate/idelta, deriv/predict_linear/double_exponential_smoothing/mad_over_time,
count/present/first/last/min/max and absence over time, float/native histogram sum/average and float variance/stddev over time, timestamp extrema/endpoints,
changes/resets, scalar/vector arithmetic
and comparisons including power/remainder, float/native sum/avg/count/group/count_values, topk/bottomk
and float min/max/stdvar/stddev with native ignored-input infos. Evaluation uses explicit frames,
whole-query label/group descriptors and replay; raw histories and complete raw instant vectors are not retained.
Output uses synchronous provisional chunks followed by commit/abort, with a separately owned collector. Stable
series identities retain legitimate duplicate projected labelsets and merge disjoint rows at cleanup owners.
The optional collector materializes the complete output and can grow with it, including raw selector results;
use the chunk interface to stream large outputs. Reported execution/collection methods retain counters on failures
as well as successes; the convenience `Result` methods return counters only on success.
Request preparation resolves duration expressions and original `start()`, `end()`, `range()` and `step()`
contexts before storage opens. Context functions keep their upstream optional-function profile. The compiled
plan stays immutable during concurrent executions; only queries with dynamic modifiers allocate a tracked
24-byte duration entry per AST node. Explicit tracked frames evaluate durations in source order, retaining
final validation, millisecond truncation, exact error spans and independently owned error bytes. Selectors,
range folds and raw matrix output use the resolved offsets/ranges. Subquery execution remains required.
Preprocessing keeps original Unix milliseconds for context functions and storage bounds. Evaluation and output
timestamps follow the pinned nanosecond clock's wrapping conversion. Fixed selector offsets retain duration
arithmetic before millisecond truncation; direct timestamp selectors use their separate per-step override.
Both builds match all 426 source preprocessing/bounds projections and 198 additional offset projections.
Fresh source parser checks match 1,544 offset syntax/position cases, preserving direct literals/functions,
single signs, parenthesized/chained duration continuations and the three modifier owners. The 356 original
AST/error cases and 784 parser/profile cases remain exact. Of 444 contextual expression cases, 325 execute
exactly and 119 still require subquery/info/other function reducers; all 198 power/duration expressions execute
exactly at page sizes 1/17/1024 in both builds.

Logical source diagnostics are shared immutable owned results. Replay does not multiply occurrences; fixed safe
range calls emit diagnostics once while retaining every output step. Default duplicate failures discard unfinished
subtree diagnostics, while delayed output-cleanup failures retain completed root diagnostics.

The Rust unit suite covers generated 100,000-point histories, page sizes 1/17/1024, the complete grouped-rate
threshold path, empty-value grouping, intermediate duplicate errors, source-child error precedence, fractional
duration threshold bits, source grouping-hash byte collisions, query-wide group order, panic/cancellation/sink
abort cleanup, retained output, shared compiled plans and resource-owner transfers. Query-held page observations
count unions of aliased byte ranges, including a returned page discarded after cancellation. `samples_fetched`
counts points received in accepted pages; operator work is a separate counter. Original provider/page snapshots
are validated before observation wrappers rewrite them. Owned histogram payloads expose immutable views and keep
their observed mutable buffers and shared immutable bounds attached to their lifetimes. The native payload is boxed
with its allocation token, keeping `OwnedValue`/`OutputPoint` at 16/24 bytes on the qualified amd64 build. Each
retained histogram adds one observed 296-byte box, plus its arrays and bounds. Sum reuses its compensation box;
average's source-defined trial copies allocate independent boxes. Float-only collection allocates no native boxes.
Mutation accounting includes
capacity growth overlap, copied reductions, custom mapping scratch and discarded average trials. A 2,048-versus-
100,000-sample native sum/average check retains the same workspace peak at each page size; both histories cross page
boundaries with an accumulator already retained. Accumulated bucket unions may still grow. These finite cases do
not certify the complete language or C integration.

Instant and range evaluation supports `histogram_count`, `histogram_sum`, `histogram_avg`,
`histogram_stdvar`, `histogram_stddev`, `timestamp` and experimental `start_timestamp`. Projections ignore floats; timestamp accepts both value
types and distinguishes raw sample timestamps from derived evaluation timestamps. Eager metadata removal follows
each source function; delayed removal occurs at final output. Duplicate checks and child diagnostics retain their
source boundaries. A direct `timestamp(metric @ ...)` lookup follows the pinned engine's fixed timestamp,
ignoring its original offset during lookup; query-wide storage bounds still include that offset. Ordinary instant
cursor reads intersect those bounds. Source preprocessing unwraps a direct parenthesized selector argument;
derived arguments use ordinary expression evaluation. An aggregation parameter's subtree is not visited by that
preprocessing helper, so surviving parentheses there retain their ordinary evaluation boundary.
After reproduced failing regressions, all 340 retained review cases match all bits in both profiles at page sizes
1, 17 and 1024, including the nine previously open `start_timestamp` cases. Histogram variance polls cancellation
during bucket traversal. Fresh Go checks match 4,134
pointwise, composed, multi-step and disjoint-alias cases in both build profiles at page sizes 1, 17 and 1024;
all 33 paired bit differences are NaN signs in nested nonfinite sums, reported separately from semantic comparison.

Float-only pointwise calls also support `abs`, `ceil`, `floor`, `exp`, `sqrt`, `ln`, `log2`, `log10`,
the circular/inverse/hyperbolic functions, `rad`, `deg`, `sgn`, `round` and all three clamp functions.
The eight UTC calendar calls accept explicit float vectors or an omitted argument. `pi`, `min_of`, `max_of`
and scalar/mixed float `atan2` use the source arithmetic. Vector/vector operations use the bounded binary owner described below.
Parameterized maps retain only one or two scalar operands and reuse row-at-a-time evaluation; native samples
are silently filtered. Source duplicate, metadata, diagnostics and multi-step ordering boundaries remain active.

The numerical kernels follow Go 1.27.1, including its fixed-size Payne-Hanek reduction, retained exponential
FMA-on route, untyped reciprocal constant rounding, signed-zero/NaN cases and UTC unsigned epoch wrapping.
`days_in_month` retains the source Date round trip rather than substituting a month-length table; extreme values
can consequently return seven. Degree/radian calls retain multiplication-before-division overflow behavior.
On the qualified Linux amd64 target, 5,854 raw numeric/calendar cases match every bit in both Rust profiles
against native, FMA-disabled, SSE4.1-disabled and combined-disabled Go captures, with explicit fixture dispatch
for the disabled-FMA arithmetic. Six hundred additional Go
direct rounding/truncation calls are retained as compiler-dispatch diagnostics; PromQL uses function values.
Original unmodified Go math assertions pass in three CPU profiles. Fresh engine checks match all 1,420 completed
cases at pages 1, 17 and 1024, with zero bit differences; twelve vector-matching cases remain
explicitly open. A generated 2,048-versus-100,000-sample map with a scalar parameter keeps the same workspace
peak and at most one open cursor at each page size. This qualifies these source owners, not the complete engine
or other architectures.

Native scalar/vector multiplication works in both operand orders; histogram/scalar division and unary minus also
work. Multiplication and division compact the resulting histogram, while unary minus preserves its bucket layout.
Invalid scalar/histogram operator pairs suppress the row and emit the exact source informational diagnostic,
including with `bool`. Native maps reuse an independently owned payload; bucket loops poll cancellation and count
work. Each arithmetic operator applies the source histogram copy's field semantics. The convenience collector
instead preserves every evaluated field, including custom-schema NaN and signed-zero fields, in its independently
owned result. A source-copy operation at this output boundary would erase those values. Permanent regressions
cover chained maps, retained results and interruption cleanup. Fresh Go checks match all 8,196 native-map cases
in both builds at page sizes 1/17/1024, with thirty separately reported NaN-sign differences. The shared histogram
kernel replay still matches all 6,440 cases semantically; it reports ten NaN-sign differences in release and one
in release-min, plus three unchanged differences between the source's numeric assertions and actual signed-zero
bits. Native trimming and vector/vector operators now share the bounded binary owner described below.

Ordinary native `rate`, `increase` and `delta` retain first/second/last histograms and replay schema and reset
passes in source order. `irate` and `idelta` retain the last two relevant values, including transitions between
float and histogram samples. Extrapolation shares the qualified float formula, including start-timestamp
extension, zero-point correction and fractional range duration. Singleton start-timestamp rates preserve their
source hint and uncompact layout; multi-sample and instant paths set gauge hints and compact at their source
boundaries. Endpoint warnings, discarded interior-error annotations and custom-bound reconciliation infos retain
their separate source phases. The existing statistics decoder supplies projected inputs below histogram
count/sum/average; full-bucket replacement would emit extra infos and change reset behavior.

Fresh independent source captures include 2,120 expressions: all 2,070 completed cases match every bit and exact
diagnostics at pages 1/17/1024 in both builds; fifty subquery cases remain open. Original native/start-timestamp
rate assertions remain unchanged. Generated 2,048-versus-100,000-sample histories keep equal workspace peaks
for all five rate families at every page size, with one open cursor and independently released result owners.
Checked schema-copy, reset, zero-threshold, layout, Add/Sub and integer-conversion paths count work and poll
cancellation inside bucket loops. Native SUM/AVG, first/last retention and changes/reset comparisons use the same
checked owners. Correction creation, overflow scans, schema reductions, sparse shifts, compaction and explicit
buffer-growth copies process bounded chunks. Interrupted growth keeps the source buffer and releases its
replacement; both live allocations contribute to the observed peak. Arithmetic errors retain their source
phases, including deliberately ignored vector-finalization flags and later range provider failures. This does
not establish full expression/C compatibility or interrupt latency inside the allocator. The current candidate
and previous signed metadata baseline match all 6,440 histogram and 1,270 statistics helpers bit for bit in both
profiles. Original Go assertions retain three histogram and fourteen statistics declared-bit differences from
their source execution.

Regression keeps four compensated sums and its source constancy check. Smoothing keeps two previous values and
one trend, including the source's second-point initialization and NaN factor behavior. Scalar arguments are
evaluated before matrix expansion; invalid-factor errors occur only after a nonempty typed window. Histograms
are ignored with the exact mixed-range info, including a one-float window that returns no value. These folds
retain constant state and one bounded storage page, with no raw window or histogram payload copy.

Independent source panels match all 622 completed of 868 fresh regression/smoothing/order-statistic expressions
semantically at pages 1/17/1024 in both builds; release-min reports 28 ordinary NaN sign/payload differences.
The 246 required quantile/MAD/composition cases remain open. The 140 original
regression/smoothing variants and 28 long-history controls match every bit. Generated 2,048-versus-100,000-sample
histories retain equal workspace peaks at every page size; provider failure, cancellation and owner release are
checked. All 5,020 offset/fixed-reference/diagnostic review cases match exactly. Extrapolation truncates Range+Offset
together while physical scans truncate them separately and intersect query-wide bounds; permanent regressions
cover the resulting alert threshold and fixed-reference membership. Of 1,058 additional time-context controls,
1,043 match exactly and 15 still require other expression families. Extreme fixed-time/storage-sentinel tests
cover the pinned integer-overflow behavior. Both profiles pass 146 units and offline locked Clippy/build checks.

Ordinary range-call preparation expands matrix metadata before opening raw cursors, then completes every logical
step of one physical series before advancing to the next. Scalar children remain unrestricted. This preserves
source helper-error precedence without retaining a sample timeline or repeatedly enumerating matrix metadata
for each series and step. All 363 focused review requests match exactly in both builds/pages, including the
eight previously failing ordering witnesses. Metadata-failure and generated step-count tests check release,
error precedence, bounded workspace and metadata work independent of the step count; output replay has its
separate existing costs.

`absent` consumes every implemented vector child before testing presence. Floats, ordinary NaNs and native
histograms count as present; later child failures remain visible. Label inference follows selector matcher order,
including repeated equality and empty-value history, while owned output labels use byte order. Direct-matrix
`absent_over_time` validates every series across all logical steps before replaying windows to invert their global
presence. It retains one presence bit and a single output descriptor, without a raw window, timestamp bitmap or
per-series presence timeline. Its inferred reserved labels follow immediate versus delayed cleanup.
Both builds/pages1/17/1024 match all 272 completed of 320 independent presence controls exactly; 48 extended-selector,
subquery and other child cases remain explicitly open. All 9,266 previously passing original expressions remain
passing, with 416 additional passes and the same 24 paired bit reports. The separate 459-request source-phase review
corpus remains exact in both builds/pages. Generated 2,048-versus-16,384-sample range-absence histories keep equal
workspace and page peaks at each page size with one live raw cursor and a streaming sink. These finite fixtures
qualify this owner; the full absence family and complete iterator-phase compatibility still require the remaining
engine work. The query-scope catalog below supplies shared upfront selection and terminal provider diagnostics.

`start_timestamp` returns raw start timestamps only for a syntactically direct selector, including parentheses,
when its independent runtime feature is enabled. Composed arguments are fully evaluated before producing no
result, preserving their failures and diagnostics. Raw zero, negative and extreme start timestamps are valid;
conversion casts to float before dividing by 1000. Statistics lookups retain the selected predecessor's start
timestamp through future access and EOF. These paths retain fixed fields beside the current sample, rather than
raw history. All 192 completed of 208 independent clock controls match exactly in both builds/pages; twelve
other expression and four vector-matching controls remain open. Generated 2,048-versus-16,384-sample histories
retain equal workspace and page peaks with one live cursor and a streaming sink. The feature flags and shared
source-phase/C/accounting requirements remain unchanged.

Selectors under histogram count/sum/average use the source's statistics decoding for ordinary range folds.
Subqueries, bucket-dependent histogram functions and trimming disable it. One tracked full histogram supplies
reset detection; consumers receive only schema, count, sum and reset hint. Prefix replay reconstructs buffered
access and moving-seek state without retaining raw windows. Two direct pinned-Go buffered iterator witnesses
distinguish unaccessed lookahead from an accessed endpoint; all 30 original nested reset-warning cases now pass.
Repeated prefix I/O/work is accounted. Ordinary instant statistics replay now preserves future-current access
before predecessor selection, repeated-current caching, different memoized/raw seek reset conditions and EOF.
Finite query-wide bounds include sibling selectors, original request endpoints, resolved dynamic durations and
subquery ranges/offsets. The bounds also include `info`'s implicit metadata lookback, inferred from vector-producing
paths and the call's ancestor scope. Heterogeneous or selector-free paths use the evaluation context; scalar
paths and nested-info metadata arguments do not provide an inferred reference. Ordinary bounds traversal still
visits both `info` arguments. Actual `info` evaluation remains required.
Range statistics replay also decodes the excluded lower boundary when sibling selectors make it available.
Its hint can affect SUM/AVG warnings and first/last values consumed by vector aggregation. All 396 fresh source
boundary cases match exactly in both builds at page sizes 1/17/1024, including integer histograms, clipped input,
float/stale boundaries and future-payload controls. Logical range output remains strictly exclusive.
All 38 fresh Go iterator witnesses match exactly in both builds at page sizes 1/17/1024. The private scan can stop
after the required current sample while still validating and observing the complete returned page. Generated
1,024-versus-100,000-point histories retain the same workspace peak and one live cursor at each page size.

Vector aggregation shares native SUM/AVG arithmetic with range folds, with separate source classification and
finalization policies. Native state is an optional accounted box. Schema incompatibility stops subsequent group
inputs; mixed-type suppression, custom-bucket infos, reset collisions, average overflow rollback and compaction
follow the vector owner. COUNT/GROUP accept native values; MIN/MAX/STDVAR/STDDEV ignore them with exact infos.
The last emitted group flag is retained for replay before a parent consumes earlier points.
Fresh Go capture has 6,452 variants: 6,372 complete and eighty remain open through unsupported set operators.
The unmodified reference comparison has 6,352 semantic matches and twenty disagreements at unspecified map
boundaries, with 62 reported paired bit differences. Separate policy qualification compares all 496 plain
aggregation map-boundary cases with 992 fresh Go instant evaluations using source-defined input order sorted by
the approved intermediate label bytes. All 496 match semantically at all three page sizes in both builds;
their eight NaN-payload differences remain reported. Original source-defined cases and original assertions are
unchanged. This qualifies the selected ordering policy separately from exact upstream equivalence.

Runtime accounting covers engine-owned buffers, frontend/error/sink owners, shared allocations and borrowed pages.
Foreign callbacks report transient allocations through query-owned acquire/resize/release handles, including cleanup
callbacks. Remaining caller-reported snapshots exclude leased bytes to avoid double charging. Unreported allocations
inside a provider remain unobservable; a callback panic cannot establish their hidden temporary peak. Regex dependency
and selector-table backing include explicit estimates and hidden temporary/high-water limitations. Accounted requested
bytes are distinct from total process allocation and RSS. No resource or concurrency admission limit is enabled.

The pinned reference has an unspecified ordering contract: intermediate multi-step results assembled by Go maps
can change a parent reduction and boolean outcome across identical requests. The source of this behavior is
promql/engine.go:1620, before final public result sorting. The implemented Rust policy sorts by label bytes at these
boundaries, including unary minus's eager alias-merging slow path. Provider, ordinary range-function, aggregation,
instant and fixed safe expression order retain their source contracts. Step-invariance classification includes
timestamp's fixed raw-selector exception. Equal full-label hashes merge different-step aliases with first-writer
labels and flags; ordinary vector operators reject same-step duplicate labelsets across types. Accounted descriptors and a flat
ordinal-binding table support one requested row's exact replay. No raw point matrix or additional raw cache is retained.

Unary matrix merging and delayed final cleanup retain separate float and histogram lists. Their duplicate checks
reject overlapping timestamps within the same kind, while retaining equal-time float/histogram pairs. Unary and
parentheses preserve both lists. Ordinary vector consumers prefer the float at a collision; the unconsumed histogram
head then blocks later histograms in that query scope. A new instant query reconstructs its own scope. Public instant
conversion instead prefers the histogram. Replay records only kind presence and the earliest colliding logical step;
instant cleanup uses metadata cohorts and retains one selected value. It does not collect raw vectors or timelines.
Highest invariant wrapper boundaries reject dual-kind rows with the source's `unexpected number of samples` error,
including instant queries; an enclosing safe consumer may first reduce or ignore the mixed row successfully.
Fresh Go witnesses cover 6,864 variants across seventeen datasets. Rust matches all 6,864 cases exactly
in both builds at page sizes 1/17/1024. Permanent regressions cover matrix
and vector consumption, scope-local blocking, fixed boundaries, negative times, page release and accounting cleanup.

A repeated pinned-Go witness for `1 / min(m * 1) > bool 0` over positive and negative zero returned true 115 times
and false 13 times in 128 default-profile requests; the optional profile returned true 118 times and false ten
times. With negative zero's label ordered first, Rust consistently returns false in both builds at all three
page sizes. These 256 repetitions qualify the approved ordering policy, separately from exact Go equivalence;
ten instant, raw-selector and fixed-expression witnesses remain exact. Original expression assertions are unchanged,
and no numerical tolerance is widened. The completed expression core uses this policy at source-unspecified map
boundaries and preserves source-defined ordering elsewhere.

Each query now opens every explicit AST selector site once, in syntax order, before expanding metadata. Identical
selector text at different sites retains separate selections. Expansion retains series handles and provider
ordinals in a tracked catalog, exhausts metadata before raw reads, and lends one handle at a time to bounded
page kernels. Physical replay reuses those handles. Original query-wide bounds are passed to the provider before
offset adjustment; each selection carries its original bounds and borrowed step/range/function/grouping hints.
Compiled preprocessing metadata preserves source helper visits, invariant wrappers and surviving parentheses,
including unvisited aggregation parameter subtrees. Hint grouping retains original label order and distinguishes
an absent grouping from an explicit empty one. These metadata projections do not establish execution of currently
unsupported parameterized aggregations or subqueries.

Requests containing evaluated INFO calls rebuild invariant and wrapper flags after resolving request durations
and contextual selector timestamps. The same postorder algorithm serves compilation and request preparation;
evaluation, replay, selection hints and roster scans all read the prepared request's flags. INFO requires a
uniform reference on every vector-producing path. Scalar paths do not supply that reference, and selector-free
vector branches prevent it. Request flags use two accounted boolean arrays indexed by AST node, with no history
or output-step array. Requests without evaluated INFO borrow the compiled flags. Concurrent requests retain
independent flags; duration failure and cancellation release preparation scratch before opening storage.

```sh
python3 tests/promql-conformance/preprocessing_flags.py --capture \
  --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/preprocessing_flags.py --reference "${PROMQL_PREPROCESSING_CAPTURE_DIR:?}" \
  --binary "${PROMQL_QUERY_BINARY:?}"
```

This panel compares 150 highest-wrapper projections from the unchanged pinned Go preprocessor. It covers equal
and different fixed references, contextual endpoints, duration `step()` in instant and one-step range requests,
selector-free branches, scalar-path exclusion, nested INFO, ancestors, unsafe functions, timestamp's direct
selector exception and unvisited aggregate parameters. The retained before-correction binary fails 59 cases.
The source, inputs, compiler, helper binary and original output remain hash-bound. These projections qualify
preprocessing; both corrected profiles pass all 150 requests. Complete-query INFO checks are described below.

### INFO selection and enrichment

`info()` prepares its complete base row descriptors, derives the source's three identifying-label presence
groups, and selects implicit info series through the typed query provider. Its optional selector remains part
of upfront syntax selection but is never expanded or evaluated as an argument. All implicit groups finish
metadata expansion before raw reads. Generated matchers filter permitted provider metadata overscan; implicit
selection/expansion/read errors retain their original bare payload and discard annotations not returned by
the failed source evaluation owner.

Productive info descriptors retain labels, ordinals and signatures. Every selected implicit native sample in
the logical scope fails, including steps without a simultaneous base value. Joins use the newest encoded original
timestamp; equal timestamps fail with the original duplicate-series error. Empty base steps skip join duplicates.
Existing base label presence wins, including present empty values; source label-builder boundaries remove empty
fields. Hash-last signatures and hash-first result labels preserve their whole-scope ownership. Approved byte
ordering applies at the source's unspecified matched-info and output map boundaries. Successful results preserve
base float/native values, evaluator timestamps, zero start timestamps and delayed name-removal flags.

Each logical step retains active row identities and one winner per signature, then replays current base operands
through explicit evaluator continuations. Raw input stays page scoped, with one raw cursor at a time for this
owner. INFO inspects histogram kind and staleness without copying its payload merely to reject it. Selected
descriptors and distinct enriched label identities can grow with cardinality and label changes across time;
they are accounted metadata, and streaming output is not a bound on total memory. All enforcement stays disabled.

```sh
python3 tests/promql-conformance/info_calls.py --capture \
  --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}" --oracle "${PROMQL_ORACLE_BINARY:?}"
python3 tests/promql-conformance/info_calls.py --reference "${PROMQL_INFO_CAPTURE_DIR:?}" \
  --binary "${PROMQL_QUERY_BINARY:?}" --page-size 1024
```

The independently declared panel contains 239 complete original queries in 41 groups. It covers default and
filtered enrichment, changing labels, duplicates/conflicts, missing identifiers, empty values, fixed references,
offsets, nested calls, integer/native/stale inputs and opaque bytes. Twelve fixed-reference queries use an
`absent(...)` base to exercise productive newest-winner joins and encoded-time duplicate errors at positive and
negative timestamps beyond binary64's exact integer range. Their independent result assertions reject empty
vectors as qualification. The original ordinary-time extreme case remains unchanged and returns an empty vector;
Prometheus's nanosecond conversions prevent that case from qualifying a join. The panel verifies the original
source/module/compiler attribution, exact artifact and input rosters, every
original group and its combined output. Candidate dataset acknowledgements and query results must occur in
their declared order; missing, rebound or unsupported outcomes fail. Comparison preserves complete labels,
timestamps, diagnostics, error types/bytes and numerical bit reports, allowing only unspecified result row order.
Falsification tests reject incomplete bindings and cross-dataset packet mistakes. Original captured results
are never refreshed to fit the implementation.

The full expression gate after INFO passes 17,452 of 21,442 variants, with zero completed semantic failures and
zero paired numerical bit differences; 3,990 variants remain OPEN. This includes 122 newly completed INFO checks
and retains every previously passing identity. The shared instant reader and INFO accounting tests cover pages
1/17/1024, 2,048 versus 100,000 samples, ownership release, cancellation, bare provider errors and discarded
unfinished diagnostics. Exact ordinary memoized initial reads, seeks, lookahead and fault precedence are
qualified by the finite shared-reader panel below; these INFO results do not establish arbitrary iterator or
complete language compatibility.

Terminal provider diagnostics retain explicit severity, opaque bytes and optional positions in independently
owned results. Expansion failures carry their own notes and completed numeric parameter notes according to source
boundaries; unfinished ancestor and ordinary raw-helper failures discard their notes. Replays do not duplicate
diagnostics. Deferred selection errors retain their buffer capacity while awaiting their source evaluation owner.
Metadata handles and output can grow with cardinality; raw history remains page scoped. Foreign transient allocation
accounting uses the allocation handles described below. Complete expression qualification remains required. All
admission limits remain disabled.

### COUNT_VALUES generated label identities

`count_values()` validates its preprocessed string parameter before evaluating the vector body. It overwrites
the destination label before grouping, adds it to `by` grouping, and retains source hash collision behavior.
Each activation consumes the child once, one sample at a time, keeping counters and the first current labels
for each group. The source's unspecified group-map boundary uses the approved deterministic label-byte order.
Discovery retains exact generated label identities across the logical scope; projection separately preserves
source full-hash aliases and first-label ownership. Default and delayed name removal retain their distinct
instant and multistep duplicate checks.

Float labels use Go's shortest fixed-decimal text, including negative zero and subnormal padding. Histogram
labels follow `FloatHistogram.String`, including productive malformed prefixes and known source runtime errors.
Negative buckets are reversed using constant traversal state; no bucket vector or raw history is retained.
Raw instant float histogram acquisition preserves all supplied fields. Explicit histogram `Copy` operations
and integer-to-float conversion retain their separate source normalization rules. Known wrapped source errors
use the existing C evaluation status and runtime error kind; unexpected Rust panics retain containment.

Counter groups and generated label metadata can grow with cardinality and histogram text size. They are precisely
accounted and support cancellation. The source's 24-bit label-field representation bound still applies. Query
memory/work/time/concurrency enforcement stays disabled. Ancestor and output replay can repeat an activation;
this implementation does not establish a polynomial work bound for arbitrary nested expressions.

```sh
python3 tests/promql-conformance/count_values_calls.py --capture \
  --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/count_values_calls.py --reference "${PROMQL_COUNT_VALUES_CAPTURE_DIR:?}" \
  --binary "${PROMQL_QUERY_BINARY:?}" --page-size 1024
```

The durable panel declares 796 original queries covering fixed-decimal boundaries, both native sample kinds,
malformed and nonfinite buckets, byte labels, grouping/full-hash collisions, changing identities, nested consumers,
invariance, both profiles, storage faults and name-error precedence. It reuses the source-bound cursor/phase
capture and checks complete outcomes with exact numerical comparison. Only unspecified instant row order is
normalized; range ordering and all source errors remain exact. Packet/binding falsification tests reject altered
requests and mismatched error classes. Permanent typed tests also check changing identities, nested COUNT cursor
growth, raw-field preservation, constant formatter scratch, inner cancellation, ownership cleanup and C error
reports surviving foreign/query/engine release. Unsupported parent reducers and subqueries remain required.

All 796 cases match exactly at pages 1/17/1024 in both release profiles, alongside all 64 builtin COUNT_VALUES
variants. The complete expression gate now passes 17,516 of 21,442 variants, preserving previous passes, with
zero completed failures or paired bit differences; 3,926 remain OPEN. Both profiles pass 294 library tests
(295 with fixtures), strict Clippy and the declared MSRV check. The retained 80,942 float-text cases and focused
cursor, INFO, histogram-alias/provider and preprocessing panels retain their previous exact outcomes.

### Paged TOPK/BOTTOMK selection

`topk()` and `bottomk()` retain whole-body series and group metadata, then use resumable source-compatible
heaps of series indices. Heap and final sort comparisons request two values sequentially through the same
explicit evaluator stack. Raw time histories and complete instant value vectors are not retained. The complete
parameter grid is evaluated before the body; a constant-size summary preserves source global validation and
error precedence. Selected values are acquired again for output, including their exact NaN and signed-zero bits.

An execution-local cache keeps one immutable selection for one effective time/scope. Cache hits reuse validated
parameters before replaying them. Snapshots own index buffers, with precisely observed lifetime and overlap;
selection state grows with series/groups and selected cardinality. All resource limits remain disabled.
Nested sum/selection controls guard a known repeated-parameter amplification. This does not establish a
polynomial work bound for arbitrary nested expressions or a production performance claim.

Actual invariant wrappers freeze their full evaluator subtree and rebind its output point times to the caller.
Unvisited parameter subtrees keep their original clock behavior. Fixed matrix windows and current-step rate
arithmetic have distinct endpoints. Cursor prefixes follow logical grid positions even when lookup endpoints
repeat. Singleton selection output preserves heap order through parents; multistep maps use the approved
deterministic label-byte order. Matrix replay retains repeated points, while ordinary consumers reproduce the
source's single-head consumption and public instant conversion prefers the first histogram, otherwise first float.
Duplicate cleanup uses the pre-merge matrix-row cohort, including inherited repetitions. Transparent parentheses
retain the outer root matrix's cleanup cohort, including fixed-reference children.

```sh
python3 tests/promql-conformance/topk_calls.py --capture --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/topk_calls.py --reference "${PROMQL_TOPK_CAPTURE_DIR:?}" \
  --binary "${PROMQL_QUERY_BINARY:?}" --page-size 1024
python3 tests/promql-conformance/topk_policy.py --base "${PROMQL_TOPK_CAPTURE_DIR:?}" --capture
python3 tests/promql-conformance/topk_policy.py --base "${PROMQL_TOPK_CAPTURE_DIR:?}" \
  --reference "${PROMQL_TOPK_POLICY_DIR:?}" --candidate "${PROMQL_TOPK_COMPARISON_DIR:?}"
```

The declared source panel contains 3,556 complete query outcomes, including heap/sort thresholds, changing k,
groups, native ignored samples, clocks, failures, parents and full-hash aliases. The first comparison remains
strictly ordered and exact. The separate policy gate binds 24 original child/injection controls at actual
unspecified map boundaries and eight canonical COUNT_VALUES instant outputs. It preserves all raw failures
and changes only these 32 declared expectations in a separate file. Confined child injection requires
ordinary distinct float points and no diagnostics; its ABS controls share one constant metric name and no grouping.
It does not support arbitrary intermediate-state injection. Meaningful mutation tests preserve source scope,
points and non-boundary outcomes and reject malformed controls.

Typed tests cover exact sum conservation over 2,048 and 100,000 raw points at pages 1/17/1024, constant raw
workspace, one cursor across the whole provider, cancellation during a heap comparison, partial-output failure
and baseline-aware release. Complete core, SRE and remaining policy qualification are still required.

### Paged LIMITK/LIMIT_RATIO selection

`limitk()` and `limit_ratio()` share the ordinal-only selection heaps and evaluator continuation. LIMITK
retains the first available K samples per group; LIMIT_RATIO tests each sample's complete raw label hash before
canonical output projection. Both accept native histograms, use positive zero as their heap comparison value,
emit the unsorted heap and forward histogram payloads unchanged. Comparison and output replay acquire one
child value at a time. Actual float/histogram head eligibility is checked before advancing a canonical vector row.

The complete parameter grid and body precede global validation. Ratio warnings preserve source construction
order and the visited parameter span; unvisited parameter subtrees retain their original spans and clocks.
The immutable selection snapshot retains the validated scalar parameter, indices and effective scope.
Raw history length does not enlarge these buffers; retained metadata grows with series/groups and selected
cardinality. Resource limits remain disabled. Finite nested replay tests do not prove a general nested work bound.

The sampler facade captures 13 literal byte-label sets through the original `HashRatioSampler` and 17 separately identified
unsigned normalization primitives. The latter reproduce the source cast/division expression and invoke its
threshold predicate, without claiming actual label-hash preimages. The 450 thresholds retain exact offset and
ratio bits, signed zero, NaN, unsigned rounding to offset 1.0 and both predicate branches. Adjacent negative
ratios may select the same sample because `1+r` rounds; they are not assumed to straddle the boundary.

The strict loader binds the entire pinned module, helper bytes, declared inputs, compiler/profile and complete
output packet. It checks normalized offset bits, exact threshold neighbors and boolean predicates, rejecting
re-sealed primitive mutations. Literal label hashes remain independently captured original results. The query
generator consumes the validated sampler capture and declares 2,240 complete queries: global and moving
parameters, heap emission, grouped hash collisions, faults, native payloads, full-hash mixed heads, scopes and
parents. Additional negative controls pass the captured rounding plateau. Native fields are copied unchanged.
All experimental runtime declarations use `upstream-tests`; disabled-profile rejection remains in the frontend gate.

```sh
python3 tests/promql-conformance/ratio_sampler.py --capture --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/ratio_sampler.py --reference "${PROMQL_RATIO_SAMPLER_DIR:?}"
python3 tests/promql-conformance/ratio_sampler.py --reference "${PROMQL_RATIO_SAMPLER_DIR:?}" \
  --binary "${PROMQL_KERNEL_BINARY:?}"
python3 tests/promql-conformance/limit_calls.py --capture --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}" \
  --sampler-reference "${PROMQL_RATIO_SAMPLER_DIR:?}"
python3 tests/promql-conformance/limit_calls.py --reference "${PROMQL_LIMIT_CAPTURE_DIR:?}" \
  --sampler-reference "${PROMQL_RATIO_SAMPLER_DIR:?}" --binary "${PROMQL_QUERY_BINARY:?}" --page-size 1024
python3 tests/promql-conformance/limit_policy.py --base "${PROMQL_LIMIT_CAPTURE_DIR:?}" \
  --sampler-reference "${PROMQL_RATIO_SAMPLER_DIR:?}" --capture
python3 tests/promql-conformance/limit_policy.py --base "${PROMQL_LIMIT_CAPTURE_DIR:?}" \
  --sampler-reference "${PROMQL_RATIO_SAMPLER_DIR:?}" --reference "${PROMQL_LIMIT_POLICY_DIR:?}" \
  --candidate "${PROMQL_LIMIT_COMPARISON_DIR:?}"
```

Query comparisons require exact numbers and ordered complete outcomes. The original 1,864-case capture remains
immutable; the expanded declarations require a fresh capture. The separate policy gate qualifies six singleton
COUNT_VALUES child injections and four instant root COUNT maps under the approved label-byte order. Injected
children preserve their sole raw `v` label and ordinary distinct float points, without diagnostics or deferred names.
The gate retains all raw differences and changes only ten declared expectations in a separate file. It still
rejects warning-position errors; ABS API output cannot substitute for raw labels in ratio hashing.

The fixture kernel invokes the production normalization and membership helpers for all 450 captured pairs.
Meaningful packet tests reject missing/reordered results, malformed bits and boolean types; valid numerical or
membership differences remain failures. Query tests cover grouped substitution, fault independence and native
field preservation. Typed resource tests extend conservation, one-provider-cursor, cancellation and partial-output
release checks to both LIMIT operators and native payloads. Complete core and SRE qualification remain open.

### Shared instantaneous cursor replay

Ordinary selectors, statistics decoding and INFO projections use one memoized paging driver derived from the
pinned `storage/memoized_iterator.go` and `promql/engine.go`. It reads the first available sample before seeking,
retains one predecessor, and performs a moving storage Seek only when needed. A logical no-op Seek retains the
current page and statistics access epoch. Future-current access occurs before predecessor selection; required
Next/Seek failures precede fallback, while successful EOF preserves an eligible predecessor. Exact endpoints
stop without another terminal read. Fixed selectors replay their source-defined prefix; direct clock lookups
retain their distinct delta and wrapped fixed-reference offset. Invariant direct clock calls calculate that
offset at the scope's first step and repeat the first result, including absence, rather than replaying a grid
of transformed timestamps. Permanent regressions cover positive/negative duration wrap, direct/parenthesized
fixed selectors and both productive and empty first-step outcomes.

Direct clock calls initialize matching series before their first lookup. Each cursor/page is released before
the next series opens; only accounted Ready/Empty/error metadata survives initialization. Initial Provider read
errors are raised at their original lookup phase after remaining series initialize. Cursor-construction,
invalid-page and interruption errors retain the typed storage contract. Ordinary owned histogram projections
poll cancellation while copying large payloads. Retained raw state is one bounded page and at most a predecessor
and current projection, independent of history length; metadata, owned outputs and replay work remain accounted
and can grow. All resource enforcement remains disabled.

The common walker owns cursor cleanup, post-operation resource accounting and complete returned-page validation.
Scheduled instantaneous selectors satisfy forward movement from the page already held when its remaining samples
can reach the target. Before the first such movement, they validate that returned page's unread tail once, preserving
source-validation precedence before later logical points. They retain one borrowed page and bounded indices;
skipped points are not projected. Movement beyond the page performs the required physical Seek and Next.
Stop and Seek still validate the unread tail's timestamps and bounds; an earlier visitor error keeps precedence.
Provider errors occur at actual physical operations, so skipped redundant callbacks do not produce an error.
General range scans retain their initial read, actual Seek calls and full-page contract through the same owner.

```sh
python3 tests/promql-conformance/cursor_calls.py --capture --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/cursor_calls.py --reference "${PROMQL_CURSOR_CAPTURE_DIR:?}" \
  --binary "${PROMQL_QUERY_BINARY:?}" --page-size 1024
```

The 82 independently declared queries include 32 no-fault and 50 operation-fault inputs, with 40 default-profile
and 42 upstream-test-profile requests. Fresh original captures reproduce the retained complete results and phase
evidence. Both Rust profiles pass at pages 1/17/1024 with exact labels, timestamps, diagnostics, error types/bytes
and paired numerical bits. The harness privately reuses canonical engine options and codecs, binds its exact
helper/module/source/input/binary roster, and rejects lost, rebound, reordered or unsupported result packets.
Falsification tests prevent identical datasets from aliasing distinct fault configurations. The panel exercises
Next timestamp/EOF and Seek target faults; it does not qualify the wrapper's unused SeekEOF or set-error controls.

These synthetic operations expose successful page prefixes before a required sample fault and permit moving Seek
to skip an unread Next fault. An atomic failed storage page returns no samples. Physical page callback traces,
arbitrary transient or call-count-dependent faults and cancellation/error races are separate from the repeatable
query-replay contract. This panel is finite source qualification, not arbitrary iterator or complete-core acceptance.

### Common single-selector schedules

Preparation certifies expressions whose rows each bind to one physical selector ordinal with unique full label
hashes. Eligible chains include parentheses, unary operations, vector/scalar arithmetic and comparisons, ordinary
pointwise functions, and unambiguous label projections. Scalar parameters may use literals, scalar arithmetic and
`time()`; vector-derived scalar parameters retain ordinary replay. The certificate is scoped to the original
selector, child roster and grid. It does not infer independence from the final output's cardinality.

A certified expression visits its original child ticks through one cursor/page and bounded current/predecessor
values. Each tick uses the ordinary operator machine with its scalar time, flags and diagnostics. Preparation
preserves postorder acquisition and first productive row order. Derived reducer validation, productive replay and
provider-backed quantile materialization share the same scheduled owner. A single certified row can also stream
standalone computed matrix output; other cohorts retain their general replay path.

Duplicate full hashes, merged source heads, joins, aggregations, sorting, INFO, nested matrix children,
histogram variance/deviation and source-incompatible wrapped or smoothed clocks retain ordinary replay. Label
projections that merge physical ordinals do not receive the certificate. These fallbacks preserve the full
expression result while potentially repeating work; the scheduler is not a general distributed planner.

Native generated regressions compare growing histories at pages 1/17/1024, including a 1 ms lookback and one-second
step, and assert fetched samples/pages as well as history-independent raw workspace. They exercise original tick
values, clocks, ordinal projections, validation/error order, cancellation, sink failure and cleanup. The separate
208-case schedule panel uses unchanged Go outcomes; only eight declared projection folds use the approved
unspecified-order policy, with canonical final label order checked and the strict ordering differences preserved.

The native release/page1024 sparse-clock witness reads 7,300,000 sample entries in 7,154 pages for 100,000
computed points, versus 7,436,964,352 entries in 7,300,000 pages before page reuse. In those captured runs,
query time falls from 151.65 s to 16.49 s. Engine workspace is 38,476 bytes at both 2,048 and 100,000 points;
borrowed source pages and provider-selected temporary backing are accounted separately. These are finite fixture
measurements, not throughput or elapsed-time guarantees for dbengine or other devices.

### Certified final range output

Final vector range queries reuse the existing single-selector certificate after checking every row and its final
label identity. Each physical series emits chronological points through one cursor/page, using the original
operator machine. Normal rows keep their ordinal IDs. Delayed cleanup and root negation retain their existing
first-productive-time IDs with a bounded prefix pass and compact `(time, row)` metadata. Final identity collisions
and unproved shapes use ordinary evaluation. Preparation owns logical statistics and diagnostics; the final visitor
observes actual work, borrowed/owned memory and successfully delivered points.

The generated regressions stream up to 100,000 points without collecting their answer. At pages 1/17/1024 they check
one active cursor and unchanged workspace/provider/borrowed peaks as history grows. A 13-case source-bound
before/after replay preserves complete results. At page size 17, selector histories of 16/32/64/128 points change from
272/801/2630/9372 fetched entries to 32/64/128/256; peak workspace remains 6897/6961/6961/6961 bytes in both builds.
These are fixture resource observations, not disk reads, whole-process RAM or production throughput measurements.

Direct `timestamp`/`start_timestamp` initialization keeps initial Provider errors until ordinary lookup. A latched
initial error disables that selector's schedule certificate, so a later tick of an earlier series cannot override
the initial lookup failure of another series. Parentheses share the direct-clock path; unary wrappers retain their
source-defined child preparation.

### Physical cursor fault qualification

The original 82-case cursor capture remains unchanged. Page 1 matches all 82 strict original outcomes. Pages 17/1024
retain 74 strict matches and eight separately qualified physical-Seek outcomes: faults attached only to an omitted
callback cannot occur. Four vector outcomes use identical-input, fault-free originals in that capture. Four
two-fault outcomes use new original Go controls removing only the omitted Seek fault, preserving the other series'
initial Next fault. Complete results are compared with ID substitution only; no result is authored by Rust.

`cursor_policy_calls.py` records the strict report and a separate physical-operation comparison, verifies the exact
declared difference roster, and also executes the independent controls. It binds candidate, inputs, original/control
results and comparison helper bytes. The raw disagreements are evidence and are never overwritten or counted as
strict matches. The existing page-reuse tests separately check actual omitted callbacks and errors when movement
crosses a page boundary.

```sh
python3 tests/promql-conformance/cursor_policy_calls.py --capture \
  --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/cursor_policy_calls.py \
  --reference "${PROMQL_CURSOR_CAPTURE_DIR:?}" --controls "${PROMQL_CURSOR_CONTROLS_DIR:?}" \
  --binary "${PROMQL_QUERY_BINARY:?}" --page-size 1024
```

### Ordinary buffered matrix replay

Ordinary range functions and raw matrix roots use one source operation owner derived from
`storage/buffer.go`, `matrixIterSlice` and the statistics iterator. It separates query-wide data bounds,
physical window endpoints and included result bounds. Even an empty included window can require an initial
read and Seek. The initial Seek is issued when required, including when it lands on the already current point.
Exact endpoints stop without reading EOF; future floats are accessed while future native values supply only
their timestamp. Only nonempty admitted windows reduce the buffer delta. Fixed `@` selectors reuse their first
physical window while unsafe scalar function arguments retain the current evaluation time.

Each helper pass validates acquisition before streaming its members. Reconstruction retains scalar schedule
state and, for statistics decoding, one accounted histogram predecessor. It allocates no raw history, time
grid or matrix of raw values across series. Complete returned pages remain validated and accounted, including
an unread page tail. Engine sample counters include every physical publication and replay read. Native histories
and schedules of 100,000 points retain the same provider, borrowed-view and workspace peaks as shorter histories
at each tested page size. Cursor errors, interruption and helper unwinding release retained pages and decoders.

Replay adds CPU and storage reads: a target may reconstruct earlier steps, and numerical helpers can require
several passes. Work therefore grows with history, step count and helper passes despite bounded raw memory.
Metadata, accumulators and owned derived output remain separately accounted and can grow. All enforcement is
disabled. General nested adapters and extended windows require their own qualification.

The permanent source-control rosters contain 1,826 ordinary, 370 clipped/error-phase, 376 timestamp-fold and
220 widened-universe zero-range cases, including 458 source construction errors. They cover every implemented ordinary family, native statistics
epochs, empty/stale steps, required and unrequested operations, tiny physical windows and fixed selectors with
varying scalar arguments. Expectations come solely from the unchanged pinned Go source. Comparisons require
exact labels, times, membership, diagnostics, errors and paired numeric bits at pages 1/17/1024.

```sh
python3 tests/promql-conformance/matrix_reader_calls.py --roster ordinary --capture \
  --upstream "${PROMQL_ORIGINAL_SOURCE_DIR:?}"
python3 tests/promql-conformance/matrix_reader_calls.py --roster ordinary \
  --reference "${PROMQL_MATRIX_READER_CAPTURE_DIR:?}" --binary "${PROMQL_QUERY_BINARY:?}" --page-size 1024
```

Use `--roster clipped`, `--roster timestamp` or `--roster zero` with its matching capture for the other rosters.
The separate zero-range capture verifies 84 armed initial timestamp/EOF errors in the unchanged Go source and
their suppression when physical endpoints coincide; its available query-wide universe prevents inert faults.

The earlier selector-catalog checkpoint passed 187 unit tests in both profiles, with 64 independent selector-hint
projections, 426 preparation scopes, 198 offset projections and retained parser/clock/phase panels matching their
pinned source. Its corpus had 9,914 passing variants and 11,528 open variants, with 24 paired bit reports. Current
expression counts are recorded with the histogram caller qualification above. Finite catalog and preprocessing
checks do not satisfy the complete expression, SRE, C or accounting gates.

Matchers, owned histogram bounds, immutable diagnostics and observed selection parents now share one local
strong-only owner. Its exact Box layout includes the reference count, allocation charge, inline payload and
alignment; nested buffers retain separate capacity charges. Trait erasure uses ordinary Box coercion before
sharing. Clones retain the same allocation without charging it again, and the final owner releases its charge
after payload destruction and Box deallocation, including an ordinary payload panic. Checked reference-count
overflow cannot mutate the counter. The complete header and payload constrain thread access. There is no weak,
mutable or copy-on-write owner interface.

Owned bounds preserve moved vector capacity, share only in the same query/category and copy into a different
accounting context. Unobserved caller Arc bounds keep their external identity until an observed copy. Empty
bounds have distinct real shared headers; absent bounds have none. Empty diagnostic results have no wrapper.
Stable Rust and the 1.91 minimum version pass 166 units and compiler rejection controls for unsafe RefCell thread
access and implicit shared-handle unsizing. Historical selected-owner Miri checks at ABI 2 cover three seeds, alignment/DST/ZST,
overflow, concurrent last-owner release, panic cleanup and bounds/diagnostic/selection lifetimes. Float values
and output points remain 16/24 bytes on these amd64 builds. The earlier finite checks qualify those four owners;
the root/control qualification below extends the same wrapper. The full C boundary remains required. Backend
regex memory continues to be explicitly estimated.

Engine/query ledgers and explicitly engine-created cancellation allocations occupy the eighth `Accounting`
memory category; the preceding seven indices retain their meanings. Roots census their own known Box layout.
A query or owned token charges only its engine ancestor, which stays alive until the child's actual deallocation
and charge release. Clones share the same allocation and do not acquire its bytes again. No root stores a lease
to itself. Idle engines have a real fixed baseline; live plans, retained empty results, diagnostics and
provider-held accounts can keep query roots charged after execution ends.

`Engine::cancellation()` creates a token whose physical charge belongs to that engine across clones and use by
other engines. Caller/default tokens remain external. Query and consuming-engine `cancellation_references`
counters separately observe guard-held aliases, divided by engine-created versus external origin. Their unit is
references, not bytes or unique tokens; nested public helper guards can hold additional aliases. These counters
do not enumerate independent caller/provider clones and do not duplicate the physical token charge. Guards
remain live through provider destruction and sink abort, then release before the final report snapshot.
Cancellation still takes precedence over the inline deadline; observation does not change error or polling order.

Productive label retention uses one checked traversal for predicates, swaps and normal suffix release. Empty
label sets and empty grouping lists still observe cancellation; grouping-name comparisons poll byte chunks.
Grouped preparation, lookup and output copies preserve raw empty-valued keys and first hash matches. Empty
values are removed at the original output phase, after raw grouping hashes. Label lookup polls long byte
comparisons; insertion/removal moves and instant cohort metadata walks are observed without additional buffers.
Public owned-value mutators retain
their inert interface; query execution uses checked owners. Interrupted local metadata is discarded with its
query context, and error/unwind destruction remains infallible. Required native-accumulator bucket observation
retains its separate open qualification. Foreign transient allocations use the callback accounting interface below.

Cleanup checks verify every memory category against exact live root counts, including `Accounting`; they do not
exempt bookkeeping from leak detection. Query snapshots include the query root that is live when the snapshot
is taken. Finite tests cover root identity, zero-capacity retention, creator/consumer attribution, concurrency,
poison recovery, scoped failures and panicking cleanup. Typed C ownership is exercised by the finite embedding checks
below, including caller-reported transient allocations. The full expression/SRE gates remain required. All resource
enforcement remains disabled.

Compilation and reported execution/collection have shared internal entry points that accept their actual operation
account. Native callers use these same paths; adapters can charge descriptors, errors and retained report/result
wrappers to that account without opening a second ledger or execution observation. Compilation retains its account
through the plan or ordered parse errors. Existing execution timing and concurrency counters retain their meanings.

Provider and output failure bytes retain their allocation charge after callback and operation cleanup. Previously
unobserved owned buffers transfer without copying and acquire their actual capacity charge at the callback boundary.
Already observed buffers retain their original physical owner and category. Observation does not add a cancellation
poll or overwrite an error returned by a callback. Upfront selector failures keep their lazy expansion phase; standalone
scans account cursor open/seek/page failures as well. The C adapter retains opaque returned errors before another
callback can invalidate them. Pinned upstream selection phases remain distinct: cancellation during selection creation
is checked on evaluator entry; an already returned terminal expansion error retains its error and warnings.

Output sinks have checked delivery delegates with borrowed control and operation accounting. Existing ordinary
callbacks use default delegates; their returned error wins over cancellation triggered inside that callback.
Checked collection polls byte/label/native copies, series lookup, points and final label ordering. Result histogram
copies preserve evaluated bits and absent versus present-empty fields. Direct inert collector calls remain available.
No cancellation poll or fallible construction follows successful external commit. The C adapter uses these checked
delegates for descriptor marshalling and independently owned collection.

### Synchronous C embedding

`include/netdata-promql.h` declares version 4 of the standalone interface. The crate emits an
`rlib` and a static library. The containing Netdata tree links it into the Agent;
the [installation and API guide](../../../../../tests/promql-integration/GETTING_STARTED.md) describes live queries.

- Engine, immutable query, control, report and result handles have paired release functions. Results and reports own
  their data independently of the query, foreign provider and engine handle. Individual getters borrow bytes and
  native arrays until the corresponding handle is released.
- Selector handles own ordered byte matchers independently of the input bytes and engine. Matcher views borrow
  until selector release. Conjunction testing consumes caller-owned labels and accounts for their traversal before
  matching. Invalid selector errors retain upstream positions and order.
- Additive precise requests distinguish Unix-millisecond projection from the pinned engine's wrapped nanosecond
  projection. Lookback is already resolved by the caller. Time/duration helpers retain exact upstream errors;
  formatter output is length-delimited text. Reached sub-millisecond range-grid errors remain observable.
- Tables, options and requests have a version/size prefix; future trailing fields are ignored. Byte and array views
  use explicit counts. Shape checks cover active tags, reserved fields, optional presence, alignment, address/count
  arithmetic and required callbacks. Pointer liveness, correct handle types and nonracing access are caller contracts.
- Provider scopes, selections, series and cursors have paired ownership. Successful NULL opaque tokens receive their
  matching close. Series metadata survives selection close. Cursor pages survive until that cursor's next operation;
  one bounded raw page is retained, validated and observed before cancellable publication. Adapted indexing reads
  its immutable headers on demand without retaining a second sample-descriptor array. Repeated reads use stable input.
- Temporary float-bit backing has five optional callbacks: create, read, write, resources and close. Provide all five
  or none; incomplete groups are rejected before dispatch. None remains valid for expressions that do not require
  backing. Successful creation owns its token, including NULL, until exactly one close; failed creation cleans its
  own partial state. Offsets and lengths are 64-bit logical slots. Successful transfers must return the exact requested
  count and preserve every NaN/signed-zero bit; slices remain borrowed for the callback only.
- Raw sample tags distinguish floats, integer histograms and float histograms. Arrays preserve absent versus
  present-empty identity; scalar fields preserve binary64 bits. C `double` bit helpers use `memcpy`.
- Output callbacks receive borrowed provisional chunks, then commit or one abort. Returned error bytes are copied
  before cleanup. Callback failures retain precedence over concurrent cancellation; successful commit is final.
- Explicit controls are charged to their creator engine, including cross-engine execution. Callback controls are
  borrowed; NULL optional call controls create private engine-owned tokens. Explicit deadlines activate once.
- Whole-export guards contain ordinary Rust unwind panics without installing hooks or a runtime. Allocation abort,
  invalid pointers, foreign exceptions/longjmp and destructor panic during active unwinding retain their documented
  runtime/caller limits. Compilation timing belongs to its compilation account; execution counters keep their scope.

Run the native embedding gate from the repository root:

```sh
python tests/promql-conformance/c_embedding.py --sanitizers --abort-check
```

Use the configured development Python environment. The driver uses installed GCC/Clang and Cargo offline; it creates
private `.tmp/c-embedding-*` evidence containing commands, source hashes, layouts, binaries and witness output.
Process identities and ordinary completion are retained without an automatic process-killing deadline. Static
archives and the Rust layout dependency roster are hashed before and after their consuming commands and included
in the evidence manifest. Both unwind profiles compare all 58 declared structures/unions, 17 callback aliases and
33 shared constants between Rust,
C11 and C++17, and check header/archive exports. The real foreign callers exercise arithmetic, repeated paging,
resource conservation, independent results, opaque errors, output delivery and cancellation/error precedence.
ASan/UBSan instrument the native C caller, not the Rust static library. Historical Miri receipts retain their ABI 2
checkpoint; current focused borrowed-page checks have their own source identity and are not a complete ABI 4 Miri
gate. `panic=abort` must fail the crate's compile guard.

The protocol primitive gate replays 230 original helper/selector outcomes and 528 nested labelset comparisons
per native profile. It excludes two caller time-parameter wrappers and twelve selector-batch wrappers. The precise
query gate compares nine original execution witnesses at page sizes 1/17/1024. Both retain immutable original
expectations, source/binary identities and new receipts; neither executes an HTTP router or live storage.

```sh
python tests/promql-conformance/protocol_primitives.py --binary target/release/promql-protocol-fixture
python tests/promql-conformance/precision.py --binary target/release/promql-query-fixture
```

Repeat with the `target/release-min/` fixture binaries. Build them with the `fixtures` feature first.

Metadata discovery uses the actual provider and provisional output callbacks, validates one series at a time,
and fetches no samples. Library units cover selector unions, malformed nonmatching labels, provider/output failures
and cancellation. The C embedding gate covers absent selectors, matches/nonmatches, normal early stop, output failure
ahead of concurrent cancellation, and retained output allocation handles after reports are released.
These generic calls do not qualify a Netdata discovery route or its union/sorting policy.

The callback index sorter uses the existing pinned Go sorting program with the enclosing account/control.
Its comparator receives immutable index values and must not access the exclusively borrowed mutable index array.
Units compare its complete permutation and comparison trace with the existing qualified routine, including ties.
Actual C callers cover success, empty/invalid arrays, prior cancellation, cancellation during comparison and owned
comparator errors ahead of cancellation. Partial permutations are discarded with failed surrounding output.
This bridge qualifies generic ordering and ownership, not Netdata's backed label comparator or HTTP serializer.

The limit parser has a separate frozen original capture:34 raw-byte cases run through the unchanged pinned
Prometheus helper on Go1.27.1 AMD64. This includes64-bit bounds on ARM32, overflow/error precedence, signs,
invalid UTF-8 and long leading-zero inputs. The capture command requires the existing toolchain and checks the
complete original api.go hash before extracting its helper. It installs or downloads nothing.

```sh
python tests/promql-conformance/protocol_primitives.py \
  --binary target/release/promql-protocol-fixture \
  --reference tests/promql-conformance/api-limit-original.json
```

Run the same frozen limit gate for `release-min`. To regenerate the unchanged original, use
`api_limit_capture.py --source <pinned-api.go> --go <existing-Go1.27.1>`; keep its recorded source/helper identity.
Successful/error reports also supply a borrowed callback account for caller preparation and response backing.
Actual C callers verify that allocation handles acquired there keep the account alive independently of the report.

With the installed Rust target libraries, GNU GCC/G++ 14, target libc and QEMU, run the same interface gate on ARM:

```sh
python tests/promql-conformance/c_embedding.py --target aarch64-unknown-linux-gnu
python tests/promql-conformance/c_embedding.py --target armv7-unknown-linux-gnueabihf
```

The target compiler supplies the verified loader prefix. Both profiles compile and execute Rust/C layout probes and
actual C11/C++17 callers; target tools, loader, binaries and link inputs are bound to the evidence. File callbacks
require 64-bit `off_t`, including on 32-bit ARM. These generated C storage cases contain at most 100,000 slots;
the adapter's separate sparse-file controls exercise logical offsets above `2^32`. Target GNU callers run without
instrumentation; the native Clang caller gate retains its separate ASan/UBSan scope.

The generated-history sum witness uses pages of 1, 7 and 1,024 samples. Increasing history from 8,192 to 65,536 points
leaves observed workspace peaks unchanged: 7,427, 7,643 and 227,099 bytes respectively in the retained native-recovery
and canonical-controller cohorts on the qualified amd64 host.
Borrowed raw-page peaks are 32, 224 and 32,768 bytes. Fetched sample/page counters equal the provider's actual physical
deliveries, including replay. These finite measurements do not bound cardinality-dependent operator state or output.

Normal callbacks obtain their actual query's borrowed accounting context through `netdata_promql_callback_accounting`.
Resource, close and abort callbacks receive it directly. Abort receives this capability even before begin when a valid
query was established; an invalid query supplies NULL. Contexts expire when their callback returns and must not be
retained. ABI4 rejects ABI1/ABI2/ABI3 tables before dispatch. The separate `abi3-rejection.c` executable uses the
exact previous header from signed commit `f14c541028a9047da34bed7eb41b705abe9bd741`, checks the version mismatch
and rejects otherwise valid old engine options. It does not call old observation getters after mismatch.
Every caller must rebuild and check
`netdata_promql_abi_version()` before using tables or observation outputs: output getters write the complete current
structure, without an input version prefix that could protect an older caller's smaller output buffer.

`netdata_promql_allocation_acquire`, `resize` and `release` observe caller-reported live allocation bytes. They do not
allocate or free foreign storage. An acquired handle retains its creator query and engine account independently of
callback, query, report and engine handles. Compiled, Workspace, Result and ProviderHeld are valid payload categories;
the engine charges each handle's known wrapper layout precisely to Accounting. Zero-byte handles are valid. Release
must not race access, and callers release their handle after freeing its foreign storage.

Allocation handles observe scratch freed before callback return and concurrent allocation overlap. Allocate/copy/free
replacement requires two handles for its two simultaneously live allocations; resizing one handle cannot reveal hidden
allocator overlap. Resource snapshots report residual owner storage, excluding descendants, adapter buffers and bytes
already covered by allocation handles. BorrowedView describes exposure separately from physical storage. Requested
allocation bytes exclude allocator-internal metadata and do not measure process RSS.

Temporary-storage observations are separate from the RAM census and raw page/sample counters. Extent reports
logical `8 * value_count` bytes; stores reports simultaneous successful scopes. Read/write calls count attempts,
and byte counters count accepted logical transfers. They do not measure physical blocks, kernel-cache residency
or partial device I/O before failure. Charges remain live through physical close; nested and concurrent scopes
contribute their actual overlap to engine and query peaks. All configured resource enforcement remains disabled.

The literal sort retains two 1,024-value pages (16 KiB payload), plus precisely accounted wrappers and fixed
controller state, per active operator. Materialization overlaps only the first value page with a raw source page;
the second value page is allocated after the materializing cursor closes. Dirty pages write back on necessary
eviction. Cleanup discards unused dirty pages and preserves an earlier query error.

Qualification providers explicitly supply unnamed disk files on Linux x86_64, AArch64 and ARM fixture targets. The Rust
backend is compiled only for tests/the `fixtures` feature; the C gate receives a validated private scratch directory
from its driver. Unsupported backing returns an explicit fixture error. No provider filesystem policy is imposed
on production, and no files are silently created by the engine.

The unchanged 2,152-case temporal panel and the independent 84-case temporary panel use these drivers:

```sh
python tests/promql-conformance/temporal_quantile_calls.py \
  --reference "${PROMQL_TEMPORAL_REFERENCE:?}" --binary "${PROMQL_QUERY_FIXTURE:?}" --page-size 1024
python tests/promql-conformance/temporary_quantile_calls.py --capture --upstream "${PROMQL_UPSTREAM_SOURCE:?}"
python tests/promql-conformance/temporary_quantile_calls.py \
  --reference "${PROMQL_TEMPORARY_REFERENCE:?}" --binary "${PROMQL_QUERY_FIXTURE:?}" --page-size 1024
```

The additional inputs cover 1,023/1,024/1,025/2,049-value cache boundaries, computed and overlapping subqueries,
fixed timestamps/offsets, stale/native transitions, nested ambiguous NaN windows and 100,000-point generated
history. Reference outcomes come from unchanged pinned Go. These snapshot-based semantic fixtures do not prove
bounded input RAM; native Rust and real C/C++ generated providers retain one raw page for that separate proof.

The native caller witnesses exercise temporary allocation and exact 7 MiB overlap through normal and cleanup callbacks,
snapshot exclusion for a retained provider buffer, valid-query abort before begin, invalid-query NULL accounting,
retained output cleanup on begin/chunk/commit failure, and allocation handles surviving all other handle releases.
Regex backend memory remains explicitly estimated and all resource caps remain disabled. Complete expression/SRE
gate results are separately attributed in the standalone overview and `core-qualification.json`.

The following label/merge figures record the earlier source checkpoint; the current complete expression and
parenthesized-matrix gates above supersede its pending reducer status.

`label_replace` and `label_join` project whole-series metadata after the vector child completes. Runtime validation
occurs before child evaluation, following the upfront selector catalog. Replacement uses Go's anchored DotNL
expression and expansion rules; join reads original label values before changing its destination. Empty-label
normalization, missing and repeated sources, byte values, name-removal flags and derived timestamps follow the
pinned source. Only the vector argument is evaluated; surviving string parentheses inside unvisited aggregation
parameters follow their separately qualified source branch in the parenthesized-matrix gate above.

Projected labels and child-ordinal bindings are accounted metadata retained for replay. Same-kind overlapping
aliases fail; disjoint aliases and equal-time float/histogram values retain the existing source merge/consumption
rules. Actual duplicate hashes use the first child's labels and flags;
the approved deterministic ordering applies only at the source's unspecified merge boundary. No raw-history,
cross-metric value matrix or presence timeline is retained. Nested calls with 2,048 versus 16,384 raw samples keep
equal workspace/provider/view peaks and at most one open cursor at pages 1/17/1024. Metadata and owned output still
grow with cardinality, and all limits remain disabled.

Both profiles match all 536 completed cases in the 552-case retained label/merge/clock panel and all 264 completed
cases in 288 additional source-derived composition, empty-label, distinct byte-encoding and fixed mixed-kind checks at
pages 1/17/1024, with no semantic or bit differences. Sixteen retained vector-matching dependencies and twenty-four
additional aggregation-parameter dependencies remain explicitly open. Checked byte loops and ordinal lookup count
work and poll cancellation. Filtered selection skips unrequested ordinals directly; nested ordered replay uses
bounded lookup while preserving gaps, repeated ordinals and pending mixed-kind phases. Finite cardinality guards
qualify those avoided scans, rather than the complexity of every existing metadata operation. These checks do not
establish the remaining complete language, SRE, C or accounting gates.

Eighty additional pinned-Go checks exercise two unequal label values with the same actual full uint64 hash,
including same-kind overlap errors, disjoint first-child merging and composition. Both profiles match all bits at
pages 1/17/1024. A permanent regression pins that collision and its negative-zero/NaN payloads. The earlier
separator-byte fixture is a distinct-full-encoding control: its collision under the grouping encoding does not
establish full-label Hash equality.

The foundation represents the full value model and records required inputs. The following are the typed core
acceptance obligations, with candidate results attributed to their declared source/binary cohorts in
`core-qualification.json`:

- Execute the Rust candidate against the complete captured DSL and computed parser/lexer fixtures, including mixed
  samples, failures and structured expectations. Reference capture is not a candidate execution result.
- Run every stable language feature and explicitly configured optional test profile against Rust, without silently
  accepting skips or unsupported features.
- Evaluate every private SRE expression on controlled positive, empty, matching/time and threshold scenarios as
  applicable. Exercise recording dependencies with their output names/labels; empty-data checks cannot prove them.
- Qualify Go regex syntax, byte/Unicode behavior, capture priority and replacement semantics in the selected Rust
  compatibility frontend over `regex-automata`.

The typed Rust core stage includes the approved C cursor/ownership interface, synchronous incremental output
callbacks plus an owned collector, panic containment and resource/concurrency/elapsed-time accounting. Resource
enforcement is disabled; fixed memory, cardinality, concurrency and default deadline caps are not selected. Future
enforcement must adapt to the device. Opaque regex memory is explicitly estimated; observable owned/page bytes
remain precise, and totals containing estimates are not guaranteed bounds. Live DBengine and Agent/Parent APIs are
qualified separately in the containing repository. Go Cloud distribution and independent rule lifecycle remain
tracked stages. Alertmanager remains excluded.

## Borrowed foreign sample pages

The C adapter validates every header and nested array in the returned page before cooperative publication. It
observes the union of exposed foreign regions, preserving overlaps and absent versus present-empty arrays. A
private inline pointer/count wrapper supplies views on demand. It detaches before Next, Seek or Close; generic
Rust `Views` and `Samples` providers retain their direct paths. Whole-page shape errors precede pending cancellation;
timestamp/tail validation stays in the walker, preserving earlier consumer errors.

The wrapper's accessor returns views tied to its own borrow. Only that immutable wrapper receives the thread
marker implementations required by the existing public page API. An embedded Rust slice inside a movable cursor
would remain protected during owner destruction even after assignment to an empty slice; retaining pointer/count
avoids that conflict with the provider freeing its allocation. No raw page or history is copied into engine storage.

The embedding driver's Rust consumer probe checks exhaustive enum matching, public fields/methods, covariance and
Copy/Clone/Debug/Send/Sync/unwind marker bounds on all declared target/profile variants. Fixture units separately
measure first-page retained workspace at pages 1/17/1024, nested exact bits, concurrent reads, malformed unused tails,
and observation release before mutation, END, callback failure and unwind cleanup.

For ownership changes, run the focused Miri tests before broader profile rebuilds. Select an already installed
nightly with Miri and rust-src; do not let a missing channel trigger installation. From the Cargo root, run
`cargo +<installed-nightly> miri test --offline --locked --features fixtures --lib <test-name>` for these tests:

- `nested_foreign_views_preserve_bits_and_union_the_actual_exposed_regions`
- `foreign_page_borrows_end_before_mutation_end_failure_and_unwind_close`
- `foreign_page_shapes_precede_cancellation_and_unread_tail_timestamps_stay_in_the_walker`

These interpret Rust implementations of the foreign callbacks. They qualify selected ownership paths, without
executing a C compiler's code or replacing actual native/foreign C/C++ caller qualification.

The [source-bound page qualification](borrowed-page-qualification.json) retains the meaningful memory failure,
current unit/Miri/caller cohorts and matched C measurements. First-page retained descriptor workspace falls from
576/4,608/147,456 bytes to zero at pages 1/17/1024; borrowed sample bytes remain 32/544/32,768. In the complete
synthetic sum fixture, a 1,024-sample page reduces workspace peak from 227,504 to 7,875 bytes in both profiles.
Longer histories of 8,192/65,536 points keep the same peak at a fixed page bound. Three repetitions preserve
complete sums, fetched counts, page counts and resource release. For 65,536 points with 1,024-sample pages, median
times change from 23.06 to 18.78 ms in release and 39.58 to 31.43 ms in release-min. These measurements describe
the fixture, not production throughput or RSS.

## Label encoding compatibility exception

Each materialized label name and value must contain fewer than **16,777,216 bytes**. The pinned
[Prometheus stringlabels encoder](https://github.com/prometheus/prometheus/blob/5241a27fe3c6983549fccc32f6e65917408c63cd/model/labels/labels_stringlabels.go#L543-L562)
accepts exactly that length but encodes it as zero. Depending on the bytes, original evaluation returns a slice
bounds error or malformed labels. The Rust core explicitly rejects that boundary instead of reproducing the
corruption. Original Go captures remain unchanged; this departure is an approved compatibility exception.

Provider fields fail with `StorageError::Invalid`; generated fields fail with the ordinary runtime message
`String too long to encode as label.` Larger fields retain that upstream message. The rule applies per actual
field: labelsets may exceed this total size, and matcher/string arguments do not acquire a separate size cap.
Memory, work, concurrency and duration enforcement remain disabled.

Validation follows materialization. COUNT checks its modified labels before grouping can remove the destination.
Label replacements check after a match generates labels; unmatched replacements and empty-value deletion remain
valid. Absence inference may provisionally Set then Del a large equality; only a produced absence sample is
validated, after child evaluation. Signature/hash encoding is fallible rather than panic-based.

The boundary tests exercise names and values immediately below, at and above the limit, generated fields,
pre-grouping COUNT errors, deletion/no-match controls and combined labelsets larger than the field boundary.
The real C/C++ embedding gate adds six provider cases and checks error classification, rejection before raw cursor
creation and paired releases. Allocation-accounting witnesses are documented above; complete expression gate
results and their finite evidence limits are recorded separately in the standalone overview.

## Compact group and join lookups

The later [copied grouping-label construction](#copied-grouping-label-construction) change reduces discarded
metadata copies while preserving these grouping identities and ordering controls.

Grouping, TOPK/BOTTOMK, both LIMIT operators, aggregate QUANTILE and vector matching use compact accounted indexes
over their existing metadata arrays. Indexes preserve each owner's original equality and first writer; they never
drive reductions or output order. Binary signatures/cache keys still compare complete encoded bytes, including
the source's undelimited concatenation aliases. Active membership, duplicate validation, reservations and output
uniqueness retain their original phases. Ordinary aggregation releases positions before reversing/removing groups.

Empty/singleton indexes have no slot allocation. Sparse slots grow at two-thirds load; active signature domains
use dense positions only when they fit the sparse replacement already required. Rebuilds charge both allocations
and check cancellation before publication. They contain positions rather than raw samples or copied payloads.
The later [four-byte slot qualification](#four-byte-position-slots) narrows ordinary payloads while retaining
full-size positions and these ownership rules.

Before/after fixtures at the accepted `038cda1169` source preserve complete answers and sample reads in both builds:

| Fixture | Counted work before → after | Workspace peak before → after |
| --- | ---: | ---: |
| SUM with 2,048 groups | 9,183,418 → 2,974,303 | 2,557,724 → 2,557,724 B |
| One-to-one join, 512 series | 10,481,432 → 722,941 | 1,280,472 → 1,301,320 B |
| Group-left/right, 512 series and one signature | 16,405,003 → 814,768 | 967,106 → 996,146 B |

The full panel compares 20 cardinality, 24 coupled-owner and 13 scheduled cases per build. Existing scheduled selector
page counts/peaks stay unchanged. Compiler layout checks retain Frame 240 B and Active 16 B; ordered Side grows 96→104 B,
cache entry 96→104 B and boxed binary Step 384→672 B. These are fixture work/accounted-memory measurements, not CPU
instruction counts, production throughput or whole-process RSS. Composite child replay and large metadata/cache
domains remain separate costs. Resource caps remain disabled.

Shared grouping changes also replay the complete retained TOPK/BOTTOMK (3,556), LIMIT (2,240) and aggregate
QUANTILE (3,558) panels at pages 1/17/1024 in both builds: 56,124 outcomes. Strict original reports retain
27, seven and two ordering differences respectively per build/page. Separate approved ordering controls pass;
the originals and their raw failures are unchanged. This is policy-qualified finite coverage, rather than a claim
that every original outcome matches literally.

Use the existing `topk_calls.py`, `limit_calls.py` and `quantile_aggregate_calls.py` helpers with explicit verified
reference directories and frozen candidate binaries. Preserve their complete strict candidate reports first;
apply the corresponding `topk_policy.py`, `limit_policy.py` and `quantile_policy.py` qualification
as separate reports using the original control captures. LIMIT qualification also binds the original ratio-sampler
capture. Inspect each helper's callable `qualify` interface: a known strict ordering failure must not prevent
the separate policy gate from running. Follow the conformance workflow's recorded process-owner route for these
helpers when process signals have not been approved. The current source, helper and binary bindings are recorded
under `lookup_index_qualification` in [core-qualification.json](core-qualification.json).

## Four-byte position slots

The current lookup stores ordinary encoded positions in four-byte slots. Positions remain full-size `usize`
values: a larger encoded position promotes the whole table to two words per slot on 64-bit targets. Empty and
singleton indexes allocate no slots. Full hashes, caller equality, first writers and owner traversal remain
unchanged; these tables contain metadata positions, never raw samples.

One tracked buffer and the existing state word keep the header budget unchanged. Width-only promotion preserves
physical probe/signature offsets without invoking rehash callbacks. Capacity growth retains the original callback
order. Both allocations remain accounted until successful replacement, including cancellation cleanup. A rare
width-only promotion can temporarily hold more bytes than an always-wide steady table; that overlap is measured.
Cold growth routines share borrowed callbacks; hot equality predicates remain specialized.

The [qualification report](compact-slots-qualification.json) retains source/binary/input identities and all costs.
The release fixture at page 17 records these whole-query observations:

| Query case | Workspace peak before → after | Workspace acquired before → after |
| --- | ---: | ---: |
| One-to-one join, 512 series | 859,008 → 846,720 B | 3,796,746 → 3,747,722 B |
| Group-left join, 512 series | 799,402 → 787,114 B | 3,341,714 → 3,300,834 B |
| Grouped abs, 4,096 series/three ticks | 6,020,640 → 6,020,640 B | 78,320,455 → 77,665,255 B |

Halving slot payload does not halve query RAM: labels, rosters and output can dominate. Acquired/released counters
describe tracked allocation traffic, separately from retained peak bytes. ARM32 slots are already four bytes;
its executed C query memory observations remain unchanged. No implicit resource limit is enabled.

Ten native index units include full-width singletons, the encoded 32-bit boundary, low-word-zero occupancy,
collisions, growth order, sparse/dense width-only promotion and interrupted replacement. Both native profiles
pass 736 units with two unchanged ignored tests. ARM32/ARM64 execute six/ten focused units. The query panel compares
5,456 complete outcomes over 682 retained cases, native pages 1/17/1024 and foreign page 17. Its 64 error cases per run
remain complete error comparisons; they intentionally do not expose allocation observations. Eight actual GNU C
callers linked against default-feature archives execute 672 query cases. These are separate from fixture-feature
executables and do not claim a new C++ or sanitizer panel.

Complete TOPK/BOTTOMK, LIMIT and aggregate QUANTILE interactions replay all 56,124 retained outcomes. Exact equality
to every authenticated signed-base report input permits reuse of its unchanged original strict and approved
ordering-policy analyses. Keep their source/input/output/report hashes and historical binary attribution, and
bind fresh current executions separately. The same 216 strict differences remain, including tie-dependent membership
and point-count differences; they are not literal Prometheus passes. Paired numerical-bit differences remain zero.

The matched timing protocol uses 26 existing owner queries per native profile, five alternating pairs, and one warm
plus one measured request per case/process. Empty/singleton/two-ID, 32/33 sort, ranking, grouping/join, mapped/output
identity and raw controls remain visible. Retain every slower primary median and repeat all at least 5% slower groups;
the report records both panels and all eight memory categories. Timings describe engine elapsed time on a shared
host, without a claim about live DBengine, concurrent throughput, RSS or physical ARM performance.

The 52 primary medians include 12 slower groups. Both groups at least 5% slower receive 20 measured samples per
side. The size-optimized one-group control remains slower in that repeat, from 28.8945 to 32.2045 microseconds;
the release singleton control changes direction. Both panels remain in the report without a causal explanation.

Sharing cold growth removes much of the preliminary code cost. The same-source release C caller's GNU size text
total changes by +44,676 B on native, +13,336 B on ARM64 and -54,612 B on ARM32 relative to the baseline. All profile
text/data/BSS/file costs are retained; GNU size text includes read-only data. Enclosing type diagnostics and query
heap observations do not establish whole-function stack or process RSS bounds.

From the crate directory, run the focused owned-index checks with either profile:

```sh
cargo test --locked --features fixtures --release position_index::tests::
cargo test --locked --features fixtures --profile release-min position_index::tests::
```

The broad panel reuses the retained complete grouping/join inputs, sort 32/33 controls and grouped Map/output-ID
inputs described by the report. Authenticated captures and detailed receipts stay in their recorded evidence
directories; the focused units above are self-contained source checks.

## Completed unique float join scans

Final evaluation can reuse a completed operand scan for a bare root one-to-one join between two raw float
selectors. It requires unambiguous signatures, hashes, output bindings and supported ordinary clocks. Eligible
grouped ranges use the [grouped extension](#completed-grouped-float-join-scans). Native operand values, filled,
unproven nested, delayed-name and other ambiguous shapes keep ordinary pair replay.

The first row completes the original full scan and pair evaluation, including comparison filtering, reservations
and duplicate checks. Its emitted cache ordinals become reusable only after successful completion and diagnostics.
Later rows read their own pair through the existing numeric/label path. An incomplete producer, failed callback,
cancelled query or partial replay publishes no replacement. Frozen first-writer cache labels remain unchanged.

The query keeps original pair positions and one current-scope bitmap, accounted as workspace. The key includes
effective query options, step index, exact lookback and data bounds. Scope changes release the old snapshot before
allocating its replacement. This avoids a raw value vector and a time-by-series cache. Ordinary root output is
step-major; the separate series-major subquery path remains ordinary replay.

Qualification includes complete changing-membership, false/boolean/arithmetic and reversed-operand-position
results, history growth with discard output, and actual C callbacks where a later operand failure must precede
a second-output failure. The closed C caller roster also checks selected-pair failure after reuse. Original physical
cursor reports remain separate from approved skipped-operation controls. Final source-bound measurements and
compatibility cohorts are recorded alongside their own binaries; earlier receipts retain their original identities.

In the matched `m / on(id) m` fixture, each metric has one point and the query has two evaluation timestamps.
Both release profiles and pages 1/17/1024 preserve every output and annotation across three interleaved repetitions:

| Selected metrics | Fetched entries before → after | Workspace peak before → after |
| --- | ---: | ---: |
| 16 | 2,208 → 348 | 47,301 → 47,317 B |
| 32 | 8,512 → 700 | 88,672 → 88,680 B |
| 64 | 33,408 → 1,404 | 173,600 → 173,608 B |

The snapshot adds accounted metadata; the fixture's peak is dominated by existing preparation. These figures
describe generated inputs, not production throughput or RSS. General computed joins and selector prefix traversal
remain separate costs.

The [join qualification](binary-replay-qualification.json) binds the exact engine sources, both optimized
profiles, original expression panels, actual C callers and separately rebuilt DBengine/API checks. It preserves
the earlier qualification cohorts and their original source identities.

## Completed grouped float join scans

Eligible `group_left` and `group_right` range roots use the same completed-pair owner as unique joins.
Full float row identities remain unique on both sides; matching signatures need be unique only on the declared
one side. Each many-side row resolves its one-side counterpart and binds an existing frozen cache ordinal.
Missing required mappings, conflicting ordinal assignments, merged outputs or ambiguous one-side histories
reject certification. Existing direct selector, rate and earned float-map clock proofs remain authoritative.

Pair positions retain original left/right order. Frozen keys follow evaluation's many-side/one-side label order,
which is right/left for `group_right`; numeric operands still use their original expression order. First-writer
labels and exact cache bytes remain unchanged. The cold pass still drains both operands and checks reservations,
duplicates, diagnostics and control before publishing membership. Later rows read their selected operands afresh.
Only pair positions and the existing single-clock bitmap survive. No raw values, history array, extra cache,
provider contract, ABI, resource limit or dependency is introduced.

The [grouped qualification](grouped-pair-replay-qualification.json) binds:

- The actual old runtime completes 18 result/label/clock/bit/release checks before failing the growth bound.
  The candidate passes at pages 1/17/1024 with noncommutative division, reversed physical inputs and fresh values.
- The permanent `grouped_join_cases.py` generator adds 264 fresh pinned originals. Arithmetic/comparisons, both
  directions, `on`/`ignoring`, include labels, missing/changing membership, one-side ambiguity/history, byte labels,
  false first writers, mapped/rate operands and native/mixed fallbacks have complete outcome comparisons.
- Both native profiles pass 647 library tests each and 1,692 complete original comparisons across three page sizes.
  The before runtime also passes all 1,692 comparisons; this panel isolates a performance change.
- Compiled ARM64/ARM32 execution passes 1,128 original comparisons and 20 focused unit executions across both
  profiles. Twelve actual C/C++ clients each pass 18 normal/provider-failure arms across both grouped directions
  and three page sizes. The short mode is `--grouped-only`; complete foreign suites and the full C roster are
  not claimed rerun. Format, strict production/fixture lint and installed Rust 1.91 compile-only checks pass.

Of 1,692 native resource groups, 282 fetch fewer entries and acquire less workspace; no group fetches more.
Six groups of one mapped expression add 81 B to workspace peak. The remaining 1,686 peaks are unchanged, as are
every other exported memory kind's peaks. Thirty-six native/mixed/history fallback groups add 6–39 work units
and at most 112 cumulative workspace bytes. Output counts and current memory observations remain exact.

For either grouped direction, page 17 and three ticks, both profiles measure:

| Many-side series | Fetched entries before → after | Work before → after | Workspace acquired before → after | Peak workspace, unchanged |
| --- | ---: | ---: | ---: | ---: |
| 16 | 7,548 → 1,203 | 448,049 → 85,910 | 2,160,232 → 441,761 B | 39,374 B |
| 32 | 28,908 → 2,403 | 1,714,916 → 174,366 | 8,112,600 → 868,577 B | 71,079 B |
| 64 | 113,100 → 4,803 | 6,716,647 → 354,933 | 31,449,784 → 1,722,209 B | 136,839 B |

Acquired bytes are allocation traffic rather than concurrent RAM. Raw samples remain paged; cardinality metadata
and independently owned collected results still grow with series count. Native query-fixture ELF text changes
by −816 B in release and +896 B in release-min; these sections do not measure installed Agent footprint.

The frozen timing panel checks all 400 complete warm/measured responses across five alternating pairs per profile.
Each response must preserve its expected result and that executable's own nonelapsed observations. All median
groups appear below; ratios divide before time by candidate time, so values below one are slower.

| Case, page 17 | Release before → after, ms | Ratio | Release-min before → after, ms | Ratio |
| --- | ---: | ---: | ---: | ---: |
| `group_left`, 16 series | 6.653 → 1.267 | 5.250× | 11.033 → 2.155 | 5.120× |
| `group_left`, 64 series | 97.001 → 4.935 | 19.655× | 162.646 → 8.578 | 18.961× |
| `group_right`, 16 series | 6.563 → 1.261 | 5.204× | 11.186 → 2.155 | 5.191× |
| `group_right`, 64 series | 95.950 → 4.895 | 19.602× | 162.865 → 8.513 | 19.132× |
| `group_left`, singleton | 0.099 → 0.097 | 1.015× | 0.157 → 0.152 | 1.030× |
| `group_right`, singleton | 0.101 → 0.108 | 0.941× | 0.156 → 0.157 | 0.988× |
| Unique join, left/right | 5.978 → 5.919 | 1.010× | 10.219 → 10.142 | 1.008× |
| Unique join, right/left | 6.158 → 5.977 | 1.030× | 10.235 → 10.162 | 1.007× |
| Raw selector, 64 series | 0.536 → 0.518 | 1.035× | 0.881 → 0.870 | 1.013× |
| Empty grouped query | 0.014 → 0.014 | 1.016× | 0.017 → 0.017 | 1.028× |

Both slower singleton medians remain in the report. Shared-host measurements have no CPU/frequency isolation and
do not establish the cause of small timing changes. These are synthetic engine execution times, including
preparation, rather than HTTP throughput, production DBengine disk I/O or RSS. Historical broader language/SRE,
live Agent and CI evidence retains its actual source and binary identities; no new universal compatibility claim follows.

## Completed instant numeric folds

A true-instant root numeric range function can reuse its completed preparation result during final output.
It requires a direct raw matrix selector, productive root output and matching preparation/evaluator headers.
The existing exact key still checks effective clocks, parameter bits, precise lookback, duration, geometry and
input bounds. Non-instant, composite and subquery requests retain their ordinary phase-specific cache lifecycle.

The query retains one float-or-absent summary per physical input series in its existing accounted fold records.
Stable compaction releases unrelated snapshots without another allocation. An all-absent root clears everything.
When another series is productive, final traversal also visits absent physical series, so their small summaries
avoid repeated scans. No raw value vector, histogram payload or history is retained.

Final series initialization and scalar-argument evaluation remain before the lookup. Labels, warnings and logical
queryable samples retain their existing path. Errors during complete preparation or required final initialization
still propagate; omitted redundant reads follow the existing provider policy. Temporary quantile backing closes
before the completed snapshot is retained, and no new backing requirement is introduced.

The 4,096-point monotone quantile control returns exactly 2,048.5 before and after the change. Its dev-unit reads
fall 278,598→139,300 at page1, 279,718→139,876 at page17 and 350,208→176,128 at page1024. The generated provider
keeps the same 11,010 B tracked workspace peak and 4,096 logical queryable samples. Histories 2,048/8,192 keep equal
raw/provider/workspace peaks and one cursor. These are finite generated observations, not production throughput or RSS.

Final matched measurements cover 40 groups across both optimized profiles, three page sizes and three interleaved
repetitions. All tracked workspace peaks are unchanged. Page17 quantile medians improve about 1.95–2.08× for
one/four/16 series; the mixed zero/NaN backing case creates one completed store instead of two.

The [instant-fold qualification](instant-fold-qualification.json) binds final optimized profiles, original
expression panels, actual C caller targets and separately rebuilt Agent checks. Earlier receipts keep their actual
source identities. Numeric summaries can still grow with selected-series cardinality; raw paging remains bounded.

## Completed direct rate join scans

The initial cohort below qualifies bare-root one-to-one float joins when an operand is an ordinary direct
`rate(raw_selector[range])`. Raw-selector eligibility stays the same. The rate producer/source/scope grids must
match the effective evaluator, and the range/offset arithmetic must produce nonwrapping window endpoints.
Grouped and wrapped operands were outside that initial cohort; their later qualifications are recorded separately.
Filled, delayed-name, invariant, extended and unproven computed shapes retain ordinary replay.

The existing physical-ordinal bindings and accounted one-step membership bitmap are reused after the complete
first final pass succeeds. No new value array, raw history or backing requirement is introduced. Native values
discard a candidate. Selected-pair reads, exact labels, diagnostics, numerical folds and logical sample statistics
keep their existing owners. Physical selection and extrapolation retain their distinct nanosecond truncation.

The fixed workload is `rate(m_total[5m]) / on(id) rate(n_total[5m])`, with two counter samples per series and two
evaluation timestamps. Both optimized profiles, pages 1/17/1024 and both right-side physical orders preserve every
output across three repetitions with alternating before/after order. At page17 with reversed right-side order:

| Pairs | Recorded work before → after | Workspace peak before → after |
| --- | ---: | ---: |
| 16 | 217,501 → 49,699 | 72,187 → 72,187 B |
| 32 | 836,580 → 106,155 | 129,576 → 129,576 B |
| 64 | 3,302,052 → 230,677 | 246,568 → 246,568 B |

Fetched samples remain unchanged in all 36 matched groups. Page17 medians improve about 2.57/4.42/8.00× in release
and 2.74/4.73/8.74× in release-min. These are generated query timings and accounted query workspace, not production
disk throughput or process RSS. Preparation and ordinary selector prefix work remain independent costs.

Thirty changing-membership/comparison/boolean/mixed-operand controls preserve complete results. Each actual C
caller also checks 12 rate cases across three page sizes: normal output, a provider error on a later series in the
first final pass before the first output error, cancellation during first output and an opaque output failure.
The normal run records its last required cursor before first output; the error case checks physical series 3.
Native call traces verify final evaluation, with no preparation frames. Output begin precedes preparation,
so a cursor constant armed there cannot establish this phase. All cases retain transactional abort/commit and
resource release. The original join and numeric-fold controls remain in the roster.

The [rate-join qualification](rate-join-qualification.json) binds the final engine, optimized profiles, expression
panels, C callers and rebuilt Agent checks. Earlier cohorts retain their own source and binary identities.

## Compact one-to-one result labels

One-to-one result projection with no included fields copies only retained labels. The existing vector therefore
grows with output fields rather than keeping capacity for discarded inputs. The shared checked field-copy body
keeps exact byte capacities, source order and partial-owner cleanup. Grouped and nonempty-include projections,
complete cache keys, first-writer labels, public types and output copies keep their existing behavior.

Empty values disappear before the original reserved-name preflight. A remaining empty name still reports the
same wrapped error before `on`/`ignoring` can remove it. The three reserved metadata fields and conditional name
removal retain their existing predicates. Controls cover reset/error order, matching, group/include overrides,
opaque values, cancellation during a retained field, actual derived empty names and discard-output statistics.

With one retained field, 8/17/65 input labels formerly retained 777/3,081/12,297 bytes. They now retain 393 bytes
each, while complete output and release checks pass. The whole-query workload has 17 input labels, one point per
series and two timestamps for `m / on(instance) m`. Both profiles, pages 1/17/1024 and three alternating matched
repetitions preserve complete results and fetched samples:

| Series | Workspace peak before → after | Fetched samples before → after |
| --- | ---: | ---: |
| 32 | 405,937 → 319,921 B | 700 → 700 |
| 128 | 1,596,468 → 1,252,404 B | 2,812 → 2,812 |
| 512 | 6,385,716 → 5,009,460 B | 11,260 → 11,260 |

All 18 matched groups use roughly 21% less workspace. Recorded work also falls, but short-query timings are mixed;
this cohort establishes a memory improvement, not a throughput gain. These are generated query/workspace
observations, not production disk throughput or process RSS. Input metadata, complete keys and collected output
remain separate costs that grow with cardinality. Contexts/provider ownership and raw paging are unchanged.

The [projected-label qualification](projected-label-qualification.json) binds the final source/profile/caller and
rebuilt Agent cohorts. Earlier qualifications keep their actual source and binary identities.

## Single-series range head preparation

Direct raw range-function preparation skips redundant step-kind collection and repeated head-summary scans.
One physical series produces zero or one value of one kind per tick, which cannot change those summaries.
The generic path still records multiple outputs and mixed kinds. Descriptor kinds across ticks, evaluation,
parameters, initialization, hashes, exact clocks, diagnostics, statistics and publication remain unchanged.
Meaningful remaining work and final execution still observe cancellation/deadlines.

The work regression checks every label, eight timestamps, exact values, empty diagnostics, `2NT` logical samples
and complete release before applying its work budget. Retained controls exercise native/mixed values, generic
heads, scalar parameters, initialization failures, precise clocks, disjoint aliases and cancellation.

The matched workload is `avg_over_time(counter_total[5m])`, two points valued 1 and 2 at seconds 1 and 2, and
eight query steps from seconds 2 through 9. Both profiles, pages 1/17/1024 and three alternating repetitions
preserve complete results and fetched samples. At page 17:

| Series | Work before → after | Fetched samples before → after | Median timing ratio, release / release-min |
| --- | ---: | ---: | ---: |
| 128 | 594,713 → 461,593 | 16,384 → 16,384 | 1.11× / 1.11× |
| 512 | 4,877,299 → 2,771,955 | 65,536 → 65,536 | 1.39× / 1.41× |

All 12 matched groups reduce work. Peak workspace falls by 96 bytes; no raw/value array or index is added.
The timings describe synthetic query execution, not production API or disk throughput. Workspace is tracked
query memory, not RSS. Other uniqueness and final replay costs remain; total preparation is still nonlinear,
and raw reads do not fall in this cohort.

The [range-head qualification](range-head-qualification.json) binds the final engine, callers and rebuilt Agent
checks. Earlier qualifications retain their actual source and binary identities.

## Range preparation label uniqueness

Ordinary range-function preparation uses the existing accounted position index instead of scanning every earlier
result hash. The check still follows complete preparation and uses full-hash equality, including unequal labelsets
with equal hashes. Current label hashes are recomputed in their original order. Delayed cleanup, kind-aware
streaming checks, metadata-preserving functions and generic merge rules retain their existing owners.

The collection index is no longer needed at this phase and is released before the new lookup table is allocated.
Neither index retains raw samples. Cancellation is checked after lookup before duplicate errors/insertion, and
after insertion. Actual collision controls cover inline and allocated tables, float/native results, later provider
failures and cancellation at the matching probe; every failed query aborts output and releases its owners.

The matched workload is an instant `avg_over_time(counter_total[5m])` at second 2, using two samples valued 1 and 2
at seconds 1 and 2. The work regression verifies every label, timestamp, exact 1.5 value, empty diagnostics,
`2N` logical samples and complete release before its work bound. Both profiles, pages 1/17/1024 and three
alternating repetitions preserve complete outputs and fetched samples. At page 17:

| Series | Work before → after | Peak workspace before → after | Median timing ratio, release / release-min |
| --- | ---: | ---: | ---: |
| 128 | 73,459 → 66,821 | 161,149 → 161,149 bytes | 1.02× / 1.06× |
| 512 | 497,373 → 373,098 | 632,552 → 630,504 bytes | 1.11× / 1.13× |

All 12 matched groups reduce work and preserve reads; workspace is unchanged at 128 series and falls by 2 KiB
at 512. The table itself can exceed the old hash vector's size; the measured benefit includes releasing finished
collection metadata. Timings describe generated query execution, not production API or disk throughput. Workspace
is tracked query memory, not RSS. Collisions can still require repeated probes, and other preparation/replay costs
remain; this does not establish linear total execution.

The [range uniqueness qualification](range-uniqueness-qualification.json) binds 555 units per profile, retained
expression/SRE panels, actual C callers and a rebuilt Agent's storage/API checks. Earlier ledger entries keep their
actual source and binary identities.

## Mapped label identity lookup

Mapped evaluation and final delayed-name duplicate cleanup use one private `UniqueKinds` owner. It keeps the original
accepted `(full label hash, kind)` entries and lazily creates an accounted position index on the second distinct
accepted identity. Empty and singleton paths allocate no index. The native evaluator frame remains 240 bytes.
Index slots and an 80-byte native boxed owner are added metadata costs on plural paths.

Equality checks the complete 64-bit hash and exact kind, including on ARM32. Kind zero is a valid distinct kind.
Float and histogram identities with the same hash can coexist where the original caller permits them; another of
the same kind is a duplicate. Unequal labelsets with equal full hashes keep the original hash-only behavior.
The index does not determine encounter order, reducer arithmetic, output IDs or labels.

The check stays after the original operation, diagnostic selection, omission, value reconciliation and name removal.
A terminal control poll follows lookup before a duplicate error or insertion. Checked tuple growth and index
rebuilding account overlapping allocations; errors and cancellation abort the query and release all owners.
The final cleanup guard and per-tick lifetime remain unchanged. No raw samples or computed values are stored here.

Qualification procedure:

- Freeze the actual baseline and candidate inputs and binaries. Compare complete labels, values, timestamps,
  membership, diagnostics and errors before any work or timing comparison. Keep both option profiles.
- Run the `mapped_identity` causal units. They cover zero/singleton allocations, promotion after a distinct second
  identity, kinds zero/one/two, full-width keys, real unequal-label/full-hash collisions, terminal cancellation at the
  decisive comparison, interrupted growth, preserved prior keys and complete ownership release.
- The actual expression growth check uses `abs(sum by(instance,job)(counter_total))` at three instants. Complete
  results in both name modes precede its immediate-name scaling bound. Direct `abs(counter_total)` is a useful
  control but already uses per-series preparation; it does not reproduce the plural grouped scan.
- Run `identity_cases()` beside retained mapped/contributor/bridge expressions. Its complete original outcomes cover
  duplicate and disjoint aliases, native/mixed values, omissions, bool/scalar-left maps, nested consumers, subqueries,
  invariant clocks, NaNs, infinities, signed zeros and both name-removal modes at instant/range requests.
- Use pages 1, 17 and 1024 for native profiles, actual default-feature C callbacks and explicit foreign scope. Execute
  the full-hash/kind and interruption owners on ARM32/64; native-only success does not establish pointer-width behavior.
- Measure fixed alternating ordinary queries after qualification writers close. Retain every primary timing, slower
  case, all eight memory categories, logical/useful/owned sample statistics, frame layout and linked binary costs.
  Keep any declared follow-up separate from the original primary results.

An initial absolute work-per-input test budget did not separate ordinary grouping and sorting costs. The retained
complete outcomes instead support a scaling check on the immediate-name grouped path and a separate bound on the
compound identity owner. At that checkpoint, delayed-name execution retained a separate hash-to-output-ID merge scan;
the following section qualifies its replacement. Neither change establishes linear total execution. Synthetic
timings do not establish live DBengine, RSS, maximum
or concurrent throughput, or arbitrary-input performance. Index metadata can increase query peak memory.

The closed qualification has 719 passing units and two existing ignored tests per native profile, 21,792 complete
original expression comparisons and 672 actual C queries. Another 192 native C queries use the signed predecessor.
Native profiles use pages 1/17/1024; ARM32/64 release uses page 17 under QEMU and executes four focused identity
owners per target. The new 512-request corpus has 185 nonempty vector/matrix outcomes, 13 scalar values, 154 empty
results and 160 expected errors. These are expression outcomes, not new function counts or branch-coverage proof.

The fixed timing panel has 60 cases per profile, six alternating pairs, one warmup pair and five measured samples
per side: 1,440 complete outcomes across 120 groups. It includes the motivating large filter, grouped raw rows,
empty/singleton/two-row maps and retained raw/rate/histogram/quantile controls.

| Profile | Query shape | Median before → after | Time reduction |
| --- | --- | ---: | ---: |
| release | 512 grouped raw rows | 26.470 → 24.939 ms | 5.8% |
| release | 7,168 filtered groups | 2172.109 → 1924.593 ms | 11.4% |
| release-min | 512 grouped raw rows | 44.850 → 41.621 ms | 7.2% |
| release-min | 7,168 filtered groups | 3650.553 → 2821.303 ms | 22.7% |

All 55 slower primary medians remain in the report. The rule declared before execution repeats every primary case
at least 5% slower with 20 more samples per side. Eight groups produce 320 further complete outcomes; the largest
remaining slowdown is 2.7% on the release raw-query control. Primary measurements are not replaced. These shared-host
observations do not prove that every input is faster or that any remaining difference has one identified CPU cause.

At 7,168 filtered groups, work falls from 165,531,458 to 88,901,456. Measured current and peak memory remain equal
in all eight categories across all 120 timing groups. Workspace acquired/released bytes increase by 0–786,576 per
query in 26 groups; every field in the other seven categories remains equal. Actual paired C logical/useful/owned-peak
sample statistics remain equal. Extra metadata is real even when another phase determines the whole-query peak.

Native C text changes by −10,648/+6,284 bytes in release/min; fixture text changes by +16,996/+4,964 bytes. Full
file/data/BSS costs and actual candidate layouts, including ARM32 Frame 208 B and index box 56 B, remain in the
[mapped identity qualification](mapped-identity-qualification.json). Format, strict default/fixture Clippy and installed
Rust 1.91 all-target compilation pass; this is compile-only MSRV evidence. Earlier ledger objects retain their
actual source and binary attribution.

## Final output identity lookup

Generic final output assigns IDs by first productive emission across the whole query. The existing gate covers delayed
name removal and an actual root unary minus; certified schedules, instant cohorts, raw/derived matrices and other
generic roots retain their own output paths. Canonical roster order can differ from first productive order.

`OutputIdentities` stores append-only full hashes and an inline optional position index, initially `None`. A hash's
position already is its ID, so the old separate ID field is removed. Equality compares all 64 bits on every target and
ignores value kind. Unequal label bytes with the same full hash keep the original merging behavior and first callback
labels. IDs survive missing ticks. This owner is query-wide; per-tick kind-sensitive duplicate validation remains
separate.

Up to two identities use a bounded full-hash scan. When a third distinct identity appears, build a local index
containing both prior identities and the candidate. Publish it only after every insertion succeeds, then append the
reserved candidate hash. Interruption releases the local index and leaves the two prior IDs usable. Larger sets retain
the existing indexed insertion and rebuilding.

Create the owner only when generic merging is needed and scheduled output has declined. Keep the original immutable gate
as the per-output dispatch condition before borrowing the owner. Preserve evaluation, final name cleanup, duplicate
validation and checked label views before lookup. Checked key reservation precedes index mutation; a successful
insertion immediately appends the reserved hash. As in the original decisive comparison, there is no terminal lookup
poll; the following checked output observes cancellation. Failure releases all owners and aborts provisional output. No
labels, sample values, raw histories or second provider are retained here.

The compact key is 8 bytes; the old hash/ID tuple is 16 bytes on all measured targets. Zero-, one- and two-identity
queries allocate no slots. Larger indexes add real metadata and copy/rebuild traffic. The inline owner is 104 bytes on
native/ARM64 and 72 bytes on ARM32, once per generic query. These type sizes are not a whole-function stack delta; the
native evaluator frame remains 240 bytes.

Qualification procedure:

- Freeze actual predecessor/current source and binary identities. Compare complete labels, IDs, values, clocks,
  membership, diagnostics and errors before measuring work, bytes or time.
- Run `execution::lookup_tests::output_identity`. Its six causal units cover complete grouped outcomes before growth
  bounds, full-width/zero/max hashes, first writer, slot-free two-identity ownership, interrupted first-index
  construction/copy/rebuilding, late generic callback IDs, real full-hash aliases after plural growth and error/cancel
  abort followed by fresh reuse.
- The actual growth expression is `-sum by(instance,job)(counter_total)`, with 64/256/1024/4096 rows at three ticks.
  Both name modes finish all complete-result checks before the fourfold-input/sixfold-work guard. Retain the old failing
  source/output and unchanged passing test; a direct owner bound does not replace whole-query checks.
- Run `output_identity_cases()` beside retained `identity_cases()` and `mapped_cases()`. The new 384 cases contain 268
  nonempty results, 52 empty results and 64 construction errors. Its aggregate-postfix `@`/`offset` forms are invalid
  and do not establish positive temporal execution; retained valid clock controls are separate.
- Add the 144 declared cardinality cases for zero/singleton/two-row and 128/1024/4096-row raw, grouped map, unary,
  parenthesized unary, filter and rate forms. Keep both option profiles and instant/range requests.
- Build `output_identity.c` against each actual default-feature archive and its reported native link libraries. It
  reuses `c_embedding.c` for numeric IDs/labels/clocks/values and calibrated success, provider, cancellation and
  output-error ownership. Check helper summaries separately from the 96 queries per build.
- Retain `histogram_contributors.c`: its delayed-name `histogram_quantiles` cases emit four distinct float outputs
  through generic merging, exercising third-identity index construction in the actual C adapter. Both windows,
  instant/range requests and pages 1/17/1024 give 12 such cases per build, within its existing 72 queries.
- Use pages 1/17/1024 in both native profiles, ARM32/64 release page 17 under QEMU and the six focused units on each
  foreign target. Keep native histogram C consumers and strict static/MSRV compile scope explicit.
- Time fixed alternating ordinary queries after qualification writers close. Include zero/singleton/two-row and bypass
  controls, keep every slower primary row and follow each at least 5% slower with 20 additional samples per side. Retain
  all eight memory categories, sample statistics, layouts and linked binary costs.

Closed qualification has 725 passing units and two existing ignored tests per native profile, 13,312 complete original
expression comparisons and 672 current C queries. Another 192 native C queries use the signed predecessor. Sixteen
additional final ARM resource outcomes compare against sixteen preserved predecessor outcomes; their timing is not
analyzed. Format, strict default/fixture Clippy and installed Rust 1.91 all-target compilation pass. MSRV is
compile-only, foreign execution uses QEMU, and no full foreign suite, live Agent, RSS or concurrent throughput is
claimed. Finite expression outcomes are not function counts, branch coverage or arbitrary-input proof.

The fixed final timing panel has 52 queries per profile, six alternating pairs, one warmup pair and five measured
samples per side: 1,248 complete outcomes across 104 groups. Zero/singleton/two-row, raw, parenthesized-unary, filter,
rate and ordinary-map bypass controls remain alongside both name-removal profiles.

| Profile | Query shape | Median before → after | Time reduction |
| --- | --- | ---: | ---: |
| release | 4,096 groups, root unary minus | 305.625 → 169.505 ms | 44.5% |
| release | 4,096 groups, delayed-name map | 360.047 → 220.812 ms | 38.7% |
| release-min | 4,096 groups, root unary minus | 483.301 → 278.952 ms | 42.3% |
| release-min | 4,096 groups, delayed-name map | 567.346 → 378.516 ms | 33.3% |

All 29 slower final primary medians remain in the [output identity qualification](output-identity-qualification.json).
The predeclared ≥5% rule selects 8 groups for 320 further complete outcomes and 20 additional samples per side. The
largest follow-up ratio is 1.1303 on `output-n0:server-default:instant:0` (release): 9.630 → 10.885 µs. Primary
observations are retained. These shared-host timings do not establish that every input is faster or identify one CPU
cause.

The report preserves four actual source variants. The initial mutable-Option dispatch slowed ordinary bypass controls;
restoring the immutable gate was measured in a separate 288-outcome release comparison. A bounded two-hash scan then
removed tiny index allocation, checked in a 1,008-outcome comparison across both profiles. Its unused startup index
motivated the final initially-None inline owner, checked in another fixed 1,008-outcome comparison. Each comparison uses
all six process orders and 24 samples per case/side. Earlier sources, binaries, slowdowns and incomplete qualification
scopes remain distinct; final full qualification and primary timings use the separately rebuilt final implementation.
Repeated outcomes are not additional language coverage.

All eight measured current and peak memory categories remain equal across the 104 final timing groups. Workspace
acquired/released bytes change by -32 to +65,536 per query. Every category, sample statistic, actual foreign resource
observation, type layout and linked cost is retained. Index metadata and temporary copy/rebuild ownership are real costs
even when another phase determines query peak.

Native C text changes by +6,556/+3,868 bytes in release/min; fixture text changes by +6,032/+3,360 bytes. Full
file/data/BSS costs remain in the qualification. Raw histories stay paged across metrics and time. These are serial
synthetic engine observations; no RAM/RSS bound, enabled cap, live HTTP/dbengine, concurrent or arbitrary-input
throughput claim is introduced.
## Bounded binary matching and pair replay

Vector/vector arithmetic, comparisons, `bool`, `and`, `or`, `unless`, `on`, `ignoring`, group modifiers and optional
fill modifiers use the pinned typed pair semantics. Native histogram addition/subtraction, multiplication/division,
equality/inequality and upper/lower trimming share checked kernels with scalar/vector maps. Native equality uses
original layouts and reset hints; trimming compacts only at the source's conditional trim boundary. Long equality,
trim, arithmetic, signature and label walks count work and observe cancellation.

Whole LHS preparation finishes before whole RHS preparation. A join retains productive child row descriptors,
exact raw matching-signature identities and its logical-scope label cache. Each logical step discovers active row
identities and sample kinds, discards inspected values, then replays the current operands through the explicit
frame stack. It does not retain raw timelines, an instant payload vector or a time-by-series presence bitmap.
Nested joins may retain an operand in each active frame; histogram arithmetic also has its checked temporaries.

Matching always uses complete active inputs before requested output filtering. Valid false comparisons reserve
cardinality and populate the label cache; incompatible or failed histogram pairs do neither. Group-right swaps
control sides while preserving original value operand direction and configured fill fields. Set operations retain
source child order and values. Last nonnil pair diagnostics, cardinality-before-output-duplicate precedence and
float-before-histogram vector head blocking preserve their source boundaries. Approved label-byte ordering applies
only at actual unspecified map assembly boundaries.

The result-label cache deliberately uses the source's undelimited concatenation of both raw label streams, with
first-writer projections retained across the entire logical scope, including false comparisons. Preparation may
insert; physical replay only reads the frozen cache. Resetting that cache per timestamp or adding side separators
changes observable range results. Exact matching signatures retain empty input values; normalized label builders
remove empty fields only at source builder boundaries. Arithmetic includes may restore metadata labels, while
delayed `bool` cleanup still removes them. Set outputs preserve inherited name-removal flags.

Accounted metadata and distinct combined-label cache keys can grow substantially, including quadratic cache growth;
replay work can multiply through nested operators. Streaming output avoids the convenience collector's retained
output matrix, but is not a total-memory bound. Admission limits remain disabled. Finite paging/memory checks do
not establish RSS limits, throughput, arbitrary provider iterator equivalence or complete language support.

The following binary checkpoint retains its original evidence counts and formerly open owners. Its implementation
status is superseded by the current expression, numerical, C and SRE gates above.

The fixture transport optionally accepts synthetic `faults` on dataset requests. It exposes terminal selection
errors and opaque warnings plus per-selector/series raw read faults. Healthy page prefixes precede a failing
landing, so page size does not move the tested failure ahead of already available values. This is a finite test
adapter over immutable input. The shared instantaneous replay panel above separately qualifies initial reads,
Seek landing, lookahead and deterministic operation-fault precedence. Consequential native NaN architecture
behavior, remaining functions/subqueries, C/runtime accounting and SRE expression acceptance remain open.

This checkpoint passes 193 unit tests in both release profiles and Rust 1.91, plus focused Miri checks without
ignoring leaks. At pages 1/17/1024, both profiles pass 284 independently captured binary cases, 28 original fault
cases, four empty-fault-field controls and 80 retained actual-full-hash cases. Permanent tests cover chronological
cache ownership, history-independent raw workspace, current-pair histogram storage, a small caller stack,
late-child failure precedence, sink/cancellation cleanup and preprocessed aggregation diagnostic positions.

The original 21,442-case expression gate completes and passes 17,330 cases; 4,112 remain OPEN. Both profiles now have
zero paired numeric bit differences in that gate. Separate retained panels preserve 20 previously attributed map-order
disagreements and expose one newly executable label-merge disagreement: unchanged Go repeated that original query
128 times with two metadata-label outcomes. Sorting the child OR map's complete labels under approved 9.A selects
the metadata-removing outcome. The original expectation and disagreement remain recorded; a separate policy check
qualifies the deterministic result. These figures do not establish complete language or alert compatibility.

### Qualified AMD64 arithmetic order

NaN payloads can change native histogram equality even when the numerical tolerance accepts both intermediates.
The qualified AMD64 path therefore preserves the actual pinned Go instruction destinations in compensated summation,
grouped SUM/AVG finalization, regression/prediction, native histogram scaling and histogram variance. The second
variance multiplication retains the original delta; its first product is still evaluated before the second to
preserve finite overflow behavior. The shared helpers use constant local state and retain existing paging and polling.
The approved numerical contract uses the retained Go AMD64, FMA-on behavior on every device. Native Go
ARM/ARM64 captures are diagnostic comparisons; they do not define a different engine policy.

Both profiles and Rust 1.91 pass 239 unit tests. Independent source panels qualify 2,662 compensated additions and
subtractions, 121 earlier summation cases, 242 native scaling cases, 220 complete aggregation/regression expressions
and 96 complete variance/scaling/equality expressions. Complete expressions match exactly at pages 1/17/1024; the
1,270 histogram-statistics helper cases also match all bits. These finite owners do not establish arbitrary numerical
or complete language compatibility.

The unchanged retained-output stability audit still reports 225 deliberate arithmetic response changes across
127,608 records. Source adjudication separates those historical Rust pins from Go correctness: 219 responses match
Go exactly, while six response instances differ only at an independently observed unspecified map boundary.
Approved label-byte ordering supplies the deterministic result there. All 120,072 accounting snapshots validate.
Original captures, expectations and failed stability reports remain preserved.

### Canonical numerical operations and semantic integer widths

The Rust core makes NaN operand priority, signaling quieting, generated-invalid sign and unary sign inversion
explicit. Addition/subtraction/multiplication/division preserve the pinned AMD64 destination, including a sole
NaN operand; finite operations retain their original order. Unary negation flips the raw sign bit. Pointwise
functions preserve each source function's separate raw/quiet/canonical NaN behavior. Logarithmic and angular
conversions retain their multiplication and addition/division steps; they are not algebraically combined.

Histogram mutation, resolution reduction, running counts, fraction interpolation and trimming use those operations.
Trimming retains source-specific positive/negative/zero accumulation order and both infinity boundary branches.
Vector/range averages, variance, regression, smoothing, ordinary/extended/instant rates and histogram averages
preserve their source operand destinations and reset/start-time guards. Go's semantic `int` is signed 64-bit for
exponents, empty-bucket compaction and custom cumulative arithmetic; decimal shifts use the AMD64 60-bit chunk.
Schema/span/bucket-index fields explicitly declared `int32` remain 32-bit. Physical Rust slice indices retain their
native width and checked physical bounds.

`portable_arithmetic.py` captures untouched pinned Prometheus and Go math/decimal/compiler owners offline.
The source-only inputs cover 2,720 native/width kernel cases, 8,710 fraction/trim cases, 7,690 complete expressions
and 94 float conversions. Native helper snapshots explicitly normalize nil bucket/custom payloads to empty lists;
the trim and complete-query gates retain their stronger packet contracts. Direct `FunctionCalls` observations are
preserved separately from complete-query diagnostic outcomes. Copy followed by compaction uses only the candidate's
own intermediate histogram. Whole queries compare every result field and value bit, without importing Go-computed
parent values. The capture requires the actual original-engine `0.5^1.5` result `3fd6a09e667f3bcd`, certifying the
retained FMA-on route on that capture host; a version/environment declaration alone is insufficient.

Eight decimal controls force the default source fallback at `internal/strconv/atof.go:647–686`: the exact decimal
`1 + 2^-53`, a positive tail beyond the 800-digit retained buffer, a value just below that tie, and a zero-padded
tie, with both signs. Source observations prove truncation and different rounding for the guard's adjacent
mantissas. The positive results are respectively `3ff0000000000000`, `3ff0000000000001`, `3ff0000000000000`,
`3ff0000000000000`; negative controls retain the sign bit. The public gate compares actual default `ParseFloat`
outcomes, including range-error classification.

```sh
python3 tests/promql-conformance/portable_arithmetic.py --capture \
  --upstream "${PROMQL_GO_MOD_CACHE:?}/github.com/prometheus/prometheus@v0.315.0"
python3 tests/promql-conformance/portable_arithmetic.py \
  --reference "${PROMQL_PORTABLE_CAPTURE_DIR:?}" --kernel "${PROMQL_KERNEL_BINARY:?}" \
  --query "${PROMQL_QUERY_BINARY:?}" --frontend "${PROMQL_FRONTEND_BINARY:?}" --page-size 17
```

Capture and comparison bind the original module/runtime sources, source inputs, probe and binary hashes, actual
runtime route and roster. Retained native release checks pass all 19,214 candidate cases. This qualifies those
owners on the tested AMD64 host; it does not establish execution on foreign Rust targets, arbitrary numerical
behavior or completion of the expression/SRE gates. Raw paging, cooperative controls and disabled admission
limits retain their existing contracts.

`logical_bucket_calls.py` declares 96 complete queries for receiving histogram addresses above signed 32-bit
and unsigned 32-bit boundaries. Each input contains only two or three spans and one or two stored buckets.
The cases cover positive/negative buckets, addition/subtraction, temporal sum/average, both parser profiles and
instant/range evaluation. Receiving addresses follow signed 64-bit Go arithmetic, with checked conversion at
physical access. Insertion appends before checking the destination slice and then the source slice; the original
error and preceding allocation remain observable. This panel compares complete errors and every numeric bit.

```sh
python tests/promql-conformance/logical_bucket_calls.py \
  --upstream "${PROMQL_UPSTREAM_SOURCE:?}"
python tests/promql-conformance/logical_bucket_calls.py \
  --reference "${PROMQL_LOGICAL_BUCKET_REFERENCE:?}" --binary "${PROMQL_QUERY_FIXTURE:?}" --page-size 1024
```

## Frontend allocation ownership

`Engine::compile` observes AST/source buffers, grouping/matching labels, argument lists, scanner diagnostics,
literal/number/format buffers, parser/recovery/validation stacks and returned parse-error lists from their first
allocation. They use actual capacity times slot size in the existing `Compiled` category. Temporary buffers
release before publication; retained buffers keep their owner through movement and cloning. The former retained
census is removed. A retained parse error or its cloned message can outlive the engine without losing its charge.
Public inert lexer/parser/literal helpers use the same algorithms with no accounting or control owner.

Checked frontend loops return cancellation and deadline outcomes separately from syntax diagnostics. Byte copies
poll in chunks of at most 1,024 bytes; scanner, recovery, decoding and validation walks poll as they advance.
Allocator calls, buffer migrations, cleanup and standard float parsing are cooperative latency boundaries.
The regex path below extends these checks through owned compile/search/replacement work. This checkpoint does not establish a hard deadline, a total allocation/RSS
bound, complete provider/error/sink/C ownership or full expression/SRE compatibility. Limits remain disabled.

## Regex cooperative control

Engine compilation, empty-selector validation, provider/core filtering and runtime `label_replace` use a checked
regex path. Owned syntax, factoring, classes, canonical serialization, program/NFA construction, optimization,
Unicode normalization, subject-offset construction, capture export and replacement scans poll as they advance.
Deep copies and variable-length shifts use explicit checked walks; fixed-size class tuples use interruptible
sorting. New scratch buffers use the current work owner; borrowed input retains its original ownership.
Fresh class results, simplified-node payloads and NFA buffers use that work owner. Existing owned inputs and mutable
container growth keep their physical allocation factories. Reusable syntax slots preserve their factory when old
contents are moved out. Exact retained-capacity tests cover literal reuse and constructors with independent input
and work owners, including the retained NFA dependency estimate.

Cancellation and deadline errors travel separately from syntax/backend error bytes and unmatched results.
Negative matchers negate only successful boolean results. Original public inert helpers and boolean matcher
methods remain available. Existing matcher implementations inherit pre/post checks around their legacy callback;
the owned selector implements checks within its semantic loops. Compiled matchers retain no query control.

Regex backend build/cache/search, standard hash-table operations, allocations, buffer migration and cleanup remain
atomic from this interface. Checks surround those operations, but cannot interrupt them internally. Partitioning
input would change anchors and capture behavior. Backend/table memory remains explicitly estimated under the
existing contract, and every admission limit remains disabled.

## Validated average acquisition reuse

Ordinary `avg_over_time` raw-matrix folds can accumulate floats during acquisition and reuse that completed
accumulator. The shortcut applies only to the initial pass, a positive non-wrapping window, and valid bounds
contained in that window. Statistics projection, buffering, anchored reductions and wider bounds keep ordinary
replay. Generic reader callbacks still run after acquisition validation.

No read credit, result or diagnostic is published from an incomplete acquisition. Native or mixed allocations,
including a stale histogram, retain consuming validation and its saved state. Later native arithmetic still
follows complete acquisition. The ordered numerical accumulator and raw page lifetimes are unchanged.

Focused controls compare all saved-state fields in 1152 cases, inject 90 actual read/seek failures, and check
cancellation, wider bounds, overflow, NaN payloads, signed zero, stale values and 2048/100000-point histories.
Fifteen C cases check full results/credits, first-final provider failures, later failures after output and stale
histogram fallback. Six native traces confirm the actual final-execution phase; output begin is insufficient.

[Average reuse qualification](average-read-reuse-qualification.json) records the final source, binaries,
complete comparisons and measured changes. Fixture timings and tracked workspace are separate from production
disk throughput and process RSS. Earlier qualification evidence retains its original source and binary bindings.

The matched fixture evaluates eight range ticks over two floats per series. Three alternating baseline/candidate
pairs cover both profiles, 128/512 series and pages 1/17/1024. All 72 complete responses match; fetched entries
halve and tracked workspace peaks stay equal in all 12 groups. Median timing ratios range from 1.250 to 1.388.
Page 17 illustrates the recorded result:

| Series | Fetched before → after | Work before → after | Workspace peak before/after | Release time ratio | Release-min time ratio |
| --- | --- | --- | --- | --- | --- |
| 128 | 16,384 → 8,192 | 454,955 → 429,611 | 161,558 bytes | 1.327 | 1.352 |
| 512 | 65,536 → 32,768 | 2,647,680 → 2,546,304 | 630,504 bytes | 1.268 | 1.250 |

The final cohort passes 562 units per profile, 128,652 retained builtin comparisons, 2,070 focused comparisons and
231,540 exact SRE comparisons. Native/ARM64/ARM32 C qualification retains 734 artifacts and runs the average
controls in every current caller. The fresh Debug Agent passes 672 storage records and 66 live HTTP/readiness
checks, including the new instant/range average cases. These finite cohorts do not prove arbitrary-input behavior,
foreign whole-Agent execution, total RSS or production disk throughput.

## Validated float rate acquisition reuse

Ordinary float `rate`, `increase` and `delta` collect first/last/count/start fields during the initial validation
traversal. Their shared `FloatStep::scan` reuses a successfully completed pure-float acquisition and credits logical
reads only after full reached-page validation and terminal control. Last-minus-first remains before chronological
reset additions; the reset pass still performs its original acquisition and consumption.

Avg and float rates share the existing eligibility predicate: no statistics projection or buffer, `reduce_delta`,
a positive non-wrapping current window, and valid consumer bounds contained in that window. Overlapping schedules
are allowed; the separate history-pruning condition `step >= range` is not required. Raw/no-matrix calls,
instant/extended rates and unproven bounds retain their previous paths.

Any populated histogram allocation, including stale history, discards tentative float state and consumes normally
with the completed progress. Later native/mixed phases remain fresh. Endpoint sample leases describe actual ownership:
a failed initial acquisition can now show up to two earlier local sample units while useful reads remain zero.
No raw array, retained value cache, new persistent field, public interface or resource limit is added.

Five new direct owners check 126 complete float cases, 507 actual open/read/seek failures, close-triggered
cancellation before credit, unused malformed tails, fallback paths and 2,048/100,000-point histories. Existing Avg
fault/control owners and 1,152 complete saved-progress/sequence comparisons qualify the shared predicate.
Both optimized native profiles pass all 691 units; four compiled ARM cohorts each execute eight focused owners.

The affected unchanged-original subset is 308 wrapped-rate requests plus 155 ordinary rate/average requests.
Native pages 1/17/1024 and foreign page 17 yield 4,630 complete comparisons, all with zero raw strict failures or
paired numerical bit differences. These counts do not requalify the wider historical panels or remove their
ordering disagreements. The historical window generator has a different pre-commit identity; all other 248 capture
artifacts match, and today's complete 2,266 generated inputs equal the frozen captured inputs exactly. The report
preserves both generator hashes and the original results without recapture.

Actual C11/C++17 consumers execute `--range-rates-only` and `--window-folds-only`: 12 programs, 24 executions and
12,672 lifecycle arms across all three targets and both profiles. Failures are calibrated to actual first-final
reads; callback counts are not fixed. Caller ratios establish lifecycle and statistics behavior, while the complete
original panels establish distinct numeric results. Foreign execution uses QEMU, not physical devices.

`rate_acquisition_cases.empty_cases()` supplies 48 empty/all-stale histories, covering float, integer histogram
and float histogram input, all three extrapolated functions, instant/range execution and `absent` wrappers.
Both native profiles at pages 1/17/1024 check 576 responses against explicit empty/one expectations. All 288 paired
nonelapsed observation groups match the actual published predecessor.

`rate_acquisition_cases.performance_cases()` reproduces all 27 fixed scaling/fallback/control inputs. Five
alternating pairs per native profile check every complete warm/measured result and each binary's own nonelapsed
observations: 1,080 responses plus 108 anchors, producing 54 timing groups. All owned qualification jobs finish
before timing begins. The [source-bound report](rate-acquisition-qualification.json) retains every sample, result
binding, observation category, binary and code-size cost.

At 1,024 groups, five-sample median engine milliseconds are:

| Function in grouped ratio | Release before → after | Release-min before → after | Fetched-entry reduction |
| --- | ---: | ---: | ---: |
| `rate` | 279.615 → 252.931 | 484.191 → 420.598 | 25% |
| `increase` | 288.917 → 248.735 | 465.901 → 420.493 | 25% |
| `delta` | 227.275 → 203.828 | 389.201 → 345.288 | 50% |

The rate input changes 655,360 → 491,520 fetched entries, 163,840 → 122,880 page deliveries and
331,783 → 249,863 provider calls in both profiles. Its workspace peak stays 4,438,614 bytes. All eight memory-kind
peaks stay equal in all 54 timing groups; workspace and borrowed-view traffic decrease in 20 of 27 groups per
profile. Borrowed-view traffic is not physical allocation or disk I/O.

Three release medians are slower: raw-root 64 groups by 1.26%, raw-root 1,024 groups by 1.93%, and the all-native
fallback by 0.94%. No release-min median is slower in this finite run; that is not a zero-overhead guarantee.
Query-fixture ELF text grows by 43,632 bytes in release and 28,048 bytes in release-min. Timings are synthetic
shared-host engine execution without CPU/frequency isolation, excluding compilation and fixture transport.
They establish neither live HTTP/DBengine disk or concurrent throughput, whole-Agent size, physical ARM timing
nor process RSS. Existing APIs, storage/provider policy, numerical reference and disabled caps remain unchanged.

## Histogram constructor key lookup

Histogram plan construction uses a temporary index over existing encoded keys to avoid scanning every earlier
group. It retains direct comparison through two groups, then promotes in first-encounter order. Classic keys omit
`le`; native correspondence retains complete labels and runs after classic group construction. Hashes select
candidates; complete byte equality remains authoritative, including empty labels and reserved metadata.

The index retires before plan publication. It adds no retained plan field, key copy, raw sample or value timeline.
Label preparation and numerical/replay order stay unchanged. Completed lookup/insertion and final constructor
polls prevent interrupted provisional work from publishing a plan.

Focused controls cover eight actual digest-collision configurations, twelve empty/reserved/native-label rows,
eight malformed/NaN/infinity/zero bound cases, 68 calibrated constructor interruptions and three tiny long-key
lookups. Complete regression outputs, logical/read statistics, fetched counters and cleanup precede the work bound.
Later active-group discovery, binding, numerical and replay scans remain separate costs.

[Histogram key qualification](histogram-key-qualification.json) binds the actual final sources, binaries,
comparisons and measurements. The earlier measured candidate remains separately identified: it increased work
for two groups with long labels, prompting the final direct-comparison choice. Finite fixture timings and tracked
workspace do not establish production throughput, process RSS or arbitrary-input equivalence.

The final matched cohort covers 12 high-cardinality groups and 60 empty/small/native-only/64KiB-label groups,
both profiles and three alternating pairs. All 432 complete responses match, with unchanged fetched entries and
tracked workspace peaks. Tiny-set accounted work is unchanged in every group. Page 17 illustrates the larger sets:

| Groups | Work before → after | Workspace peak before/after | Release time ratio | Release-min time ratio |
| --- | --- | --- | --- | --- |
| 128 | 1,850,410 → 884,375 | 416,520 bytes | 1.004 | 1.034 |
| 512 | 22,282,232 → 6,244,368 | 1,643,400 bytes | 1.040 | 1.129 |

High-cardinality median timing ratios range from 0.995 to 1.130. Reduced accounted work does not imply a
proportional time gain; later histogram preparation and numerical replay remain. The source-bound final cohort
passes 568 units per profile, 128,652 retained builtin comparisons, 2,070 focused comparisons and 231,540 exact
SRE comparisons. Current native/ARM C qualification retains 734 artifacts. The fresh Agent passes 672 storage
records and 74 HTTP/readiness checks, including the added live histogram expressions.

## Histogram active-group distribution

Temporary group-ID indexes replace repeated active-group discovery, native/classic correspondence and bounds
distribution inside histogram step preparation. Direct lookup and ordinary distribution through four groups
avoid measured small-set table overhead. Existing ranges hold reference counts and scatter cursors. Sorting changes
positions, so the discovery index retires before enumeration and the surviving groups are indexed afterward.
Source-order scatter preserves every bound bit and physical ordinal before numerical sorting. No new persistent
field or raw history array is added.

Focused checks force reverse encounter/label order, sparse IDs, skipped groups between survivors, duplicate,
NaN and signed-zero bounds, malformed warnings, first native suppression and later native handling. Seven active
groups within an eighteen-group plan preserve twelve exact bounds; calibrated interruptions discard partial state
before value selection. Direct preparation work falls from 64,912 to 18,498 at 128 groups and from 871,756 to 96,559
at 512 groups. These are accounted work units, not timing ratios.

[Active distribution qualification](histogram-distribution-qualification.json) binds the final current source,
binaries, complete outputs/statistics/release and actual measurements. The earlier through-two candidate remains
separately identified: it added 22 instant/440 range work units at four groups and showed small-query timing cost,
prompting the final through-four crossover. Historical source and binary identities are preserved.

Complete query checks validate labels, ticks, bits, diagnostics, logical/read statistics,fetched counters and cleanup
before work bounds. Current range read assertions are distinct from protocol comparisons, whose baseline
observations do not expose basic sample statistics. Generated query timings and tracked workspace are separate
from production throughput and total RAM.

The final matched cohort covers 612 complete alternating responses in 102 query/profile groups. All tracked
workspace peaks and read counters match the previous revision. Accounted work falls in all 30 larger groups and
is unchanged in all 72 empty/small/long-label controls. Page-17 medians from three alternating pairs are:

| Query | Groups | Work before → after | Workspace peak, bytes | `release` ratio | `release-min` ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| Instant | 128 | 884,375 → 791,547 | 416,520 | 1.153 | 1.064 |
| Instant | 512 | 6,244,368 → 4,693,974 | 1,643,400 | 1.361 | 1.298 |
| Eight-tick range | 16 | 1,977,186 → 1,914,218 | 59,254 | 1.012 | 1.011 |
| Eight-tick range | 32 | 8,351,869 → 7,707,445 | 110,374 | 1.115 | 1.036 |
| Eight-tick range | 64 | 38,438,681 → 32,732,201 | 213,696 | 1.203 | 1.125 |

Ratios divide the previous median elapsed time by the current median. Across all larger groups they range from
0.994 to 1.371; reduced work does not guarantee a proportional timing gain. Both profiles pass 572 units,
128,652 retained builtin comparisons, 2,070 focused comparisons and 231,540 exact SRE comparisons. Native/ARM
C qualification retains 734 artifacts, and the fresh Agent passes 672 storage records and 82 HTTP/readiness checks.

A focused follow-up retains 8,640 complete small-query responses with repeated warmed timing. The large cold
empty-case discrepancy does not persist. Warmed ratios are 0.976–1.027 for `release` and 0.941–0.983 for
`release-min`, including up to 6.3% longer small-query medians in the latter. These controls keep their identical
accounted work and workspace, but the measurements do not establish a universal speedup.

The retained active-distribution cohort projects the two collected gauges into three groups per host. In that
cohort, its three-tick fleet range exceeded the HTTP client's ordinary 30s allowance in both compared Debug Agents.
Those functional checks used a targeted 180s client allowance and completed under both storage codecs; ordinary
checks remain 30s. Fleet quantile sums are 3, local/child sums 1.5. No collector, stored metric or production-timeout policy
changes. The earlier failed runs remain separate evidence; longer waiting is not a throughput fix. Nested derived
replay remains an independently tracked performance opportunity.

## Completed set-input discovery

Repeated evaluation of `and`, `or` and `unless` can reuse one complete child-discovery pass at
an exact scope. The private Plan keeps active-input membership, including inputs suppressed by matching.
It retains no sample values or history. The original full left/right discovery must finish before publication;
a later selected body can reuse that metadata without forcing a truncated Vector caller to drain.

Eligibility is restricted to compatible ordinary forward instant or range clocks over raw selectors, transparent
parentheses, prepared label projections and other qualified sets. Check original immediate-child aliases before
projection copies strip source ranges or move descriptors, then check owned side-row labels, float kinds, heads
and ordinals. Matching-signature duplicates remain legal. Native/mixed, multiple, aliased, copied, invariant,
wrapping and other expression shapes continue through the original implementation. The original discovery and
prefix cohorts below qualified range consumers; the direct singleton extension is described under
[instant set discovery](#instant-set-discovery).

Warm reconstruction preserves side order and later phases: Begin, choice, selected/unverified operand
rereads, labels, duplicate/diagnostic completion and output truncation. Completed prefixes below can omit verified
nonselected acquisitions; the original discovery-only cohort retained every reread. Right-to-Begin publication certifies
completed discovery. Existing Pair replay still publishes only after Done. Successful roster preparation clears
all set snapshots before ordinary or standalone derived-matrix final consumption, preserving a complete first
final discovery. One current exact key replaces the previous scope before allocating its bitmap.

The focused owner tests cover suppressed inputs, sparse membership at 64-bit boundaries, reversed side ordinals,
legal duplicate signatures, all eleven key fields, old-before-new replacement, failed/abandoned discovery,
unchanged selected reread errors, native/repeated/descending capture invalidation, full-hash/head/source/ordinal
ambiguity, original-versus-copied aliases, empty-versus-missing descriptor owners and cancellation. A direct
successful-preparation assertion verifies snapshot clearing for both final consumers. Complete query checks
validate labels, ticks, float bits, diagnostics, logical totals, actual useful/fetched reads, one cursor and release
before the throughput bound. Raw history remains paged across series and time.

Selected/unverified acquisitions and later histogram/projection replay remain independently measured costs. The bitmap and
its accounted owners can add a small workspace allocation; reduced reads are not a universal memory reduction,
production disk-throughput claim or arbitrary-input equivalence proof.

[Set discovery qualification](set-discovery-qualification.json) binds the final sources and both profiles,
581 units per profile, 128,652 builtin and 2,070 focused comparisons, 231,540 exact SRE comparisons,
68 executed ARM discovery-owner checks and 734 C/C++ artifacts. The rebuilt Debug Agent passes 672 storage
records and 82 live HTTP/readiness checks; the unchanged six-group range uses the ordinary 30s client allowance
under both codecs. Earlier failed/180s runs and the first pre-lint candidate retain their separate identities.

The matched cohort retains 576 complete alternating responses across 96 query/profile groups, including
empty/tiny/64KiB-label controls, three page sizes and three pairs. All outputs match; larger disjoint and deep
queries reduce both work and fetched entries. Page-17 medians are:

| Three-tick query | Fetched entries before → after | Work before → after | Workspace peak before → after, bytes | `release` ratio | `release-min` ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| Disjoint set, 16 inputs | 28,608 → 10,752 | 2,611,677 → 1,190,594 | 79,215 → 79,463 | 2.140 | 2.206 |
| Disjoint set, 32 inputs | 112,512 → 39,936 | 10,247,509 → 4,431,616 | 145,200 → 145,448 | 2.319 | 2.356 |
| Disjoint set, 64 inputs | 446,208 → 153,600 | 40,784,949 → 17,160,097 | 277,552 → 277,808 | 2.370 | 2.420 |
| Six-group histogram sum | 2,319,948 → 125,388 | 498,789,241 → 39,382,001 | 187,837 → 189,077 | 12.914 | 12.638 |

Ratios divide previous median query time by current median. The deep query falls from 3,768ms to 292ms in
`release`, and 6,727ms to 532ms in `release-min`; these are synthetic engine measurements. The nine direct
query units separately validate unchanged logical totals and actual useful reads, which the protocol observation
payload does not expose.

A 7,200-response follow-up repeats empty/one/two/four-input and raw-gauge controls, discarding the first twenty
repetitions for timing. Warmed `release` ratios range from 1.139 to 1.687; `release-min` ranges from 0.953 to 1.675.
The empty `release-min` median increases from 67.740µs to 71.055µs, about 4.9%, with unchanged workspace and
18 additional work units. The first cold empty ratio of 0.809 is therefore not the warmed effect. Small disjoint
sets can add 248 bytes of metadata; no universal speedup or memory decrease is claimed. At that source, prefix and
later projection/histogram replay remained independent opportunities; the prefix correction is qualified below.

### Sparse reconstruction from existing membership words

Warm discovery now enumerates the existing bitmap words and their lowest set bits in ascending physical position.
The actual left-side length maps positions across sides; unused final-word padding is ignored. This preserves
physical row order even when descriptor ordinals run backward. Every word, including an empty word, and every
active row is charged and polled; the final check still precedes Begin. No new buffer, field or matching cache is added.

The unchanged-runtime regression compares complete cold/warm labels, ordinals, float bits, rereads and release
before its work bound. With three active inputs, 128/512 rows per side previously charged 256/1,024 visits;
current reconstruction charges 7/19. Side lengths 0/1/63/64/65/129, all set operators, empty/sparse/dense membership,
cross-side words and unused tail bits are checked. Fewer than two descriptors retain the existing generic route.
The cancellation control measures eight actual word/member charges for its sparse 129/129 fixture, exercises
each charge and the final check, and preserves the completed owner and frontier while releasing partial arrays.

The matched query experiment retains 69 cases per profile at pages 1/17/1024, including sparse and dense 128/512
source-series sets. All 828 responses preserve complete outcomes, reads, calls, pages, backing and workspace peaks.
Sparse inputs have all series initially and two at later ticks, with an explicit 100ms lookback.

| Sparse source series, page 17 | Whole-query work, before → after | Workspace bytes, unchanged | Release ratio | Release-min ratio |
| --- | --- | --- | --- | --- |
| 128 | 3,240,399 → 2,989,479 | 542,528 | 1.074 | 1.114 |
| 512 | 42,184,524 → 38,108,892 | 2,130,848 | 1.133 | 1.189 |

Ratios divide the previous median time by the current median. The 512-series sparse range falls from 270.784ms
to 238.893ms in release and 489.880ms to 411.951ms in release-min. Dense reconstruction adds one work unit per
word; the dense 128/512 whole-query increase stays below 0.089%. Latency varies across the other workloads.
The 7,200-response warmed tiny/raw panel observes ratios 1.008–1.039 in release and 1.034–1.058 in release-min.
These are alternating synthetic queries on a shared host during SRE replay, not isolated-machine, disk-throughput
or process-RSS measurements. The initial slower padded dense case remains recorded separately from its follow-up.

The padded 32-series/page-1 release case initially measured 0.894×. A separate 960-response follow-up after bulk
SRE replay, with 105 warmed observations per phase/profile/case, measures 0.993× for that case in release and
1.014× in release-min. The larger initial slowdown is not reproduced; the release median is still about 0.7%
slower. Both observations remain preserved, and neither establishes universal throughput.

[Bitmap reconstruction qualification](set-member-bitmap-qualification.json) binds the actual sources/binaries,
596 units per profile, 128,652 builtin, 2,070 focused and 231,540 exact SRE comparisons, 132 ARM owner executions
over 128 distinct target/profile/test slots, and 18 C/C++ callers. The freshly rebuilt Agent passes 672 storage
records and 82 HTTP/readiness checks. Historical cohorts retain their original identities.

## Instant set discovery

Ordinary instant and equal-bound range scopes use the same certified membership/prefix owner as eligible
multi-tick sets. Instant execution retains effective interval 1. Merely removing the old unequal-bound guard
does not activate reuse: a classic histogram Count requests a singleton Vector body without bypassing ordering.
If its output roster has moved to a parent, the Binary body executes directly.

Both initialization and advancement now recognize a certified set singleton with no ordered roster. Plain
publication and projection moves retain its emitted set ordinal. If canonical remapping exists, entry retains
OrderedRows and its original source translation. Existing Matrix/bypass and actual Matrix preparation-owner
routes remain; Pair activation and unfiltered Vector behavior remain unchanged. Complete cold discovery,
first-final preparation clearing, fresh requested/unverified reads, duplicate handling and original truncation
still apply. No raw values, history or new persistent cache/Step fields are retained.

The complete-query regression reproduces the six-group expression used by the live API fixture with two
one-point source series. It checks exact output, logical total 12, physical reads, one cursor and release before
its acquisition bound. Ordinary instant, equal-bound and unequal-bound one-tick requests use pages 1/17/1024
and both name-removal modes. Instant scalar collectors use `scalar`; range requests produce matrix rows.
The simple set-operator controls also check all three operators and complete outcomes. Run these from the crate:

```sh
cargo +stable test --locked --features fixtures instant_histogram_set_replay
cargo +stable test --locked --features fixtures single_tick_set_operators
cargo +stable test --locked --features fixtures completed_set_preparation
```

These development-profile checks establish behavior. Optimized qualification uses both release profiles.
The actual C/C++ caller has `--instant-sets-only` and `--instant-sets-delayed-only` arms for three request shapes,
pages 1/17/1024, complete histogram outputs, discovery/selected failures, required reads after output,
cancellation/output errors and release. Its default roster includes the ordinary mode.

[Generated controls](instant_set_cases.py) include reversed source order, full vector histograms and sets,
empty/tiny inputs, scalar and grouped reductions, both name modes and multi-tick controls. Complete before/current
comparison preserves row-array order. The fixed timing panel retains all samples and small-control costs;
it does not establish CPU isolation or universal throughput.

The exact two-point Release diagnostic reduces fetched entries 99,952→390, provider calls 199,935→811 and
work 55,837,009→473,309. Its tracked workspace peak increases 130,358→130,694 bytes. The nested-union control
reduces 4,116→231 entries and adds 1,240 peak bytes; prelabelled classic and direct gauge controls keep their original
reads/work/peaks. These are finite generic-provider observations, separate from live DBengine measurements.
The retained admission-only pilot had no read gain and added metadata; it is not counted as a speedup.

[Instant set qualification](instant-set-qualification.json) binds 693 units in each optimized build,
4,242 strict original set-expression comparisons and 432 actual C/C++ lifecycle arms. The generated panel preserves
all 528 complete before/current result pairs, including row-array order. Its 54 timing groups retain 756 measured and
108 warmup responses. Page 17 median engine times are:

| Query | Release before → current (ms) | Release-min before → current (ms) |
| --- | ---: | ---: |
| Six-group histogram | 405.322 → 4.318 | 697.494 → 6.849 |
| Nested unions | 17.041 → 2.136 | 29.159 → 3.413 |
| Small `or` reduction | 0.227 → 0.226 | 0.316 → 0.323 |
| Small `and` reduction | 0.179 → 0.187 | 0.261 → 0.276 |
| Empty set reduction | 0.069 → 0.074 | 0.087 → 0.089 |

Eight Release and 21 Release-min timing groups are slower. Their largest median increases are about 6.6% and 5.7%,
respectively; all raw and small controls remain in the report. Across the 528 complete comparisons, observed workspace
peak increases range 0–1,240 bytes. Executable text grows 768 bytes in Release and 304 bytes in Release-min. Lower fetched
entries and cumulative allocation traffic are not universal peak-memory reductions or zero-overhead guarantees.

The freshly rebuilt containing Release Agent passes 82 HTTP/readiness checks and 224 measured responses with both
codecs and normal owned exits. Its unchanged histogram expression improves 2434.536→18.388 ms on raw pages and
2344.758→19.054 ms on Gorilla. The live panels run at different times on a shared host; the generic alternating
comparison provides separate causal evidence. Larger/cold data, physical ARM performance, saturation throughput
and Rust-only or total-system RSS are not established. The ordinary limits and adaptive-policy decisions are unchanged.

## Completed set-operand prefixes

The current exact set-discovery owner records one successfully completed kept prefix. During a later targeted
body in its original set-ordinal domain, verified nonselected operands reconstruct seen/duplicate state with the immutable descriptor's
canonical full hash without rereading values, copying labels or emitting filtered samples. The requested operand stays fresh.
Discovery grants no credit; success includes the original operand, label, hash, seen and terminal control phases.

Initialization and advancement share the owner/filter predicate. The existing bypassed Matrix route and actual
Matrix preparation owner remain; certified direct singleton sets also qualify without an ordered roster.
Unfiltered Vector and Pair activation remain unchanged. Eligible All bodies seed progress while returning
every value; inactive contexts do neither. Key replacement, new discovery publication and preparation clearing
reset the frontier. Ordinary Done with no pending discovery preserves All-body progress. Unverified prefixes and
suffixes, first-pass failures, selected failures and original truncation remain required; there is no extra drain.

This scalar fits the existing replay payload and retains no raw samples, value cache, label copy or additional
owned buffer. Its synchronous borrowed scope adds no Step fields. Current callers serialize same-Plan execution:
certified descendants are proper raw/projection/set children, an OrderedRows wrapper finishes or truncates its
child, preparation/final Machines run sequentially, and reentrant queries own fresh Rosters. Future overlapping
same-Plan Steps, shared Rosters or ancestor redirects must revisit this assumption; exact keys alone do not solve
an overlapping A→B→A incarnation.

Causal owners check fresh requested and unverified reads, backward/sparse targets, All seeding/Done, discovery
without credit, missing suffixes, failed acquisitions, all eleven key fields, reset/clear and all three set
operators. A genuine unequal-label full-hash collision retains exact-byte matching signatures and the original
deferred Finished duplicate error. Calibrated cancellation must precede terminal credit and successful skipped
reconstruction. A direct helper control measures its actual post-seen boundary; whole-body budgets alone do not
establish that boundary because the surrounding loop also polls.
Complete workload controls check labels, ticks, float bits, diagnostics, logical totals, useful/fetched reads,
single-cursor paging and release before imposing the new acquisition bound.

[Set prefix qualification](set-prefix-qualification.json) retains the actual source-bound cohorts and measured
tradeoffs. Existing discovery-only, red, provisional and failed control/helper receipts preserve their original
identities. Full hashing of requested/unverified operands, active-membership reconstruction and later projection/histogram work
remain independent costs; reduced reads are not a general production disk-throughput or process-RSS claim.

The preceding prefix cohort passes 591 units per profile, 128,652 builtin comparisons, 2,070 focused comparisons and 231,540 exact SRE
comparisons. ARM64 and ARM32 execute 112 discovery/prefix owner checks across both profiles, covering 108 distinct
target/profile/test slots because one test belongs to both selectors. Compiled layouts match the prior discovery
cohort: `ReplayRows/Replay/CompletedStep/Plan/Step` remain `48/64/128/328/680` bytes on AMD64/ARM64 and
`32/40/112/224/496` bytes on ARM32.

Actual C/C++ qualification retains 18 caller variants, ABI3 rejection, layouts and the native caller sanitizer/abort
checks. The private runner uses separate per-task Rust/abort targets, preserving previous link inputs. Its 1,033
artifact files include those build outputs; this count is not additional C test coverage. The freshly linked Debug
Agent passes 672 storage records and 82 HTTP/readiness checks under both codecs, with ordinary 30s client reads
and normal owned exits. The initial private helper-import failure is retained separately from passing C evidence.

The matched cohort retains 576 complete alternating responses across 96 query/profile groups, three page sizes
and three pairs. Larger disjoint/deep cases reduce both work and fetched entries; none of the 96 observed workspace
peaks increases. Grouped quantiles use 32 fewer bytes; other peaks are unchanged. Page-17 measurements are:

| Three-tick query | Fetched entries before → after | Work before → after | Workspace peak, bytes | `release` ratio | `release-min` ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| Disjoint set, 16 input series | 10,752 → 1,824 | 1,190,594 → 430,286 | 79,463 | 3.880 | 3.930 |
| Disjoint set, 32 input series | 39,936 → 3,648 | 4,431,616 → 1,319,608 | 145,448 | 5.512 | 5.574 |
| Disjoint set, 64 input series | 153,600 → 7,296 | 17,160,097 → 4,521,637 | 277,808 | 7.596 | 7.199 |
| Six-group histogram sum | 125,388 → 4,329 | 39,382,001 → 2,965,238 | 189,077 | 16.118 | 16.517 |

Ratios divide the previous published build's median query time by the current median. The deep query falls from
292.796ms to 18.165ms in `release` and from 528.898ms to 32.021ms in `release-min`. Separate complete direct units
also validate useful reads and logical participation, which the protocol observations do not expose.

A 7,200-response warmed follow-up repeats empty/one/two/four-input and raw-gauge controls, discarding the first
twenty repetitions. Ratios range from 0.981 to 1.749 in `release` and 1.007 to 1.799 in `release-min`. The raw-gauge
`release` median rises from 19.965µs to 20.355µs, about 2%; empty and small disjoint controls are unchanged or faster.
These synthetic measurements ran on a shared host while SRE replay was also active. They establish neither a
universal speedup, CPU isolation, production disk throughput nor process RSS. Historical measurements retain their
actual sources and binaries.

### Canonical hashes for skipped set operands

Verified skipped operands reuse their descriptor's canonical full hash. Raw construction and every label projection
compute that hash with the exact owned label bytes; copying and merging preserve the pair. Fresh emission copies
the same prototype and validates its bytes before granting prefix credit. Grouping hashes and arbitrary descriptors
do not qualify for this shortcut. Requested, cold and unverified operands retain their full hashing and errors.

The complete work regression first checks values, labels, timestamps, diagnostics, logical totals, provider fetches,
single-cursor paging and release. With unchanged runtime, doubling the largest input count exceeded its work bound;
the new implementation passes. A directly calibrated skipped helper reconstructs two earlier hashes, then exercises
the final seen insertion. Cancellation at the actual terminal work prevents the phase transition and releases state.
The original whole-body budgets remain general controls; their outer polls do not prove the helper's terminal check.

[Canonical hash qualification](set-prefix-hash-qualification.json) binds 593 units per profile, 128,652 builtin,
2,070 focused and 231,540 exact SRE comparisons. ARM64 and ARM32 execute 120 discovery/prefix checks covering 116
distinct target/profile/test slots; the existing layouts are unchanged. Fresh C/C++ qualification retains 18 caller
variants, ABI3 rejection, layouts and the native caller sanitizer/abort checks. The rebuilt Debug Agent passes 672
storage records and 82 HTTP/readiness checks under both codecs. Historical cohorts keep their own identities.

The matched panel compares 684 complete responses in 114 query/profile groups, with pages 1/17/1024 and three
alternating pairs. It adds 64KiB label padding to the 16/32/64-input shapes to exercise the redundant byte work.
Every group's fetched samples, calls, pages, output count, temporary backing and tracked workspace peak are unchanged.
Page-17 disjoint-set sums are:

| Input series | Label padding per series | Work before → after | Workspace peak, bytes | `release` ratio | `release-min` ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 16 | 0 | 430,286 → 302,318 | 79,463 | 1.193 | 1.151 |
| 32 | 0 | 1,319,608 → 799,480 | 145,448 | 1.320 | 1.296 |
| 64 | 0 | 4,521,637 → 2,424,613 | 277,808 | 1.465 | 1.445 |
| 16 | 65,536 | 466,066,922 → 270,877,034 | 6,540,693 | 1.964 | 2.919 |
| 32 | 65,536 | 1,335,302,374 → 541,949,926 | 12,946,448 | 2.541 | 4.517 |
| 64 | 65,536 | 4,283,339,878 → 1,084,744,294 | 25,846,224 | 3.513 | 7.028 |

Ratios divide the preceding published build's median query time by the current median. The ordinary 64-input case
falls from 30.283ms to 20.664ms in `release`; its padded case falls from 356.337ms to 101.426ms. The six-group
histogram sum improves by 1.142/1.112 in the two profiles, with unchanged reads and peak. Separate direct units use
`counter_total` labels and report their own useful-read/logical statistics; their counts differ from these `m` inputs.

The 7,200-response warmed small-query panel preserves every complete outcome. Ratios range from 0.989 to 1.103 in
`release` and 0.976 to 1.016 in `release-min`; some tiny medians are up to about 2.4% slower. The raw-gauge control
changes from 20.570µs to 20.790µs and 30.990µs to 31.560µs respectively. Both measurement processes run on a shared
host while SRE replay is active. These finite synthetic timings and tracked peaks are not CPU-isolated, production
disk-throughput or process-RSS evidence. Membership reconstruction and later projection/replay remain separate costs.

## Copied grouping-label construction

The retained cohort qualified four copied grouping paths: prepared copied roster groups, quantile groups,
temporary top-k group keys and per-evaluation aggregate lookup. They use the existing checked filtered-copy
owner and share the exact by/without predicate with the in-place transfer path. Transferred descriptor labels
still retain their existing capacity. Full input and ranking labels remain owned by their original plans.
The aggregate lookup has since moved to [borrowed labels](#borrowed-aggregate-lookup-labels); the three retained
key-owning paths continue to use checked copies. Measurements below retain their original source attribution.

Grouping preserves raw empty values, original label order, first hash matches and existing binding positions.
`by` can explicitly keep `__name__`; `without` excludes it. `__type__` and `__unit__` follow ordinary grouping
selection. Binary result projection has different reserved-name/reset semantics and is not reused here.
That cohort retained diagnostic views, output reset, parameter validation, warnings and numerical operations.

A retained quantile-plan regression keeps two fields from 8, 17 and 65 input labels. The unchanged native owner
retains 1,039, 3,343 and 12,559 bytes, including its group descriptor. Direct construction retains 655 bytes in
all three shapes; current ARM32 retains 463 bytes. Complete group labels/hash and release are checked before
the capacity comparison. These are isolated owned group bytes, not total query memory or process RSS.

Controls cover omitted labels, long retained values, long grouping names, raw empty keys, source ordering,
ordinary type/unit names, collision/first-writer behavior and partial release. A completion poll follows the
last accounted copy charge before returning the key; the terminal interruption is calibrated from actual work.
No new metadata provider, view/cache, persistent plan field, raw sample array or implicit query cap is added.

The matched panel retains 3,024 complete alternating responses across 504 query/profile groups: six grouping
expressions, instant/range, pages 1/17/1024, zero/tiny/128/512 groups and short/64 KiB omitted values. Complete
results, fetched samples, provider calls, pages, output counts, temporary backing and release agree. All larger
cases acquire fewer workspace bytes and charge less work. Quantile and nested copied-group peaks decrease;
top-k and raw ownership-transfer peaks remain unchanged.

For 512 groups, two source series/group, short omitted values and page 17:

| Query | Peak workspace before → after, bytes | Release timing ratio | Release-min ratio |
| --- | --- | --- | --- |
| Quantile, instant | 3,146,758 → 2,556,934 | 1.045 | 1.010 |
| Quantile, range | 3,147,165 → 2,557,341 | 1.042 | 0.992 |
| Top-k, instant | 2,964,992 → 2,964,992 | 1.195 | 0.999 |
| Top-k, range | 3,059,360 → 3,059,360 | 1.051 | 0.960 |
| Sum of top-k, instant | 5,021,056 → 4,431,232 | 1.128 | 1.059 |
| Sum of top-k, range | 5,077,397 → 4,487,573 | 1.098 | 1.002 |
| Raw sum, instant | 1,986,792 → 1,986,792 | 1.158 | 1.171 |
| Raw sum, range | 1,986,792 → 1,986,792 | 1.306 | 1.174 |

Ratios divide the old median by the new median. Quantile peak savings approach 19% in the short-label cases,
but retained input/output metadata dominate the 64 KiB cases. The original larger-case timing ratios span
0.719–1.326; several cases are slower. Those measurements ran alongside qualification on a shared host and
remain retained. They do not establish a universal speedup, process RSS bound or production disk throughput.

A separate warmed tiny panel checks 34,560 complete responses in 48 query/profile groups at cardinalities 0/1/4,
discarding the first 20 repetitions and retaining 300 timings per phase/group. Ratios span 0.981–1.248 for release and 0.974–1.172
for release-min. Empty/tiny costs remain visible rather than being counted as large-query speed gains.

The slower-case follow-up runs after the owned bulk SRE replay and other qualification tasks finish. It retains
480 complete responses across ten evidenced profile/case pairs, with three alternating binary pairs, eight
repetitions and two discarded warmups (18 timings per phase/group). Complete results and resource counters
match the original measurements. Ratios span 0.970–1.244. The original 0.719x quantile case measures 0.997x;
the 0.823x top-k range measures 1.084x, and the 0.862x nested sum range measures 1.116x. Two release-min range
cases remain about 3% slower. These observations remain disclosed; lower allocation volume does not guarantee
lower elapsed time for every query on a shared host.

[Copied grouping qualification](grouped-label-qualification.json) binds the actual red/development/final phases,
600 units per profile, 128,652 retained builtin and 2,070 focused comparisons, and 231,540 exact SRE comparisons.
All 18 C/C++ callers, 72 distinct ARM target/profile grouping checks, 672 storage records and 82 live HTTP/readiness
checks pass. The cohort preserves all 46 preceding ledger objects and their source/binary identities. Caller
sanitizers exclude Rust instrumentation; ARM checks execute the library and callers, not a complete foreign Agent.

## Borrowed aggregate lookup labels

Ordinary aggregate lookup now hashes the selected fields of the existing immutable sample labels. It no longer
creates a temporary grouping key or diagnostic view array. Reached warnings borrow the checked metric name after
aggregation and diagnostic gates, before child-span resolution. Prepared key owners, ranking labels, output
copies and provider paging retain their existing lifetimes.

Selection uses the existing by/without predicate and source-order name/255/value/255 encoding. Raw empty values,
delimiter aliases and hash-only first matches remain meaningful. No grouping and by() return literal zero;
by(missing) and other empty selections retain the ordinary empty digest. Label/name/hash walks and the terminal
charge are checked. Eliminated copies remove actual work; checked warning-name lookup can add comparison work.

Both native profiles pass 739 units with two unchanged ignored tests, 1,403 complete cases at pages 1/17/1024,
and 336 actual default-feature C queries in total. Each ARM target passes 13 focused roster units and 204
complete selected queries under QEMU. The combined current query count is 8,826. Formatting, strict
default/fixture Clippy and installed Rust 1.91 compile-only checks pass.

The additional 720 ordinary cases cover all eight aggregation operators, by/without, nesting, joins,
native/mixed values, warnings and changing membership. Together with the plain grouped control they produce
1,442 fresh complete before outcomes on signed fixtures. Existing owner baselines retain their actual
source/binary attribution. The 56,124 fresh reducer outcomes equal exact retained analysis inputs; all 216
strict-original differences remain failures under strict comparison, including membership and point-count
differences. The unchanged approved ordering-policy analyses are reused with historical attribution.

| Query | Workspace acquired before → after, bytes | Workspace peak before → after, bytes | Release ratio | Release-min ratio |
| --- | --- | --- | --- | --- |
| One group/input | 14,584 → 13,938 | 7,406 → 7,406 | 0.893 | 1.007 |
| 2,048 groups | 7,971,742 → 6,638,666 | 1,594,434 → 1,594,434 | 0.901 | 0.914 |
| 512 inputs, one group | 1,186,046 → 1,054,974 | 262,298 → 262,298 | 0.910 | 0.933 |
| 4,096 groups, sum | 42,837,112 → 32,048,248 | 4,240,520 → 4,240,520 | 0.896 | 0.877 |
| 4,096 groups, abs(sum) | 77,665,255 → 56,088,844 | 6,020,640 → 6,020,640 | 0.871 | 0.882 |

Memory columns are paired release page17 observations. Tracked allocation traffic is distinct from peak RAM and
process RSS; the large selected peaks remain unchanged. Across all 1,339 observation-bearing release cases, 654
reduce workspace peak and 1118 reduce acquired/released workspace bytes. All eight categories retain equal
current/overflow and nonincreasing peaks/traffic in paired native comparisons.

Timing ratios divide current by before median engine time. The declared 32 cases per profile use five
alternating pairs, one warmup and one measured result per case/process after owned qualification closes: 1,280
complete outcomes and 64 groups. 55 primary medians are no higher and 9 are higher. All slower cases remain in
the report. Every ratio at least 1.05 receives twenty measured repetitions per side; the follow-up retains 132
complete outcomes across 3 groups. Shared-host observations do not establish universal, live or concurrent
speedups.

The repeated ratios span 0.994–1.011. Both primary and repeat panels remain visible without attributing their
differences to a cause.

Actual default C output-caller text changes by +7,280 bytes in release and +224 bytes in release-min; their
data/BSS changes and all eight matched fixture/C linked pairs are retained. No persistent type fields changed,
so no additional diagnostic type-layout compilation was performed. Foreign checks establish selected behavior
under QEMU, not physical ARM throughput.

From src/crates/promql-engine, the focused regressions run with:

```sh
cargo test --locked --features fixtures --release borrowed_group_hash_
cargo test --locked --features fixtures --profile release-min borrowed_group_hash_
```

The [borrowed lookup qualification](borrowed-aggregate-qualification.json) records source/binary identities, all
timing/resource rows and retained local raw-evidence paths. Public focused tests are portable; complete
comparative captures remain local, and no new whole-panel replay runner is provided. Existing input generators
and conformance procedures retain their original roles. No new metadata provider, persistent label owner, raw-
history array, implicit cap or operator workflow is introduced.

## Borrowed aggregate-result labels

Ordinary aggregate finalization now borrows clean immutable roster labels. The private sample label owner reuses
the earlier borrowed/owned representation; consumers copy only for real mutation or retained ownership. This
removes copies discarded by existing-descriptor lookups. Raw empty values keep the existing copied/reset
fallback after numerical or native-histogram finalization.

Samples and pending actions carry the frozen roster lifetime, independent of mutable Machine frames. Short
replay lifetimes stay short. New descriptors and count_values own their labels; failed conversion releases
partial copies, and failed descriptor preparation discards its provisional local index. Borrowed no-op reserved-
label removal checks the initial name and scans without copying. Real removal invokes the existing policy after
checked materialization. Selector, mutable-cache, ranking and public result owners retain their existing behavior.
Frozen projection emission is covered in the following section.

Both native profiles pass 744 units with two unchanged ignored tests, 1,487 complete cases at pages 1/17/1024,
and 336 actual default-feature C query cases in total. Each ARM target passes 18 focused owner units and 288
complete selected queries under QEMU. The combined current query count is 9,498. Formatting, strict
default/fixture Clippy and installed Rust 1.91 compile-only checks pass. Five new controls cover source
lifetime, machine re-entry, conversion interruption/recovery, normalization, reserved removal and independent
public results.

The 84 additional physical-owner cases cover nested maps, both name-removal modes, count_values, ranking, joins,
set operations, projections and subqueries. They produce 168 fresh complete signed-before outcomes; two explicit
info profile-gate errors do not establish successful info coverage. Other baselines retain their actual
source/runtime identities. The 56,124 fresh reducer outcomes equal exact retained analysis inputs; all 216
strict-original differences remain failures, including membership and point-count differences. Unchanged
approved ordering-policy analyses keep their historical attribution.

| Query | Workspace acquired before → after, bytes | Workspace peak before → after, bytes | Release ratio | Release-min ratio |
| --- | --- | --- | --- | --- |
| One group/input | 13,938 → 13,839 | 7,406 → 7,406 | 0.978 | 1.104 |
| 2,048 groups | 6,638,666 → 6,430,880 | 1,594,434 → 1,594,434 | 0.975 | 0.991 |
| 512 inputs, one group | 1,054,974 → 1,054,974 | 262,298 → 262,298 | 0.980 | 1.005 |
| 4,096 groups, sum | 32,048,248 → 27,645,048 | 4,240,520 → 4,240,520 | 0.896 | 0.945 |
| 4,096 groups, abs(sum) | 56,088,844 → 49,924,364 | 6,020,640 → 6,020,640 | 0.967 | 0.998 |

Memory columns are paired release page17 observations. Allocation traffic is distinct from peak RAM or process
RSS; selected large peaks remain unchanged. Across all 1,421 observation-bearing release cases, 0 reduce
workspace peak and 1163 reduce acquired/released workspace bytes. All eight categories retain equal
current/overflow and nonincreasing peaks/traffic in paired native comparisons.

Timing ratios divide current by before median engine time. The declared 32 cases per profile use five
alternating pairs, one warmup and one measured result per case/process after owned qualification closes: 1,280
complete outcomes across 64 groups. 40 primary medians are no higher and 24 are higher. Every ratio at least
1.05 receives twenty measured repetitions per side; the follow-up retains 88 complete outcomes across 2 groups.
Both panels retain every slower result; shared-host measurements do not establish universal, live or concurrent
speedups.

Repeated ratios span 0.972–0.992; the report retains both primary and repeat results without attributing their
differences to a cause.

Actual default C output-caller text changes by -3,272 bytes in release and +3,020 bytes in release-min. All
eight matched linked pairs and data/BSS/file costs remain visible. Separate actual diagnostic builds on AMD64,
ARM32 and ARM64 find all 57 existing type-size/alignment pairs unchanged; the new private enum matches
OwnedLabels on each target. Diagnostics are excluded from normal runtime binaries and timing.

From src/crates/promql-engine, run the focused regressions with:

```sh
cargo test --locked --features fixtures --release sample_labels::tests::
cargo test --locked --features fixtures --profile release-min aggregate_label_borrows_outlive_machines_and_materialize_at_descriptor_boundaries
```

The [aggregate-result label qualification](aggregate-result-labels-qualification.json) records exact
source/binary identities, actual layout comparisons and every timing/resource/code-cost row. Portable focused
tests are committed; full comparative captures remain local, with no new whole-panel replay runner. This
introduces no new metadata provider, retained label cache, raw-history array, implicit cap or operator workflow.

## Borrowed projection-result labels

Productive label_join and label_replace emission now borrows exact immutable prepared descriptor labels through
the existing SampleLabels representation. Binding lookup completes before a terminal control check. It adds no
representation, owner, field or cache. Projection bytes are already prepared; aggregate empty-value
normalization must not be applied here.

The frozen roster outlives Machine frames and re-entry. Actual label mutation, first descriptor retention,
count_values and independent public collection keep checked/accounted materialization. The extended real owner
test first fails on the unchanged producer, then checks borrowed lifetime, no-copy existing retention, failed
conversion/recovery, final-propagation interruption and output ownership after internal owners end. Both
delayed-name modes and pages1/17 are exercised.

Both optimized native profiles pass 744 units with two unchanged ignored tests, 1,599 complete cases at pages
1/17/1024 and 240 actual default-library C cases each. Each ARM target passes 18 focused owner units and 400
complete queries under QEMU. The 10,394 matched outcomes remain separate from 960 selected original-associated
projection replays. Strict original disagreements and approved ordering analyses retain their historical
attribution; broader historical reducer failures remain failures. Formatting, strict default/fixture Clippy and
installed Rust 1.91 compile-only checks pass.

| Release page17 case | Workspace acquired, before → current | Workspace peak, before → current |
| --- | --- | --- |
| label_join | 52,882 → 41,884 B | 14,192 → 13,360 B |
| label_replace | 61,391 → 50,411 B | 14,192 → 13,360 B |
| abs(label_join), actual removal | 108,250 → 108,250 B | 18,111 → 17,871 B |
| sum-512 control | 1,783,544 → 1,783,544 B | 455,410 → 455,410 B |

Across 9,978 observation-bearing cohort comparisons, 920 lower workspace allocation traffic and 616 lower
workspace peak. None increases a recorded memory category. Actual removal can keep unchanged traffic. These
counters measure query allocations, not process RSS; provider/statistic observations remain unchanged.

The timing panel retains 40 groups across both native profiles: 1,600 full queries, five alternating pairs, one
warmup plus three measured requests per case/arm. All complete results compare exactly. 5 primary medians are
higher; every ratio at least 1.05 receives a separate 20-sample-per-side follow-up (0 groups, 0 queries).
Primary and repeat samples remain visible. Engine elapsed excludes result serialization/compression; these are
shared-host fixture timings, not default-library, concurrent or live-provider throughput.

Large ordinary forms retain 4,096 series. Repeated grouped joins use 256 after the original 4,096-series grouped
projection baseline took 108–111 seconds per query and fetched 151,080,960 entries from 12,288 fixture points.
Its stopped capture retains 75 complete prefix results and the interrupted final row with verified byte hashes.
It is a failed performance witness, not a passing large-group timing panel. Repeated-read remediation is a
separate improvement.

| Timing case | Release current/before median | Release-min current/before median |
| --- | --- | --- |
| 1-16-join | 0.850 | 0.836 |
| 4096-512-join | 0.869 | 0.876 |
| 4096-512-real-removal | 0.986 | 1.046 |
| 256-512-grouped-join | 0.998 | 1.024 |
| 4096-512-plain-control | 0.958 | 1.014 |

The qualification retains all eight matched linked fixture/C executable pairs, including text/data/BSS/file
costs. No new internal representation or layout measurement is introduced. The actual default C callers cover
histogram/output owners and 72 owned-label cases per profile, including 24 productive projection cases; this
does not requalify every ABI caller.

From src/crates/promql-engine, run the extended source test in either optimized profile:

```sh
cargo test --locked --features fixtures --release aggregate_label_borrows_outlive_machines_and_materialize_at_descriptor_boundaries
```

The [projection-result qualification](projection-result-labels-qualification.json) records source, runtime and
complete resource/timing identities. New query captures stream to gzip; full EOF comparison verifies raw
hashes/counts and compressed hashes. Resource baselines match page, architecture and profile; elapsed strings
become integers only in analysis. The source tests are committed; full comparative captures remain local. No
public API, provider, storage, numerical, language or implicit-cap contract changes.

## Completed vector-sort publication

`sort`, `sort_desc`, `sort_by_label` and `sort_by_label_desc` complete their existing sorting callbacks before
checking duplicate hashes. Through 32 eligible inputs, publication retains direct pairwise comparisons.
Larger permutations use a temporary accounted position index over the existing cached full hashes. It stores
physical input positions, leaves the permutation untouched and retires before snapshot publication.

Duplicate identity remains hash-only, including unequal labels with the same full hash. Numeric sorting excludes
histograms; label sorting includes them. Delayed name removal bypasses publication validation only for equal
start/end bounds. Child, string and comparator errors still precede a pending duplicate, and a decisive duplicate
precedes cancellation triggered by its comparison charge. The common terminal poll prevents a cancelled query
from publishing a snapshot. Exact time/start/end/step/position cache keys, hits, invalidation and range warnings remain.

The actual unchanged-runtime owner regression first checks the complete permutation, snapshot key, stage, warning
and release, then fails its growth bound at 8,128/130,816 work units for 128/512 rows. The indexed owner charges
1,662/6,749 instead. This isolates publication work; it does not measure the earlier sorting callbacks or whole-query speed.

Controls cover 32/33/43 boundaries, sparse and nonmonotone physical positions, repeated inputs, actual unequal-label
hash collisions, zero cached hashes, delayed gates, every snapshot-key component, same-key hits and native/mixed
membership. Interruption is injected at all 1,580 charged boundaries in the native owner panel, including promotion,
rehashing and the terminal charge. Every interrupted publication releases its temporary lookup and retains no snapshot.

Complete-query failure controls use independent counters for each source. A fault on the last source's second
acquisition reaches the sort scan after raw preparation succeeds. Numeric faults on the first comparator operand's
third acquisition leave the entire scan complete before comparison fails. A fixed first-page fault would fail raw
preparation and does not establish these sort phases. Thirty actual fault/no-fault outcomes check first errors,
abort without output, retained error bytes and complete closure at pages 1/17/1024.

The retained pinned source panel covers special float values, native filtering/inclusion, natural label order and
nested string errors. Those originals are below the lookup cutoff. A separate accepted-engine comparison pads
special values above it and exercises 32-float-plus-histogram, 33-float-plus-histogram and 33-histogram inputs,
both delayed modes and both query modes. Complete row order, values, histograms and diagnostics remain unchanged.

The complete matched panel contains 3,960 before/final responses across 660 query/profile groups: five routes,
11 cardinalities from 0 to 512, instant/range execution and pages 1/17/1024. All outputs, diagnostics, sample/page/call
counts, output samples, backing use and final release match. All 660 whole-query workspace peaks are unchanged.
Through 32 rows, work and acquired workspace bytes are unchanged; above 32, work falls and allocation traffic rises.

At 512 rows with page size 17, complete numeric sort work falls from 1,635,417 to 1,511,463 for an instant query
and from 95,401,562 to 94,657,838 for three range ticks. Label sort falls from 856,934 to 732,858 and from
91,062,074 to 90,317,618. These complete-query savings are smaller than the isolated publication savings because
sorting callbacks and sorted-row replay still perform their original reads.

| Complete route, 512 rows | Workspace peak, before and final | Extra acquired workspace bytes |
|---|---:|---:|
| Numeric sort, instant | 672,288 | 16,352 |
| Numeric sort, three range ticks | 758,304 | 98,112 |
| Label sort, instant | 672,320 | 16,352 |
| Label sort, three range ticks | 758,336 | 98,112 |
| `abs(sort(m))`, three range ticks | 1,042,976 | 147,168 |

The additional bytes are allocation traffic, not increased peak or process RSS. The lookup is temporary and
retired before the snapshot; neither retains raw sample values. Hash probing has no universal linear bound and
32 is a measured private policy, not a proven optimum. Sorted-row replay remains a separately tracked cost.

Timing runs begin after all 18 owned bulk qualification phases finish. Three alternating before/final pairs
retain every complete matched result, including cold tiny outliers. A separate 768-response followup at 32/512
rows uses 18 warmed measurements per phase/group; an 8,640-response empty/tiny followup at 0/1/4 rows uses 60.
Both profiles retain exact results and resource counters. Ratios below divide the before median by the final median.

| Warmed route, 512 rows and page 17 | Release ratio | Release-min ratio |
|---|---:|---:|
| Numeric sort, instant | 1.138 | 1.085 |
| Numeric sort, three range ticks | 0.988 | 1.046 |
| Label sort, instant | 1.133 | 1.142 |
| Label sort, three range ticks | 0.994 | 1.056 |

The two release range cases remain about 1.2%/0.6% slower; the first prototype also retains slower range results.
Warmed empty/tiny ratios range from 0.957 to 1.045 with unchanged work and acquired bytes. These are shared-host
synthetic measurements without CPU isolation. They do not establish universal speed, an optimal crossover, RSS
or production disk throughput, and the later followups do not replace the original observations.

The [sort publication qualification](sort-publication-qualification.json) preserves the actual source and executable
phases, initial compiler failure, corrected meaningful red, first prototype observations and final qualification.
The formatter-overlap development run is explicitly unqualified; its passing test diagnostics do not replace a
source-frozen run. Final qualification passes 608 units per profile, 128,652 built-in and 2,070 focused comparisons,
231,540 exact SRE expressions, 456 retained pinned sort comparisons and 960 expanded accepted-engine responses.
It executes 140 distinct ARM target/profile sort checks and all 18 C/C++ callers; the fresh Agent passes 672 storage
records and 82 live HTTP/readiness checks. All 47 preceding ledger objects and 114 preceding C/ARM link inputs
remain unchanged. Foreign checks execute the library and callers; caller sanitizers exclude Rust instrumentation.
Historical cohorts retain their original identities.

## Completed vector-sort emission

Bare `sort`, `sort_desc`, `sort_by_label` and `sort_by_label_desc` range queries can reuse a completed delivery
certificate for their exact sorting snapshot. Eligibility requires an unchanged direct selector, unique float rows,
consistent single-source metadata, matching effective grids and positive forward execution with unequal bounds.
Instant queries, computed sort inputs, native/mixed, shifted/modifier and uncertified cases retain ordinary supported
evaluation. Eligible inner sorts also use the distinct [nested-prefix procedure](#nested-vector-sort-prefixes).

The first final body reads every original sorted operand, including the suffix after a provisional selected result.
Only successful `Done`, after its last control check, grants credit. Later same-snapshot bodies walk physical
permutation positions, skip certified nonselected operands and acquire the selected catalog row afresh. Physical
input positions, catalog ordinals and canonical output ordinals remain distinct. Sparse absent targets require no
extra selected read after completed delivery.

Final execution clears any progress from preparation. Unequal bounds with one effective tick can retain an existing
preparation permutation, but the first final root Emit still runs completely. Exact time/start/end/step/position
replacement starts without credit. An old live owner cannot credit a replacement. Effective lookback, precision,
name handling and storage bounds remain fixed by the eligible query scope.

One Plan bool and one snapshot progress word retain the state; the Step has no new field. Snapshot/Strong thread
traits remain intact, while the Plan remains confined to serial use. No raw values, history or new membership array
is cached. The shared schedule predicate reads borrowed metadata and uses the same checked arithmetic directly,
without constructing an evaluator solely to prove eligibility.

The complete-query read/work regression checks all labels, float bits, clocks, canonical order, logical participation,
actual provider reads, one concurrent cursor and release before applying its growth bound. An independent terminal
regression reproduces the premature Emit-to-Done credit and verifies that abandonment and final-charge cancellation
grant none. Actual query controls retain suffix failure after one provisional result and later selected failure after
42 earlier results; both abort without committing. Diagnostic ownership is released explicitly before checking that
only the returned provider error remains.

Sparse/domain, exact-key/old-owner, fresh-value/missing-row/kind, metadata and actual Machine-context controls
complement those query failures. Interruption at all 190 cold/warm Emit charges and six certificate charges checks
credit and release. Existing publication, callback, hash-collision, natural-order and native-membership controls
remain separate. Whole-query memory, allocation traffic and elapsed time require complete matched measurements;
these owner controls do not establish process RSS, production disk throughput or a universal work bound.

The historical full-Done qualification below retains its original source/binary identity; it is not a fresh run of
the nested-prefix extension. Its complete matched panel contains 4,392 responses across 732 query/profile groups,
with three alternating before/final pairs, both profiles, pages 1/17/1024 and cardinalities 0 through 512. It includes
36 unequal-bounds, single-effective-tick cases per profile. All ordered results, diagnostics, output samples, backing
and release match.
All 288 eligible groups reduce reads, pages, provider calls, work and acquired workspace bytes. The 444 fallback
groups preserve reads and work. Every measured native workspace peak rises by exactly 8 bytes.

| Complete range route, 512 rows, page 17, three ticks | Before | Final |
|---|---:|---:|
| Numeric sort fetched samples | 2,533,596 | 183,507 |
| Numeric sort work | 94,657,838 | 9,272,295 |
| Numeric sort acquired workspace bytes | 519,335,396 | 45,400,829 |
| Numeric sort workspace peak | 758,304 | 758,312 |
| Label sort fetched samples | 2,374,656 | 24,567 |
| Label sort work | 90,317,618 | 4,932,075 |
| Label sort acquired workspace bytes | 487,282,528 | 13,347,961 |
| Label sort workspace peak | 758,336 | 758,344 |

Numeric sorting still performs its original comparator reads. Skipped operands still require permutation walks;
this does not establish linear whole-query work. Allocation traffic is cumulative requested bytes, not retained
memory. The separate borrowed schedule-proof extraction saves 960 acquired bytes per eligible query in the native
prototype while preserving work, reads and peaks. Current compiled payload/accounted sizes are Plan 168/192,
Snapshot 96/128 and Step 4,704/4,728 bytes on AMD64/ARM64; ARM32 measures 112/128, 80/112 and 4,088/4,104.
The retained old query executable has no DWARF; those current layouts are not measured before/after type-size claims.

The original fully qualified candidate also retains 768 warmed complete comparisons and 8,640 empty/tiny
comparisons. At 512 inputs and page 17, its before/candidate time ratios are:

| Complete query | Release | Release-min |
|---|---:|---:|
| Numeric sort, instant | 0.995 | 0.952 |
| Label sort, instant | 0.989 | 0.967 |
| Numeric sort, three range ticks | 11.964 | 11.753 |
| Label sort, three range ticks | 35.232 | 35.404 |

Ratios below one are slower. Empty/tiny ratios range from 0.818 to 1.306; the slowest empty numeric instant
case changes from 6,590 to 8,060 ns. These shared-host observations have no CPU isolation. They do not establish
timing causality, universal speed, an optimal crossover, RSS or production disk throughput. Later trials retain
separate source and executable identities and do not replace these observations.

A subsequent private dispatch trial passed 90 sort-focused checks and 1,920 complete comparisons with identical
outputs and resource counters. Its 512-input instant ratios against the selected candidate were 0.971/0.999 in
release and 1.017/1.021 in release-min for numeric/label sorting. Its 32-input range controls were also mixed.
The trial was rejected; the selected source is exactly the original qualified candidate. Its own source, binaries,
complete outcomes and slower observations remain separately bound in the qualification record.

[Completed sort-emission qualification](sort-emission-qualification.json) preserves the meaningful query-growth red,
the terminal-credit red, earlier compiler/test-expectation/lint phases and all actual source/binary identities.
Final checks pass 618 units per profile, 128,652 built-in and 2,070 focused comparisons, 231,540 exact SRE evaluations,
456 retained sort originals and 960 expanded special/native responses. All 18 C/C++ callers and 180 distinct ARM
checks pass; the rebuilt/relinked Debug Agent passes 672 storage records, the canonical 82 HTTP/readiness checks
and a separate additive 88-check HTTP probe, including six complete sort-related results over its own DBengine data.
Historical ledger and caller inputs remain preserved. Foreign checks execute the library/callers, not a full foreign
Agent; caller sanitizers exclude Rust instrumentation. These finite panels include no CI or arbitrary-input proof.

## Nested vector-sort prefixes

An eligible inner sort in a range expression such as `abs(sort(m))` can reuse a successfully delivered prefix.
The prefix counts positions in its sorted permutation, rather than the largest physical or catalog ordinal seen.
Selected values are always read afresh; unverified suffixes retain ordinary acquisition and failure behavior.
Root sorts still require successful full `Done`. Computed sort inputs and other uncertified shapes remain supported
through ordinary evaluation. No raw values, history, membership array, dependency or public API is added.

The existing direct-selector proof is installed after each sort node completes preparation. Parent preparation
can reuse progress, then the existing roster loop clears it after all preparation Machines drop. Final execution
therefore starts cold even when unequal bounds contain one effective tick and retain the same permutation key.
An old live Step credits its exact snapshot. A replacement starts at zero; partial progress cannot revoke Done.
The single atomic word replaces the old bool within unchanged measured Plan/Snapshot/Step layouts on AMD64,
ARM64 and ARM32. The Plan remains serial; the snapshot keeps its existing sharing traits.

[Nested-sort qualification](nested-sort-prefixes-qualification.json) binds:

- A meaningful old-runtime growth failure after all 18 complete result/statistics/release checks, followed by the
  passing candidate. Sparse nonmonotone permutations, fresh values, absent targets, exact keys, old live owners,
  preparation reset and every calibrated prefix interruption boundary have separate causal controls.
- Actual four-series queries at pages 1/17/1024 preserve preparation-prefix, first-final, unverified-suffix and
  selected-value failures with respectively 0/0/1/2 provisional samples, no commit and complete cleanup.
- Both optimized native profiles pass 643 library tests each, 1,644 complete before/candidate results and 456
  unchanged pinned Prometheus sort comparisons. The panel includes changing ranks, gaps, staleness, ordinary NaNs,
  exact duplicate-label failures and coupled numeric/label/parameter/join wrappers.
- Compiled ARM64/ARM32 execution passes 840 complete query comparisons and 32 focused unit executions. Twelve
  actual C/C++ callers cover native and foreign profiles, normal delivery and provider/output failure paths.
  This is selected foreign library/caller execution, not complete foreign suites or a rebuilt Agent.

Of 1,644 native resource groups, 252 fetch fewer sample entries and acquire fewer workspace bytes; none increase
either measure. Every workspace peak stays unchanged. Eighteen tiny/error groups charge 7, 16 or 22 additional
work units. At 512 series, page 17 and three range ticks, both profiles measure:

| `abs(sort(m))` observation | Before | Candidate |
|---|---:|---:|
| Fetched sample entries | 2,622,282 | 276,792 |
| Accounted work | 98,796,797 | 13,584,489 |
| Acquired workspace bytes | 545,023,526 | 72,016,376 |
| Peak workspace bytes | 1,042,984 | 1,042,984 |

Allocation traffic is cumulative, not peak RAM. Numeric comparator reads and repeated cold sorting phases remain;
the change does not establish linear work for arbitrary nested expressions. Native query-fixture ELF text changes
by +6,208 bytes in release and −480 bytes in release-min; this is not installed Agent footprint.

The frozen timing panel checks 400 complete warm/measured responses across five alternating pairs per profile,
after qualifying eight ordinary-control responses. Each measured response must preserve its complete expected
result and that executable's own nonelapsed observations. Ratios are median before time divided by candidate time;
values below one are slower.

| Query, page 17 | Release ratio | Release-min ratio |
|---|---:|---:|
| `abs(sort(m))`, range, 512 series | 8.082 | 8.679 |
| `abs(sort(m))`, range, 128 series | 3.272 | 3.373 |
| `abs(sort(m))`, range, 2 series | 0.986 | 1.007 |
| `abs(sort(m))`, range, empty | 0.756 | 1.007 |
| `sort(m)`, range, 512 series | 0.970 | 1.001 |
| `abs(sort(m))`, instant, 512 series | 0.967 | 0.992 |
| `abs(sort_by_label(m,"id"))`, range, 33 series | 3.871 | 3.983 |
| `round(sort(m),.5)`, range, 33 series | 1.848 | 1.832 |
| `m`, range, 512 series | 0.977 | 0.981 |
| `abs(m)`, range, 512 series | 0.953 | 0.990 |

The empty release median rises from 14,380 to 19,020 ns. Every timing sample and slower group is retained; shared-host
measurements have no CPU/frequency isolation and do not establish causality for the small changes. These are engine
query timings over synthetic provider data, not HTTP throughput, production DBengine disk I/O or process RSS.
Final C-consumer warning handling changes only that test source: native Rust artifacts keep their actual earlier
compiler manifest, with all Rust/Cargo/header inputs byte-identical to final source. Earlier language, SRE, Agent
and CI cohorts retain their identities; this panel adds no language feature or arbitrary-input compatibility proof.

## Borrowed binary scheduling proof

The raw/rate binary replay gate uses the existing pure schedule predicate directly. It resolves the same selectors,
uses the same prepared effective lookback, timestamp markers and grid, and preserves left-before-right short circuit.
The former temporary evaluator only supplied these borrowed values; it performed no evaluation, work charge,
control poll or provider callback. Actual evaluators and every existing binary certification rule remain in place.

The [binary scheduling qualification](binary-schedule-qualification.json) compares 1,020 complete current results
with the accepted engine: raw/raw, rate/rate, mixed, computed, set, offset and fixed-time expressions, cardinalities
0/1/4/32/128, pages1/17/1024, both right-side orders and instant/range execution. Native add/equality and precise
lookback controls retain their complete accepted results. Analytic float checks verify membership, values and clocks;
all original row order and diagnostics are compared unchanged between executable phases.

Exactly 540 query/profile cases save 960 acquired and released workspace bytes; 480 cases retain their allocation
counts. All measured whole-query workspace peaks remain unchanged. Outputs, reads, work, provider/output calls,
temporary backing, references and cleanup also match. This reduces allocation traffic without retaining new state,
sample values or history; it does not establish lower peak RAM or RSS.

Current checks pass 618 units in each native profile, formatting, both lint configurations and the Rust1.91 compile
check. Four actual native C/C++ callers link and execute against the newly built static archives. Existing binary
and rate first-pass, selected-pair, failure, clock and cleanup controls remain part of those units/callers. Earlier
language, SRE, ARM and Agent cohorts keep their original source/binary identities and are not fresh runs of this change.

The first warmed panel contains 4,608 complete responses and ratios from0.872 to1.047. It retains large release
rate/mixed cases that were8–15% slower. An unchanged-source concern repeat contains2,304 complete responses with
ratios0.977–1.039. At128inputs/page17, that repeat reports:

| Complete query | Release | Release-min |
|---|---:|---:|
| Raw join, instant | 1.006 | 1.021 |
| Raw join, three range ticks | 1.017 | 1.028 |
| Rate join, instant | 1.014 | 1.015 |
| Rate join, three range ticks | 1.015 | 1.029 |
| Mixed raw/rate join, instant | 1.019 | 1.011 |
| Mixed raw/rate join, three range ticks | 1.004 | 1.021 |

Ratios are before/current median times; below one is slower. Both panels use alternating pairs, complete outcomes
and exact per-phase resource checks after owned builds/qualification close. CPU is not isolated. The repeat does
not replace the initial observations or prove their cause; no universal throughput or production-disk claim follows.
Initial analytic-expectation, active-parent inventory and timing-selection failures retain their actual development
phases. Corrected qualification binds closed writers and the actual current source and executables.

## Completed direct abs join scans

This section retains the initial abs-only cohort and its actual source/binary identities. The current typed class
and broader evidence are in [direct float-map joins](#completed-direct-float-map-join-scans).

The existing completed one-to-one float join route also accepts a direct `abs(selector)` operand. The operand must
have earned the existing scheduled-selector certificate, including prepared-child and projected-label uniqueness.
Its wrapper and leaf producer/source/scope grids must match the evaluator. The ordinary selector scheduling proof,
root/matching restrictions and unique-float side/pair certificates remain authoritative.

The first final body still completes discovery, arithmetic, filtering, duplicates and diagnostics before publishing
reuse. Later rows acquire their selected wrapper operands afresh. Wrapper ordinals retain their existing mapping to
physical sources; no raw values, timeline, new cache representation or provider requirement is introduced.

`abs` ignores native histogram points. A changing float/native history can therefore qualify through its actual
float output rows. Sparse ordinals, stale membership and different side orders remain significant. Projected aliases
that fail the existing uniqueness proof retain ordinary evaluation: disjoint float/native aliases can still succeed,
while an unwrapped side retaining both metric names can report the original many-to-one error.

The [abs join qualification](abs-join-qualification.json) records 1,854 complete native accepted-engine matches and
3,708 compiled ARM64/ARM32 matches across both profiles. It covers left/right/both operands, negative values and
signed zero, comparison/boolean results, native/mixed histories, aliases, fractional/precise clocks and rate peers.
Nested, indirect, fixed, grouped and delayed-name controls retain their existing evaluation routes.

Both native builds pass 621 units. Each foreign build runs the three focused owners. Twelve actual C/C++ callers
include 27 direct abs cases each. The cold/warm unit independently measures preparation acquisitions, then checks
later discovery, later pair delivery and fresh second-canonical selection failures. The warm failure checks confirm
that unrelated sources were not reacquired. These qualify compiled library/caller behavior; earlier language, SRE, oracle
and containing-Agent cohorts retain their actual historical identities.

For `abs(m) / on(id) n`, distinct negative inputs, reversed right order, page17 and three range ticks:

| Series per side | Fetched entries before → after | Recorded work before → after | Workspace peak before → after |
|---|---:|---:|---:|
| 32 | 51,072 → 4,200 | 1,403,196 → 138,777 | 105,022 → 105,022 B |
| 128 | 794,112 → 16,872 | 22,528,213 → 607,043 | 409,002 → 409,002 B |

The 128-series case reduces acquired workspace from 154,371,524 to 5,111,263 bytes. Allocation traffic is cumulative;
it is not retained RAM. Across the complete native panel, 330 query/profile groups fetch fewer entries, and
168 groups raise workspace peak by at most 172 bytes to retain membership metadata. Empty and single-series cases
remain unchanged; the larger representative peaks above also remain unchanged. Raw samples stay paged on both axes.

The qualification separately retains 3,840 warmed complete-query responses for empty/single/four/128-series inputs,
raw/left/right/both-abs/fixed shapes, instant/range execution and both profiles. Timings use alternating pairs with
complete result/resource checks. Native qualification had finished; foreign fixture runs could still be active.

| Profile | 128-series range shape | Median before → after | Before/after ratio |
|---|---|---:|---:|
| release | Left abs | 494.35 → 13.18 ms | 37.49× |
| release | Right abs | 493.99 → 13.05 ms | 37.85× |
| release | Both abs | 554.60 → 15.35 ms | 36.14× |
| release-min | Left abs | 856.12 → 21.65 ms | 39.54× |
| release-min | Right abs | 841.66 → 21.53 ms | 39.08× |
| release-min | Both abs | 955.25 → 25.43 ms | 37.56× |

The raw/fixed large controls have ratios from 0.975 to 1.016. Some small release medians are slower: the empty
both-abs instant case changes from 10,955 to 11,904.5 ns (ratio 0.920), and the four-series both-abs instant case
from 167,523.5 to 179,243.5 ns (ratio 0.935). These observations remain in the qualification.

After all owned qualification jobs closed, a targeted repeat checked 3,072 complete responses with 36 retained
samples per phase/query. Its ratios span 0.944–1.163. The release empty and four-series both-abs instant cases have
ratios 1.058 and 1.041; release-min retains slower medians of 16,495 → 17,470 ns and 268,508 → 282,637.5 ns
(ratios 0.944 and 0.950). Both panels remain intact; the repeat does not replace the initial observations or prove
their cause.

Timings measure generated inputs on a shared host, without isolated CPU or production disk/RSS claims. A final
test-comment clarification retains a distinct final-source identity alongside the actual compiled sources;
executable tokens and line counts are unchanged.


## Scheduled derived quantile rank traversals

An earned scheduled single-selector subquery classifies exact quantile ranks during its existing complete
validation traversal. Each required rank pass uses one traversal; the original qualification below required sixteen.
Membership, ordinal Fetch and provider-owned temporary materialization keep their original phases. This traversal
reuse applies to scheduled quantiles. The later [rank completion change](#early-completion-of-temporal-ranks) also
reduces unnecessary passes in ordinary quantiles and both MAD medians.

The rank counters remain provisional until the complete child driver and Reader finish successfully. A late page
error, an error after the final visitor, or terminal cancellation still prevents rank completion and output. The
small Rank-only adapter reuses the literal selector, records each sample's work immediately and adds no control
poll or allocation. Internal scheduled replays suppress annotations; preparation still owns final diagnostics.

The [rank traversal qualification](quantile-rank-traversal-qualification.json) binds 864 complete native before/after
matches, 776 compiled ARM64/ARM32 matches, 56,124 retained reducer outcomes and 14,664 retained temporal/backing/scheduled
outcomes. Original reducer order differences remain separate from the passing approved ordering reports. Both final
native profiles pass 624 units. Formatting, production/fixtures lint and the Rust 1.91 compile check pass. Four actual
native C/C++ callers pass with 32 records each. Every caller includes six derived-quantile normal/provisional-error
cases at pages 1/17/1024. Each foreign profile runs the three
new focused owner tests.

For `quantile_over_time(.5,(m*1)[4096s:1s])`, one series and page 17, distinct values are 1…4096.
The ambiguous pattern repeats −1, −0, +0 and 1 and uses the existing exact-sort backing path.

| Input pattern | Fetched entries before → after | Recorded work before → after | Workspace peak before → after |
|---|---:|---:|---:|
| distinct | 290,816 → 159,744 | 27,166,570 → 14,983,594 | 22,278 → 22,278 B |
| ambiguous | 299,008 → 167,936 | 27,980,302 → 15,797,326 | 39,134 → 39,134 B |

Across 864 native query/profile groups, 408 fetch fewer entries. Every measured workspace, provider-held and
borrowed-view peak is unchanged. At that checkpoint, the generated 2,048/100,000-point test kept flat peaks, one open
cursor and full release. Its computed ambiguous case read `41*N` at both sizes; the retained red failed at 2,048 points
with `73*N`. This is finite growth evidence. Raw values remain paged on both axes; no history array, cache, cap or
backing policy is added. The later rank completion cohort below measures the same generated history again.

The warm timing panel checks 1,200 complete responses after all owned qualification jobs close. It uses five
alternating before/current pairs per profile, one warm run before each measured query, and retains empty/small/raw
controls, page effects and both instant/range modes. The 4,096-point instant examples above report:

| Profile | Input pattern | Median before → after | Before/after ratio |
|---|---|---:|---:|
| release | distinct | 685.68 → 368.04 ms | 1.86× |
| release-min | distinct | 1068.80 → 578.66 ms | 1.85× |
| release | ambiguous | 726.87 → 391.01 ms | 1.86× |
| release-min | ambiguous | 1093.40 → 617.57 ms | 1.77× |

All 60 median ratios span 0.881–1.909. Eleven medians are slower. The lowest ratios include
`quantile-distinct-n1-page17-raw-instant` (release, 23,629 → 26,830 ns, 0.881×) and
`quantile-distinct-n0-page17-computed-range` (release-min, 31,160 → 34,320 ns, 0.908×).
An unchanged-source follow-up checks 1,872 complete responses for six affected/matched tiny controls, with 72 measured
samples per phase/group. Its ratios span 0.994–1.044; the slower median is
20,805 → 20,929 ns for `quantile-distinct-n1-page17-raw-instant` (release).
Both panels remain intact. The follow-up does not replace the first observations or establish their cause.
These are generated shared-host engine timings, without isolated CPU, production disk or process-RSS claims.

A test-local callback type alias fixes a lint failure. Fresh final native query binaries and archives are byte-identical
to the fully qualified prototype artifacts; the reports retain both source identities. Final units, native callers
and foreign queries execute the final sources. The first timing harness attempt stopped before any query because it
selected the wrong active fixture dataset; its failed artifacts remain separate from the corrected measurements.
Earlier full language/SRE/Agent/foreign-C cohorts retain their original source and binary identities.


## Exact rank work accounting

The shared temporal quantile/MAD selector now records its three unconditional work units through one existing
`work(3)` call. No control check, callback or fallible operation separated the original calls. Literal key, digit,
lane and numerical operations retain their order. The caller's separate charge and all finish-pass charges remain.
Source inspection establishes two fewer accounting-mutex acquisitions per classified float; it is not a measured
whole-query lock count. This change adds no field, allocation, raw history, cache or query cap.

The [work-accounting qualification](rank-work-accounting-qualification.json) binds 936 complete native matches and
800 compiled ARM64/ARM32 matches. Every exported native nonelapsed observation matches exactly, including work,
page reads, output counts, all memory owners and temporary backing. The baseline includes 72 independently
analytic MAD outcomes. Both native profiles pass 626 units, format/lint/MSRV checks and four C/C++ callers with
32 records each. Each foreign profile executes five focused tests. All 56,124 retained reducer and 14,664 retained
temporal/backing/scheduled outcomes pass their existing declared contracts; raw ordering differences stay separate.

The two new tests also pass with the original charges. They check saturation near `u64::MAX`, sticky overflow,
unchanged memory and interruption at the next existing poll. This is an optimization, with no claimed previous
correctness failure. Concurrent snapshots may skip the old intermediate work+1/work+2 states inside the helper;
completed work and actual control boundaries remain unchanged.

Warm measurements run after all owned qualifiers close: 38 cases per profile, five alternating before/current
pairs, one warm then one measured query for each active dataset, and 1,520 complete checked responses. Examples are:

| Profile | Case | Median before → after | Before/after ratio |
|---|---|---:|---:|
| release | `quantile-distinct-n4096-page17-raw-instant` | 9.268 → 9.282 ms | 0.998× |
| release | `quantile-distinct-n4096-page17-computed-instant` | 366.945 → 365.922 ms | 1.003× |
| release | `stage35-mad-n512-page17-raw-instant` | 2.297 → 2.124 ms | 1.081× |
| release-min | `quantile-distinct-n4096-page17-raw-instant` | 13.287 → 12.695 ms | 1.047× |
| release-min | `quantile-distinct-n4096-page17-computed-instant` | 577.227 → 576.876 ms | 1.001× |
| release-min | `stage35-mad-n512-page17-raw-instant` | 3.247 → 2.978 ms | 1.090× |

All 76 median ratios span 0.954–1.101; 32 medians are slower. The two lowest ratios are:

- `stage35-mad-n512-page17-computed-range`, release: 474.231 → 497.171 ms, 0.954×.
- `quantile-distinct-n0-page17-raw-range`, release-min: 19,649 → 20,560 ns, 0.956×.

The slower computed MAD range triggers an unchanged-source follow-up: six fixed affected/matched cases, 624 checked
responses, 24 measured samples per variant/group, ABBA phase order in release and BAAB in release-min. Its 12 median
ratios span 0.998–1.368. The release computed MAD range repeats at 541.494 → 479.502 ms (1.129×).
Both panels remain intact; the repeat does not establish the earlier cause.
Repeat phases also vary on empty controls that classify no floats. These ratios do not isolate the saved locks or
establish a universal quantile speedup.
The public receipt retains every timing sample and median. These are generated shared-host engine timings,
without CPU isolation, production DBengine or process-RSS claims. Accepted baseline binaries are verified copies
of the preceding stage's actual final builds, with their original compiler receipt. Candidate builds are fresh.
Query JSON does not export basic statistics; their unchanged owners retain unit and C assertion coverage.
No original-Go capture, live Agent, SRE replay or full foreign C caller roster was rerun for this private change.
Earlier cohorts keep their actual source and binary identities; all 52 previous ledger objects are unchanged.


## Early completion of temporal ranks

This historical cohort introduced uniform-population completion. The later
[common-prefix refinement](#common-prefix-temporal-ranks) changes which inputs need full-depth replays.
Its measurements below retain their original source and binary identities.

The shared temporal quantile/MAD selector stops refining a rank when a complete successful replay proves that both
candidate populations each contain one normalized key. Repeated values can therefore finish before all sixteen
four-bit passes. This changes physical reads and work; selected ranks, interpolation and logical sample participation
keep their existing owners. The [rank completion qualification](rank-completion-qualification.json) records the
actual source, binary, result, resource and timing cohorts.

The selector reuses its two prefix words to retain the first admitted full key in each lane. Admission masks both
sides, so the saved lower bits cannot change that pass's population. Seen flags distinguish a valid zero key from
an absent population; nonuniform flags remain sticky after any differing key. Both ordinary rank/control validations
run before completion. When another pass is needed, reconstruct each prefix from its old high bits and selected
digit, clearing all lower bits. The first admitted key need not have the selected digit.

This proves normalized keys, not original NaN payloads or zero signs. Membership, ordinal Fetch and exact
provider-owned sort backing still decide original provenance. Scheduled counters remain provisional until the entire
child driver and Reader succeed. A later page failure, post-visitor error or terminal cancellation still prevents
completion. Successful completion may omit future redundant physical passes; no fixed callback roster is promised.

This cohort used adjacent low-bit keys and positive subnormals for sixteen-pass temporal and thirty-three-scan MAD
controls. The later refinement retains these numerical cases and uses prefix ladders for genuine full depth.
They also cover missing ranks, cancellation between lanes, reversed inputs, asymmetric
lanes, early two-class completion and invalidation after the last scheduled visitor. The long-history sources generate
one page at a time; fixture JSON datasets are qualification inputs, not evidence that the runtime retains raw history.

Default-feature layout diagnostics on native AMD64, ARM32 and ARM64 keep the selector at 296 bytes, temporal Step
and derived Kernel at 440 bytes, and MAD Median/Step at 352/368 bytes. Enclosing derived Step and evaluator Frame
also retain their measured sizes. No heap owner, raw-value cache, provider policy, API, ABI or implicit cap is added.
These type measurements do not establish a whole-function stack bound or process RSS.

Both native optimized profiles pass 729 units with two ignored measurements each. Four rebuilt query cohorts match
40,192 complete original outcomes: native release/release-min at pages 1/17/1024 and compiled ARM32/ARM64 release
under QEMU at page 17. Each foreign target executes twelve focused units. Four actual default-feature GNU C callers
check generated backed queries, existing calibrated failures and raw/computed four-series quantiles; native C
observations match the previous archives. Formatting, strict default/fixture Clippy and installed Rust 1.91 compilation
pass. The MSRV check is compile-only; these C runs add no C++ or sanitizer claim.

For the generated 100,000-point mixed-zero history at page 1:

| Route | Fetched entries before → after | Recorded work before → after | Workspace peak before → after |
| --- | ---: | ---: | ---: |
| raw | 3,600,038 → 800,010 | 14,336,255 → 3,136,143 | 30,296 → 30,296 B |
| computed | 4,100,000 → 1,300,000 | 644,974,257 → 194,172,465 | 39,100 → 39,100 B |

All twelve generated size/page/route rows reduce reads and work at unchanged peaks. The 2,048/100,000-point controls
retain exact results, logical counts, one active cursor/page and balanced cleanup. A separate 512-point discrete
computed case reduces fetched entries from 19,968 to 5,632 with the same original outcome, crossing its synthetic
15N read regression bound. That test bound is not an enabled query limit.


The required reducer interaction check also passes 56,124 comparisons under its separate approved ordering
rules. Its strict reports retain 216 original ordering differences and zero numerical bit differences.

Paired timing starts after every owned qualification job closes. Each of 54 cases per native profile has five
alternating process pairs, one warm and one measured query per case/process. All 2,160 warm/measured outcomes
match their freshly captured original results; nonelapsed observations repeat exactly within each phase.

`large-early` alternates 1 and 2; `large-ordinary` contains 1…4096; `large-backed` cycles −1, −0, +0 and 1.

| Profile | Computed 4,096-point instant case | Median before → after (ms) | Before/after ratio |
| --- | --- | ---: | ---: |
| release | large-early, quantile | 320.095 → 84.366 | 3.794× |
| release | large-early, mad | 1162.593 → 154.646 | 7.518× |
| release | large-ordinary, quantile | 317.178 → 168.860 | 1.878× |
| release | large-backed, mad | 1156.982 → 194.270 | 5.956× |
| release-min | large-early, quantile | 522.741 → 141.242 | 3.701× |
| release-min | large-early, mad | 1951.541 → 265.934 | 7.338× |
| release-min | large-ordinary, quantile | 531.491 → 283.852 | 1.872× |
| release-min | large-backed, mad | 1943.650 → 321.289 | 6.050× |

All 108 primary candidate/before median ratios span 0.122–1.057; 14 are slower. The qualification retains every
sample, all eight memory categories and physical counts. Generated logical/one-page assertions remain distinct from
the JSON observations. Every memory peak/current/overflow field matches; 80 groups reduce workspace allocation traffic
and 80 reduce cumulative borrowed exposure. Neither is simultaneous RAM or process RSS.

The primary cases at least 5% slower are:

| Profile | Case | Median before → after (ns) | Candidate/before ratio |
| --- | --- | ---: | ---: |
| release | `timing:single:computed:quantile:instant` | 52,840 → 55,859 | 1.057 |
| release | `timing:large-near:raw:quantile:instant` | 8,714,530 → 9,152,688 | 1.050 |
| release-min | `timing:empty:raw:mad:instant` | 11,480 → 12,139 | 1.057 |

Every primary ratio at least 1.05 receives a balanced follow-up: 3 groups, 132 complete outcomes, twenty measured
samples per phase/group. Its candidate/before median ratios span 0.949–1.008. Both panels remain intact; the repeat
does not replace primary evidence or establish a cause.

| Profile | Repeated case | Median before → after (ns) | Candidate/before ratio |
| --- | --- | ---: | ---: |
| release | `timing:single:computed:quantile:instant` | 56,384 → 53,725 | 0.953 |
| release | `timing:large-near:raw:quantile:instant` | 8,741,295 → 8,815,210 | 1.008 |
| release-min | `timing:empty:raw:mad:instant` | 11,400 → 10,820 | 0.949 |

The matched default-feature C executable adds 5,584/416 bytes of text in release/release-min, with 0/24 bytes
of data and unchanged BSS. Fixture text changes by −428/+448 bytes; its other linked costs remain in the report.
Long constant histories are semantic controls; this timing panel uses singleton/empty one-class cases. These
are shared-host engine timings at page 17, without isolated CPU, production-disk, live/concurrent throughput,
physical-ARM throughput or process-RSS claims. Earlier containing-Agent/install/language cohorts keep their
original identities. No new operator command or installation procedure is introduced.


## Common-prefix temporal ranks

The temporal quantile/MAD selector now skips groups of key bits that a completed replay proves constant. This
extends uniform-population completion: distinct nearly equal values can avoid intermediate passes while retaining
the same fixed-size state. The [qualification](rank-prefix-qualification.json) records complete results and costs
against the previous uniform-only implementation.

One byte records the minimum common leading length within both admitted populations. Each lane keeps its own
first full normalized key and seen bit. Both ordinary rank/control validations and whole child/Reader success still
precede completion. A jump beyond the ordinary next nibble fills proven bits from the representatives; without a
true jump, counted-digit reconstruction remains authoritative. The first representative can belong to an unselected
digit. The last nibble returns ordinary reconstructed full keys. NaN/zero source provenance, provider-owned exact
backing, MAD deviation arithmetic and interpolation keep their existing owners.

`rank_completion_cases.py` shares its request builder without changing any of its 540 complete objects.
`rank_prefix_cases.py` adds 540 original-engine outcomes around nibble boundaries, unselected-first reconstruction
and genuine full-depth inputs. Adjacent/subnormal and clipped old inputs remain acceleration/numerical controls.
The q=0 ladder has positive zero plus one value at every nibble; the MAD ladder adds a strict zero majority, so its
deviations repeat the full ladder. The separate clipped 35-point MAD control keeps 18 zeros and all 16 powers,
with the original seek target. All final-depth faults remain reachable.

All four rebuilt query cohorts pass 44,512 complete original comparisons. Both native profiles pass 730 units
with two unchanged ignored measurements; each compiled ARM target runs 14 focused tests under QEMU. Native
pages are 1/17/1024; foreign release uses page 17. Required reducer interactions retain 56,124 policy comparisons,
their 216 strict original differences and zero numerical bit differences. Those strict differences include
tie-driven membership and point-count changes; the separate approved ordering policy does not erase them.
Formatting, strict default/fixture Clippy and installed Rust 1.91 compilation pass; MSRV is compile-only.

Four actual default-feature GNU C callers retain existing backing/failure/four-series controls. A second native
C caller generates 2,048/100,000 adjacent values through one-page callbacks, checks complete outputs and logical
counts, and balances every owner. All eight category peaks and owning sample peaks stay flat as history grows
and equal between previous/current implementations. It performs 48 before/current queries across both profiles.
No new raw array, history cache, backing policy, API, ABI or implicit cap is introduced.

For the generated 100,000-point history at page 1 (identical counts in both native profiles):

| Route | Fetched entries before → after | Recorded work before → after | Workspace peak before → after |
| --- | ---: | ---: | ---: |
| raw | 3,400,036 → 600,008 | 33,701,395 → 5,700,607 | 11,107 → 11,107 B |
| computed | 3,900,000 → 1,100,000 | 471,102,629 → 123,900,997 | 20,753 → 20,753 B |

The existing mixed-zero history keeps all twelve observations unchanged; its earlier improvement belongs to the
uniform-only cohort. A separate 512-point computed near-key query reduces fetched entries from 19,968 to 5,632
at 20,734 bytes of workspace, crossing its synthetic 15N read bound. This bound is not an enabled query cap.

Paired engine timing starts after owned qualification jobs close. All 54 previous case objects/outcomes remain,
and four new 512-point majority-zero ladder controls preserve genuine no-jump quantile/MAD costs. There are 58
cases per native profile, five alternating pairs and one warm/one measured query per case/process: all 2,320
complete outcomes match upstream exactly. Nonelapsed counters repeat within each phase. The old family ID `full`
denotes consecutive subnormals, now an acceleration control; `prefix-ladder` is the genuine full-depth control.

| Profile | Instant case | Median before → after (ms) | Before/after ratio |
| --- | --- | ---: | ---: |
| release | large-near, raw, quantile | 8.810 → 1.580 | 5.576× |
| release | large-near, computed, quantile | 328.967 → 86.350 | 3.810× |
| release | large-near, computed, mad | 710.345 → 196.574 | 3.614× |
| release | full, computed, mad | 150.905 → 42.263 | 3.571× |
| release | prefix-ladder, raw, mad | 2.195 → 2.172 | 1.011× |
| release-min | large-near, raw, quantile | 12.789 → 2.276 | 5.618× |
| release-min | large-near, computed, quantile | 549.411 → 141.824 | 3.874× |
| release-min | large-near, computed, mad | 1146.291 → 322.665 | 3.553× |
| release-min | full, computed, mad | 248.503 → 69.801 | 3.560× |
| release-min | prefix-ladder, raw, mad | 3.078 → 3.027 | 1.017× |

All 116 primary candidate/before ratios span 0.178–1.253; 53 groups are slower.
Every category's current/peak/overflow fields match. 28 groups reduce workspace allocation traffic;
28 reduce cumulative borrowed exposure. These are separate from simultaneous RAM and process RSS.

Every primary ratio at least 1.05 receives a balanced follow-up: 4 groups,
176 complete outcomes and twenty measured samples per side/group. All primary evidence remains.

| Profile | Repeated case | Primary ratio | Repeated candidate/before ratio |
| --- | --- | ---: | ---: |
| release | `timing:empty:computed:mad:instant` | 1.253 | 1.038 |
| release | `timing:single:raw:quantile:instant` | 1.180 | 0.992 |
| release | `timing:ordinary:raw:quantile:instant` | 1.068 | 0.991 |
| release | `timing:large-backed:raw:mad:instant` | 1.077 | 1.000 |

Default-feature type diagnostics preserve the 296-byte selector and all measured enclosing Step/Kernel/Median/Frame
sizes on native/ARM32/ARM64. The actual receive symbol grows by 50/32/32 bytes on those targets; ARM32 uses two
CLZ instruction sites inside its lane loop. These are static code sites, not executed-instruction or throughput
counts. The report retains full default-C and fixture text/data/BSS/file costs for both native profiles and
ARM32/ARM64 release, measured from their already-qualified executable copies.

All timings are finite shared-host engine elapsed measurements at page 17. They do not establish isolated CPU,
concurrent throughput, live/dbengine-disk or physical-ARM performance, total process RSS or a whole-function stack
bound. Long one-class histories are not part of this timing panel. Earlier containing-Agent/install/language/SRE
cohorts retain their actual identities; no operator command or installation flow changes.


## Completed direct float-map join scans

The initial completed direct-join extension used the existing typed `PointwiseFunction::Float` class after checking
actual arity one. That cohort covered all 32 one-argument members: ordinary mathematical maps, default `round` and
explicit-vector calendars. At that checkpoint, parameterized round/clamps kept ordinary evaluation. The
[parameterized extension](#completed-parameterized-float-map-joins) records that checkpoint's additional eligibility;
the [composition extension](#completed-composed-float-map-joins) records the broader inner path.
The [earned-operand extension](#completed-earned-operand-joins) records current outer eligibility.
This earlier cohort and its numbers retain their original source and binary identities.

That checkpoint required a direct selector, an earned scheduled-selector marker, matching wrapper/leaf producer,
source and scope grids, and all existing root/matching/uniqueness restrictions. The first final body must complete
discovery, arithmetic, filtering, duplicate checks and diagnostics before reuse is published. Later rows read their
selected wrapper operands afresh. Wrapper ordinals remain distinct from physical source positions. Domain NaNs
remain present floats; native histogram points use the existing filtering owner. No raw history array is added.

The [float-map join qualification](float-map-join-qualification.json) records 10,372 complete native accepted-engine
matches and 3,656 compiled ARM64/ARM32 matches. It covers all 32 maps, ordinary and special values, native/stale
membership, aliases, boolean/filter outcomes, offsets/fixed/precise fallbacks, and pages 1/17/1024. Three invalid
clamps per native profile preserve their compile errors and absent observations. Both native profiles passed 627
units and format/lint/MSRV checks; each foreign profile ran four focused owners. Twelve actual C/C++ callers passed
36 records each with that checkpoint’s `FOUR_SERIES_FLOAT_MAP` fixture and `direct_float_map_replay` parser category.

The broadened growth test first ran against the original abs-only runtime. All 24 complete query/value/clock/label,
logical-statistics and cleanup checks passed before the fixed growth bound failed for ceil/round/year at all three
page sizes. Abs controls passed. The actual unit exit 101, a separate evidence-reader exit 1, and the successful
closed-stream finalizer retain their distinct identities. That checkpoint’s growth, sparse wrapper ordinals and independently
calibrated preparation/cold-pair/warm-selected error tests pass. The registry query additionally checks that domain
NaNs are present and compare false, while infinities retain normal comparison behavior.

Of 10,366 native resource groups, 2,514 fetch fewer entries and acquire less workspace; none fetch more entries.
For page size 17, 128 series and three range ticks, left ceil/round/year queries each change as follows:

| Observation | Before | After |
|---|---:|---:|
| Fetched sample entries | 794,112 | 16,872 |
| Accounted work | 22,528,213 | 607,043 |
| Acquired workspace bytes | 154,371,524 | 5,111,263 |
| Peak workspace bytes | 409,002 | 409,002 |

Allocation traffic and live peak are different measures. In 2,442 small range-query groups, existing completed-join
metadata raises the whole-query workspace peak by 82–312 bytes, at most 1.66% in this panel. Provider and borrowed
page peaks do not increase. Pair descriptors and one exact-scope membership bitmap are the existing reuse owners;
old completed scopes are released before replacement. Aggregate observations do not identify the allocation instant
responsible for every peak difference. This is no global peak-memory reduction or process-RSS claim. Query caps stay
disabled, and raw samples remain paged across both axes.

Another 186 mixed right-wrapper range groups reject reuse after conservative uniqueness checks. Each adds 32
cumulative workspace bytes and 16 work units, with unchanged reads and whole-query peaks. All 5,156 instant
observation groups and all 384 scaling-group peaks match exactly. These finite costs remain in the retained report.

Matched warm measurements run after owned qualification closes: 24 cases per profile, five alternating pairs,
one warm then one measured query per active dataset, and 960 complete checked responses. Each variant's nonelapsed
observations match its own qualified run. The following generated examples use 128 series, page17 and three ticks:

| Profile | Expression | Median before → after | Before/after ratio |
|---|---|---:|---:|
| release | `ceil(m) / on(id) n` | 541.970 → 13.126 ms | 41.291× |
| release | `round(m) / on(id) n` | 518.715 → 13.134 ms | 39.493× |
| release | `year(m) / on(id) n` | 539.674 → 13.289 ms | 40.611× |
| release-min | `ceil(m) / on(id) n` | 803.615 → 21.481 ms | 37.410× |
| release-min | `round(m) / on(id) n` | 818.111 → 21.467 ms | 38.111× |
| release-min | `year(m) / on(id) n` | 827.228 → 21.393 ms | 38.667× |

All 48 median ratios span 0.929–41.291; 14 medians are slower. The two lowest ratios are:

- `scaling-n0-page17-ceil-left-range`, release-min: 18,910 → 20,349 ns, 0.929×.
- `scaling-n0-page17-round-left-range`, release-min: 18,240 → 18,950 ns, 0.963×.

Every sample and median, including empty/abs/raw/rate/instant controls, remains in the public receipt. These are
shared-host engine timings, without CPU isolation, production DBengine or universal-throughput claims.
Accepted predecessor artifacts are reused with their actual original compiler receipt; candidate builds are fresh.
Earlier expression, SRE and broader integration cohorts keep their source/binary identities; all 53 prior ledger
objects remain unchanged. Query JSON does not export basic-statistics snapshots; actual unit/C assertions cover
their unchanged owners. No new original-Go capture, live Agent, full foreign unit suite or caller sanitizer claim
is made. See the [initial abs cohort](#completed-direct-abs-join-scans) for its separate earlier measurements.


## Completed parameterized float-map joins

The parameterized checkpoint added `round`, `clamp_min`, `clamp_max` and `clamp` when the existing
scheduled-selector marker proves that every scalar parameter reads no series. Literal, unary, parenthesized,
binary scalar arithmetic and `time()` follow that existing proof. The preliminary actual-arity-one restriction
is removed; the separate direct `rate` branch retains it. No new purity walk, function list or runtime state is added.

That checkpoint required the shared typed Float class, a direct selector and all wrapper/leaf producer/source/scope
grids. The [composition extension](#completed-composed-float-map-joins) below broadens the proven inner path.
Root, matching, unique-kind, clock and successful-completion gates remain in place. Parameters still evaluate
through the normal evaluator before vector acquisition. Source-reading `scalar(n)` and even `scalar(vector(.5))`
do not earn this conservative marker and retain ordinary evaluation. Zero-argument calendars remain ordinary;
indirect and clock/native helpers kept their previous paths at that checkpoint.
Current outer eligibility is in the [earned-operand extension](#completed-earned-operand-joins).

A full clamp can omit every row for a tick. Reuse keeps the current exact scope and actual membership.
With four IDs and constant inputs of 1, `clamp(a, 0, .5-(time()-2)^2) / on(instance) b` emits nothing at seconds 1
and 3, and exactly four points of 0.5 at second 2. NaN bounds remain present floats; boolean comparisons emit
positive zero. Sparse native/stale controls check independently calculated clipped values and shuffled physical
positions. Raw samples remain paged across both axes.

The [parameter-map qualification](parameter-map-join-qualification.json) records 6,664 complete native
accepted/candidate matches and 4,352 compiled ARM64/ARM32 matches at pages 1/17/1024. It covers 17 parameter forms,
ordinary/special/native/stale inputs, aliases, arithmetic/filter/boolean joins, changing membership, source-reading
scalar parameters, and retained fallbacks and one-argument maps. Errors remain errors; three invalid clamps per
native profile have no observation record. Both native profiles pass all 630 units, format, strict production and
fixture Clippy, and MSRV 1.91 compile-only checks. Each foreign target/profile runs seven focused owners. Twelve
actual C/C++ callers pass 40 records each, including four new parameterized queries in the existing Float-map category.

The old runtime passes all 48 growth-test result/label/clock/statistics/credit/release checks before its fixed bound
fails in all 12 new parameter-map/page groups. All 12 existing one-argument groups pass the bound. Current growth
and cleanup checks pass. The original baseline-reader mistake and two development test failures remain separate
from the corrected passing owners and keep their actual source/build identities.

Failure controls calibrate preparation separately from final evaluation. First- and fourth-output failures and
normal completion measure actual acquisition landmarks. They distinguish first-empty scans, the next scope,
the cold populated pair suffix, later selected reads and the last empty scope. The first successful body must
finish every pair before publishing reusable membership. Late failures still abort provisional output. A separate
private-frame test proves cancellation inside each scalar clamp parameter before any new vector cursor opens;
uninterrupted twins reach the vector and yield independently expected values.

Of 6,658 exported native resource groups, 1,374 fetch fewer entries and acquire less workspace; none fetch more.
For 128 series per operand, page size 17 and three range ticks, `round(m, .5) / on(id) n` changes as follows:

| Observation | Before | After |
|---|---:|---:|
| Fetched sample entries | 595,584 | 12,654 |
| Accounted work | 22,330,494 | 603,634 |
| Acquired workspace bytes | 154,382,468 | 5,122,207 |
| Peak workspace bytes | 412,234 | 412,234 |

All 384 scaling-group peaks and all 3,266 instant observation groups match exactly. In 186 small range groups,
existing completed-join metadata adds 146–312 bytes at peak, at most 1.43% in this panel. No provider or
borrowed-page peak increases. Another 96 mixed right-wrapper range groups reject reuse after conservative checks:
each adds 32 cumulative workspace bytes and 16 work units, with unchanged reads and peaks. These costs are retained.
There is no global peak-memory reduction claim. Pair bindings and one exact-scope bitmap remain the existing owners;
old completions release before replacement. Accounted owner bytes are neither allocator overhead nor process RSS.

Warm measurements follow closure of all owned qualification runs: 26 fixed cases per profile, five alternating
pairs, one warm then one measured query per active dataset, and 1,040 checked responses. Every complete result and
each variant's own nonelapsed observations match its qualified run. These examples use 128 series per operand and page 17:

| Profile | Expression | Median before → after | Before/after ratio |
|---|---|---:|---:|
| release | `round(m, .5) / on(id) n` | 492.467 → 12.709 ms | 38.750× |
| release | `clamp(m, 0, Inf) / on(id) n` | 512.747 → 13.191 ms | 38.871× |
| release-min | `round(m, .5) / on(id) n` | 842.250 → 22.317 ms | 37.740× |
| release-min | `clamp(m, 0, Inf) / on(id) n` | 862.755 → 22.246 ms | 38.782× |

All 52 median ratios span 0.838–38.871.
15 medians are slower; the two lowest ratios are:

- `scaling-n0-page17-clamp-min-left-range`, release-min: 21950 → 26200 ns, 0.838×.
- `scaling-n0-page17-clamp-max-left-range`, release: 14890 → 15520 ns, 0.959×.

All samples and medians, including empty/raw/rate/abs/data-dependent/instant controls, remain in the receipt.
These are shared-host engine timings, without CPU isolation, production DBengine or universal speedup claims.
The unchanged pinned numerical, reference, storage, contexts, ABI and disabled-cap contracts remain authoritative.
No new original-Go capture, SRE or live-Agent replay, full foreign unit suite, layout, sanitizer or RSS claim is made.
Earlier cohorts and all 54 prior ledger objects retain their actual identities. Query JSON does not export basic
statistics; current unit/C assertions cover the unchanged owners.


## Single-lock received-page accounting

`QueryAccounting::page` updates received pages and samples under one observations lock. It uses the existing
ordered saturating additions, sticky overflow and poison recovery. A zero-sample call still adds one page.
Generic and C adapters keep their existing validation, callback, error and cancellation checkpoints. Accepted
outer pages remain counted when later sample conversion or validation fails. Adapted pages are counted once.

A concurrent snapshot no longer sees the page increment before the sample increment within that same helper
call. Completed counters and the next callback/control snapshot remain exact. No query state, retained sample,
metadata owner, allocation, ABI, storage format, numerical behavior, default or resource cap is added.

Fresh evidence in [the page accounting qualification](page-credit-qualification.json) includes:

- Both native profiles: 123 complete queries each, including 12 independently checked long float/native sums
  and 12 exact provider errors. All exported nonelapsed observations, including every memory field, match the
  accepted predecessor. Windows contain 8,192 or 65,536 samples; Rust page sizes are 1, 17 and 1,024.
- Seven focused owners pass on the original two-lock implementation before the runtime edit. The boundary test
  covers 12 zero/maximum/overflow/cast cases. Extended phase tests check C nested/consumer/timestamp failures,
  early no-page failures, generic stopped scans, returned-page cancellation and recovered ledger locks.
- The current runtime passes 631 native units/profile, format, strict production/fixture Clippy and Rust 1.91
  compile checks. Seven focused owners/profile execute on compiled ARM64 and ARM32: 28 unit executions.
- Four actual native C/C++ executables produce 40 records each, with independently counted received pages and
  samples. Their long-window pages remain 1/7/1,024; this step adds no C long-window page-17 claim.
- The initial baseline reader incorrectly required zero workspace while a provider-error report retained its
  34-byte message. The fixture completed successfully and checked final release. The failed reader is preserved;
  its correction reused the closed release output and ran only the previously unrun release-min baseline.

The source removes one mutex acquisition per credited page. Across these 246 qualified queries, that is 1,943,576 fewer
acquisitions by source derivation. This is separate from elapsed-time measurement. No runtime lock profiler,
new memory-layout measurement, RSS, fresh Go/SRE/live DBengine or full foreign expression panel is claimed.

The fixed timing panel has 23 cases/profile, nine alternating before/after pairs, and one warm query before
each measured query: 1,656 complete checked responses and 46 median groups. Each response matches its qualified
output and own nonelapsed observations. All ratios range from 0.950 to 1.043; 25 medians are slower.
The table shows all 65,536-sample groups. Ratios above 1 mean faster; ratios below 1 mean slower.

| Profile | Values | Page | Before ms | After ms | Before/after |
|---|---|---:|---:|---:|---:|
| release | float | 1 | 61.252 | 61.682 | 0.993 |
| release | float | 17 | 18.001 | 18.947 | 0.950 |
| release | float | 1024 | 15.512 | 14.877 | 1.043 |
| release | native | 1 | 160.968 | 168.301 | 0.956 |
| release | native | 17 | 72.832 | 74.004 | 0.984 |
| release | native | 1024 | 67.292 | 69.830 | 0.964 |
| release-min | float | 1 | 97.183 | 96.040 | 1.012 |
| release-min | float | 17 | 24.443 | 24.025 | 1.017 |
| release-min | float | 1024 | 19.723 | 19.611 | 1.006 |
| release-min | native | 1 | 280.478 | 275.192 | 1.019 |
| release-min | native | 17 | 126.577 | 128.248 | 0.987 |
| release-min | native | 1024 | 116.852 | 116.603 | 1.002 |

These are engine elapsed times from generated queries on a shared host. Fixture input arrays live in the test
provider, outside the engine accounting. Engine raw samples remain paged. Small groups and shared-host
variation limit timing conclusions; no universal throughput improvement is established.

The mixed original timings prompted a separate fixed follow-up on all six 65,536-sample cases. It uses 13 pairs
per profile, one warm query and three measured repeats per case. Identical-binary controls run the accepted
binary on both aliases; accepted/current comparisons use the two qualified variants. All 2,496 responses
preserve complete outcomes and their own qualified nonelapsed observations. Only these owned children are
pinned to CPU 111, selected from the parent allowed set. This provides no exclusive CPU or frequency control.

| Profile | Values | Page | Identical-binary ratio | Accepted/current ratio | Faster pairs / 13 |
|---|---|---:|---:|---:|---:|
| release | float | 1 | 0.990 | 1.005 | 6 |
| release | float | 17 | 1.006 | 1.000 | 10 |
| release | float | 1024 | 1.008 | 0.997 | 7 |
| release | native | 1 | 0.987 | 0.999 | 7 |
| release | native | 17 | 1.013 | 1.002 | 9 |
| release | native | 1024 | 1.014 | 1.004 | 7 |
| release-min | float | 1 | 0.999 | 1.044 | 12 |
| release-min | float | 17 | 0.996 | 1.017 | 11 |
| release-min | float | 1024 | 0.993 | 1.029 | 8 |
| release-min | native | 1 | 0.996 | 1.024 | 9 |
| release-min | native | 17 | 1.001 | 1.013 | 11 |
| release-min | native | 1024 | 0.993 | 1.009 | 9 |

Ratios use all 39 measured samples per alias; faster-pair counts compare the two three-repeat medians within
each pair. Original timings, identical-binary controls, every sample and every slower group remain in the
qualification. In this follow-up, release ratios are near parity (0.997–1.005); release-min ratios show
modest positive results (1.009–1.044). Identical-binary ratios span 0.987–1.014. The original larger release
slowdowns did not recur under this separate condition. Affinity provides no hardware isolation, and these
results establish no universal gain or causal explanation for host/profile differences.

## Completed composed Float-map joins

The composed checkpoint resolved the existing earned schedule through Float operands such as
`round(abs(m), .5) / on(id) n`. Its inner path admitted parens/unary, vector/pure-scalar arithmetic and comparisons,
label projections, supported timestamps and histogram count/sum/avg conversions. That checkpoint retained an outer
typed Float restriction; bare outer Binary and other mapped forms kept their ordinary path. The
[earned-operand extension](#completed-earned-operand-joins) records current outer eligibility. Raw-leaf/grid, root,
one-to-one matching, unique Float operand/output and successful-completion checks remain in the current design.
This checkpoint's qualification and numbers retain their original source and binary identities.

The marker already proves every prepared vector child, pure scalar side parameter and projected-label uniqueness.
Matching outer/leaf source/value grids excludes intermediate boundary collapse; scalar argument clocks may intentionally
be invariant. Direct `timestamp(m)` can retain fallback because its leaf producer is missing, while timestamp of a
computed vector can qualify. With samples at 750/1750/2750 ms and evaluation at 1000/2000/3000 ms, direct timestamp observes
.75/1.75/2.75 seconds; unary-plus input observes 1/2/3 seconds. Both full outcomes are checked independently.

Selection retains outer wrapper ordinals. Existing mappings translate through canonical, intermediate and physical
source rows, including changed label identities and nonuniform values. Native input is permitted only after prepared
operands/output satisfy the existing Float certificate. First-final delivery still finishes every pair before reuse;
later rows evaluate their selected original operands afresh. Empty/populated/empty scope replacement, present NaN,
required provider errors and provisional abort remain distinct. The zero-bucket histogram fixtures retain the pinned 11-unit sample weight.

The [composed-map qualification](composed-map-join-qualification.json) binds 4,126 complete native matches, 420 independent
full-value checks, 8,156 compiled ARM64/ARM32 matches, 632 native units per profile and 16 focused compiled ARM unit executions.
Twelve actual native/ARM C/C++ callers pass 44 records each. Four added C queries use post-output fault arming; distinct
Rust owners establish native projection values and cold/pre-first-output phases. Native format, strict production/
fixture Clippy and MSRV 1.91 compile-only checks pass. The 75 expected/retained errors per native profile remain errors;
nine construction errors/profile have no observation record. Error/empty checks do not establish successful function coverage.

Before the runtime change, all 24 growth result/label/time/statistics/read-credit/release checks passed before all 12 fixed
growth groups failed their bounds. Current growth and calibrated cold-discovery/full-pair/warm-selected, rewritten-label,
native and changing-membership owners pass. Original fixture-input, expectation-reader, test-weight and plan-key reader
mistakes are retained separately from successful owners; they do not become passing engine results.

Of 4,108 exported observation groups, 1,524 fetch fewer entries, do less work and acquire less workspace; none increase those
costs. All 2,006 instant observation groups and 240 scaling workspace peaks match exactly. In 396 small range groups, existing completion
metadata adds 312 bytes at peak, at most 1.10% here. Provider/borrowed peaks never increase. This is not a global memory win.
For 128 series per side, page 17 and three range ticks, `round(abs(m), .5) / on(id) n` changes as follows:

| Observation | Before | After |
|---|---:|---:|
| Fetched sample entries | 595,968 | 13,038 |
| Accounted work | 35,622,635 | 977,853 |
| Acquired workspace bytes | 165,977,580 | 6,411,955 |
| Peak workspace bytes | 486,052 | 486,052 |

Raw history remains paged across both axes. No new production field, cache, owner, proof walk, metadata authority,
ABI, storage format, default or enabled limit is introduced. Accounted bytes exclude allocator overhead and fixture
input arrays and are not process RSS.

After all qualification writers closed, the fixed warm panel ran 16 cases/profile, seven alternating pairs, one warm
then one measured query per case/variant: 896 complete checked responses and 32 median groups. Every result and each
variant's own qualified nonelapsed observations match. Shared-host execution timings retain every sample and slower
group in the qualification. Examples at 128 series/side and page 17:

| Profile | Operand | Median before → after | Before/after ratio |
|---|---|---:|---:|
| release | nested-round | 642.568 → 17.344 ms | 37.05× |
| release | inner-times | 656.149 → 17.829 ms | 36.80× |
| release | time-computed | 684.343 → 18.806 ms | 36.39× |
| release | label-join | 658.362 → 16.671 ms | 39.49× |
| release-min | nested-round | 1101.975 → 31.576 ms | 34.90× |
| release-min | inner-times | 1118.456 → 31.815 ms | 35.16× |
| release-min | time-computed | 1104.621 → 33.227 ms | 33.24× |
| release-min | label-join | 1120.951 → 29.891 ms | 37.50× |

The panel retains 5 slower medians. There is no exclusive-CPU/frequency or universal speed claim.
No new Go/SRE/live-Agent/DBengine, full foreign-unit, layout, sanitizer, RSS or CI-pass qualification is inferred.
Earlier cohorts retain their original source/binary identities.


## Completed earned-operand joins

Completed joins reuse the existing earned schedule for direct mapped operands, including `(m*100) / on(id) n`,
`(-m) / on(id) n`, label projections and histogram count/sum/avg conversions. The marker remains the single authority
for its prepared vector path, pure scalar side arguments and projected-label uniqueness. It must resolve to an
actual raw Selector with matching outer/leaf producer, source and scope grids. The outer typed-Float syntax gate
is removed; raw Selector and separate actual-arity-one rate paths retain their behavior.

All root, no-fill, one-to-one matching, delayed-name, invariant/boundary, actual unique Float operand/output,
exact-key and successful-completion gates remain. Native-preserving results, source-reading scalar parameters,
histogram stdvar/stddev, group/sort/subquery and missing clock proofs keep ordinary evaluation. Direct timestamp
can lack its leaf producer; timestamp of a computed vector can qualify. No feature is dropped.

First-final delivery still drains every pair before publishing reuse. Later rows read the selected original
operands afresh through existing ordinal mappings. Bare label projection keeps child encounter order, which can
differ from final root label order. The phase tests retain an early cold failure before output, the actual cold
suffix after first output, and warm selected failures with every unselected physical row unchanged. Mixed Binary
scalars in either position finish before vector acquisition. Empty/populated/empty scopes and present NaNs remain
distinct. No new production field, owner, cache, raw history array, metadata authority or public contract is added.

The [earned-operand qualification](earned-operand-join-qualification.json) records:

- 993 queries/profile: direct arithmetic, unary/paren, scalar-position filtering and bool, label rewrites,
  irregular timestamps, integer/float native histograms, sparse/stale/zero/signed data and ordinary fallbacks.
- 1,986 complete native matches against the accepted engine, with exact raw order, numerical bits, errors and
  annotations. All 828 independent full-value/label/time/membership expectations pass; those independent checks
  compare NaN class, while accepted/current comparison retains exact NaN bits.
- 1,196 complete compiled ARM64/ARM32 matches and 424 independent checks on a fixed distinguishing page-17 subset.
  Twenty focused compiled unit executions exercise pages 1/17/1024. This is not a full foreign unit-suite claim.
- 632 native units/profile, format, strict production/fixture Clippy and installed MSRV 1.91 compile-only checks.
  Twelve actual native/ARM C/C++ callers pass 48 records each. Four added positive-float C queries use post-output
  fault arming; native-value and cold/pre-first-output evidence comes from the distinct Rust owners.

Before the runtime change, all 60 complete growth result/label/time/statistics/read-credit/release checks passed,
then 18 newly admitted growth groups failed their bounds; the 12 earlier composed groups passed. Current bounds
pass. Calibrated phase tests contain 819 records, covering discovery, full cold pairs, warm selection and early
cold label failure. The original reader mistakes and copied label fault landmark remain in separate development
receipts. The fault correction changes only tests; the runtime transformation and unchanged direct-rate suffix
have exact reconstruction proofs. No failed run becomes successful qualification.

Of 1,986 exported observation groups, 774 fetch fewer entries, do less work and acquire less workspace. All 966
instant observation groups and all 240 scaling workspace peaks match exactly. Forty-two small range groups add
168 or 312 bytes at peak, at most 1.40% here. Six native-preserving fallback groups add 15 work units and 32
cumulative workspace bytes each, with unchanged reads and peaks. All seven other accounted memory kinds have
unchanged peaks and no higher acquisition. Accounted bytes exclude fixture inputs and allocator overhead and
are not process RSS. Metadata and complete output can still grow with cardinality; caps remain disabled.

For 128 series per side, page 17 and three range ticks, `(m*100) / on(id) n` changes as follows:

| Observation | Before | After |
|---|---:|---:|
| Fetched sample entries | 595,968 | 13,038 |
| Accounted work | 30,320,763 | 859,522 |
| Acquired workspace bytes | 159,445,188 | 6,097,483 |
| Peak workspace bytes | 418,304 | 418,304 |

After qualification writers closed, eight fixed cases/profile ran five alternating pairs, one warm and one
measured query per case/variant: 320 complete checked responses and 16 median groups. Complete outcomes and each
variant's own qualified nonelapsed observations match. Selected shared-host execution medians are:

| Profile | Operand | Median before → after | Before/after ratio |
|---|---|---:|---:|
| release | scale-right, 128 series | 573.342 → 15.672 ms | 36.58× |
| release | label-join, 128 series | 573.426 → 13.510 ms | 42.44× |
| release | histogram-count control | 0.876 → 0.513 ms | 1.71× |
| release-min | scale-right, 128 series | 965.086 → 26.273 ms | 36.73× |
| release-min | label-join, 128 series | 979.146 → 23.029 ms | 42.52× |
| release-min | histogram-count control | 1.497 → 0.854 ms | 1.75× |

All five slower medians remain in the report: release empty label-join, source-scalar, raw and previously eligible
Float controls, plus release-min empty scale-right. Their before/after ratios are 0.988, 0.990, 0.954, 0.931 and
0.994 respectively. Timings have no exclusive CPU/frequency control and do not establish universal throughput.
There is no fresh Go/SRE/live-Agent/DBengine, full foreign-unit, layout, sanitizer, RSS or CI-pass claim.
All 57 previous qualification ledger objects retain their original content and source/binary attribution.


## Boolean selector search

Selector fallback now asks the existing PikeVM search for a boolean result with no capture outputs. It previously
created capture storage and copied successful ranges, then discarded them. Literal/set/simple matcher ordering,
original-byte prefix checks, subject normalization and offsets, Input/span and fresh execution-local cache ownership
remain. Full capture, FindAll and label replacement use the same setup and finish while their scratch remains alive.
The backend's full initial slot tables remain allocated; this change adds no persistent cache or public option.

The [selector qualification](selector-boolean-qualification.json) records:

- 242 fixed queries/profile: 224 positive/negative selectors, 12 capture/replacement controls, four reached provider
  errors and two unselected-error controls. Sizes 32/512/4,096, page 17, direct/count and instant/range outputs cover
  numeric captures, ASCII word rules, nullable/absent labels, newlines, Unicode and malformed byte subjects.
- 484 complete native matches and 476 independent full value/label/time/membership expectations. Raw comparison
  retains exact order, bits, errors and annotations. The eight reached native provider errors remain errors.
- The retained 21,320 original regex cases pass in both profiles on native AMD64, compiled ARM64 and ARM32:
  127,920 fresh comparisons. Original Go source facades and capture provenance are validated and reused unchanged.
- 504 complete compiled ARM query matches on the fixed subset, 12 focused ARM unit executions, 633 native units per
  profile, format, strict production/fixture Clippy and installed MSRV 1.91 compile-only checks.
- Eight actual C/C++ callers pass 48 records each. The existing primitive record adds 24 positive/negative fallback,
  empty/absent and invalid-byte outcomes. Native callers use both profiles; foreign callers use release, while
  foreign release-min regex, complete queries and focused units execute separately. No ABI/layout changes are made.

The old runtime passes all 18 new boolean outcomes and control/release checks, then fails the allocation assertion
with 672 bytes of discarded valid-subject capture ranges. The candidate passes with zero such workspace. An existing
empty-selector validation unit also required that removed allocation; its test-only correction preserves rejection,
backend observation and cleanup. Exact inverse and production-prefix proofs retain earlier comparison/binary
identities. The completed query and regex comparisons are not repeated after that test-only change.

Of 484 observation groups, 328 reduce work, cumulative workspace acquisition and dependency estimates. No exported
memory kind or work group increases. All 128 optimized matcher controls and 24 capture controls retain exact
nonelapsed observations. Four query workspace peaks decrease; the other 480 are unchanged. In 328 groups the
reported regex-dependency peak decreases. These estimates are not exact allocator measurements or process RSS.

For a 4,096-series numeric selector/count range query, acquired workspace changes from 11,136,680 to 9,957,032 bytes;
with extra capture groups, it changes from 13,495,976 to 9,957,032 bytes. Both query workspace peaks remain 1,251,560
bytes. The corresponding dependency-estimate peaks change from 1,856 to 1,824 and 5,168 to 5,088 bytes. Sample reads,
provider calls, output counts and temporary-storage observations are unchanged. Inputs and allocator overhead are
excluded; raw history remains paged and metadata/output can still grow with cardinality while caps are disabled.

After qualification writers close, eight fixed cases/profile run five alternating before/after pairs, each with a
warm and measured query: 320 complete checked responses. All outcomes and each variant's own nonelapsed observations
remain exact. Every median group is retained below; these are shared-host execution times without CPU isolation.

| Profile | Case | Median before → after (ms) | Before/after ratio |
|---|---|---:|---:|
| release | ascii-n32-numeric-pos-count-range | 0.351 → 0.333 | 1.054× |
| release | ascii-n4096-numeric-pos-count-range | 41.297 → 38.527 | 1.072× |
| release | ascii-n4096-numeric-neg-count-range | 51.780 → 50.868 | 1.018× |
| release | ascii-n4096-captures-pos-count-range | 44.883 → 40.885 | 1.098× |
| release | ascii-n4096-literal-pos-count-range | 12.203 → 12.084 | 1.010× |
| release | ascii-n4096-alternation-pos-count-range | 21.104 → 20.533 | 1.028× |
| release | ascii-n4096-replace-capture-direct-range | 277.513 → 279.014 | 0.995× |
| release | bytes-n32-non-digit-pos-direct-range | 0.151 → 0.144 | 1.049× |
| release-min | ascii-n32-numeric-pos-count-range | 0.595 → 0.573 | 1.039× |
| release-min | ascii-n4096-numeric-pos-count-range | 72.515 → 70.063 | 1.035× |
| release-min | ascii-n4096-numeric-neg-count-range | 90.551 → 87.370 | 1.036× |
| release-min | ascii-n4096-captures-pos-count-range | 83.462 → 76.437 | 1.092× |
| release-min | ascii-n4096-literal-pos-count-range | 20.032 → 20.346 | 0.985× |
| release-min | ascii-n4096-alternation-pos-count-range | 34.081 → 34.442 | 0.990× |
| release-min | ascii-n4096-replace-capture-direct-range | 400.311 → 383.566 | 1.044× |
| release-min | bytes-n32-non-digit-pos-direct-range | 0.244 → 0.243 | 1.004× |

All 3 slower median groups remain in the report. This finite panel is neither universal
throughput nor complete-language proof. There is no new Go capture, SRE/live-Agent/DBengine, full foreign-unit,
sanitizer, RSS or CI-pass claim. Independent RE2/Fowler/exhaustive suites remain outside the evidence. All 58 prior
qualification ledger objects retain their content and original source/binary attribution.


## Exact C label-descriptor capacity

The C adapter still copies descriptors while borrowing provider-owned label bytes. It now supplies the known label
count to a shared private vector grower. Growth remains incremental: cancellation, work and field validation precede
each push. The hint trims geometric excess; required sizes still win and existing capacity never shrinks. Ordinary
growth selects the original policy at compilation, with the original push body. The untracked allocator path remains.
Contexts ownership, raw paging, public interfaces and disabled query limits are unchanged.

Required owned label copies are qualified separately in [bounded owned-label copies](#bounded-owned-label-copies).

The [capacity qualification](label-descriptor-capacity-qualification.json) records:

- Nine label counts (0/1/3/4/5/8/9/16/17), 63 byte/pointer/lifetime checks and 17 malformed/cancellation scenarios.
  The old runtime completes label and cleanup checks before failing the intended final capacity assertion.
- Five native retained-capacity groups improve; none increase. Nine descriptors retain 512 → 288 bytes, while
  17 retain 1,024 → 544 bytes. These are isolated requested Workspace bytes, excluding input and allocator overhead.
- A separate terminal-cancellation case after nine valid labels reduces replacement peak 768 → 544 bytes and
  cumulative acquisition 896 → 672 bytes. Error priority, work and close/release phases remain exact.
- 64 selected Debug fixture units (15 accounting, 49 C interface), including low hints, existing larger capacity,
  untracked growth, overlapping allocations and query-owner release. Formatting, strict production/fixture Clippy
  and installed Rust 1.91 compile-only checks pass. This is not a fresh full-unit-suite execution.
- Twelve actual optimized C/C++ callers: both profiles on native AMD64, compiled ARM64 and ARM32 under QEMU.
  Each passes 48 record categories and seven metadata cases, including nine ordered borrowed labels, zero sample
  cursors and complete query/selection/series cleanup. Prior two-label cases remain.
- Eight ordinary Rust-provider query controls retain complete outcomes and every nonelapsed observation. This
  provider does not reach C descriptor conversion; these controls exercise shared default-growth behavior.

The intermediate dynamic-hint variant also passed those checks but added unused hint handling to ordinary label
views. Its size-optimized label-replacement timing remained slower against two baseline controls in four of five
CPU-affinity triples. The final private const-policy split removes runtime hint selection from ordinary growth.
Both variants keep their actual sources, binaries, complete timing results and limitations in the report.

Five alternating before/final pairs per case/profile, each warm plus measured, retain all 160 checked responses.
Every final median group appears below. These are shared-host ordinary-path controls, not a throughput claim for
C label conversion.

| Profile | Case | Median before → final (ms) | Before/final ratio |
|---|---|---:|---:|
| release | ascii-n32-numeric-pos-count-range | 0.331 → 0.330 | 1.003× |
| release | ascii-n4096-numeric-pos-count-range | 37.657 → 38.908 | 0.968× |
| release | ascii-n4096-literal-pos-count-range | 11.940 → 11.962 | 0.998× |
| release | ascii-n4096-replace-capture-direct-range | 275.296 → 274.918 | 1.001× |
| release-min | ascii-n32-numeric-pos-count-range | 0.578 → 0.562 | 1.028× |
| release-min | ascii-n4096-numeric-pos-count-range | 70.659 → 70.591 | 1.001× |
| release-min | ascii-n4096-literal-pos-count-range | 20.348 → 19.902 | 1.022× |
| release-min | ascii-n4096-replace-capture-direct-range | 382.920 → 382.073 | 1.002× |

All 2 slower final median groups remain in the report. Engine-accounted bytes are not process RSS; foreign callers
do not measure isolated foreign allocation savings. There is no new Go, SRE, live-Agent/DBengine, full foreign-unit,
regex/frontend, layout, sanitizer or CI-pass qualification. All 59 prior ledger objects retain their content and
actual source/binary identities.

## Bounded owned-label copies

Required owned label copies now retain less spare descriptor capacity. Full copies capture their exact iterator
length once; filtered copies use the input count as an upper bound. Both supply that bound to the existing
incremental vector push after the original work check and name/value copy. There is no eager reservation or second
predicate pass. Independently owned results, existing metadata ownership and raw-sample paging remain unchanged.

Later label insertion needs a coupled rule: when a compact copy fills, the two local append sites grow to the next
ordinary power-of-two boundary, with the existing minimum of four slots. Thus 3/9/17 slots grow to 4/16/32, rather
than 6/18/34. Ordinary growth, untracked vectors, existing larger capacity and the shared allocator stay unchanged.
No byte copy, control/work charge, swap, error or cleanup phase moves.

The [owned-label qualification](owned-label-capacity-qualification.json) records:

- Six new units, including 40 complete byte/order/work/ownership/release checks before the old runtime fails its
  final capacity bound. Filtering visits each reached predicate once; interruption preserves failure priority,
  replacement overlap and the existing partial insertion after value-copy cancellation. Repeated output chunks
  keep one label owner, and owned results survive source replacement.
- `owned_label_cases.cases()` supplies 172 stable-profile range requests: full/mapped/projected labels, add/replace/
  delete/join/nested operations, grouped includes, boundary counts and an empty selector. Both native profiles at
  pages 1/17/1024 pass 1,032 complete pinned comparisons with no paired bit difference. The before panel retains
  its actual predecessor binaries and explicitly reuses 342 closed page-17 outcomes.
- Across all 1,032 before/candidate pairs, Workspace peaks are lower in 852 and equal in 180; retained Result bytes
  are lower in 750 and equal in 282. Neither increases in this panel. Work, reads, pages, provider calls, output,
  cancellation and temporary-storage observations are identical. All eight memory kinds appear in the report.
- For 64 series with 17 labels, page 17 and three ticks, Workspace peak falls 237,480 → 145,320 bytes and retained
  Result bytes fall 224,064 → 131,904. The 32-label power-of-two control keeps identical resource observations.
- Both complete native suites pass 653 units. Compiled ARM64/ARM32 under QEMU, both profiles, pass 688 complete
  original comparisons and 32 focused unit executions. Twelve actual candidate C/C++ clients each pass 72 arms:
  9/17 labels, selector/add/projected aggregation, three page sizes, stream/collect and provider failure. Four
  additional native before clients link the same C source with predecessor archives; their 288 paired arms keep
  work/reads identical and have no higher measured Workspace/Result peak.
- Formatting, strict production/fixture Clippy and installed Rust 1.91 all-targets fixture compilation pass.

The first Debug candidate exposed 17 higher mutable Workspace peaks, up to 35,520 bytes. Its concrete 9→18 and
10→20 growth explains the local insertion correction above. Initial and corrected Debug receipts remain separate
from the final optimized source; no diagnostic result is substituted for optimized qualification.

Five alternating before/candidate timing pairs per profile retain all 400 complete responses. Selected medians:

| Profile | Case | Median before → candidate (ms) | Before/candidate ratio |
|---|---|---:|---:|
| release | 17 labels, 64 series, full | 1.249 → 1.136 | 1.100× |
| release-min | 17 labels, 64 series, full | 2.098 → 2.032 | 1.033× |
| release | 32 labels, 64 series, full | 1.768 → 1.747 | 1.012× |
| release-min | 32 labels, 64 series, full | 3.093 → 3.198 | 0.967× |
| release | 9 labels, 64 series, nested add | 2.846 → 2.928 | 0.972× |
| release-min | 9 labels, 64 series, nested add | 4.992 → 5.079 | 0.983× |
| release-min | 2 labels, 64 series, replace by join | 1.453 → 1.533 | 0.948× |

The report retains every sample and all 20 median groups, including 13 slower candidate medians (0.13–5.51%).
Native fixture ELF `.text` grows 32,128 bytes in release and 480 bytes in release-min. These are shared-host synthetic
engine timings and requested engine bytes, not universal throughput, Agent-size or process-RSS measurements.
Smaller copies can change later replacement overlap; measured nonincreases are not a general bound. Limits remain
disabled. There is no fresh broader-language/SRE/live-Agent/DBengine/full-foreign-suite/sanitizer/CI-pass claim.
All 64 prior qualification ledger objects retain their content and actual source/binary identities.

## Selector backend capture tables

Selector fallback compiles explicit capture markers into backend empty states. One shared private construction path
keeps the original parsed names, flat program and prefix/OnePass decisions before that translation. Implicit group
zero remains. Capture-producing GoRegex entry points and label replacement keep the full backend representation.
No extra matcher field, retained cache, query limit or public ABI is introduced.

The [capture-table qualification](selector-capture-tables-qualification.json) records finite evidence:

- The original runtime completes 21 matching/accounting checks across seven capture-bearing patterns, then fails
  only the final unused-storage bound. The selected implementation retains two implicit-group slots. Named, sparse,
  duplicate-name, repeated and nullable captures have smaller backend cache reports; names and prefix bytes match
  full capture consumers. A named invalid-byte example changes 3,424 to 1,952 estimated cache bytes.
- Partial-construction cancellation occurs at 76 work units after the backend has acquired memory, in both policies.
  The test checks complete release. Borrowed program ownership, independent concurrent search accounts, full capture
  offsets/replacements, ordinary unwind and small-stack controls complement that witness.
- Both native profiles pass 18 selected optimized regex units and all 21,320 original regex outcomes each. ARM64 and
  ARM32 pass eight focused units and 11,188 original anchored selector outcomes per profile. The originals call the
  actual upstream FastRegexMatcher interface; unanchored Find endpoints do not establish whole-subject acceptance.
- Complete query results and diagnostics match the accepted source for 484 native and 504 compiled ARM queries.
  Native logical counters and engine-owned memory observations are exact. No new language, SRE or live-Agent
  qualification is claimed by this panel.
- Eight linked C/C++ callers exercise capture-bearing positive/negative selectors, missing labels, nullable/invalid
  byte subjects and full `$1` replacement queries, including output/provider failures at pages 1/17/1024. Both native
  profiles and foreign release profiles run the short `c_embedding.c` entry with `--regex-only`; the default entry
  still selects the complete embedding suite.
- The final test-only correction uses the same standard parser directly with inert Work so it also compiles without
  the fixtures feature. Exact inverse and production-prefix checks preserve earlier runtime binaries' actual source
  identities; fresh production/fixture Debug checks, strict lint and MSRV evidence cover that correction.

Across 484 native resource groups, 200 reduce estimated regex peak and cumulative acquired bytes, none increase,
while all other memory and logical observations remain unchanged. For 4,096 series and three range ticks:

| Selector/count case | Estimated regex peak | Estimated acquired regex bytes | Engine Workspace peak, unchanged |
|---|---:|---:|---:|
| Numeric capture | 1,824 → 992 B | 24,281,088 → 13,205,504 | 1,251,560 B |
| Four explicit captures | 5,088 → 992 B | 67,731,456 → 13,205,504 | 1,251,560 B |

These are opaque backend reports under the existing estimate contract. Cumulative allocation is not concurrent RAM;
this panel does not measure process RSS or total device memory.

An initial const specialization increases release fixture ELF `.text` by 46,496 bytes. It was replaced before broad
qualification with the shared constructor. Against the accepted fixture, selected release `.text` changes
2,993,077 → 2,987,221 bytes; release-min changes 1,807,941 → 1,808,389 bytes. These executable sections do not measure
an installed Agent's total footprint. GNU `size` aggregate text categories are recorded separately.

The first fixed timing panel checks 320 complete warm/measured responses: eight cases/profile, five alternating
pairs. All results and each binary's own nonelapsed observations match. Eleven of 16 medians are slower; retain all
of them. Ratios divide the accepted median by the candidate median, so values above one mean faster:

| Profile | Case | Before → candidate, ms | Ratio |
|---|---|---:|---:|
| release | `ascii-n32-numeric-pos-count-range` | 0.337 → 0.342 | 0.988× |
| release | `ascii-n4096-numeric-pos-count-range` | 38.799 → 39.058 | 0.993× |
| release | `ascii-n4096-numeric-neg-count-range` | 50.959 → 52.984 | 0.962× |
| release | `ascii-n4096-captures-pos-count-range` | 40.691 → 39.008 | 1.043× |
| release | `ascii-n4096-literal-pos-count-range` | 11.998 → 12.503 | 0.960× |
| release | `ascii-n4096-alternation-pos-count-range` | 20.693 → 20.717 | 0.999× |
| release | `ascii-n4096-replace-capture-direct-range` | 274.582 → 275.544 | 0.997× |
| release | `bytes-n32-non-digit-pos-direct-range` | 0.147 → 0.143 | 1.028× |
| release-min | `ascii-n32-numeric-pos-count-range` | 0.557 → 0.557 | 0.999× |
| release-min | `ascii-n4096-numeric-pos-count-range` | 68.876 → 67.347 | 1.023× |
| release-min | `ascii-n4096-numeric-neg-count-range` | 88.287 → 85.185 | 1.036× |
| release-min | `ascii-n4096-captures-pos-count-range` | 75.961 → 67.461 | 1.126× |
| release-min | `ascii-n4096-literal-pos-count-range` | 19.666 → 19.807 | 0.993× |
| release-min | `ascii-n4096-alternation-pos-count-range` | 33.141 → 33.710 | 0.983× |
| release-min | `ascii-n4096-replace-capture-direct-range` | 373.477 → 377.462 | 0.989× |
| release-min | `bytes-n32-non-digit-pos-direct-range` | 0.234 → 0.237 | 0.986× |

These shared-host measurements have no CPU exclusivity or frequency control. Their original slower controls remain
in the report alongside the separately fixed baseline/candidate/baseline follow-up; neither panel proves a universal
throughput improvement.

The focused follow-up checks 450 more complete responses on one allowed CPU. Each fixed case has five rotating
baseline/candidate/baseline triples, four measured executions after a warmup. All results and own nonelapsed
observations match. Median ratios are:

| Profile | Case | Baseline 1 / candidate | Baseline 2 / candidate | Candidate slower than both, triples |
|---|---|---:|---:|---:|
| release | `ascii-n4096-numeric-neg-count-range` | 0.993× | 0.987× | 3/5 |
| release | `ascii-n4096-literal-pos-count-range` | 0.938× | 0.932× | 2/5 |
| release | `ascii-n4096-replace-capture-direct-range` | 1.027× | 1.004× | 0/5 |
| release-min | `ascii-n4096-captures-pos-count-range` | 1.116× | 1.111× | 0/5 |
| release-min | `ascii-n4096-literal-pos-count-range` | 0.993× | 0.982× | 3/5 |
| release-min | `ascii-n4096-replace-capture-direct-range` | 0.972× | 0.938× | 4/5 |

The original slower results remain. The identical replacement baseline binaries themselves differ by about 3.7% in
the release-min medians; large individual host-time outliers also remain in the raw record. This does not prove all
candidate slowdowns are host noise. The checked instruction structures of 16 hot owners are identical after removing
specified relocations while retaining internal branch offsets, ordinary constants and registers. That comparison
excludes physical placement and relocated data/GOT contents. Small compilation-only argument/branch changes remain;
no new per-row search instructions were found in those inspected owners.


## Scheduled selector label ownership

A scheduled point already owns its source labels. Its selector completion now moves that owner through a shared
private completion helper, removing a temporary label-view array and a second deep copy. Ordinary borrowed
selectors still copy labels only for present samples and keep their checked copy and terminal control check.
Scheduled selection retains its identity guard, take-once behavior and logical sample accounting, then explicitly
checks control for both present and absent points. Timestamp rebinding, name removal, the existing sample lease,
scalar-before-vector phases, scheduling admission and the second diagnostic drain remain unchanged.

The first source-to-owned copy is still required. This change adds no cache, field, provider/metadata lifetime,
public interface, dependency or limit. Raw data remains paged across both series and time.

The [label transfer qualification](scheduled-label-transfer-qualification.json) records:

- An end-to-end streamed regression: 64/128 ticks, pages 1/17/1,024, both name-removal modes and a 32 KiB arbitrary-byte
  label. All 12 complete label/value/clock, provider-close and accounting-release cases finish before the unchanged
  runtime fails its cumulative allocation bound. The candidate passes the same cases. This is a test bound;
  runtime resource enforcement remains disabled.
- `scheduled_label_cases.cases()` supplies 104 new cases: scalar operations on either side, filtering/boolean maps,
  unary/composed maps, timestamps between source samples, sparse/stale/native histories, derived and ordinary parent
  controls, empty selectors, name collisions and long labels. Another 172 unchanged owned-label cases retain their
  original captures. Both native profiles at pages 1/17/1,024 pass 1,656 complete pinned comparisons with no paired
  bit difference. The before panel explicitly reuses 1,240 closed predecessor outcomes and executes 416 new ones.
- All complete outcomes and exported nonelapsed/nonmemory/nonwork observations match, including reads, pages,
  provider calls, output, cancellation references and temporary backing. The native JSON fixture does not export
  basic sample statistics; those fields are not part of these 1,656 paired observations. Workspace peaks fall in 864 pairs
  and remain equal in 792; acquired/released bytes and work fall in 1,218 and remain equal in 438. None increases.
  Result memory and every observation of the other six memory kinds remain unchanged in these pairs.
- Both complete native suites pass 654 units. Compiled ARM64/ARM32 under QEMU, both profiles, pass 1,104 complete
  original comparisons and 24 focused unit executions. Twelve actual candidate C/C++ programs each pass 72 legacy
  and 96 scheduled arms; four additional native predecessor programs link the same C source. The 672 paired native
  arms retain reads and complete outcomes; all eight memory kinds and work are reported. Of those pairs, the 384
  scheduled arms also export and compare total queryable samples, useful samples read and peak samples. The 288
  legacy pairs do not export those fields. These modes cover streamed/collected results and provider failure at
  three page sizes. The default full C suite
  is not claimed rerun.
- Formatting, strict production/fixture Clippy and installed Rust 1.91 all-targets fixture compilation pass.

Representative release observations at page 17:

| Query shape | Workspace acquired before → candidate (bytes) | Workspace peak before → candidate (bytes) |
|---|---:|---:|
| 64 series, 17 labels, three mapped ticks | 6,101,672 → 3,617,576 | 280,936 → 280,936 |
| One series, 32 KiB label, 64 mapped ticks | 8,786,860 → 4,522,668 | 141,511 → 108,252 |
| Two series, source timestamps between ticks | 79,130 → 61,434 | 9,562 → 9,064 |

Acquired bytes measure cumulative allocation traffic, not simultaneous RAM. The long-label case removes 4,264,192
acquired bytes and 33,259 peak bytes. Complete result ownership and raw/provider storage observations are unchanged.

Five alternating before/candidate timing pairs per profile retain all 400 warm/measured complete responses:

| Profile | Case | Median before → candidate (ms) | Before/candidate ratio |
|---|---|---:|---:|
| release | 9 labels, 64 series, mapped | 3.073 → 2.466 | 1.246× |
| release-min | 9 labels, 64 series, mapped | 5.470 → 4.476 | 1.222× |
| release | 17 labels, 64 series, mapped | 5.249 → 3.929 | 1.336× |
| release-min | 17 labels, 64 series, mapped | 9.413 → 7.334 | 1.283× |
| release | 32 KiB label, 64 mapped ticks | 1.441 → 1.208 | 1.193× |
| release-min | 32 KiB label, 64 mapped ticks | 3.692 → 3.823 | 0.966× |
| release | Empty mapped selector | 0.014940 → 0.015150 | 0.986× |
| release-min | Empty mapped selector | 0.020020 → 0.022000 | 0.910× |

All 20 groups remain in the report, including five slower candidate medians (0.18–9.89%). Native fixture ELF
`.text` grows 528 bytes in release and 944 bytes in release-min. These are shared-host synthetic engine timings and
requested engine bytes, not isolated device throughput, Agent size, process RSS or fresh HTTP/DBengine results.
The finite panels strengthen compatibility evidence; they add no language feature and do not prove all-input
PromQL equivalence. Earlier broad language/SRE/live-Agent/CI cohorts retain their actual historical identities.

## Scheduled evaluator frame reuse

Scheduled evaluation now passes the row reader's existing frame allocation through fresh per-point construction.
Only an empty vector, no larger than that original allocation, can survive a completed callback. Grown vectors are
released without reserving a replacement. Unused selected points are released before reuse; parameters, clocks,
annotations, selector guards, control checks and both evaluation drains keep their existing phases. Errors and
panics drop the active evaluation before unwinding through the provider. Ordinary and raw-reader construction remain.

The [frame reuse qualification](scheduled-frame-reuse-qualification.json) records:

- Three new owner tests cover fresh state, bounded capacity, unused-point release, 34 variable-histogram-page and
  callback/source-failure cases, and 12 complete streamed allocation cases. The unchanged runtime finishes those
  12 semantic/release cases before failing the allocation bound; the candidate passes. That bound is test-only.
- Thirty new original cases cover shallow/deep maps, filters, changing scalar parameters and fixed/start/end clocks.
  Together with 276 preserved cases, both native profiles at pages 1/17/1,024 pass 1,836 complete pinned comparisons.
  Before evidence explicitly reuses 1,716 closed outcomes and executes 120 new ones. Exported nonmemory/nonelapsed
  observations, including work, reads and provider calls, match exactly. Native JSON omits basic sample statistics.
- Workspace peak falls in 1,296 pairs and is equal in 540; allocation traffic falls in 1,374 and is equal in 462.
  Neither increases. All observations of the other seven memory kinds match. The 672 paired native C arms also
  preserve complete outcomes and work; their 384 scheduled arms additionally compare the three exported sample fields.
- Both optimized native suites pass 657 units. Compiled ARM64/ARM32 under QEMU, both profiles, pass 1,224 complete
  original comparisons and 24 focused unit executions. Twelve current C/C++ programs each pass 72 legacy and
  96 scheduled arms; four predecessor native executions are explicitly reused. Full foreign/default C suites are
  not claimed rerun. Formatting, strict production/fixture Clippy and installed Rust 1.91 fixture compilation pass.

Representative release observations at page 17:

| Query shape | Workspace acquired before → candidate (bytes) | Workspace peak before → candidate (bytes) |
|---|---:|---:|
| One series, 256 mapped ticks | 1,011,404 → 519,884 | 9,996 → 9,036 |
| One series, 256 ticks, eight nested `abs` calls | 14,323,563 → 13,825,323 | 34,370 → 33,410 |
| 64 series, 17 labels, three mapped ticks | 3,617,576 → 3,248,936 | 280,936 → 280,936 |

Five alternating timing pairs per profile preserve all 400 warm/measured responses. These shared-host timings do
not establish a throughput gain: nine of 20 candidate medians are slower, by 0.07–22.65%. The 256-tick map changes
1.305→1.332 ms in release and 2.120→2.096 ms in release-min. All groups and raw timings remain in the report.
Native fixture ELF `.text` grows 41,008 bytes (1.36%) in release and 2,976 bytes (0.16%) in release-min.

Acquired bytes measure cumulative allocation traffic; peak bytes measure requested engine workspace. Neither is
process RSS or allocator overhead. The unchanged 1,000-nested-sum/128 KiB small-stack test aborts both predecessor
and candidate Debug binaries; all three new exact Debug tests and both full optimized suites pass. This change adds
no language feature, cache, persistent field, public/provider interface, dependency or limit. Raw input stays paged
across both axes, and earlier broad language/SRE/live-Agent/CI cohorts retain their actual historical identities.

## Completed mapped rate join discovery

Joins such as `clamp_min(rate(m_total[5m]),0) / on(id) n_total` can reuse completed discovery through pure Float
functions, vector/scalar operations, unary expressions and parentheses. This original cohort ended at the ordinary
direct-rate checks; the [family extension](#completed-range-rate-join-discovery) now admits the complete existing
range-rate class. It shares the existing wrapper-purity predicate and checks every effective producer/source/
scope grid against the original header. Rate receives no raw-point schedule marker.

Pair replay still evaluates the original operand ordinal and its complete subtree, including fresh scalar
parameters. Full successful cold discovery, unique Float/source/head/hash bindings, exact scope keys and existing
matching cardinality gates remain authoritative. Fixed/invariant/extended windows, source-reading scalar arguments
and unproven transformations retain ordinary evaluation. No raw value/history cache, Pair representation, API,
provider, storage, metadata authority, dependency or implicit limit changes.

For 64 shuffled pairs, two evaluation timestamps and page17, seven alternating before/candidate pairs produce
these engine-execution medians. Every warm and measured response matches its qualified result and its own
nonelapsed observations (336 responses total):

| Mapped operand shape | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| `clamp_min(rate(...),0) / n` | 87.82 → 6.89 ms | 155.11 → 11.73 ms |
| `(rate(...)*100) / n` | 88.83 → 7.82 ms | 156.64 → 13.28 ms |
| `abs(rate(...)) / n` | 83.00 → 6.85 ms | 149.48 → 11.53 ms |
| `clamp_min(rate(...),0) / rate(...)` | 78.51 → 8.41 ms | 144.91 → 14.02 ms |

The clamp case reduces recorded work 6,171,792→352,545, fetched samples 41,600→9,596 and workspace allocation
traffic 21,138,694→2,001,300 B. Its workspace peak stays 191,678 B. Existing direct-rate/raw-map controls preserve
all exported nonelapsed observations in 36 native pairs; their small timing differences are not independent
throughput findings. Timings use a synthetic provider on a shared host, not live DBengine disk I/O, HTTP,
concurrent-query throughput or process RSS.

The existing accounted Pair maps and one-step membership bitmap have a cost when enabled for new shapes. Across
804 native pairs, workspace peaks stay equal in708 and rise in96, by at most312 B. Allocation traffic falls in306,
stays equal in456 and rises in42 by at most64 B. Work falls in306, stays equal in438 and rises in60 by at most18
units. All eight memory kinds and ownership fields remain in the qualification; lower cumulative traffic is not
lower peak memory. Native query-fixture `.text` grows1,600/720 B in release/release-min.

The [input generator](mapped_rate_cases.py) supplies134 complete cases covering both profiles, shuffled original
ordinals, left/right/both wrappers, scalar order, changing/empty membership, ordinary NaNs, stale/native/mixed
samples, resets, fixed/shifted/submillisecond windows, grouped joins and duplicate-name errors. Both optimized
profiles at pages1/17/1024 pass804 complete native comparisons with no bit differences. Three supplemental original
boundary cases add18 candidate comparisons, including distinct physical/extrapolation endpoint rounding and a
range/offset sum exceeding signed64-bit nanoseconds. Each native profile passes all659 library units.

Compiled ARM64/ARM32 under QEMU pass536 complete page17 comparisons and20 focused unit executions across both
profiles. Twelve actual candidate C/C++ programs each assert36 mapped and12 direct arms at all three page sizes:
complete success output, a required cold source failure before pending output failure, cancellation and opaque
output error. All144 native mapped observation pairs match the three exported basic sample-statistics fields;
the native JSON fixture does not export them. The Rust failure unit separately calibrates preparation and first/
second-output cursor landmarks before failing a required cold suffix or selected raw operand after one output.

The [qualification](mapped-rate-join-qualification.json) preserves actual original/compiler/binary identities,
all resource increases, timing samples and failed development fixture checks. The old runtime completes all24
mapped regression cases before its final test-only work bound fails; current optimized suites pass that bound.
Earlier broad language, SRE, live-Agent and Debug cohorts retain their own identities and are not reattributed.

## Completed range-rate join discovery

Completed binary discovery now covers the whole existing `RangeFunction::Rate` class: `rate`, `increase`,
`delta`, `irate` and `idelta`. One private predicate preserves the established `rate` short circuit and uses
the typed dispatcher for other members. Its membership is checked against that dispatcher for every registered
function name. Pure Float/scalar/unary/parenthesis wrappers use the same terminal proof.

The original subtree and physical ordinal still own evaluation. Arity, raw Matrix/Selector shape, source and
scope grids, full successful cold discovery, exact keys and matching cardinality retain their existing gates.
Fixed/invariant/extended or unproven shapes fall back; checked duration addition and separately truncated physical
and extrapolation endpoints remain distinct. No raw-point marker, value/history cache, Pair layout, ABI, provider,
storage, metadata, dependency or implicit limit changes.

Seven alternating before/candidate pairs per profile measure the same 64 shuffled pairs at two timestamps and
page17. All560 warm/measured complete outputs and their own nonelapsed observations match qualified results:

| Function | Shape | Release before → after | Release-min before → after |
| --- | --- | ---: | ---: |
| `increase` | direct | 69.71 → 5.33 ms | 120.57 → 9.07 ms |
| `increase` | clamp | 86.87 → 6.86 ms | 151.67 → 11.83 ms |
| `delta` | direct | 68.22 → 4.59 ms | 119.51 → 7.89 ms |
| `delta` | clamp | 84.94 → 5.82 ms | 152.28 → 10.12 ms |
| `irate` | direct | 68.07 → 4.67 ms | 119.42 → 7.98 ms |
| `irate` | clamp | 84.90 → 5.91 ms | 150.86 → 10.23 ms |
| `idelta` | direct | 68.72 → 4.63 ms | 118.91 → 8.00 ms |
| `idelta` | clamp | 85.38 → 5.92 ms | 151.01 → 10.28 ms |

For `increase`, direct/clamp work falls4,977,311→274,480 and6,171,795→352,545. Workspace allocation traffic
falls19,807,492→1,694,194 B and21,138,726→2,001,300 B; peaks stay169,576 B and191,678 B. These are
cumulative requested bytes and tracked peaks, not RSS. The other new members retain the same large-case peaks.
Across4470 native query pairs, workspace peaks stay equal in3930 and rise in540 by at most312 B. All eight
memory kinds, ownership fields, work and read increases remain in the qualification.

Existing `rate` controls are measured separately; retain their costs:

| Existing control | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| direct | 5.36 → 5.39 ms | 9.09 → 9.09 ms |
| clamp | 6.89 → 6.87 ms | 11.93 → 11.90 ms |

The first candidate repeated an approximately3.3% release-min rate-control cost. One bounded refinement
preserves the old `rate` short circuit; its separate504-response panel measures1.45%/1.22% direct/clamp cost,
with0.35%/0.02% same-binary arm variation. Setup classification runs only once/four times for these shapes;
major hot-owner instructions remain equivalent while addresses shift. This does not establish the cause of
the remaining cost or a universal zero-cost result. No further layout experiments are part of this cohort.

The [input generator](range_rate_cases.py) reuses the preceding mapped-rate fixtures and adds distinguishing
reset results, all25mixed-function combinations in each feature profile, instant typed tails, gauge hints,
start times and15nonempty fractional/overflow boundaries. Equal-function ratios alone do not prove dispatch.
All745 pinned complete outcomes pass both optimized native profiles at pages1/17/1024:4470comparisons with
zero numeric bit differences. Each final native profile passes all661library units with none ignored.

Final ARM64/ARM32 query fixtures and static archives execute2980complete page17 comparisons across both
profiles under QEMU. Twelve actual final C/C++ programs assert2880success, cold-provider, cancellation and
output-error arms across all three page sizes. All960native paired C observations preserve the three exported
basic sample-statistics fields; native query JSON does not export those fields. The earlier28focused foreign
unit executions and745Debug comparisons retain their first-candidate source/binary identities; they are not
final-source foreign unit-suite or Debug claims. Both final native suites assert the predicate membership,
complete result/order/release behavior, work bound and calibrated cold-suffix/warm-selected failures.

The old production runtime completes120complete regression cases before failing only the final test-only work
bound. That bound is not an enabled query cap. Format, strict production/fixture Clippy and installed Rust1.91.0
all-targets fixture compilation pass; the latter is a compile-only MSRV check.

Final native query-fixture `.text` changes: release -16 B, release-min +32 B. The
[qualification](range-rate-join-qualification.json) binds actual source/compiler/input/binary versions, the
first candidate and bounded refinement, all timing samples and resource increases. These are synthetic engine
measurements on a shared host, not live DBengine disk I/O, HTTP, concurrent throughput, device performance or
RSS. No language feature is added or dropped. Raw inputs stay paged on both axes; complete owned results and
derived metadata/state may grow. Earlier broad language, SRE and live-Agent evidence retains its own identity.

## Completed per-series fold join discovery

The shared ordinary-window join proof now admits all per-series Fold functions alongside the five existing Rate
functions. The [input generator](fold_join_cases.py) lists the finite family. It includes count/presence,
first/last, min/max, sum/average, variance/deviation, timestamp selection, `changes` and `resets`. Global
`absent_over_time` stays with its synthetic-row evaluator; it is explicitly excluded from per-series certification.

Both direct and mapped terminals retain full cold discovery, exact scope keys, original operand subtrees and
physical ordinals. Fixed/invariant/extended exclusions, effective clocks, arity and checked physical/extrapolation
endpoints remain unchanged. First/Last retain names. The four optional timestamp functions need explicit experimental
options; their positive-path originals and candidates set delayed name removal false. No raw marker, retained
history, new cache, Pair layout, ABI, provider, contexts metadata, storage, dependency or cap change is added.

Seven alternating before/candidate pairs per optimized profile measure 64 shuffled pairs at two timestamps and
page17. Every reference/warm/measured result and each binary own nonelapsed observations is checked. The table
shows direct joins; mapped/clamp cases, all samples and Rate/raw controls are retained in the full qualification.

| Function | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| `count_over_time` | 68.18 → 4.53 ms | 120.39 → 7.87 ms |
| `present_over_time` | 67.83 → 4.51 ms | 120.43 → 8.01 ms |
| `first_over_time` | 67.58 → 4.63 ms | 120.32 → 7.91 ms |
| `last_over_time` | 67.97 → 4.59 ms | 120.61 → 7.99 ms |
| `min_over_time` | 67.70 → 4.53 ms | 120.67 → 8.01 ms |
| `max_over_time` | 67.00 → 4.55 ms | 124.00 → 8.02 ms |
| `sum_over_time` | 67.31 → 4.55 ms | 120.54 → 8.03 ms |
| `avg_over_time` | 66.92 → 4.16 ms | 120.61 → 7.52 ms |
| `stdvar_over_time` | 68.67 → 4.53 ms | 121.22 → 7.98 ms |
| `stddev_over_time` | 68.23 → 4.56 ms | 121.37 → 8.10 ms |
| `ts_of_first_over_time` | 68.04 → 4.52 ms | 122.91 → 7.96 ms |
| `ts_of_last_over_time` | 68.27 → 4.59 ms | 120.98 → 8.16 ms |
| `ts_of_min_over_time` | 67.70 → 4.60 ms | 121.01 → 7.99 ms |
| `ts_of_max_over_time` | 67.19 → 4.59 ms | 120.48 → 8.05 ms |
| `changes` | 67.42 → 4.65 ms | 120.77 → 8.11 ms |
| `resets` | 68.03 → 4.63 ms | 120.91 → 8.14 ms |

For the direct `sum_over_time` case, work falls 4,970,399→267,568 and fetched samples
36,480→4,476. Workspace allocation traffic falls
19,709,188→1,595,890 B; peak remains 169,576 B.
Across all 13,596 native pairs, workspace peaks stay equal in 11,916 and rise in 1,680 by at most
312 B. All eight memory kinds and work/read increases are retained. Allocation traffic and tracked peaks
are distinct from process RSS.

Rate and raw controls also have seven paired measurements per profile:

| Existing control | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| rate direct | 5.35 → 5.38 ms | 9.01 → 9.22 ms |
| rate clamp | 6.80 → 6.95 ms | 11.73 → 11.95 ms |
| raw direct | 3.91 → 3.98 ms | 6.65 → 6.89 ms |
| raw clamp | 5.13 → 4.95 ms | 8.52 → 9.01 ms |

All 2,266 fresh pinned complete outcomes pass both optimized native profiles at pages1/17/1024:13,596
comparisons with zero paired bit differences. Distinguishing numerical outputs, First/Last metric names,
optional positive admission, global absence and sparse/native/mixed controls are included. The48single-ID clock
boundaries establish complete outcomes/fallback; they cannot install a multi-row Pair. Each native profile passes
all663library units with none ignored. Four consequential Debug owners pass; no full Debug-suite claim is made.

ARM64 and ARM32 execute9,064complete page17 comparisons across both profiles under QEMU, plus eight focused
unit executions. Twelve candidate C/C++ programs assert9,792success, cold-provider, cancellation and output-error
arms across all three pages. Two predecessor native C programs assert1,632arms;3,264native before/candidate
observations compare the exported sample statistics. All differences are retained in the qualification; native
query JSON does not export those sample fields. Passing `--window-folds-only` to a caller compiled from
`c_embedding.c` selects this mode with experimental/start-timestamp options; ordinary callers keep defaults.

The old production runtime completes384result/order/release cases, then fails only the final test-only work
bound. The candidate passes that bound and calibrated cold-suffix/warm-selected provider/output failure controls.
The approved equal-time mixed `changes`/`resets` guard is covered directly by its reducer unit in both native
profiles and all four ARM builds: six kernel-owner executions. The newly eligible raw-selector paths retain
strictly increasing physical cursor timestamps. This kernel evidence is separate from complete-query comparisons;
the nonadvancing original is not executed by this cohort.

Format, strict production/fixture Clippy and installed Rust1.91all-targets fixture compilation pass. The latter
is a compile-only MSRV check. New regression/failure owners are selected by the `per_series_fold_joins` unit filter.

Query-fixture `.text` changes: release +224 B, release-min +80 B. The [qualification](fold-join-qualification.json)
binds actual source/input/compiler/binary identities and retains all timing/resource costs. These are synthetic
engine measurements on a shared host, without CPU/frequency isolation. They do not establish live HTTP, DBengine
disk I/O, concurrent throughput, device performance or RSS. Raw samples stay paged across both axes; owned output
and derived metadata/state may grow. No language feature is added or dropped. Earlier broad language, SRE and
live-Agent cohorts retain their own identities.


## Grouped aggregate contributors

Ordinary grouped operands can now reuse complete join discovery and evaluate only the selected group's original
contributors. This covers SUM/AVG/COUNT/MIN/MAX/GROUP/STDVAR/STDDEV, by/without, admitted Float maps and nested
ordinary aggregates over supported raw or per-series window leaves. Actual Pair certification still requires Float
outputs, unambiguous identities and the existing root, cardinality, clock and option guards. This is a performance
change; no language feature is added or dropped. This initial cohort admits the eligible vector-matching root;
the [wrapped-join extension](#wrapped-grouped-joins) covers common enclosing expression shapes. The
[histogram extension](#histogram-group-contributors) separately admits histogram consumers; Pair-only output-kind
and cardinality restrictions do not apply to that extension.

Existing group projection records child **output ordinals**, including aliases and hash-only group collisions,
before child labels are moved or dropped. The plan retains accounted identities and ranges. It retains no child
labels, raw samples or aggregate payload cache. Sorted membership controls inclusion in the original traversal;
it does not reorder reducer arithmetic. Singletons address their group directly. Nested plans own different active
ordinal unions and keep mutable borrows inside each operation. Raw samples remain paged across both axes.

The first complete ReplayKey must traverse the full child and reach successful Done. Only subsequent selections
on that key may skip nonmembers. Preparation credit clears before final delivery; clock/flag/bounds changes require
another full traversal. The aggregate certificate never grants the raw one-point scheduling marker. Selected values
follow their original evaluator, including the existing cache of completed same-key window folds. Consequently, a
window-backed operand need not open a new cursor on every replay. Cold/warm failure controls use mixed window/raw
operands in both directions and inject errors into the genuinely read raw side; symmetric windows remain in
complete numerical coverage.

The old production runtime completes all 192 result/order/release cases, then fails only the final test-only work
bound. The candidate passes the bound for all eight operators across raw/window/mapped/nested shapes, pages and
source order. The retained driver correction reuses the original terminal result; it does not rerun the causal case.
Run these units with the `grouped_aggregate_joins` filter and the direct clock/ownership units with
`aggregate_replay::tests`, under either optimized Cargo profile. Existing hash-collision and late-group owners run
in the full library suite.

`group_contributor_cases.cases()` supplies 720 distinct-value original cases. Use it as the `inputs` argument of the
existing `cursor_calls.capture`/`qualify` APIs, with `construction_errors=True`; qualify with `exact_numeric=True`
and `ordered=False` (range row order remains enforced). It includes both option profiles, by/without, maps, nested
aggregates, Rate/Fold operands, booleans, fixed/offset/cardinality fallbacks and stale/native/mixed/NaN/infinity/zero
states. All 4,320 native comparisons pass at pages1/17/1024 with no numeric bit differences. Both profiles pass all
668 library units. Seven earlier focused Debug owners pass; no full Debug-suite claim is made.

The shared projection change also replays all retained TOPK/LIMIT/aggregate-quantile panels: 56,124 separate
policy-qualified comparisons across both profiles/pages. Their strict original reports retain 27/7/2 ordering
disagreements per panel, with 32/10/8 independently retained declared-policy boundaries. Those strict reports are
not rewritten or counted as exact original passes. Another 4,532 complete retained window outcomes pass at page17
across both profiles, covering the factored root gate and established window terminals.

ARM64 and ARM32 execute 2,880 complete grouped outcomes at page17 across both profiles under QEMU, plus 16 focused
unit executions. Twelve actual C/C++ programs assert 4,608 grouped success/cold-nonmember-provider/cancellation/output
arms across all three pages. Four native callers also assert 3,264 prior-window control arms; two predecessor C
programs assert 768 grouped arms. All eight memory kinds and three basic sample statistics are exported; 1,536 native
before/candidate observations retain every statistic difference. Pass `--aggregate-joins-only` to a caller compiled
from `c_embedding.c` for the new mode. Its normalized ratios prove ownership phases, not individual numerical dispatch.

The **original causal baseline** is separate from the timing dataset below. At 32 groups with four members per side,
three stored samples and two output instants, both optimized profiles have these counters:

| Original baseline | Work units before → after | Fetched samples | Workspace acquired | Workspace peak |
| --- | ---: | ---: | ---: | ---: |
| raw grouped ratio | 81,673,811 → 605,429 | 1,675,008 → 9,936 | 529,075,824 → 3,796,048 B | 123,952 → 125,832 B |
| rate grouped ratio | 79,893,257 → 745,769 | 49,152 → 49,152 | 442,451,312 → 5,975,504 B | 245,968 → 249,728 B |

Standalone work is unchanged in that baseline; raw/rate workspace peaks rise 32/64 B. The optional roster pointer
adds a native per-node cost even when no contributor plan is demanded. Demanded plans add membership/construction
storage. Across all 4,320 new native paired cases, workspace peak rises by 32–1888 B. All memory categories,
allocation-traffic increases and other costs are retained in the qualification. These are tracked bytes, not RSS.

The **fixed timing panel** uses `group_contributor_cases.performance_cases()` and six retained controls. Its distinct
values and by(cluster,zone) queries differ from the causal baseline. There are seven alternating before/candidate
pairs per profile, with complete reference/warm/measured results and each binary's own nonelapsed counters checked:
2,856 responses across 34 cases. Representative joined medians are:

| Timing workload | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| raw, 32 groups × 4 members | 1531.73 → 11.02 ms | 2729.24 → 19.45 ms |
| rate, 32 groups × 4 members | 1173.81 → 19.31 ms | 2127.60 → 33.71 ms |
| raw, 8 groups × 64 members | 473.03 → 38.35 ms | 808.15 → 69.58 ms |
| rate, 8 groups × 64 members | 422.22 → 72.94 ms | 708.36 → 121.00 ms |

Standalone and existing-query medians from the same fixed panel are retained, including slower candidate medians:

| Timing control | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| standalone raw, 32 groups | 2.26 → 2.28 ms | 3.91 → 3.90 ms |
| standalone rate, 32 groups | 5.77 → 5.71 ms | 9.75 → 10.05 ms |
| existing raw direct | 4.03 → 3.93 ms | 6.78 → 6.96 ms |
| existing raw clamp | 4.98 → 4.94 ms | 8.71 → 8.90 ms |
| existing rate direct | 5.40 → 5.50 ms | 9.32 → 9.41 ms |
| existing rate clamp | 7.00 → 7.07 ms | 11.99 → 12.12 ms |
| existing sum_over_time direct | 4.72 → 4.65 ms | 8.11 → 8.13 ms |
| existing sum_over_time clamp | 5.98 → 5.87 ms | 10.41 → 10.51 ms |

The larger standalone rate control at eight groups × 64 members measures 22.69 → 22.74 ms in release and
36.72 → 39.09 ms in release-min. The latter is an observed 6.5% slowdown. This shared-host panel does not establish
its cause; the complete timing arrays and all other control cases remain in the qualification.

Format, strict production/fixture Clippy and installed Rust1.91 all-targets fixture compilation pass; MSRV evidence
is compile-only. Query-fixture `.text` grows 48,656/8,496 B in release/release-min. The
[qualification](group-contributor-qualification.json) binds actual source, input, compiler, binary and process evidence.
Timings use a synthetic engine/provider on a shared host without CPU/frequency isolation. They do not establish
live HTTP, DBengine disk I/O, concurrent throughput, device performance or RSS. Owned output and derived metadata
can grow; all implicit caps remain disabled. Earlier language, SRE and live-Agent panels retain their identities.

## Mapped group contributors

Scalar comparisons and Float maps over ordinary groups now reuse contributor membership at their own ordered
boundary. This includes grouped rates followed by a threshold, `abs`, pure scalar arithmetic and nested consumers
such as `sum(abs(sum by(job)(rate(m_total[3s]))))`. No language feature is added or dropped.

An unequal-header Vector Binary/Call seeds through the existing pure-map/Float proof. Plain and unary-only
aggregates, same-clock instant/equal-header maps and unsupported scalar/leaf paths retain their existing admission.
Invariant and source/scope boundaries stop inherited demand; eligible inner consumers can seed their own local
clock. A parent that consumes All can still have an ordered map child that selects singleton rows. Root-only
admission would miss that repeated work.

The existing Plan keeps original child output ordinals, hash-only aliases and source traversal order. The first
final evaluation on every exact ReplayKey must complete the full child successfully. Preparation credit clears
before delivery. Later selections reuse only membership; values, diagnostics and arithmetic follow the original
evaluator and its existing completed-window caches. No raw history, aggregate payload, new metadata provider or
persistent node context is added. Raw histories stay paged across both axes.

Run the three causal units with the `mapped_group` filter under either optimized Cargo profile. They cover complete
values, labels, timestamps and ownership before actual label-visit bounds; reverse source order and different
counter slopes make ordinal errors observable. The actual OrderedRows body preserves cold nonmember, warm selected
and next-clock nonmember errors without bypassing root order. It also checks explicit cancellation and output abort
at pages1/17/1024 and both name-removal modes. Window-backed warm replay can use existing folds; the error landmarks
therefore use the actually read raw-selector body.

`group_contributor_cases.mapped_cases()` supplies the complete expression panel. It covers all eight ordinary
reducers, by/without, comparisons, bool, scalar-left, Float/time/arithmetic maps, nested consumers, sparse/changing
outputs, no survivors, native/mixed values, NaN/infinity/zero and scope/fallback controls. Existing grouped joins and
histogram bridge cases remain separate comparison panels. The new C panel reuses the paged callback and calibrated
failure owners in `c_embedding.c`. Short-window rates need source-defined extrapolation; its unit-valued callback
assertions use Boolean-normalized positive results. The helper's query observations and summary records are checked
separately.

For the small three-tick dataset, let M be input series and P be surviving rows. The unchanged engine makes
`(3*P+14)*M` label-view calls for the filtered query, compared with `11*M` for the plain aggregate. At 8 jobs and
30 buckets, native release at page17 has these observations:

| Filtered grouped rate | Before | Candidate |
| --- | ---: | ---: |
| Label views | 329,280 | 9,498 |
| Work units | 68,314,094 | 2,688,198 |
| Workspace acquired | 337,851,576 B | 17,932,034 B |
| Workspace peak | 664,368 B | 672,840 B |

Contributor identities/ranges and construction storage are real added metadata. Lower traffic does not imply a
lower peak, and small output sets can pay setup without a gain.

Both optimized native profiles pass all 714 library tests, with the same two existing ignored tests. The selected
panels pass 16,800 complete original comparisons: 2,100 requests at pages1/17/1024 in both native profiles and page17
on ARM32/ARM64 release under QEMU. There are zero semantic mismatches or paired bit differences. Actual default-feature
C archives pass 672 query/failure arms across those four builds; 192 native predecessor arms provide paired sample
statistics, all unchanged. Format, strict default/fixture Clippy and installed Rust1.91 all-target compilation pass;
the MSRV check is compile-only.

The fixed timing panel retains 1,080 complete outcomes across 90 case/profile groups, including five measured samples
per side after warmup. The filtered 8-job/30-bucket case has these medians:

| Profile | Before | Candidate |
| --- | ---: | ---: |
| release | 673.74 ms | 49.82 ms |
| release-min | 1238.50 ms | 84.36 ms |

All 37 slower primary medians remain in the report. The predefined follow-up repeats every case at least 5% slower
for 20 additional samples per side: the tiny plain control changes from +5.56% to +0.76%, and the size-optimized large
quantile from +5.03% to +1.79%. These 80 complete additional outcomes do not replace the primary observations or
establish isolated CPU causality. Across all 90 timing groups, workspace peak increases by 0–8,472 B and traffic
changes by −319,919,542 to +632 B; every field in the other seven memory categories stays equal. Linked C text changes
by −912/+188 B and fixture text by +124/−268 B in release/release-min; all data/BSS changes are also retained.

The [qualification](mapped-group-contributors-qualification.json) retains actual source and binary identities,
complete original comparisons, all memory categories, linked sizes and all slower timing cases. Diagnostic callback
counters establish visitation, rather than ordinary speed. Larger earlier runs are historical scaling witnesses;
they do not replace alternating before/candidate timing. Synthetic shared-host engine measurements do not establish
RSS, maximum or concurrent throughput, live HTTP/DBengine performance, cold-disk behavior or physical ARM speed.
All implicit caps remain disabled, and explicit cancellation/deadline controls stay active. Broader language, SRE
and live-integration panels retain their original attribution.

## Wrapped grouped joins

Common pure alert comparisons and Float functions now retain grouped-join reuse. Examples include a grouped ratio
followed by a threshold, `abs`, `clamp_min`, scalar arithmetic, parentheses or unary operators. The same admission
also reaches supported unaggregated raw/Rate/Fold joins. This changes execution cost, without adding or dropping
language features. Existing clock, delayed-name, input-kind, uniqueness, cardinality and leaf guards remain active.

Three coupled repetitions are addressed: contributor demand through the wrapper, repeated operand membership
discovery, and repeated value acquisition for already-processed non-target pairs. Input membership is reusable
only after both original streams finish successfully. Separate pair credit distinguishes suppressed and kept
outcomes, reproduces their original reservation/seen effects, and retains the requested pair's fresh evaluation.
An unknown suffix remains ordinary work. If a previously kept target now produces no sample, traversal continues;
a prefix never earns full Done or a direct singleton shortcut. Preparation and every ReplayKey change clear credit.

Only accounted identities, hashes and compact pair states are retained. Raw samples remain paged across time and
series; no raw-history array, saved pair values or child-label copy is added. Existing replay-enum and Step maximum
layouts are preserved through optional boxed metadata. Small-query peak memory and executable size can increase.
Metadata restoration and prefix walks remain potentially quadratic; this does not establish linear complexity.

The `wrapped_group_joins` unit filter covers 576 distinguishing numerical/order/release cases across all eight
ordinary aggregates, raw/Rate/Fold and mapped shapes, three page sizes and both source orders. The old production
runtime completes every result check and then fails its final work bound. Direct `pair_discovery::tests` and
`vector_matching::tests` cover sparse/backward requests, suppressed and unmatched inputs, exact keys, replacement,
preparation clearing, alias rejection, credit-word boundaries, reservations, seen effects, cancellation after seen
and before credit, and continuation into an unvisited failing suffix. The preparation-wiring owner covers 48 moved,
copied and fallback shapes. Earlier group, hash-collision, late-group and set-prefix owners retain their coverage.

`wrapped_group_cases.cases()` supplies 1,348 complete original requests across both profiles, all eight aggregates,
by/without, nested/maps, raw/windows, comparisons, sparse/native/mixed and IEEE states, fixed/offset and unsupported
fallbacks. Use the existing original-cursor capture and comparison APIs with exact numeric checks. Original reports
remain distinct from the Decision9 ordering policy: seven strict numerical failures and 20 bit differences are
already present in the published implementation. Repeated unchanged Go execution yields multiple complete outcomes
at each of the eight implicated STDVAR/STDDEV boundaries.

`wrapped_group_policy.declarations()` covers all 32 structurally equivalent cases: eight operators, both profiles,
and the mapped-inner/derived-fallback shapes. Capture the declared child using the unchanged original, sort decoded
full label bytes, inject it beside the untouched opposite raw metric, and capture the declared outer expression.
The helper requires unique dropped-name Float rows at every requested tick, with no diagnostics. These retained
captures are finite, dense Floats; this method does not qualify sparse/native children or internal flags generally.
The original map boundary is unspecified, while aggregation consumes its matrix sequentially. This applies the
existing deterministic-order policy without changing tolerance or replacing the strict reports.

Compile `c_embedding.c` as C11 or C++17 and pass `--wrapped-aggregate-joins-only` for the 480-arm callback panel:
eight operators, five raw/Rate/map/nested/Fold wrapper shapes, pages 1/17/1024 and success/provider/cancel/output arms.
The existing `--aggregate-joins-only` retains its 384 bare controls. Normalized C ratios establish lifecycle phases;
the distinct-value original comparisons establish numerical outcomes.

Both optimized native profiles pass all 676 library units. All 8,088 complete query comparisons pass the separate
exact ordering reference at pages 1/17/1024 and also equal the published implementation's complete packets, with
zero paired bit differences. This is policy-qualified compatibility, not 8,088 strict original passes. The original
seven failures and 20 bit differences per cohort remain in their reports. All 35 focused Debug owners pass; the
final source only tightens their causal work assertion and adds the C fixture mode. Both full optimized suites
execute the tighter assertion. No full Debug-suite claim is made.

Across those 8,088 native paired queries, workspace peaks are unchanged in 4,284 cases and rise in 3,804 by at most
2,424 B. Work increases by at most 211 units in the cases that do not benefit. All eight memory categories and
allocation traffic remain separate in the qualification; tracked peaks are not process RSS.

The original causal baseline uses 32 groups, four members per side, three stored samples and two output instants.
Both optimized profiles have the following counters. This dataset differs from the fixed timing panel.

| Original causal baseline | Work units before → after | Fetched samples | Workspace acquired | Workspace peak |
| --- | ---: | ---: | ---: | ---: |
| raw grouped ratio, threshold | 238,969,528 → 1,336,150 | 4,919,040 → 19,056 | 1,553,217,616 → 7,978,688 B | 124,592 → 129,240 B |
| rate grouped ratio, threshold | 233,502,870 → 1,522,310 | 73,728 → 73,728 | 1,294,602,192 → 10,695,744 B | 263,664 → 268,312 B |

The unchanged rate read count reflects the existing completed-window cache; the optimization removes repeated
intermediate work. Bare root joins keep the same memory/read/allocation counters and add two work units. Other
wrapper shapes and all small/scaling costs remain visible. Query-fixture `.text` changes by −50,576 B in release
and +6,720 B in release-min; these are measured executable sections, not the Rust static archive's file size.

ARM64 and ARM32 execute all 5,392 page17 query comparisons across both profiles under QEMU, with zero policy or
native-packet bit differences, plus 140 focused unit-test executions. Twelve actual candidate C/C++ programs assert
5,760 wrapped lifecycle arms; four native programs also assert 1,536 bare controls. Two predecessor C programs assert
960 wrapped arms. The 1,920 native before/candidate C observations retain all 1,152 differences in exported sample
statistics. Those counts describe read/sample ownership; they do not change expression results. No full foreign
unit-suite or physical ARM performance claim is made.

The fixed timing panel uses `wrapped_group_cases.performance_cases()` and six retained controls. It covers group
counts 4/8/16/32, member counts 1/4/16/64, raw/Rate bare, threshold and abs joins, and standalone aggregates. Five
alternating before/candidate pairs per profile check all 3,720 reference/warm/measured responses and each binary's
own nonelapsed counters. Representative medians at 32 groups and four members are:

| Timing workload | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| raw ratio, threshold | 4625.95 → 22.99 ms | 7827.72 → 38.92 ms |
| raw ratio, abs | 1662.10 → 16.15 ms | 2832.99 → 27.07 ms |
| rate ratio, threshold | 3291.01 → 33.72 ms | 6125.39 → 57.11 ms |
| rate ratio, abs | 1204.11 → 25.75 ms | 2270.94 → 43.40 ms |
| bare raw ratio | 11.13 → 11.33 ms | 19.18 → 19.46 ms |
| bare rate ratio | 19.56 → 19.39 ms | 33.12 → 32.78 ms |
| standalone raw | 2.29 → 2.32 ms | 3.94 → 3.91 ms |
| standalone rate | 6.00 → 5.82 ms | 9.56 → 9.73 ms |

Existing-query controls from the same panel retain slower candidate medians as well as improvements:

| Timing control | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| raw direct | 3.99 → 4.06 ms | 6.99 → 6.82 ms |
| raw clamp | 4.99 → 5.08 ms | 8.84 → 8.74 ms |
| rate direct | 5.60 → 5.51 ms | 9.49 → 9.42 ms |
| rate clamp | 7.13 → 7.10 ms | 12.23 → 12.15 ms |
| sum_over_time direct | 4.63 → 4.66 ms | 8.24 → 8.18 ms |
| sum_over_time clamp | 5.93 → 5.99 ms | 10.55 → 10.48 ms |

All 124 case/profile timing arrays and resource costs are retained in the
[qualification](wrapped-group-qualification.json). No median is over 10% slower in this finite panel; that does not
prove zero overhead or assign a cause to small changes. Native timing began alongside one remaining ARM64 emulator
on a host exposing 112 available CPUs. These are synthetic shared-host paged-provider measurements without
CPU/frequency isolation, not live HTTP, DBengine disk I/O, concurrent throughput, device performance or RSS evidence.

Format, strict production/fixture Clippy and installed Rust1.91 all-targets fixture compilation pass; the MSRV check
is compile-only. The qualification verifies 170 closed process records and binds actual source, reference, helper,
compiler and binary identities. All 71 earlier qualification-ledger objects remain unchanged. No ABI, provider,
contexts, storage, arithmetic or dependency change is introduced. Explicit cancellation/deadlines remain active;
implicit caps remain disabled. Earlier broader language, SRE and live-Agent evidence keeps its original attribution.

## Completed pair-metadata traversal

Eligible warm joins now consume their existing completed input bitmap directly. They avoid rebuilding active-row,
signature, reservation and seen tables for each requested output, and avoid repeatedly walking the same processed
physical prefix. This extends the private mechanism described above without admitting new expression shapes or
changing language support. Original cold discovery, root full-pair replay, set operators and unsupported fallbacks
retain their existing behavior.

The extra certificate requires every statically possible matching pair to have a frozen output binding, and every
bound output to have a distinct canonical full hash, including suppressed comparisons. Globally unmatched rows are
allowed. An unbound possible match or hash conflict retains ordinary traversal, fresh operand reads and its reached
error. The temporary proof indexes are query-accounted and released before execution.

For the exact replay key, a physical many-side cursor advances only over inactive, unmatched or successfully
credited rows. Unknown predecessors run before a later requested pair. Backward requests remain fresh; an absent
or freshly suppressed target continues into the unverified suffix and ordinary Finished. Full unscoped traversal
reads every active pair freshly. A prefix never grants full Done. Cursor publication follows control checks;
successful pair credit retains the original phases. Preparation and key replacement clear both together, and no
temporary borrow spans a child action.

A private enum retains either ordinary pair hashes or the certified cursor. Certification releases the obsolete
hash buffer. Ordinary prefix consumers of a certified plan derive the same canonical hash from frozen labels.
This adds no raw values, timeline, child-label copies or page cache. Physical-prefix metadata selection is amortized
over rows and requests for its exact key; child computation and other metadata work can still repeat. This does not
establish linear whole-query execution or an optimal endpoint.

`nested_pair_frontier_avoids_rebuilding_metadata_for_every_output` checks54 complete value/label/time/release cases:
32/128/512 series, three leaf/grouped/threshold expressions, pages1/17/1024 and both physical orders. The unchanged
published production runtime completes those outcome checks, then fails only the final work-growth bound. For16×
more rows, the old work multipliers are102–120×; current multipliers are18.8–20.1×, below the40× test bound. Fetched
and logical queryable sample counts are identical in every pair. This is a finite causal test, not a general
complexity proof.

Direct pair/Step owners cover reversed65-pair physical/cache order, sparse membership, credit-word and side-word
boundaries, backward/suppressed/missing/full requests, globally unmatched rows, exhaustive binding, actual full-hash
collisions, empty warm tables, fallback reservation/seen behavior, ordinary prefix consumers after hash release,
exact-key/preparation clearing, terminal interruption before publication and fresh suppression reaching a failing
unknown suffix. All29 focused Debug units pass. Final runtime equals that Debug source; only a test loop changes
to zipped iteration for strict Clippy, and both optimized full suites execute that corrected test.

Both optimized native profiles pass682 units each, with no ignored tests. The existing1,348-case original corpus
at pages1/17/1024 produces8,088 complete comparisons equal to the published predecessor and the separate exact
Decision9 ordering expectations. All32 existing dense Float declarations are reused. The raw strict reports retain
seven failures and20 bit differences per cohort, exactly unchanged; policy passes are not universal strict-original
passes. No new Go capture, tolerance, default feature, interface or arithmetic change is introduced.

`pair_frontier_cases.performance_cases()` regenerates all30 actual page17 scaling inputs exactly:4/16/64/256/1024
groups with one member, plus32 groups with four members, using bare grouped raw ratios, raw `abs`, thresholds, rate
`abs` and unaggregated leaf `abs`. At1,024 groups, both profiles have these same observations:

| Workload | Work units before → after | Workspace acquired before → after | Workspace peak before → after |
| --- | ---: | ---: | ---: |
| Bare raw grouped ratio | 9,616,209 → 9,616,209 | 50,734,156 → 50,750,556 B | 2,990,780 → 2,990,780 B |
| Raw grouped ratio, `abs` | 132,826,849 → 14,828,813 | 595,036,154 → 74,512,074 B | 3,375,894 → 3,367,702 B |
| Raw grouped ratio, threshold | 254,374,236 → 18,684,876 | 1,002,102,272 → 98,670,800 B | 3,719,424 → 3,711,232 B |
| Rate grouped ratio, `abs` | 134,848,763 → 16,850,727 | 614,737,358 → 94,213,278 B | 4,446,806 → 4,438,614 B |
| Ungrouped raw ratio, `abs` | 132,450,104 → 13,341,763 | 556,839,602 → 36,315,522 B | 3,309,842 → 3,301,650 B |

Across all30 pairs per profile, work falls in24 and is unchanged in six. Workspace peaks fall in23, stay equal in
five and rise8B in two small bare controls. Acquired/released workspace falls in24 and rises in all six bare
controls, by80–16,400B. Every other memory kind and every current-memory field are unchanged. Fetched samples,
pages, provider calls, output samples, temporary storage and cancellation references are identical. Cumulative
allocation traffic is not peak RAM or process RSS; query JSON does not export basic sample statistics.

The native replay/plan layout measurements retain ReplayRows48B, Replay64B, CompletedStep128B and Plan328B;
Step grows680→688B. This accounts for the retained bare-control allocation cost. Query-fixture `.text` changes
by−9,904B in release and+4,800B in release-min. These are executable sections, not static-archive sizes.
ARM64 has the same layouts. ARM32 retains ReplayRows32B, Replay40B, CompletedStep112B and Plan224B, while
Step grows496→504B in both profiles. These are actual compiled measurements, not native-layout assumptions.

The broader 8,088 paired comparisons also retain the small fallback costs: work falls in 3,468 and stays equal in
4,620. Workspace peaks fall in 3,468, stay equal in 318 and rise exactly 8 B in 4,302. Allocation traffic falls in
3,468, stays equal in 18 and rises in 4,602 by at most 320 B. All seven other memory kinds and all memory-current
fields match. Every fetched/page/provider/output count, temporary observation and cancellation-reference observation
matches. Successful snapshots can retain owned Result storage; all snapshots retain the query's Accounting owner.
They do not claim post-owner-drop total memory. Separate unit lifecycle checks verify release after owners drop.

Actual native C11 and C++17 consumers each assert 480 wrapped and 384 bare lifecycle arms in both profiles.
All 3,456 native rows match their same-language/profile/mode published predecessor's status, output count, fetched
count and three basic sample statistics. Wrapped work and workspace traffic fall; bare work stays equal and its
workspace peak rises 8 B. These observations retain every memory kind. C rows expose no page, provider-call,
temporary-storage or close/abort counters; the complete query panels and caller assertions cover their distinct
scopes. Normalized C ratios establish lifecycle phases, while original distinct-value cases establish arithmetic.

ARM64 and ARM32 execute both optimized profiles under QEMU: 500 selected complete page17 queries and 41 focused
unit executions per cohort. All 2,000 query results equal both the published same-target/profile packets and the
current native packets, with zero paired bit differences. Eight foreign C11/C++17 programs assert another 3,840
wrapped lifecycle arms. Together with native consumers, 16 actual candidate programs assert 5,760 wrapped and
1,536 bare arms. This is compiled foreign execution; no full foreign unit-suite or physical-device timing claim
is made.

All 3,840 foreign caller rows preserve the same-target/profile/compiler published predecessor's status, output,
fetched count and three basic sample statistics. Work and workspace traffic fall in every wrapped row; workspace
peaks fall by 24 or 120 B. Other memory kinds and all current-memory fields match. Error snapshots can retain a
borrowed error message, so these rows do not imply an all-zero post-drop snapshot. The complete expression packets
qualify values; C/C++ rows retain their actual passing result assertions and lifecycle observations.

Formatting, strict production/fixture Clippy and installed Rust 1.91 all-target fixture compilation pass. The MSRV
check is compile-only. Public API/ABI, contexts, storage/provider policy, arithmetic, dependency locks, temporary
backing and panic containment are unchanged. Explicit cancellation/deadlines remain active and implicit caps stay
disabled. Earlier broader language, SRE and live-Agent cohorts retain their actual source and binary identities.

The fixed timing panel covers the 30 scaling cases, four standalone controls and six earlier raw/Rate/Fold join
controls. Five alternating before/candidate pairs per optimized profile check all 2,400 reference/warm/measured
responses, complete results and each binary's own nonelapsed observations. At 1,024 groups and one member, medians
are:

| Timing workload | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| Bare raw grouped ratio | 131.82 → 131.87 ms | 218.37 → 218.88 ms |
| Raw grouped ratio, `abs` | 895.75 → 194.91 ms | 1,559.41 → 334.50 ms |
| Raw grouped ratio, threshold | 1,727.86 → 264.42 ms | 3,001.27 → 433.62 ms |
| Rate grouped ratio, `abs` | 987.40 → 273.36 ms | 1,725.02 → 475.27 ms |
| Ungrouped raw ratio, `abs` | 849.92 → 137.42 ms | 1,494.28 → 237.92 ms |

Lower-cardinality and standalone costs remain visible. At 32 groups with four members, grouped raw `abs` improves
16.39→14.72 ms in release and 26.87→25.62 ms in release-min; its threshold improves 22.51→20.73 and 38.78→35.66 ms.
Twelve release and eight release-min medians are slower. The largest increases are 3.63% and 2.63%, respectively;
for example, the eight-group standalone rate control changes 1.45→1.50 ms in release. These finite observations
do not prove zero overhead or identify the cause of small timing changes.

All 80 timing arrays, every scaling observation, all eight memory categories, native/foreign layouts, binary/source
identities, unchanged sample-statistics counts and explicit costs are retained in the
[qualification](pair-frontier-qualification.json). It verifies 266 consumed terminal process records and preserves
all 72 pre-existing top-level qualification-ledger entries. Timings began after the owned builds and ARM checks
closed. The host remains shared, without CPU/frequency isolation; these measurements establish no live HTTP,
DBengine disk I/O, concurrent throughput, physical-device performance or process RSS result.


## Borrowed singleton float output

The C adapter borrows one stack sample descriptor for a chunk containing exactly one float. It preserves the
series ID, label order/bytes, timestamps, start timestamp, float bits and reserved fields through the synchronous
callback and returned-error copying. Label conversion remains accounted; empty, multiple-point and histogram
chunks retain the general pointer-stable buffers. Both per-point work/control phases and callback cadence remain.
There is no persistent buffer, batching, retained point array, ABI change or new limit.

Run these from the Cargo root, `src/crates/promql-engine`:

```sh
cargo test --locked --features fixtures --profile release --lib c_abi::tests::output_points:: -- --test-threads=1
cargo test --locked --features fixtures --profile release-min --lib c_abi::tests::output_points:: -- --test-threads=1
cargo test --locked --features fixtures --profile release --lib c_abi::tests::output_points::measure_streamed_point_descriptors -- --exact --ignored --nocapture --test-threads=1
python3 tests/promql-conformance/test_c_embedding.py
```

Repeat the explicit measurement with `--profile release-min` for the size profile. It streams bounded fixture
values through the actual C adapter and an `extern "C"` test callback, checks counts/checksums/work/cleanup, and
reports allocation and elapsed counters. It does not collect a result history. Generic native sinks bypass this
adapter and cannot establish its allocation reduction. The direct payload checks cover empty/single/multiple
floats, byte labels, signed zero, subnormal/NaN/stale/Inf bits, extreme clocks/start times and histogram descriptor
growth. Existing delivery controls also run the series branch through errors, cancellation, abort and final commit.

The separately compiled C11/C++17 caller has `--owned-labels-only` for streamed/collected output and provider-error
controls. Its default mode includes the complete instant-set panel. `c_embedding.py` provides a validated fixture
scratch directory; direct caller execution must set `NETDATA_PROMQL_FIXTURE_SCRATCH` to a private existing directory.
The reader checks all default records, including every ordered set-case row and summary; its Python controls reject
missing, duplicate, reordered, malformed and unknown rows. The recorded correction revalidates already successful
C transcripts; a reader change does not turn an earlier execution into a new library build.

Recorded singleton output results

The [qualification](singleton-output-qualification.json) retains the complete before/current records and source
identities. Both optimized profiles pass 696 full units before a test-only module-declaration reorder; fresh final
binaries pass 52 C-interface and 15 accounting tests per profile. Eight actual current C11/C++17 callers cover both
native profiles and ARM32/ARM64 release under QEMU, each passing the full 103-record roster and 72 label-output
arms. C-only ASan/UBSan passes the same panels. The real containing Agent passes 672 storage/API records and
82 HTTP/readiness checks plus 224 measured complete responses, with both owned Agents exiting normally.

For 100,000 unlabelled float callbacks, temporary Workspace acquisition/release falls 12.8 MB to zero and its
isolated heap peak falls 128 B to zero. Each successful two-point streamed C/C++ label-output arm saves 256 B;
all 18 such arms per caller preserve work and the other observations, apart from nonincreasing Workspace peaks.
Collected and pre-output provider-failure controls remain equal. All 16 real Agent output/account records,
including history-independent plain/gzip peaks, remain exactly equal to the predecessor. Allocation traffic is
separate from retained RAM, stack and process RSS.

Eight alternating process pairs per optimized profile include one warmup pair, 96 complete case pairs and 12 timing
groups. The following are seven-pair median callback-loop times; complete raw samples and slower controls remain
in the report. These are test-fixture C-table callbacks, not an external-C-only or maximum-throughput benchmark.

| Callback workload | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| 100,000 floats | 14.388 → 5.189 ms | 21.865 → 7.166 ms |
| 100,000 floats, one label | 17.570 → 8.709 ms | 28.079 → 13.606 ms |
| 10,000 floats, 17 labels | 3.931 → 3.090 ms | 7.352 → 5.886 ms |
| 50,000 two-float chunks | 7.597 → 7.427 ms | 12.095 → 11.658 ms |
| 100,000 empty chunks | 9.902 → 9.041 ms | 12.272 → 11.883 ms |
| 10,000 native-histogram chunks | 1.762 → 1.707 ms | 2.940 → 2.963 ms |

The size-profile histogram control is 0.77% slower in this finite run. The examined default-release callback's
initial stack reservation rises 616→680 B, with six saved registers in both prologues; this is not a complete call
stack. The actual Agent's executable text grows 1,504 B; data/BSS stay equal. No whole-process memory decrease is
claimed. These costs accompany removed heap allocation and owner operations.

Fixed warm live API median wire times, including complete plain/gzip response delivery, are:

| Query | Raw before → after (ms) | Gorilla before → after (ms) |
| --- | ---: | ---: |
| Scalar | 0.363 → 0.377 | 0.338 → 0.395 |
| Fleet gauge | 0.915 → 1.021 | 1.061 → 0.781 |
| Local counter rate | 0.993 → 1.005 | 0.912 → 0.751 |
| Fleet average | 0.979 → 1.071 | 1.024 → 0.788 |
| Six-group histogram | 19.063 → 18.959 | 17.761 → 16.931 |
| 100,000-point delivery, plain | 106.937 → 96.389 | 107.360 → 92.698 |
| 100,000-point delivery, gzip | 116.919 → 103.691 | 120.596 → 103.550 |

Large-response latency falls about 10–14% against the retained predecessor in these separately run warm cohorts.
All small-control changes remain in the table and raw report; their variations cannot establish zero overhead or
causal speedups. The host is shared, without CPU/frequency isolation. Two hosts, one tier, warmed caches and
client overlap do not establish cold-disk, high-cardinality, saturation, concurrent evaluator or physical ARM
performance. Historical language/SRE panels retain their original identities; this panel adds no language feature.

The original caller-reader failure is preserved: the C programs completed successfully after their scratch
setting was supplied, but the old reader expected 48 records and omitted 55 already-emitted set-panel records.
The corrected reader validates all 103. Valid compiled programs and transcripts were reused; missing fixture
setup and reader failures are not counted as passing engine executions. Source phases and the test-only final
formatting change are explicit in the report.

## Direct empty-vector owners

`AccountedVec::new` acquires its existing zero-byte query lease directly, without creating and dropping an
intermediate `AllocationContext`. Acquiring that lease before the infallible empty vector also avoids unnecessary
unwind state. An empty tracked vector still retains exactly one query owner; optional-context `new_in`, untracked
vectors, later growth, checked shared-owner guards and accounting/control boundaries are unchanged.

The [qualification](empty-vector-owner-qualification.json) preserves immutable before, first-candidate and final
source/binary identities. Both final optimized profiles pass 696 units with two explicitly ignored measurements.
Eight actual C11/C++17 programs cover both native profiles and ARM32/ARM64 release under QEMU; each preserves all
103 full-suite records and 72 label-output arms. Formatting, strict production/fixture Clippy and installed Rust
1.91 all-target compilation pass. The MSRV check is compile-only; foreign release-min and physical ARM throughput
are outside this panel.

Run the bounded constructor measurement from the Cargo root:

```sh
cargo test --locked --features fixtures --release accounting::tests::measure_empty_vector_owners -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --features fixtures --profile release-min accounting::tests::measure_empty_vector_owners -- --exact --ignored --nocapture --test-threads=1
```

The unchanged measurement body was added before baseline preservation. Nine cases construct/drop one million
empty vectors each, without retaining an array. Direct, existing-context and untracked u8/u64/zero-sized values
assert complete query/engine snapshot and capacity equality. The existing actual-adapter measurement runs beside
them. Eight alternating process pairs per profile/probe discard the first warmup pair, yielding 240 complete
case pairs, 210 measured pairs and 30 groups. All nonelapsed fields match exactly.

| Workload | Release before → after | Release-min before → after |
| --- | ---: | ---: |
| 1,000,000 direct u8 owners | 28.597 → 17.660 ms | 28.356 → 22.992 ms |
| 1,000,000 direct u64 owners | 27.938 → 17.591 ms | 28.201 → 23.130 ms |
| 1,000,000 direct zero-sized owners | 27.509 → 17.233 ms | 27.438 → 21.760 ms |
| 100,000 float callbacks | 5.132 → 3.933 ms | 6.674 → 6.455 ms |
| 50,000 two-float callbacks | 7.165 → 5.641 ms | 11.234 → 9.914 ms |
| 10,000 histogram callbacks | 1.707 → 1.399 ms | 2.852 → 2.532 ms |

These are seven-pair median fixture-loop times. Unchanged-context and untracked controls vary too: for example,
release-min untracked u64 is 8.968→9.163 ms, while context u64 is 26.961→20.224 ms. The full report retains all
groups and raw samples; unrelated control variation does not establish causal speedups or zero overhead.

Actual emitted-code checks select the concrete raw u8 constructor symbol and verify its address against all
demangled aliases. Resolve PIC indirect calls through actual ELF GOT relocations: an objdump display alias or
missing printed callee name is insufficient. Both release unit and default-feature C caller reduce checked atomic
increments from two to one. Both size-profile owners remove the temporary context/new_in route and call
`QueryAccounting::hold` directly. Their initial stack reservation falls 24→0 B and saved registers two→one;
release falls 40→32 B with the same register-count change. These are constructor prologues, not whole-query stacks.

The containing Release Agent gains 29,700 B of executable text and 24 B of data; BSS stays equal. Removing a
temporary reference does not remove an allocation: all constructor snapshots, actual C/C++ records and the 16
complete real Agent output/account records remain equal. No heap-byte, peak-RAM or process-RSS reduction is claimed.
The original private baseline reader's wrong shared-test count and metadata readers' alias/GOT assumptions remain
recorded separately from successful engine executions. No runtime check was repeated to repair those readers.

The real containing Agent passes the unchanged six-phase, 672-record storage/API panel and 82 HTTP/readiness
checks plus 224 complete measured responses. Both private Agents exit normally. The fixed warm live API panel
includes these median wire times against the retained singleton-output predecessor:

| Delivery workload | Raw before → after (ms) | Gorilla before → after (ms) |
| --- | ---: | ---: |
| 100,000 points, plain | 96.389 → 91.916 | 92.698 → 95.161 |
| 100,000 points, gzip | 103.691 → 103.663 | 103.550 → 105.923 |

All complete bodies, statistics, clocks, serial medians and two-client spans are rechecked. Other controls are
retained too: the raw six-group histogram is 18.959→20.445 ms. Mixed live results do not establish an end-to-end
speedup. These separately run cohorts use a warm, small two-host/one-tier fixture on a shared workstation, without
CPU isolation. Large `vector(1)` delivery does not read 100,000 stored DBengine samples; client overlap does not
establish concurrent evaluator throughput. No cold-disk, high-cardinality, saturation or full-language claim is added.


## Histogram group contributors

Histogram consumers now request the existing ordinary-aggregate contributor plans. This removes repeated work on
unrelated groups in expressions such as `histogram_quantile(.5,sum by(job,le)(rate(buckets[3s])))`. The shared
classifier covers `histogram_quantile`, `histogram_fraction` and optional `histogram_quantiles`, including all eight
ordinary reducers. Native/mixed values keep their general evaluation path; join-only Float/cardinality restrictions
are not histogram restrictions. This is a performance change, with no language feature added or dropped.

A transient bit on the existing accounted preparation-task stack carries demand to the resolved vector argument,
then through supported maps and ordinary nested aggregates. Scalar/string arguments, unsupported shapes, invariants
and clock boundaries stop propagation. A histogram reached inside another scope seeds its own demand. Existing
selector/grid eligibility, child output ordinals, successful full-child completion, exact keys and preparation
clearing still govern filtering. Values use their original evaluators. No raw-history array, value cache, copied
child labels or persistent per-node context array is added.

The [qualification](histogram-contributor-qualification.json) records both optimized profiles: 698 passing units and
two ignored measurements each. The 436-request bridge covers all three consumers/eight reducers, independent name
modes, by/without, native/mixed inputs, aliases, maps/nesting, sparse/missing data, fixed/offset scopes and
unsupported fallbacks. All 2,616 native and 2,616 compiled ARM32/ARM64 release comparisons at pages 1/17/1024 match
the pinned original semantically and bitwise. Foreign binaries retain their actual precursor source identity; a
later correction changes only the excluded Rust test module, with byte-identical production prefix and other inputs.

Use `histogram_contributor_cases.cases()` to produce the wire dataset and requests. Verify an existing
independent-name probe with `plural_histogram.load`, then execute that bound `capture/plural_histogram.go` probe
with the new dataset and request JSON files. Convert Rust packets with `query.dataset_hex`/`request_hex`, validate
them with `checked_candidate_packets`, and use `compare.compare` with exact numerics and range ordering. The fixture
keeps one current dataset; changing cohorts requires a dataset packet. Preserve explicit name overrides and the
probe's 60,000 ms default subquery interval. Original outcomes and source/binary identities are retained separately
from timings.

Run the `ordinary_histogram_contributors_bound_unrelated_groups` and
`histogram_group_contributors_keep_cold_and_selected_failures` unit filters in either optimized profile. The first
checks complete scalar outcomes and bounded cardinality work for all three functions. The second drives the call
body directly, checks successful completion, and calibrates genuinely subsequent contributor reads before injecting
cold/warm failures and cancellation. API callbacks can follow buffered output-head preparation; they do not by
themselves identify those evaluator phases. Separate API sink abort/cancellation checks verify release.

The checkpoint's 36-case `histogram_contributors.c` version reused the paged provider and asserted complete
labels/kinds/timestamps/bits, diagnostics, logical totals and release across raw/window/name/page cases.
Eight actual before/candidate programs across both native profiles and ARM32/ARM64
release pass 288 query assertions. Logical totals stay 4/12; raw quantile useful reads fall 80→32 and fetched
entries 240→96, while sample peak stays 6. Rate inputs retain their existing fold reuse and the same physical
counts. Eight rebuilt existing C/C++ callers also preserve all 103 full-suite and 72 label records each. Format,
strict production/fixture Clippy, installed Rust 1.91 compilation and 41 real HTTP/readiness checks pass; the owned
Agent exits normally.

The finite timing panel has six alternating before/candidate process pairs, discards the first pair, checks all 504
complete query outcomes and each binary's stable nonelapsed observations, and retains 15 workload groups:

| Workload | Before → after | Speedup |
| --- | ---: | ---: |
| instant histogram, 1 job | 0.708 → 0.458 ms | 1.54× |
| instant histogram, 4 jobs | 7.151 → 1.624 ms | 4.40× |
| instant histogram, 16 jobs | 104.779 → 6.426 ms | 16.31× |
| three-tick vector range, 4 jobs | 12.002 → 3.353 ms | 3.58× |
| three-tick vector range, 16 jobs | 168.826 → 21.593 ms | 7.82× |
| three-tick scalar range, 16 jobs | 337.495 → 49.933 ms | 6.76× |

These are native-release fixture execution medians on a shared host, without CPU/frequency isolation. Each job has
two workers, three classic bounds and three stored points per input series. The instant case with 16 jobs
reduces work 9,829,428→404,702 and workspace traffic 50,706,362→2,789,938 B. Peak workspace rises 145,960→148,288 B
for membership. The 436-case bridge has 0–1,872 B higher workspace peaks, with all seven other memory-kind peaks
unchanged and identical native-profile nonelapsed observations. This does not establish lower RSS. Non-histogram
controls retain identical tracked allocations/work; tiny-vector and one-job grouped-rate timing controls are about
1.1%/0.4% slower. All controls remain in the report. The containing Agent gains 8,477 B of text, loses 40 B of data
and keeps the same BSS.

At this checkpoint, ordered range consumers still repeat some All discovery: candidate vector-range work grows
213,178→1,690,703 when
jobs grow 4→16; scalar-range work grows 432,986→4,214,386. That residual remains a separate measured opportunity.
The unchanged small live API checks establish integration regressions, not a live speedup, cold-disk,
large-cardinality, concurrency/saturation, physical ARM throughput or complete-language proof. Raw paging,
provider-owned temporary backing and disabled implicit caps remain unchanged.

## Ordered histogram streaming

Literal `histogram_quantile` and `histogram_fraction` can retain one existing evaluator behind the ordered-output
adapter when their frozen bindings already match canonical output identities. Every binding must participate once,
with matching labels/hash/drop flags and a single Float head. Scalar parameters must resolve to numeric literals.
The runtime filter must be `All`. Dynamic scalar parameters, selected/no-match filters, aliases, reordered bindings,
empty rosters and optional plural quantiles keep their existing route. No language feature is added or dropped.

The certificate is published after the histogram's own complete node finishing. Own logical preparation is excluded;
later ancestors consume the already-frozen child through the adapter. Child annotations and logical primary sample
units keep their original owners. Each invocation performs fresh full-vector discovery and ordinary fresh native or
classic value reads. The certificate stores order eligibility; it grants no discovery credit and retains no values.

The adapter preserves descriptor hash/head/drop handling and aligns each sparse current sample by its emitted
canonical ordinal. A whole-query roster may be nonempty while a particular tick has no output. Vector forwarding
keeps the shared child until actual Done; parent truncation drops it through the ordinary frame lifetime.

The distinguishing control compares instant, equal-bound range and unequal-bound one-tick range requests at the same
evaluation time. The one-tick form cannot revisit time: its old excess work isolates per-row construction. Include
three-tick ranges and grouped-rate/raw controls. Check complete results before work predicates; a work counter is not
a CPU measurement. The regression filters are `ordered_literal_histograms_do_not_restart_for_every_group` and
`ordered_histogram_streaming_keeps_acquisition_and_filter_phases`.

The phase test drives actual adapter execution, calibrates cold and genuinely later acquisitions, then injects
provider errors/cancellation and checks release. It covers classic/native admission, reordered-native fallback,
One/Ordinals/Contributors, empty selections, ancestor truncation and provider-backed scalar re-entry. Own logical
preparation keeps the original path; early certification must not be confused with cached discovery completion.

Generate focused expressions with `histogram_streaming_cases.cases()`. It retains the 436 contributor expressions
and adds 288 cases for sparse leading/middle/trailing groups, an entirely empty tick, equal numeric bucket spellings,
native values, wrappers, dynamic parameters, selected parents and fallback clocks. New stable cases use the default
profile; plural quantiles retain the explicit test profile. Compare the complete bound original probe output, exact
bits, independent name overrides and range order using the existing checked packet/comparison helpers.

The existing C histogram caller now includes unequal-end one-tick ranges beside instant requests: 72 full queries
per program across functions, raw/rate inputs, names and pages. It asserts complete membership, labels, timestamps,
bits, diagnostics, logical totals and cleanup; physical reads and live-sample peaks remain observations.

A Plan flag and a longer-lived Step have measurable costs. Preserve all peak categories, allocation traffic, slower
controls and actual caller statistics beside finite timings. Lower allocation traffic does not prove lower peak RAM
or process RSS. The initial adapter qualification below predates the separate frozen-emission improvement.

Frozen Quantile/Fraction emission reuses the ordinal already selected by `choose`, with checked bounds and the
existing key/occurrence validation. This includes dynamic parameters and selected filters. Own logical preparation
and Quantiles retain their original searches. The existing ordered certificate has a separate use: canonical
assembly merges equal hashes before certification, so the complete singleton binding bijection proves distinct
binding hashes. Certified emission outside own preparation can omit hash computation, duplicate scans and seen
storage. Uncertified output retains hash-only duplicates, including unequal byte labels with the same full hash.

Omitting the hash walk must retain the control poll after label copy, before phase publication and fallback sample
ownership. Test cancellation at the final label-copy work charge and require zero sample peak. The distinguishing
unit filters are `frozen_histogram_emission_reuses_selected_identity_with_bounded_work`,
`certified_histogram_emission_polls_after_copy_before_sample_ownership` and
`histogram_full_hash_collisions_keep_uncertified_merge_and_duplicate_behavior`. The growth control checks complete
outputs and cleanup before measuring emission alone, keeping group-presence scans outside the measured interval.
Repeated selected ordinals emit once; actual disjoint/overlapping collision timelines retain merge/error behavior.
The [separate qualification](histogram-emission-qualification.json) records its actual source, binaries and costs.

The emission qualification passes 703 units with two ignored measurements per optimized native profile, 4,344 native
original comparisons at pages 1/17/1024 and 1,448 compiled ARM32/ARM64 release comparisons at page 17. Four current
default-feature C programs cover all three page sizes and 288 complete queries. Sample statistics and fetched entries
are unchanged. Other ABI, Agent/live, installation and language cohorts retain their historical source identities.

At 128 bindings, isolated certified emission falls from 19,200 to 1,536 work units; an uncertified last-only selection
falls from 150 to 23. Whole-query timing differences are modest and comparable to control variation: 256-group scalar
quantile range medians are 332.44/326.77 ms, and fraction medians are 305.75/298.02 ms. Fourteen of 84 workloads are
0.08–4.41% slower in the finite alternating panel. This does not establish a general throughput improvement. Across
724 same-input cases, workspace peak changes are -32..0 bytes and allocation traffic changes are -16,704..0 bytes;
the other seven memory categories are unchanged. Keep traffic, simultaneous peak and process RSS distinct.

The initial adapter [qualification](histogram-streaming-qualification.json) records 700 passing units and two ignored measurements
in each optimized native profile. All 4,344 native and 4,344 compiled ARM32/ARM64 release comparisons match the
pinned original semantically and bitwise at pages 1/17/1024. The release page-17 cohort reuses the verified output
of the identical earlier candidate binary. Earlier release/ARM32/containing artifacts keep their actual source
identity: a later correction changes only the excluded `lookup_tests.rs` physical-read expectation. Every other
348-input core entry is byte-identical. No historical reference or runtime accounting algorithm was changed.

The corrected eight-tick raw histogram test reads 7,168/14,336/28,672 useful samples for 16/32/64 groups. Raw
preparation contributes `16g`, and histogram preparation/final consumption contribute `216g` each, giving `448g`.
The previous per-output final discovery cost `72g² + 376g`; logical queryable totals and complete outputs stay exact.
The first full release run rejected the smaller physical count; the rebuilt full rosters pass the corrected check.

Eight actual before/candidate C programs pass 576 complete queries. Raw one-tick Quantile/Fraction useful reads
fall 36→32 and fetched entries 108→96, with the same 6/7 sample peaks. Rate inputs retain their existing folded
reads and logical totals 4/12 stay unchanged; plural quantiles retain their prior route. Eight existing C/C++ callers
pass 103 full and 72 label records each. Their precise comparison allows only 15 affected work/read records;
statuses, chunks, calibrated first/failing cursor phases and all other fields remain exact. Format, strict
production/fixture Clippy, installed Rust 1.91 compilation and 41 real HTTP/readiness checks pass, with normal Agent exit.

The timing panel runs six alternating before/candidate process pairs, discards the first pair, and checks all
1,728 complete outcomes plus each binary's stable nonelapsed observations. Each of its 72 workload groups has ten
measured samples per side. Inputs have 1/4/16 jobs, two workers, three classic bounds and five stored points per
series; ranges evaluate at 3000–5000 ms. The unequal-end one-tick form uses 3000–3001 ms with a 1000 ms step.

| Workload | Before → after | Speedup |
| --- | ---: | ---: |
| one-tick vector range, 16 jobs | 7.662 → 4.074 ms | 1.88× |
| one-tick scalar range, 16 jobs | 17.467 → 6.680 ms | 2.62× |
| three-tick vector range, 16 jobs | 22.753 → 12.016 ms | 1.89× |
| three-tick scalar range, 16 jobs | 53.050 → 20.085 ms | 2.64× |
| three-tick scalar range, 4 jobs | 6.592 → 5.034 ms | 1.31× |

These are native-release fixture medians on a shared host without CPU/frequency isolation. All 72 groups remain
in the report: 42 have slower medians by 0.1–3.9%, including small single-group cases and unchanged-work controls.
The panel does not establish their causes or a universal speedup. No additional timing run was used to replace
those results. Instant/equal-bound routes and non-histogram controls retain their original work counts.

The 16-job three-tick scalar case reduces work 4,242,034→1,217,319 and workspace traffic 24,706,684→7,976,790 B;
peak workspace is 153,542→153,550 B. The one-tick scalar peak falls 152,870→151,998 B. Across the 724-expression
page-17 bridge, workspace peak rises 0–40 B; all seven other tracked memory-kind peaks remain unchanged. Both
native profiles have identical nonelapsed observations at each page size. The containing Agent gains 4,586 B of
text and 120 B of data, with unchanged BSS. Lower traffic does not establish lower process RSS. Frozen binding and
seen-hash searches remain; no global linear-work, live speedup, concurrency/saturation, cold-disk or physical-ARM
throughput claim follows from this finite panel. Raw paging, provider-owned backing and disabled implicit caps remain.
