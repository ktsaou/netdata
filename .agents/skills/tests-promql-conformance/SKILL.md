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
- Exercise pages 1, 17 and 1024 when changing cursor/replay ownership or temporal evaluation.
- Record compiler profile, binary hash, source hashes and comparison policy for each new cohort.
- Bind the comparison runner and imported helper bytes before and after each new cohort.
- Finalize artifact hashes after their owned writers finish; exclude a receipt from its own artifact map.

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
- Follow the conformance README for pinned differential panels; preserve its stated tolerance and bit diagnostics.
- Shared grouping/ranking/quantile changes MUST include the complete retained reducer panels and separate declared
  ordering controls in `src/crates/promql-engine/tests/promql-conformance/README.md#compact-group-and-join-lookups`.
  Preserve strict original failures; a policy report does not replace or modify them.
- Copied grouping keys MUST preserve raw empty values, source order and the shared by/without predicate;
  preserve ownership-transfer paths and full ranking labels. Measure retained capacity separately from inputs,
  and check omitted/retained byte walks plus terminal cancellation; see the same guide's copied grouping-label section.
- Completed sort publication MUST preserve callback/error phases, hash-only duplicates, eligible input kinds,
  delayed-name gates and snapshot lifecycle. Check lookup growth and terminal interruption, actual child acquisition
  phases and temporary allocation costs; see the same guide's completed vector-sort publication section.
- Completed sort emission MUST preserve first-final full delivery, fresh selected values and exact snapshot ownership.
  Check cold suffix/warm selected failures, context/key replacement, sparse ordinals and every credit charge; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-vector-sort-emission`.
- Cursor-fault qualification uses Netdata's `src/crates/promql-engine/tests/promql-conformance/cursor_policy_calls.py`.
  Its procedure and separate original/control reports are in
  `src/crates/promql-engine/tests/promql-conformance/README.md#physical-cursor-fault-qualification`.
- Foreign page-wrapper changes MUST preserve public marker/lifetime properties and whole-page validation phases.
  Run the focused ownership checks before broader rebuilds; the installed-tool and actual caller scopes are in
  `src/crates/promql-engine/tests/promql-conformance/README.md#borrowed-foreign-sample-pages`.
- Completed binary replay MUST preserve full first-pass failure priority before reusing membership metadata; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-unique-float-join-scans` for its causal controls.
- Instant fold reuse MUST preserve required final initialization and exact-key checks, including absent summaries;
  see `src/crates/promql-engine/tests/promql-conformance/README.md#completed-instant-numeric-folds` for causal controls.
- Direct rate join certification MUST preserve physical ordinals and effective clocks, including distinct window
  truncation; see `src/crates/promql-engine/tests/promql-conformance/README.md#completed-direct-rate-join-scans`.
- Callback failure controls MUST establish their actual phase: output begin precedes preparation. The same guide
  records calibrated cursor landmarks and native traces for the first-final rate case.
- Label projection MUST preserve virtual reset and reserved-name errors before matching; measure retained vector
  capacity independently of inputs. See `src/crates/promql-engine/tests/promql-conformance/README.md#compact-one-to-one-result-labels`.
- Shared allocation hints MUST preserve ordinary/untracked growth, checked required sizes and replacement lease
  ordering. Include ordinary-provider timing and inspect optimized default-path code: equal results and resource
  counters do not establish zero runtime overhead. The capacity/error/cleanup and actual optimized C caller panel is
  in `src/crates/promql-engine/tests/promql-conformance/README.md#exact-c-label-descriptor-capacity`.
- Single-series head optimizations MUST retain generic multiplicity/kind handling and meaningful control polling;
  see `src/crates/promql-engine/tests/promql-conformance/README.md#single-series-range-head-preparation` for causal controls.
- Average acquisition reuse MUST preserve full saved traversal state, completed validation before read credit,
  stale-histogram consumption and later native arithmetic phases; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#validated-average-acquisition-reuse` for causal controls.
- Histogram constructor lookup MUST preserve encoded-key identity, separate passes and tiny-set cost; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#histogram-constructor-key-lookup` for collision,
  ordering and terminal interruption controls.
- Check remaining references after source/header/harness moves, and disposition historical references explicitly.
- Active histogram distribution MUST retain group identity, skipped ranges and pre-sort bound/ordinal order; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#histogram-active-group-distribution` for sparse/native,
  control and complete-query qualification. Keep failed deep derived-replay witnesses separate from passing panels.
- Set discovery reuse MUST retain suppressed inputs, selected/unverified rereads and cold first final discovery.
  Check physical side-row order, empty sides, tail padding, zero-word and final reconstruction controls, original
  aliases, exact keys and successful preparation clearing; calibrate control budgets against actual traversal. See
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-set-input-discovery`.
- Completed set-prefix reuse MUST grant credit only after successful operand/label/hash/seen/control phases, retain
  fresh requested values, reuse only canonical immutable descriptor hashes, calibrate post-seen controls and preserve
  deferred duplicates, All/Done and serial same-Plan ownership; see
  `src/crates/promql-engine/tests/promql-conformance/README.md#completed-set-operand-prefixes`.

## Evidence

- `tests/promql-conformance/README.md`: panel commands and scope.
- `tests/promql-conformance/core-qualification.json`: retained source/binary cohorts and limitations.
- `src/storage.rs`, `src/c_abi/`: provider, ownership, control and accounting contracts.
- `IMPORT.json`: original-to-current path mapping and original source hashes.

## Update rules

Update this workflow when a new canonical panel, ownership defect, comparison exception or qualification gap is found.
