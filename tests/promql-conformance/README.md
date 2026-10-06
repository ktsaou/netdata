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
The shared frontend now also handles Go hexadecimal floats with sticky truncation, subnormal ties-to-even rounding
and range-error classification. Its decimal scratch is accounted, and its input walks support cancellation/deadlines.

`float_literals.py` captures all 264 original Go 1.27.1 float-test inputs and independently generated boundaries,
specials, raw bytes, long mantissas and round trips. It verifies the pinned runtime source hashes, retains the source,
probe, requests and results in ignored `.tmp/`, and compares value bits and error classes exactly. The 12,154-case
panel passes in both release profiles. All 243 unit tests pass in both profiles and Rust 1.91; the existing
157 lexer, 356 AST, 784 parser-option and 32 literal cases continue to match. The complete expression panel remains
13,330 passing variants and 8,112 unsupported variants, with no lost passes or new paired numerical differences.
This is a finite parser-helper check; histogram expression execution remains a separate required surface.

```sh
python3 tests/promql-conformance/float_literals.py
python3 tests/promql-conformance/float_literals.py --capture "${PROMQL_FLOAT_CAPTURE_DIR:?}" \
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

Power, remainder, min/max and natural exponential now follow the pinned source algorithms too. Exp uses Go's
amd64 polynomial, scaling order and AVX/FMA dispatch; power reuses that dispatch for fractional exponents.
All 547 additional math inputs match exact bits in both build profiles against the native Go run and separate
FMA-disabled and AVX-disabled runs. The resulting power threshold and duration cases demonstrate that dispatch
can change series membership. Variable power/remainder loops poll cancellation and account work without an
iteration cap. Broader platform qualification remains required.

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

Both release profiles pass all 1,270 comparisons bit-for-bit, including exact input snapshots, nil helper diagnostics
and reciprocal/threshold outcomes. The earlier eight NaN differences are corrected on the qualified AMD64 target.
Fourteen original declaration-versus-Go differences are reported separately and satisfy the pinned
assertions. Overflow and compensation consumers distinguish reordered arithmetic that changes a comparison.
Composite expression coverage is measured separately by the evaluator gates below.

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

## Remaining qualification

The productive Rust path connects the owned frontend to typed page cursors, float/native rate/increase/delta
and irate/idelta, deriv/predict_linear/double_exponential_smoothing,
count/present/first/last/min/max and absence over time, float/native histogram sum/average and float variance/stddev over time, timestamp extrema/endpoints,
changes/resets, scalar/vector arithmetic
and comparisons including power/remainder, float/native sum/avg/count/group and float min/max/stdvar/stddev with native ignored-input infos. Evaluation uses explicit frames,
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

The numerical kernels follow Go 1.27.1, including its fixed-size Payne-Hanek reduction, actual exponential CPU
dispatch, untyped reciprocal constant rounding, signed-zero/NaN cases and UTC unsigned epoch wrapping.
`days_in_month` retains the source Date round trip rather than substituting a month-length table; extreme values
can consequently return seven. Degree/radian calls retain multiplication-before-division overflow behavior.
On the qualified Linux amd64 target, 5,854 raw numeric/calendar cases match every bit in both Rust profiles
against native, FMA-disabled, SSE4.1-disabled and combined-disabled Go captures. Six hundred additional Go
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
cancellation inside bucket loops. Their temporary arrays and shifted merge cells are accounted. Native Kahan
aggregation and other remaining callers still require those interruption hooks; this rate qualification does
not establish complete runtime accounting or full expression/C compatibility. Replaying the shared 6,440-case
histogram helpers preserves their ten/one NaN-sign differences in release/release-min and three declared-bit
reports. Replaying all 1,270 statistics helpers preserves their eight paired and fourteen declared-bit reports.

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

Runtime accounting remains unfinished: storage-error/sink owners and provider-internal transient peaks are not
fully instrumented. The frontend buffer checkpoint below replaces its retained census. Regex dependency/table estimates expose known
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

Each query now opens every explicit AST selector site once, in syntax order, before expanding metadata. Identical
selector text at different sites retains separate selections. Expansion retains series handles and provider
ordinals in a tracked catalog, exhausts metadata before raw reads, and lends one handle at a time to bounded
page kernels. Physical replay reuses those handles. Original query-wide bounds are passed to the provider before
offset adjustment; each selection carries its original bounds and borrowed step/range/function/grouping hints.
Compiled preprocessing metadata preserves source helper visits, invariant wrappers and surviving parentheses,
including unvisited aggregation parameter subtrees. Hint grouping retains original label order and distinguishes
an absent grouping from an explicit empty one. These metadata projections do not establish execution of currently
unsupported parameterized aggregations or subqueries.

Terminal provider diagnostics retain explicit severity, opaque bytes and optional positions in independently
owned results. Expansion failures carry their own notes and completed numeric parameter notes according to source
boundaries; unfinished ancestor and ordinary raw-helper failures discard their notes. Replays do not duplicate
diagnostics. Deferred selection errors retain their buffer capacity while awaiting their source evaluation owner.
Metadata handles and output can grow with cardinality; raw history remains page scoped. Bootstrap,
raw-error and sink-error accounting still require their remaining owners. Exact memoized initial reads, Seek
landing and lookahead failure precedence remain mandatory qualification work. All admission limits remain disabled.

Both builds pass 187 unit tests. The 64 independent selector-hint projections, 426 preparation scopes, 198 offset
projections and retained parser/clock/phase panels match their pinned source. The original corpus retains all
previous 9,684 passing variants and adds 230: 9,914 pass, 11,528 remain open, and the same 24 paired bit reports
remain. These are finite expression, catalog and
preprocessing checks; they do not satisfy the complete expression, SRE, C or accounting gates.

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
access and implicit shared-handle unsizing. Production Miri checks cover three seeds, alignment/DST/ZST,
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

Cleanup checks verify every memory category against exact live root counts, including `Accounting`; they do not
exempt bookkeeping from leak detection. Query snapshots include the query root that is live when the snapshot
is taken. Finite tests cover root identity, zero-capacity retention, creator/consumer attribution, concurrency,
poison recovery, scoped failures and panicking cleanup. Typed C ownership is exercised by the finite embedding checks
below. Complete foreign transient accounting and the full
expression/SRE gates remain required. All resource enforcement remains disabled.

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

`src/crates/promql-engine/include/netdata-promql.h` declares version 1 of the standalone interface. The crate emits an
`rlib` and a static library. Production Agent/plugin build integration belongs to a later stage.

- Engine, immutable query, control, report and result handles have paired release functions. Results and reports own
  their data independently of the query, foreign provider and engine handle. Individual getters borrow bytes and
  native arrays until the corresponding handle is released.
- Tables, options and requests have a version/size prefix; future trailing fields are ignored. Byte and array views
  use explicit counts. Shape checks cover active tags, reserved fields, optional presence, alignment, address/count
  arithmetic and required callbacks. Pointer liveness, correct handle types and nonracing access are caller contracts.
- Provider scopes, selections, series and cursors have paired ownership. Successful NULL opaque tokens receive their
  matching close. Series metadata survives selection close. Cursor pages survive until that cursor's next operation;
  one bounded raw page and its accounted view descriptors are retained. Repeated reads use the provider's stable input.
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
private `.tmp/c-embedding-*` evidence containing commands, source hashes, layouts, binaries and witness output. Both
unwind profiles compare all 50 declared structures/unions, 12 callback aliases and 33 shared constants between Rust,
C11 and C++17, and check header/archive exports. The real foreign callers exercise arithmetic, repeated paging,
resource conservation, independent results, opaque errors, output delivery and cancellation/error precedence.
ASan/UBSan instrument the C caller; the Rust core is exercised separately under Miri. `panic=abort` must fail the
crate's compile guard.

The generated-history sum witness uses pages of 1, 7 and 1,024 samples. Increasing history from 8,192 to 65,536 points
leaves observed workspace peaks unchanged: 4,795, 5,883 and 225,339 bytes respectively on the qualified amd64 host.
Borrowed raw-page peaks are 32, 224 and 32,768 bytes. Fetched sample/page counters equal the provider's actual physical
deliveries, including replay. These finite measurements do not bound cardinality-dependent operator state or output.

Current foreign resource snapshots observe allocations surviving callback return. They cannot observe scratch
allocated and freed inside a callback, or reconstruct concurrent temporary peaks. A precise foreign allocation-event
mechanism remains a required design decision. Regex backend memory remains explicitly estimated, all resource caps
remain disabled, and complete expression/SRE compatibility remains unqualified.

`label_replace` and `label_join` project whole-series metadata after the vector child completes. Runtime validation
occurs before child evaluation, following the upfront selector catalog. Replacement uses Go's anchored DotNL
expression and expansion rules; join reads original label values before changing its destination. Empty-label
normalization, missing and repeated sources, byte values, name-removal flags and derived timestamps follow the
pinned source. Only the vector argument is evaluated; surviving string parentheses inside unvisited aggregation
parameters retain their separate, currently open source branch.

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

The fixture transport optionally accepts synthetic `faults` on dataset requests. It exposes terminal selection
errors and opaque warnings plus per-selector/series raw read faults. Healthy page prefixes precede a failing
landing, so page size does not move the tested failure ahead of already available values. This is a finite test
adapter over immutable input, not the complete Go memoized/raw cursor state machine. Exact initial reads, Seek
landing, lookahead and their fault precedence remain separately required. Consequential native NaN architecture
behavior, remaining functions/subqueries, C/runtime accounting and SRE expression acceptance remain open.

This checkpoint passes 193 unit tests in both release profiles and Rust 1.91, plus focused Miri checks without
ignoring leaks. At pages 1/17/1024, both profiles pass 284 independently captured binary cases, 28 original fault
cases, four empty-fault-field controls and 80 retained actual-full-hash cases. Permanent tests cover chronological
cache ownership, history-independent raw workspace, current-pair histogram storage, a small caller stack,
late-child failure precedence, sink/cancellation cleanup and preprocessed aggregation diagnostic positions.

The original 21,442-case expression gate completes and passes 13,330 cases; 8,112 remain OPEN. Both profiles now have
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
Other targets retain their prior implementation pending the cross-device numerical contract.

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
