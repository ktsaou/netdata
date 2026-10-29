# Journal page descriptor reuse

The demand reader and retention-expiry check search journal page descriptors by
end timestamp. Each search previously copied the selected descriptor once more
when its binary search finished. Both now retain the descriptor at the latest
right-bound update and use that copy. Every original search probe, cancellation
poll, CRC check, selected-entry validation and file-reference operation remains.

Keeping the last probe would be incorrect: with endpoints 10, 20, 30 and 40,
searching for 25 probes indices 2 then 1, but must select index 2. The tests
check this case explicitly, including the actual copied-offset sequence.

There is no new heap allocation, retained cache or query-sized state. Each
search has one additional 20-byte source-level local descriptor; actual stack
layout depends on the compiler. The complete internal-check executable, including
the expanded storage fixture, adds 104,416 bytes of text and 64 bytes of BSS;
combined data sections are unchanged. This is not a query-memory measurement.
Indexed journal bytes remain held by the
existing mapping reference. This assumes the existing immutable journal model;
external in-place modification between probes is not an equivalence claim.

## Qualification

Run `python3 tests/promql-integration/journal_pages.py --cc gcc` (or `--cc clang`).
The runner extracts actual search/CRC code, journal layouts and mapped-access
handling. Its 23 cases cover empty/singleton tables, first/middle/gap/last/past-end
selection, upper bounds, second-probe cancellation, selected versus discarded
invalid extent indices, and reference/frame cleanup. Surrounding types, locks,
references and control are explicit shims; extent geometry accepts only the
fixture's 4096-byte extent. `--expect-baseline` selects the previous duplicate-read
expectation when testing an unchanged baseline checkout.

The baseline passes its original expectations and fails the reduced-copy
expectation in 18 selected-search cases. GCC and Clang pass all 23 current cases.
The real Agent fixture persists four separate pages, evicts cached descriptors,
checks nine actual reader positions and two promise-expiry boundaries. Both
compression formats pass the normal and internal-check storage suites. The
actual HTTP suite passes 154 checks and 24 host-readiness checks.

An initial real-page fixture expectation was corrected: Netdata's storage-number
roundtrip of 100 yields 100.00000000000001. The test now compares against the
exact pack/unpack result, without a tolerance or production behavior change.

## Isolated measurements

Six alternating blocks of 10,000 lookups per observation, excluding the first
block from summaries, produced these median paired elapsed ratios:

| Search | Page records | Candidate / baseline |
|---|---:|---:|
| Candidate selection | 4 | 0.908 |
| Candidate selection | 128 | 0.942 |
| Retention overlap | 4 | 0.861 |
| Past-end control | 4 | 1.008 |

Timing builds do not enable copied-offset instrumentation. The measured benefit
is approximately 6–14% within these isolated searches. It does not establish
whole-query throughput improvement; sample decoding, broader selection and
other work remain. No full live performance replay is attributed to this change.

[journal-page-search-qualification.json](journal-page-search-qualification.json)
binds source hashes, binaries, baseline/red/current cases, timings and suites.
[journal-page-search-observations.jsonl.gz](journal-page-search-observations.jsonl.gz)
retains the compact case, timing and storage observations. Focused live queries,
Grafana and advancing reception must pass after deployment. Existing long-history
and concurrent timeouts remain open; cumulative histogram projection still
requires a design decision. This change does not alter Rust numerical behavior,
public APIs, storage formats, metadata ownership or query limits.
