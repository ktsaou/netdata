# PromQL conformance foundation

This directory defines the independent reference and comparison contract for completing the Rust engine.
It targets the complete pinned language, including all required SRE recording and alert expressions.
The Rust engine is not qualified by these reference self-tests or by parsing the SRE corpus.

The historical C/C++/Rust experiment remains in `tests/promql-prototype/`, with its original evidence intact.
This harness uses a separate Go module and the same dependency locks. It does not run a daemon or change metrics.

## Reference and profiles

The reference is **Prometheus v3.15.0**, module `v0.315.0`, commit
`5241a27fe3c6983549fccc32f6e65917408c63cd`. Commit identifiers, module checksums and source digests in this
directory identify upstream source; they are not credentials or private identities.

- `server-default`: stable functions and syntax, duration expressions enabled, `@` and negative offsets enabled.
  Delayed name removal and start-timestamp evaluation are disabled.
- `upstream-tests`: the four parser options used by the upstream builtin tests are enabled, together with delayed
  name removal and start-timestamp evaluation. Optional syntax remains attributable to this profile.
- `language-catalog.json`: the pinned function signatures, 90 functions (73 stable, 17 experimental), 14 aggregations
  and profile options. This is an inventory, not a list of implemented Rust features.
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
retains a uniquely named input directory for reproduction.

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
- Strings use base64 `string`; a missing string field represents an empty string. Output histograms are float histograms.
- Matrix output may contain a float and a histogram at the same timestamp. Timelines remain strictly increasing
  within each kind; comparison pairs points by timestamp and kind. Equal-time cross-kind serialization order is
  unspecified. Input timelines remain strictly increasing across all kinds, and vector rows contain one point.

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

These are reference fixtures and passing original assertions, not a Rust conformance result. Candidate execution
and the remaining helper surfaces still require qualification.

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

`src/crates/promql-engine` is a standalone Cargo workspace, edition 2024 with an MSRV declaration of 1.91.
It is outside the Agent/plugin default build. The library uses pinned `regex-automata` and `regex-syntax`;
the optional `fixtures` feature adds `serde_json` for the test binary's projections.
The checked toolchain is Rust 1.97.1; the MSRV itself has not been built here.

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
  the plugin workspace's abort profile. A C panic boundary still requires implementation and qualification.
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
the pending PromQL `label_replace` evaluation and label mutation. PikeVM construction currently omits meta-engine
accelerators; its query performance has not been measured. Go syntax limits and optimizer strategy thresholds are
preserved as upstream behavior. No engine regex or query resource quota is enabled.

Opaque regex-backend allocations and selector hash-table backing memory are reported as explicit estimates.
Retained regex plans, keys, names and compilation scratch use observed buffer capacities. Invalid-byte subjects,
offsets, normalization, retry frames, captures and replacements use the current search owner; shared compiled
matchers retain one charge. Public backend memory reports are observed after construction/mutations and searches.
Their hidden compilation/search capacities and temporary peaks, and hash-table resizing peaks, remain unknown.
Observable engine-owned buffers and borrowed storage pages remain subject to precise accounting. A total containing
estimates cannot be a guaranteed bound. This accounting trade-off retains the qualified matcher and all 21,320
original regex cases pass unchanged in both build profiles. Runtime accounting itself remains unfinished.

## Typed sample and initial histogram kernels

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

Frexp, Ldexp, Exp2, natural/binary logarithms and public square root are implemented for the pinned Go 1.27.1 linux/amd64
profile. Log preserves the actual assembly's subnormal extraction, negative-NaN handling and inclusive half-sqrt-two
branch. Log2 retains the exact-power shortcut and ordered multiplication/addition. Sqrt uses the same compiler
primitive as Go's public intrinsic. Exp2 preserves the public generic reduction and polynomial order; Ldexp
retains signed special values, subnormal scaling and machine-int wrapping. These operations use constant auxiliary memory.

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

`HistogramFraction` and `HistogramQuantile` follow pinned `promql/quantile.go:225–516`. Each numeric scan borrows
the histogram, retains one bucket plus iterator/rank state, and returns one scalar plus an optional typed info
identity. It does not copy or compact input buckets. This preserves custom infinite-bound early returns,
slice-length zero-side checks and the quantile NaN continuation's final-bucket overwrite. Diagnostic formatting
uses the qualified Go byte quoting and original query byte positions; numeric scans allocate no message strings.
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

Both release profiles pass all 7,611 comparisons. Each reports 80 numeric bit diagnostics, consisting of 40 NaN
results and their 40 reciprocals; all discrete outcomes match. Strict-bit mode fails those reported diagnostics.
Twenty-four original DSL declaration-versus-Go differences remain separately reported and satisfy the original
`almost.Equal` assertions. These helper gates do not establish complete Rust expression evaluation.

## Native histogram statistics

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

Both release profiles pass all 1,270 comparisons, including exact input snapshots, nil helper diagnostics and
reciprocal/threshold outcomes. Each reports eight NaN-only numeric bit differences; strict-bit mode fails those
diagnostics. Fourteen original declaration-versus-Go differences are reported separately and satisfy the pinned
assertions. Overflow and compensation consumers distinguish reordered arithmetic that changes a comparison.
Float omission, name removal, inherited warnings and full expression outcomes still require the Rust evaluator.

## Classic histogram bucket kernels

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
4,176 cases, with four NaN-only candidate bit differences and three finite declaration-versus-Go
differences. Every finite candidate value matches Go's bits; strict-bit mode rejects the four NaN diagnostics.
Complete grouped expressions,
diagnostic formatting, storage replay and C accounting remain later qualification gates.

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

The full run after native histogram projections, timestamp support, intermediate ordering, native scalar/vector
maps and native vector aggregation matches 6,753 of 21,442 variants, with zero completed semantic failures and 14,689 open variants at page sizes
17 and 1024. Its 24 paired bit differences are
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

## Remaining qualification

The productive Rust path connects the owned frontend to typed page cursors, float rate/increase/delta,
count/present/first/last/min/max over time, float/native histogram sum/average and float variance/stddev over time, timestamp extrema/endpoints,
changes/resets, scalar/vector arithmetic
and comparisons, float/native sum/avg/count/group and float min/max/stdvar/stddev with native ignored-input infos. Evaluation uses explicit frames,
whole-query label/group descriptors and replay; raw histories and complete raw instant vectors are not retained.
Output uses synchronous provisional chunks followed by commit/abort, with a separately owned collector. Stable
series identities retain legitimate duplicate projected labelsets and merge disjoint rows at cleanup owners.
The optional collector materializes the complete output and can grow with it, including raw selector results;
use the chunk interface to stream large outputs. Reported execution/collection methods retain counters on failures
as well as successes; the convenience `Result` methods return counters only on success.
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
`histogram_stdvar`, `histogram_stddev` and `timestamp`. Projections ignore floats; timestamp accepts both value
types and distinguishes raw sample timestamps from derived evaluation timestamps. Eager metadata removal follows
each source function; delayed removal occurs at final output. Duplicate checks and child diagnostics retain their
source boundaries. Histogram variance polls cancellation during bucket traversal. Fresh Go checks match 4,134
pointwise, composed, multi-step and disjoint-alias cases in both build profiles at page sizes 1, 17 and 1024;
all 33 paired bit differences are NaN signs in nested nonfinite sums, reported separately from semantic comparison.

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
bits. Native trimming and vector/vector operators remain required implementation.

Selectors under histogram count/sum/average use the source's statistics decoding for ordinary range folds.
Subqueries, bucket-dependent histogram functions and trimming disable it. One tracked full histogram supplies
reset detection; consumers receive only schema, count, sum and reset hint. Prefix replay reconstructs buffered
access and moving-seek state without retaining raw windows. Two direct pinned-Go buffered iterator witnesses
distinguish unaccessed lookahead from an accessed endpoint; all 30 original nested reset-warning cases now pass.
Repeated prefix I/O/work is accounted. Ordinary instant statistics replay now preserves future-current access
before predecessor selection, repeated-current caching, different memoized/raw seek reset conditions and EOF.
Finite static query-wide bounds include sibling selectors, original request endpoints, static subquery ranges and
offsets. Dynamic durations and `info`'s implicit metadata scope remain required with their unfinished owners.
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

Runtime accounting remains unfinished: control-object allocations, general frontend compilation temporaries and
provider-internal transient peaks are not fully instrumented. Regex dependency/table estimates expose known
limitations rather than exact allocator totals. Reported buffer capacities and exposed page ranges must not be read as exact total
query allocation or RSS. Provider callbacks return resource snapshots on every outcome; normal ownership transfers
are reconciled together. Unwinding skips additional provider callbacks, so a newly allocated panicking callback may
have an unobserved peak. No configured resource or concurrency admission limit is enabled.

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
and no numerical tolerance is widened. Future implementations of currently unsupported map boundaries must use
the same policy and preserve source-defined ordering elsewhere.

Replay repeatedly enumerates selection metadata and skips unneeded raw cursors. In a finite release fixture probe
with three output steps and eight raw points per series, increasing from 32 to 128 series kept borrowed-page
observations at 2,176 bytes. Workspace rose from 40,320 to 157,440 bytes for `m * 1`, while provider calls rose from
3,037 to 30,541. A second nested map increased the 128-series calls to 83,917. This confirms a metadata cost that
can be quadratic and compounded by nested replay; it does not establish total RAM, production latency or
dbengine performance. Metadata and independently collected output may grow; all admission limits remain disabled.

The foundation represents the full value model and records required inputs. The following remain mandatory in the
typed Rust core stage:

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
remain precise, and totals containing estimates are not guaranteed bounds. Live dbengine, APIs, Go Cloud distribution and independent rule lifecycle are
later tracked stages. Alertmanager remains excluded.
