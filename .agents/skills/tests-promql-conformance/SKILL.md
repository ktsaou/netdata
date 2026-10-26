---
name: tests-promql-conformance
description: "Required when changing the PromQL frontend, evaluator, C ABI, accounting or compatibility claims. Use before selecting qualification or declaring same-input compatibility."
---
# PromQL conformance workflow

Preserve actual source/input/binary evidence while changing the independent engine.

## Scope

The Cargo root is `src/crates/promql-engine`. Commands and paths below are relative to that directory;
Netdata root instructions govern authorization and review.

Use for `src/`, `include/netdata-promql.h`, Cargo profiles and `tests/promql-conformance/`.
Netdata metadata, dbengine formats, HTTP transports and rule scheduling are caller integration work.

## Mandatory knowledge

- The reference is Prometheus v0.315.0 and Go 1.27.1, AMD64 with the retained FMA route. See `contract.json`.
- Default stable and explicit upstream-test profiles differ. An optional-feature witness does not qualify the default.
- Historical panels retain their original source and binary identities. See `core-qualification.json` and `IMPORT.json`.
- Raw history must remain paged on both axes. Temporary backing belongs to the provider; unavailable backing is an error.
- C ABI4 owns explicit handles, borrows callback buffers and commits provisional output only after successful evaluation.
- Ordinary C-boundary panics must unwind. An abort build is rejected by `src/lib.rs`.

## Best practices

- Compare complete outputs, labels, timestamps, warnings/errors and exact expression membership.
- Reuse verified source captures through explicit paths; provide an existing Go module cache for new captures.
- Read compressed historical evidence through EOF, verifying compressed and original raw digests before reuse.
  Keep historical attribution and compress new query captures without restoring duplicate raw files.
- Exercise pages 1, 17 and 1024 when changing cursor/replay ownership or temporal evaluation.
- Record compiler profile, binary hash, source hashes and comparison policy for each new cohort.
  For compiled footprint or timing comparisons, also retain relevant inherited Rust/Cargo/profile overrides,
  resolved tool identities and verbose effective compiler commands. Use an environment allowlist; keep missing
  historical settings explicitly unknown.
- Bind the comparison runner and imported helper bytes before and after each new cohort.
- Finalize artifact hashes after their owned writers finish; exclude a receipt from its own artifact map.
- Review fixed executable growth proportionally under
  `src/crates/promql-engine/tests/promql-conformance/DESIGN.md#proportional-compiled-footprint-review`.
  Report absolute, relative and cumulative costs separately from runtime memory; minor fixed increases do not
  require repeated exceptions. Preserve historical failed size gates and complete all other required checks.

## Bad practices

- Do not turn empty-data or parse-only checks into successful function-coverage claims.
- Do not edit retained reference results to match the candidate, or reattribute old binaries to new source.
- Do not replace bounded replay with a complete metric/time array or a hidden RAM spill fallback.
- Do not apply native-host math when it changes the selected portable arithmetic contract.

## Workflow checklist

1. Read `tests/promql-conformance/contract.json`, `DESIGN.md` and the affected qualification cohort.
2. Trace the affected core and C callback owners, including error/cancellation cleanup.
3. Choose the meaningful unit/differential/C embedding panels for that behavior.
   Create `tests/promql-conformance/.tmp` before fixture-backed units. Build the pinned oracle with `build.py`
   before running `test_numeric_oracle.py`; this test executes the original numerical helpers.
4. Run both release profiles when arithmetic, layout or optimization can affect behavior.
5. Retain new receipts under ignored `.tmp/`; update public qualification only with actual new evidence.

## Validation checklist

- `cargo test --locked --features fixtures --release` checks owned core behavior.
- `cargo test --locked --features fixtures --profile release-min` checks the independent unwind profile.
- `tests/promql-conformance/c_embedding.py` builds actual C/C++ callers; inspect `--help` for installed targets.
- `protocol_primitives.py --binary <protocol-fixture>` replays frozen parsing/formatting/selectors. Its two
  time-parameter and twelve batch-wrapper exclusions are caller coverage, not passing core cases.
- Its separate `--reference .../api-limit-original.json` gate replays34 unchanged pinned AMD64 limit outcomes,
  including64-bit width on ARM32. It has no wrapper exclusions; retain its distinct original/source/binary identity.
- Metadata discovery must fetch zero samples, release each provisional series and close providers before commit.
  Qualify normal END only for metadata rows, malformed nonmatches, first-error cleanup and owned output/report credits.
  Include selector-roster preparation in control/work/timing checks; cancellation must precede size-dependent copies.
- Native protocol quoting reuses the existing Go Unicode17 printability table through its checked primitive getter.
  Include control/nonprinting, supplementary, invalid Unicode and output-pointer boundaries in rebuilt C callers.
- `precision.py --binary <query-fixture>` replays nine precise-clock originals at pages 1/17/1024.
  Run both primitive gates for both profiles; neither qualifies an HTTP router or live provider.
- Histogram visitors and fixed-size basic statistics enlarge the ABI4 observation/layout roster. Rebuild actual
  C/C++ callers, verify generated layouts and keep caller sanitizer scope distinct from the Rust archive.
- Callback index sorting reuses the pinned Go controller; preserve equal-key comparison/swap provenance.
  Comparators must not access the exclusively borrowed index array. Check permutation/trace, empty and invalid
  shapes, error-before-cancellation and prompt borrowed-view release; rebuild the callback/layout C roster.
- Basic statistics describe actual paged ownership. Check useful reads against unused fetched prefixes and
  validation-only acquisition; check logical totals across primary folds/subqueries independently of replays.
  Include output/copy/replacement lifetimes and pinned native-histogram weights in peak checks. Do not equate
  these counts with RAM bytes or recreate upstream's materialized matrices to match its physical peak.
- Retained generic runners may use process-signalling timeout helpers. For a new qualification without explicit
  signal approval, use a recorded Popen/communicate owner and preserve child identity/completion receipts.
- Selector backend qualification MUST use anchored-selector originals; unanchored Find endpoints do not establish
  whole-subject acceptance. Prefix/group-zero, sparse/nullable captures, partial-construction ownership, full capture
  controls and binary-footprint checks are in
  `src/crates/promql-engine/tests/promql-conformance/README.md#selector-backend-capture-tables`.
- Source-defined selector prefix combinations MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#literal-prefix-selector-examples` for original table
  binding, preserved historical captures and complete complementary selector outcomes.
- Source-defined suffix and optional-character selectors MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#suffix-and-optional-character-selector-examples` for
  duplicate declarations, literal positions and complete complementary outcomes.
- Follow the conformance README for pinned differential panels; preserve its stated tolerance and bit diagnostics.
- Shared UTF-8 validation MUST preserve valid cross-batch scalars, valid U+FFFD, first-invalid-byte work and caller
  error/control phases. Compare actual fallback and capture consumers beside optimized selector controls; work
  totals are unchanged by fewer accounting updates. The finite boundary/original/timing procedure is in
  `src/crates/promql-engine/tests/promql-conformance/README.md#bounded-utf-8-validation`.
- Absence/preprocessing changes MUST include the label-sensitive parameter and ordinary/matrix controls in
  `src/crates/promql-engine/tests/promql-conformance/README.md#absence-argument-preprocessing`.
- Shared grouping/ranking/quantile changes MUST include the complete retained reducer panels and separate declared
  ordering controls in `src/crates/promql-engine/tests/promql-conformance/README.md#compact-group-and-join-lookups`.
  Preserve strict original failures; a policy report does not replace or modify them.
- Compact position slots MUST retain full-size positions, dense offsets and replacement leases; follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#four-byte-position-slots`.
  Its authenticated complete-result equality route MAY reuse unchanged strict/policy analyses with separate
  historical attribution and fresh current executions; original strict differences remain failures.
- Temporal rank completion MUST preserve both validated populations, source provenance and genuine final-depth
  fault controls; follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#early-completion-of-temporal-ranks`.
- Common-prefix rank jumps MUST preserve counted-digit reconstruction without a true jump and genuine prefix-ladder
  final-depth controls. Include no-jump timing costs; follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#common-prefix-temporal-ranks`.
- Mapped hash/kind uniqueness MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#mapped-label-identity-lookup` for compound identity,
  terminal controls, lazy metadata ownership and the limits of total-work claims.
- Final output identity lookup MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#final-output-identity-lookup` for first-emitted IDs,
  full-hash aliases, callback/control phases and measured metadata costs.
- Copied grouping keys MUST preserve raw empty values, source order and the shared by/without predicate;
  preserve ownership-transfer paths and full ranking labels. Measure retained capacity separately from inputs,
  and check omitted/retained byte walks plus terminal cancellation; see the same guide's copied grouping-label section.
- Prepared grouping ownership MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#prepared-grouping-key-ownership` for retained/transfer paths,
  repeated canonical contributors, raw/hash-only identity, selection-word boundaries and original larger-label fallback.
  Verify only-bit63/raw-order selection, checked copy/terminal cleanup, actual transient layouts/shared ordinary code,
  and constructor-versus-query acquisition. Preserve unsuccessful candidate measurements and every slower control.
- Parameterized preparation MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#parameterized-aggregate-preparation` for hash-only
  ranking, first-key copied quantile and later selection-word ownership. Preserve all input metadata/raw
  identity/parameter phases; exercise second-key masked/hash/copy controls and bit63 after a completed first
  group. Require exact original singleton/fallback work/memory; preserve rejected candidates and slower timings.
- Borrowed aggregate lookup MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-aggregate-lookup-labels` for selected-byte
  identity, zero/empty digests, reached diagnostic borrowing, ownership, controls and complete operator comparisons.
- Borrowed aggregate results MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-aggregate-result-labels` for frozen lifetimes,
  normalization, real/no-op mutation, retained ownership, interruption and actual layout/resource comparisons.
- Quantile result labels MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-quantile-result-labels` for frozen descriptor
  lifetimes, raw-empty normalization, retained/mutated output, terminal checks and separate original/order-policy scope.
- Canonical ranking-result ownership MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-ranking-result-labels` for the exact first
  representative, raw empties, fresh values/snapshots, replacement-site controls and complete C winner labels.
- Borrowed projection results MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-projection-result-labels` for exact frozen
  bytes, mutation/retention, terminal controls and page/target-matched compressed comparisons.
- Borrowed binary/set results MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-binary-result-labels` for exact cache/side
  bytes, mutable preparation ownership, terminal controls, Machine retention and fixture/C statistic scope.
- Completed sort publication MUST preserve callback/error phases, hash-only duplicates, eligible input kinds,
  delayed-name gates and snapshot lifecycle. Check lookup growth and terminal interruption, actual child acquisition
  phases and temporary allocation costs; see the same guide's completed vector-sort publication section.
- Completed sort emission MUST distinguish root successful Done from nested successful permutation prefixes.
  Preserve cold first-final progress, fresh selected values, unverified suffix failures and exact snapshot ownership;
  check preparation reset, sparse/nonmonotone ordinal translation and every credit charge. The root and nested
  procedures are in `src/crates/promql-engine/tests/promql-conformance/README.md#completed-vector-sort-emission`
  and `src/crates/promql-engine/tests/promql-conformance/README.md#nested-vector-sort-prefixes`.
- Cursor-fault qualification uses Netdata's `src/crates/promql-engine/tests/promql-conformance/cursor_policy_calls.py`.
  Its procedure and separate original/control reports are in
  `src/crates/promql-engine/tests/promql-conformance/README.md#physical-cursor-fault-qualification`.
- Foreign page-wrapper changes MUST preserve public marker/lifetime properties and whole-page validation phases.
  Run the focused ownership checks before broader rebuilds; the installed-tool and actual caller scopes are in
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-foreign-sample-pages`.
- Completed binary replay MUST preserve full first-pass failure priority before reusing membership metadata; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-unique-float-join-scans` for its causal controls.
- Unchanged provider resource batches MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#unchanged-provider-resource-snapshots` for ownership
  rejection, exact counters, transfer peaks and actual callback phases.
- Completed grouped subquery clocks MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-clocks-in-grouped-subqueries` for selected-group
  completion, mixed-history diagnostics, common-plan costs and actual failure/truncation boundaries.
- Completed binary subquery clocks MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-clocks-in-binary-subqueries` for actual
  Done/phase clearing, fresh both-side discovery, effective original options and separate strict/order-policy reports.
- Instant fold reuse MUST preserve required final initialization and exact-key checks, including absent summaries;
  see `src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-numeric-folds` for causal controls.
- Ordinary-window join changes MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-per-series-fold-join-discovery` for the
  shared Rate/Fold proof, global-absence exclusion, original-operand phases, numerical/name distinctions,
  clock/fallback boundaries, calibrated failures, C statistics and Pair costs. Window outputs earn no raw-point marker.
- Completed singleton aggregate values MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-singleton-aggregate-values` for actual
  pending-side/clock admission, terminal publication, independent sample ownership and bounded table costs.
- Literal scalar arithmetic reuse MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#literal-scalar-arithmetic` for immediate-number
  admission, actual operand phases, unchanged arithmetic, fallbacks and retained-versus-new evidence attribution.
- Generic derived range changes MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#known-empty-derived-histogram-probes` for actual
  Float completion, type-union provenance, backing fallbacks and source-bound physical/error/memory scopes.
- Ordinary selector clock changes MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#single-clock-plain-selectors` for joint window/driver
  admission, stateful fallbacks, newly actual Seek errors and distinct Rust/provider reconstruction costs.
- Shared schedule/group ownership MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#shared-scheduling-and-group-proof-owner` for distinct
  proof semantics, productive singleton provenance, terminal controls and actual per-target layout/getter costs.
- Empty group contributor ownership MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#empty-group-owners` for zero-only elision,
  productive singleton provenance, full range unions, opposite/parameter faults and actual C statistics.
- Selection grouped inputs MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#selection-group-contributors` for body demand,
  nested parameter priority, clock/Done/freshness, raw alias policy and distinct C/Engine activation scopes.
- Quantile grouped inputs MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#quantile-group-contributors` for local body demand,
  both cold/warm identity exclusions, exact clocks, parameter ordering and separate C/statistic scopes.
- Grouped-contributor changes MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#grouped-aggregate-contributors` for output ordinals,
  cold completion, nested membership ownership, existing window-cache phases and measured metadata costs.
- Histogram-consumer admission and evaluator-body failure calibration are owned by
  `src/crates/promql-engine/tests/promql-conformance/README.md#histogram-group-contributors`.
- Ordered label-projection contributor admission and its cold/selected/next-clock controls MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#label-projection-group-contributors`; retain instant/
  equal-header coverage, whole-group aliases and measured setup/peak costs.
- Ordered scalar-map contributor admission MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#mapped-group-contributors` for local map demand,
  cold/next-clock validation, source ordinals, native/mixed outcomes and metadata costs.
- Ordered histogram adapters MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#ordered-histogram-streaming` for frozen-child ownership,
  literal/identity/filter admission, sparse alignment, child continuation and measured lifetime costs.
  The rejected frozen-label ownership trial and its historical scope are recorded in
  `src/crates/promql-engine/tests/promql-conformance/README.md#rejected-frozen-histogram-label-borrowing`.
- Wrapped-join changes MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#wrapped-grouped-joins` for pre-copy eligibility,
  complete input discovery, kept/suppressed pair phases, fresh requested values, suffix continuation and separate
  original/order-policy qualification.
- Local ordered binary admission MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#ordered-binary-discovery` for pre-copy proof,
  source/final ordinals, cold/warm/next-clock phases, fresh outer parameters and actual read/peak costs.
  Delayed raw aggregate admission also follows
  `src/crates/promql-engine/tests/promql-conformance/README.md#delayed-raw-pair-discovery`.
- Local grouped join operands MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#local-grouped-join-operands` for resolved child demand,
  separate group/Pair credit, actual member/clock failures, exact portable results and retained numerical/cost scopes.
- Direct pair-metadata traversal MUST also follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-pair-metadata-traversal` for exhaustive
  binding/full-hash certification, physical prefix ownership, fresh backward/full requests, hash-buffer release,
  ordinary fallback phases and measured Step/layout costs.
- Callback failure controls MUST establish their actual phase: output begin precedes preparation. The same guide
  records calibrated cursor landmarks and native traces for the first-final rate case.
- C output descriptor qualification is owned by
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-singleton-float-output`;
  use its actual-adapter allocation and callback/reader panels. Known-count output labels additionally follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#c-output-label-descriptor-capacity`
  for lazy control-before-growth, metadata END and measured allocation traffic versus peak memory.
- Label projection MUST preserve virtual reset and reserved-name errors before matching; measure retained vector
  capacity independently of inputs. See `src/crates/promql-engine/tests/promql-conformance/README.md#compact-one-to-one-result-labels`.
- Shared allocation hints MUST preserve ordinary/untracked growth, checked required sizes and replacement lease
  ordering. Include ordinary-provider timing and inspect optimized default-path code: equal results and resource
  counters do not establish zero runtime overhead. The capacity/error/cleanup and actual optimized C caller panel is
  in `src/crates/promql-engine/tests/promql-conformance/README.md#exact-c-label-descriptor-capacity`.
- Empty-vector ownership measurement and concrete optimized constructor inspection are owned by
  `src/crates/promql-engine/tests/promql-conformance/README.md#direct-empty-vector-owners`.
- Single-series head optimizations MUST retain generic multiplicity/kind handling and meaningful control polling;
  see `src/crates/promql-engine/tests/promql-conformance/README.md#single-series-range-head-preparation` for causal controls.
- Sum and average acquisition reuse MUST preserve full saved traversal state, completed validation before read
  credit, stale-histogram consumption and later native arithmetic phases. Causal controls are owned by
  `src/crates/promql-engine/tests/promql-conformance/README.md#validated-average-acquisition-reuse` and
  `src/crates/promql-engine/tests/promql-conformance/README.md#validated-sum-acquisition-reuse`.
- Count acquisition units MUST preserve native weights, consumed membership and anchored/fallback credits; follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#count-from-completed-acquisition-units`.
- Present acquisition reuse and changing histogram replay membership are qualified by
  `src/crates/promql-engine/tests/promql-conformance/README.md#shared-present-acquisition`.
- Float rate acquisition reuse MUST retain uncertified chronological reset passes, full validation/terminal control
  before credit, and fresh consuming state for any histogram allocation, including stale history. Failed probes
  can truthfully own earlier endpoint sample units. Check all three kinds, shared Avg eligibility, actual operation
  failures, empty/stale histories and page-bounded memory; the finite procedure and costs are in
  `src/crates/promql-engine/tests/promql-conformance/README.md#validated-float-rate-acquisition-reuse`.
  Completed zero-or-one-reset windows also follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-single-reset-float-windows` for strict
  adjacency, ordered correction, pre-cancellation accounting and honest read/reference reductions.
  Root instant literal Boolean metadata transfer follows the same guide's
  `#completed-instant-boolean-rate-preparation` section for sparse ordinals, completed-output ownership,
  child diagnostics, logical totals and fresh final reads.
  Literal arithmetic follows
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-literal-rate-arithmetic`.
  Instant-rate acquisition also follows
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-rate-float-windows` for native
  fallback, terminal validation and provisional endpoint ownership.
  Expired native ownership follows
  `src/crates/promql-engine/tests/promql-conformance/README.md#reused-native-instant-rate-endpoints`.
- Histogram constructor lookup MUST preserve encoded-key identity, separate passes and tiny-set cost; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#histogram-constructor-key-lookup` for collision,
  ordering and terminal interruption controls.
- COUNT_VALUES lookup changes MUST preserve hash-only groups, exact-label bindings, whole-cache transfer and
  formatting/full-label validation before hits. Exercise the linear/indexed cutover, allocated interruption
  and distinct C binding/collection scopes; follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#generated-label-lookup-indexes`.
  Borrowed input copying also follows
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-count_values-input-labels` for
  virtual replacement/full validation,raw-order fallback and terminal discarded-destination cancellation.
  The rejected hit-copy investigation and its historical scope are recorded in
  `src/crates/promql-engine/tests/promql-conformance/README.md#rejected-count_values-hit-ownership`.
- Check remaining references after source/header/harness moves, and disposition historical references explicitly.
- Active histogram distribution MUST retain group identity, skipped ranges and pre-sort bound/ordinal order; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#histogram-active-group-distribution` for sparse/native,
  control and complete-query qualification. Keep failed deep derived-replay witnesses separate from passing panels.
- Direct classic quantile probes MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#direct-classic-quantile-probes` for completed-sweep
  admission, correction/coalescing fallbacks, redispatch, ownership and full-query measurements.
- Dense owned collection MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#dense-owned-result-collection` for identity proof,
  fallible sorting, generic fallback, callback attribution and complete-query costs.
- Set discovery reuse MUST retain suppressed inputs, selected/unverified rereads and cold first final discovery.
  Check physical side-row order, empty sides, tail padding, zero-word and final reconstruction controls, original
  aliases, exact keys and successful preparation clearing; calibrate control budgets against actual traversal. See
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-set-input-discovery`.
- Fixed grouped ranking qualification follows
  `src/crates/promql-engine/tests/promql-conformance/README.md#fixed-end-of-range-grouped-ranking` for collapsed
  ranking demand, exact source/value clocks, fresh selected reads and sparse endpoint membership.
- Completed set-prefix reuse MUST grant credit only after successful operand/label/hash/seen/control phases, retain
  fresh requested values, reuse only canonical immutable descriptor hashes, calibrate post-seen controls and preserve
  deferred duplicates, All/Done and serial same-Plan ownership; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-set-operand-prefixes`.
- Instant set reuse MUST qualify the actual direct singleton body in addition to ordered Matrix consumers.
  A direct body requires a certified set and absent ordered metadata; preserve canonical-to-source translation,
  Pair activation, unfiltered Vector truncation, effective instant interval1 and first-final clearing. Use the
  complete histogram/read regression and both name-removal modes; instant collectors store scalar results in
  `scalar`, while range requests of scalar expressions produce matrix rows. See the same guide's
  `#instant-set-discovery` section for finite original, caller and live qualification.
- Borrowed reserved-name detection MUST follow
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-reserved-label-detection` for both modes,
  raw names, first-error/copy/mutation controls and early/last/no-op costs.

## Evidence

- `tests/promql-conformance/README.md`: panel commands and scope.
- `tests/promql-conformance/core-qualification.json`: retained source/binary cohorts and limitations.
- `src/storage.rs`, `src/c_abi/`: provider, ownership, control and accounting contracts.
- `IMPORT.json`: original-to-current path mapping and original source hashes.

## Update rules

Update this workflow when a new canonical panel, ownership defect, comparison exception or qualification gap is found.


Fixed-map grouped reuse follows
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-clocks-through-qualified-fixed-maps`:
trace resolved Call edges and actual Group/Pair completion separately.
The ratio-prefix extension follows
`src/crates/promql-engine/tests/promql-conformance/README.md#successful-prefixes-in-grouped-ratios`.

Unary/Float group composition follows
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-clocks-through-unary-float-groups`.

Mixed-history instant-rate probes follow
`src/crates/promql-engine/tests/promql-conformance/README.md#mixed-history-instant-rate-probes`.

Direct grouped selectors follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-clocks-in-grouped-selector-subqueries`.

Literal-comparison set operands follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-comparison-set-discovery`.

Ordinary-window set operands and actual callback read boundaries follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-ordinary-window-set-discovery`.

Independent inner/outer grouped completion follows
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-clocks-in-nested-grouped-subqueries`.

Nonmerging label projections over completed groups follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-clocks-through-projected-groups`.

Ordinary Min/Max acquisition follows
`src/crates/promql-engine/tests/promql-conformance/README.md#validated-extrema-acquisition-reuse`.

Ordinary variance/deviation acquisition follows
`src/crates/promql-engine/tests/promql-conformance/README.md#validated-variance-acquisition-reuse`.

Ordinary First/Last acquisition and provisional native copying follow
`src/crates/promql-engine/tests/promql-conformance/README.md#validated-endpoint-acquisition-reuse`.

Ordinary Deriv/PredictLinear acquisition, prediction clocks and calibrated interruption follow
`src/crates/promql-engine/tests/promql-conformance/README.md#validated-regression-acquisition-reuse`.

Ordinary Changes acquisition, predecessor ownership and raw/derived timestamp controls follow
`src/crates/promql-engine/tests/promql-conformance/README.md#validated-changes-acquisition-reuse`.

The rejected Float Resets prefix experiment and native/mixed follow-up costs are recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#rejected-float-resets-acquisition`.

Float predecessor owner reuse MUST preserve checked-copy phases, query affinity, overlap and output lifetime; see
`src/crates/promql-engine/tests/promql-conformance/README.md#reused-float-predecessor-ownership`.


The rejected descriptor-hash reuse experiment is recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#rejected-descriptor-hash-reuse`.


Consuming final native ownership follows
`src/crates/promql-engine/tests/promql-conformance/README.md#consumed-final-native-endpoint`.


Prepared native endpoint retirement follows
`src/crates/promql-engine/tests/promql-conformance/README.md#prepared-native-endpoint-retirement`.


The rejected terminal native predecessor experiment is recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#rejected-terminal-native-predecessor-retirement`.


The rejected dense single-span compaction experiment is recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#rejected-dense-single-span-histogram-compaction`.


Aligned histogram merging follows
`src/crates/promql-engine/tests/promql-conformance/README.md#aligned-histogram-bucket-merging`.


Literal Sum/Avg preparation and its native fallback checks follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-literal-sum-and-avg-preparation`.


Literal Count/Min/Max preparation and empty-descriptor checks follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-literal-count-and-extrema-preparation`.


Literal Changes/Resets preparation and direct-window admission follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-literal-changes-and-resets-preparation`.


Literal Deriv preparation and empty mixed warnings follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-literal-deriv-preparation`.

Cached progress geometry and owning-key reconstruction follow
`src/crates/promql-engine/tests/promql-conformance/README.md#compact-cached-window-geometry`.

Existing-pass kind observation and ordinary instant Rate/Increase filters follow
`src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-literal-rate-filters`.

Precise duration identity and compact cached Rate input follow
`src/crates/promql-engine/tests/promql-conformance/README.md#compact-precise-range-keys`.

Atomic work publication and coherent overflow snapshots follow
`src/crates/promql-engine/tests/promql-conformance/README.md#atomic-work-accounting`.

Borrowed direct-selector group labels follow
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-direct-selector-group-labels` for raw keys,
active positions, Float/native diagnostic ownership, post-read interruption and full-label fallbacks.

Completed rate/increase group labels follow
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-rate-and-increase-group-labels` for
call/selector identity, metadata exclusions, completed diagnostics and distinct streamed/retained C owners.
The direct rate-family extension is qualified in
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-direct-rate-family-group-labels`.

The rejected completed Sum/Avg group-label experiment and its exact restoration are recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#rejected-sumavg-group-label-borrowing`.

The rejected checked output-label capacity trial and its timing repeatability limitation are recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#rejected-checked-label-view-compaction`.

Unchanged-binary timing calibration and limits of pooled short-run medians are recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#unchanged-binary-timing-calibration`;
retain paired blocks and absolute drift without reclassifying historical rejected candidates.

Complete mapped-label owner availability and the need to compare current provider label bytes are recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#complete-mapped-label-availability-census`;
ordinal identity alone does not establish generic-provider label-byte identity.

Complete mapped-label borrowing follows
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-complete-mapped-labels`: check current
full bytes and one physical source, preserve raw empties and final comparison polling; count mismatch fallback work.

Numeric Float-map label reuse follows
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-numeric-map-labels`: preserve
parameter/diagnostic order and filtering, require actual owner hits, and retain non-Float fallbacks.

Scalar-arithmetic label reuse follows
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-scalar-arithmetic-labels`: preserve
both scalar sides and parameter outcomes; use the existing metadata-removal predicate and complete current-byte proof.

Scalar-comparison label reuse follows
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-scalar-comparison-labels`: distinguish
ordinary metadata retention from Boolean removal, verify vector values on scalar-left comparisons, and prove
actual owner hits without accidentally exercising only the scheduled evaluator.

Mapped Rate/Increase label reuse follows
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-mapped-rate-labels`: verify exact
active call/selector/kind/range, child-plus-map metadata projection and immediate consumer ownership.
Nested descriptor rematerialization can add work despite lower allocation traffic; retain that failed
trial and require actual borrow hits plus unchanged nested-path resources for the narrowed admission.

The complete mapped rate family follows
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-mapped-rate-family-labels` for
whole-window versus final-pair type selection and separate counter-name diagnostics; keep consumer exclusions.

Mapped Sum/Avg consumer qualification and delayed-root rematerialization follow
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-mapped-sum-and-avg-labels`.

The rejected root Rate-label ownership experiment, metadata-free copy regression and timing limits are recorded in
`src/crates/promql-engine/tests/promql-conformance/README.md#rejected-root-rate-label-ownership`.

Mapped extrema/count/presence/variance ownership and typed-window checks follow
`src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-mapped-reducer-labels`.
