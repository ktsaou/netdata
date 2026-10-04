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

## Remaining qualification

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
enforcement must adapt to the device. Live dbengine, APIs, Go Cloud distribution and independent rule lifecycle are
later tracked stages. Alertmanager remains excluded.
