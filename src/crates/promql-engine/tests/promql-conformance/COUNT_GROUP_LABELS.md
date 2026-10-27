# Direct Count aggregate labels

A direct aggregate such as `sum by(app_group)(count_over_time(m[5m]))` needs the group labels after the Count
value is complete. The evaluator previously copied every source label, then hashed those labels again for the
aggregate. Count now uses the existing direct-selector group-label borrowing path.

Admission remains narrow: the current selection must be the exact Count operation/range and the direct child of a
collecting aggregate. Only Float results, ordinary selectors, and empty or single-label `by` grouping qualify.
The evaluator still validates current provider labels, applies matchers and computes the group identity from those
current bytes. It borrows the already-owned matching group labels only after the Count value and diagnostics are
complete. A missing group uses the ordinary path. No additional metadata cache or provider guarantee is introduced.

Multi-label grouping, `without`, reserved metadata grouping and other parent/operation shapes retain their previous
paths. Metadata removal follows the existing Rate contract. Native inputs may qualify because Count produces a
Float result; native sample validation and range semantics still run before borrowing.

## Qualification

[count-group-labels-qualification.json](count-group-labels-qualification.json) binds both optimized binaries and:

- 1,100 passing units per profile, two ignored, and strict Clippy.
- 1,932 exact Count comparisons with the pinned original, including sparse, empty, stale and mixed histories.
- 448 exact complete-result comparisons during before/after timing, including the unchanged predecessor.
- Actual borrowing hits, both name-removal modes, pages 1/17/1,024, native/mixed/stale inputs, source failures,
  missing groups and calibrated cancellation after source acquisition.
- Ordinary fallback controls for wide grouping, `without` and reserved metadata labels.

Native C-interface units are included. Prior broad SRE and foreign-compiler/platform captures retain their original
source attribution; this bounded admission did not repeat those suites. Live deployment results are separate.

## Measured costs

The synthetic fixture has 128 sources, 901 points per source and 24 extra labels with 80-character value suffixes.
Each range
has 21 clocks, 30 seconds apart. Medians below use six alternating pairs after a warmup; the receipt also retains
8-source cases, both page sizes and all control measurements.

| Query / profile | Page size | Before → after |
| --- | ---: | ---: |
| Grouped Count sum, release | 17 | 149.50 → 126.96 ms |
| Grouped Count average, release | 17 | 148.43 → 126.01 ms |
| Grouped Count sum, release-min | 17 | 200.67 → 159.86 ms |
| Grouped Count sum, release | 1,024 | 180.54 → 158.83 ms |
| Grouped Count sum, release-min | 1,024 | 237.04 → 197.20 ms |

In the 128-source/page-17 sum, workspace allocation traffic falls from 85.47 MB to 43.69 MB and work from
39.45 million to 25.73 million units. Work includes bytes processed, not only iterations. All measured memory-category
peaks and fetched-entry counts remain unchanged. This saves label processing; it does not fix page-tail rereads.

Controls vary on the shared workstation. The final release unaggregated Count control at 128 sources/page-1,024 changes
156.17→161.86 ms, while the preceding trial varied in the opposite direction. Wide-group controls remain close to
baseline. These results establish a targeted benefit, not a universal speedup or a claim of zero overhead.
The fixture executable's text grows by 48 bytes in release and 64 in release-min; total text/data/BSS size is unchanged.
