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
accelerators; its query performance has not been measured. Go syntax limits and optimizer strategy thresholds are
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
full correction sweep, literal Go binary-search prefix probes and one endpoint sweep; fraction makes a
terminal-count sweep and a rank sweep. For N inputs and M coalesced runs, count reads are bounded by
N*(2+ceil(log2 M)) and 2N respectively. Child-expression replay adds its own work. No raw or corrected count
array, history buffer or additional resource cap is introduced. The canonical numerical contract below
now supplies the retained AMD64 operand behavior on every device.

```sh
python3 tests/promql-conformance/classic_replay.py --capture \
  --mod-cache "${PROMQL_GO_MOD_CACHE:?}" --reference "${PROMQL_CLASSIC_CAPTURE_DIR:?}" \
  --binary "${PROMQL_KERNEL_BINARY:?}"
python3 tests/promql-conformance/classic_replay.py --supplement "${PROMQL_CLASSIC_OPERAND_CAPTURE_DIR:?}" \
  --reference "${PROMQL_CLASSIC_CAPTURE_DIR:?}" --binary "${PROMQL_KERNEL_BINARY:?}"
```

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

Grouping, TOPK/BOTTOMK, both LIMIT operators, aggregate QUANTILE and vector matching use compact accounted indexes
over their existing metadata arrays. Indexes preserve each owner's original equality and first writer; they never
drive reductions or output order. Binary signatures/cache keys still compare complete encoded bytes, including
the source's undelimited concatenation aliases. Active membership, duplicate validation, reservations and output
uniqueness retain their original phases. Ordinary aggregation releases positions before reversing/removing groups.

Empty/singleton indexes have no slot allocation. Sparse slots grow at two-thirds load; active signature domains
use dense positions only when they fit the sparse replacement already required. Rebuilds charge both allocations
and check cancellation before publication. They contain positions rather than raw samples or copied payloads.

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

## Completed unique float join scans

Final evaluation can reuse a completed operand scan for a bare root one-to-one join between two raw float
selectors. It requires unambiguous signatures, hashes, output bindings and supported ordinary clocks. Native,
grouped, filled, nested, delayed-name and other ambiguous shapes keep the existing pair replay.

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

Bare-root one-to-one float joins can reuse completed discovery when an operand is an ordinary direct
`rate(raw_selector[range])`. Raw-selector eligibility stays the same. The rate producer/source/scope grids must
match the effective evaluator, and the range/offset arithmetic must produce nonwrapping window endpoints.
Grouped, filled, delayed-name, invariant, extended, wrapped and other computed shapes retain ordinary replay.

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
