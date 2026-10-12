---
name: project-conformance
description: "Required when changing the PromQL frontend, evaluator, C ABI, accounting or compatibility claims. Use before selecting qualification or declaring same-input compatibility."
---
# PromQL conformance workflow

## Purpose

Preserve actual source/input/binary evidence while changing the standalone engine.

## Scope

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
- Follow the conformance README for pinned differential panels; preserve its stated tolerance and bit diagnostics.
- Check remaining references after source/header/harness moves, and disposition historical references explicitly.

## Evidence

- `tests/promql-conformance/README.md`: panel commands and scope.
- `tests/promql-conformance/core-qualification.json`: retained source/binary cohorts and limitations.
- `src/storage.rs`, `src/c_abi/`: provider, ownership, control and accounting contracts.
- `IMPORT.json`: original-to-current path mapping and original source hashes.

## Update rules

Update this workflow when a new canonical panel, ownership defect, comparison exception or qualification gap is found.
