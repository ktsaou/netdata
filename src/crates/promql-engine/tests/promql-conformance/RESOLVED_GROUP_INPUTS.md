# Resolved direct aggregate inputs

Direct Float selector, ordinary rate-family and Count inputs can reuse an aggregate's already resolved group
position and frozen raw labels. This extends label borrowing to multi-key `by` aggregations without repeating the
grouping hash and lookup when the aggregate receives the sample.

## Ownership and validity

- Provider labels are still acquired, validated and matched freshly. Sample evaluation, diagnostics and credits
  finish before labels are substituted. The owner remains the immutable group roster.
- The current Selection must be the exact direct child of a collecting ordinary Aggregate. Grouping is global
  or `by` with fewer requested keys than source labels. `without`, native result values, intervening expressions,
  extended windows and missing active groups retain ordinary ownership.
- Rate-family and Count operations reject every reserved metadata key in the grouping list. Count over native
  input may produce an eligible Float result; its source validation and range diagnostics still execute.
- The private label variant carries the consuming node and active compact position. Propagation removes that
  hint at entry. Only its first frame may use it, after checking the consumer, bounds and identical raw-label
  owner. Other paths perform ordinary lookup. Active positions are not roster ordinals.
- Actual mutation or conversion to owned labels discards the hint. Reserved-label removal retains it only when
  removal changes nothing. Aggregate output normalization, raw empty values, hash-only first-writer aliases,
  arithmetic order, cancellation and independent output ownership retain their existing rules.

No label catalogue, shared allocation, raw-history buffer, ABI change or query limit is added. Native layout
checks compare the previous and current representations: labels remain 48 bytes, evaluated samples 112 bytes,
and optional evaluated samples 112 bytes. Foreign layout checks are reported separately.

## Required checks

Exercise direct Instant, Count and rate-family inputs under all ordinary reducers with one and several keys,
pages 1/17/1,024 and both name-removal modes. Include full retained-key and long-label controls, reserved names
in multi-key lists, sparse/missing groups, active positions differing from ordinals, raw empty values and hash
aliases. Preserve complete native/mixed diagnostics and calibrated cancellation after source acquisition.

Ownership checks must demonstrate single consumption, no-op removal, real mutation, materialization and retained
copies. Check the complete original reduction panel and retained expression outcomes; historical ordering and
numerical differences must remain explicitly attributed. Run both optimized profiles and actual C consumers.

The prior broader borrowing experiment remains rejected evidence: it added a grouping lookup and then repeated
the hash/lookup during propagation. This implementation passes the existing resolved position to its immediate
consumer instead. It does not revive the rejected duplicate-work route or add a byte-count heuristic.

## Controlled measurements

The fixture uses 128 source series, three grouping keys, 11 output clocks and optional 24 extra labels with
80-character value suffixes. Some sources stop early so the complete disappearance expression returns actual
results. Six alternating measured pairs follow an excluded warmup. The baseline is the accepted Count-page
implementation; the same raw samples, expressions and output resolution are used on both sides.

For the wide-label disappearance expression:

| Profile | Page size | Before | After |
| --- | ---: | ---: | ---: |
| release | 17 | 106.65 ms | 82.01 ms |
| release-min | 17 | 145.97 ms | 107.71 ms |
| release | 1,024 | 104.17 ms | 83.00 ms |
| release-min | 1,024 | 145.35 ms | 108.70 ms |

At page size 1,024, workspace allocation traffic falls from 66,957,859 to 29,144,955 bytes. Its peak remains
807,041 bytes. No measured memory-category peak increases, and fetched entries remain unchanged. Allocation
traffic is cumulative activity, not simultaneous RAM. Narrow-label disappearance gains are smaller; one eight-source
release control is 0.064 ms slower. Nonadmitted `without` controls vary by up to 3.3%, including 49.04→50.17 ms
in the larger compact-profile case. These are finite shared-host observations, not universal throughput bounds.

The fixture's release text grows by 1,680 bytes and compact text by 256 bytes. Total text/data/BSS changes by
−16 bytes in release and zero in compact. These are executable costs, separate from query memory.

Qualification and live deployment evidence are recorded independently; controlled gains do not establish that
the concurrent or long-history live timeouts are resolved.

## Qualification

[The source-bound receipt](resolved-group-inputs-qualification.json) records:

- 1,104 passing unit tests per optimized profile, two ignored tests per profile, and strict Clippy.
- 4,320 comparisons with pinned original ordinary reductions, plus 23,448 comparisons with authenticated prior
  complete outcomes. The latter retain their original strict/policy differences; they are not 23,448 new strict
  upstream passes.
- 1,932 exact Count comparisons; 96 baseline and 1,344 timing comparisons with original outcomes.
- Actual native GCC/Clang C/C++ callers in both profiles: 58 structures, 17 callbacks and 33 shared constants.
- Focused direct-selector ownership and layout tests under ARM32/ARM64 QEMU. ARM32 retains 32-byte labels and
  88-byte samples; ARM64 retains 48-byte labels and 112-byte samples. These are not foreign throughput results.
- Every measured expression, profile, page size, timing repetition and resource category, including slower controls.

The frozen fixture binaries precede one test-only assertion correction for the newly borrowed full-key case.
The receipt preserves both source identities and that exact correction. Production source is identical; current
full unit, lint and C-caller builds include the corrected assertion. Earlier failed expectations remain retained.
