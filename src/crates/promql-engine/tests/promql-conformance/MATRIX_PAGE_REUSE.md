# Reusing pages during matrix acquisition

Matrix acquisition sometimes advances through several logical windows before consuming the requested window.
Previously, each forward seek asked the provider to reposition and return another page, even when the target
was in the page already borrowed. Matrix replay now uses the storage walker's existing in-page seek handling.

This reuses one current page within one traversal. It does not retain a cursor or raw samples across separate
matrix acquisitions. Initial seeks, initial-current seeks, targets outside the current page and the final
consuming traversal still use the provider. The complete page tail is validated before an in-page landing;
the logical seek origin still reaches histogram normalization and statistics handling.

The generic walker and Count's separate page observer keep their previous behavior. There is no new allocation,
cache, provider capability, public setting or resource limit. Failures belonging only to removed provider calls
are no longer reached; failures from actual operations still propagate. Existing cursor tests cover malformed
tails, cancellation, physical boundary failures and release of borrowed pages.

## Qualification

The new matrix tests check Float and native-histogram values, complete progress, logical sample credits and
released ownership at page capacities 1, 17 and 1,024. A separate regression pins the physical final seek while
showing that intermediate same-page seeks disappear. Its unchanged-source run fails the read-cost assertion;
the value/progress test passes before and after the change.

The [C fixture](matrix_page_reuse.c) generates one counter series through bounded pages. It verifies every
returned label, timestamp and Float bit, annotations, cursor cleanup and engine accounting after each query.
It supplies no optional completed-value backing, so it measures the replay path rather than cached subquery
values. Its operations are:

| Number | Expression |
| ---: | --- |
| 0 | `m_total` |
| 1 | `rate(m_total[2s])` |
| 2 | `sum_over_time(m_total[2s])` |
| 3 | `avg_over_time(m_total[2s])` |
| 4 | `count_over_time(m_total[2s])` |
| 5 | `rate(m_total[2s]) / rate(m_total[10s])` |
| 6 | `max_over_time(rate(m_total[2s])[20s:10s])` |

Ordinary single-window controls already reuse adjacent progress and need no intermediate local seeks, so their
read/work counts are unchanged. Unequal-window expressions save a few initial reads. The nested expression can
repeatedly replay earlier windows and provides a larger opportunity, especially with a coarse outer step.
These observations do not establish that live counter reconstruction across separate cursors is solved.

## Measured results

The [qualification report](matrix-page-reuse-qualification.json) binds the source, explicit library builds,
executables and [4,032 C query observations](matrix-page-reuse-observations.jsonl.gz). Both Rust profiles pass
1,114 tests with two ignored, and both C++ embedding suites pass. Three focused retained-original panels
(`matrix-copy`, `unvisited-phases`, `clipped`) pass 4,470 complete comparisons across both profiles and pages
1/17/1,024, including histogram fields, error phases and exact numerical checks under the existing comparison contract.

The timing panel covers 112 cases in six alternating blocks, with three queries per process. Block zero is warmup;
the remaining five blocks provide 15 paired query observations per case. Representative nested-query cases use
page capacity 1,024, a 60-second outer step and an explicit deadline:

| Profile | Input samples | Baseline median ms | Candidate median ms | Median paired ratio |
| --- | ---: | ---: | ---: | ---: |
| release | 257 | 6.155 | 3.308 | 0.523 |
| release | 1,025 | 1,082.823 | 143.596 | 0.134 |
| release-min | 257 | 7.826 | 4.132 | 0.529 |
| release-min | 1,025 | 1,291.995 | 177.808 | 0.138 |

At 1,025 samples, fetched entries fall from 16,983,474 to 1,514,768. All measured logical read credits and
memory-category peaks stay unchanged. Direct selector/range controls retain every non-timing observation.
Dense nested replay at page capacity 17 has a smaller benefit: its release paired ratio is 0.950, while
release-min is 1.000. This is not a universal speedup or a live timeout-removal claim.

The mixed panel includes slower controls. Release-min's 257-point average and 1,025-point rate controls have
paired ratios of 1.071 and 1.070. A separate [480-query recheck](matrix-page-reuse-controls.jsonl.gz), with two
warmup and ten measured blocks, does not reproduce those increases: paired block-median ratios are 1.003 and
1.012. Both measurements remain in the report; sub-millisecond controls also vary on this shared host.

The C caller's compiled text grows by 74,616 bytes in release (7,971,214 to 8,045,830) and 304 bytes in release-min
(5,525,013 to 5,525,317). Data and BSS are unchanged. This is a fixed process-wide code cost, not per-query memory.

## Reproduction

Build the crate with `cargo build --locked --features fixtures --release --lib`, or use `--profile release-min`.
Build the library explicitly: building only the fixture binary may leave the top-level static archive stale.
Compile `matrix_page_reuse.c` as C11 with `-O2 -Wall -Wextra -Werror`, the crate's `include/` directory and the
resulting `libnetdata_promql_engine.a`, linking `dl`, `pthread` and `m`. Reuse the existing Cargo target directory.

The executable takes six positional arguments:

| Argument | Meaning |
| --- | --- |
| count | 32–65,537 one-second counter samples |
| page | Provider page capacity: 1, 17 or 1,024 |
| step | Query step in seconds: 1, 10 or 60 |
| operation | Expression number from the table above |
| repeats | 1–20 complete collected queries |
| deadline | 0 for none; 1 for an explicit one-hour deadline |

Freeze source-bound baseline and candidate executables. Alternate execution order and retain all counter and
memory observations beside elapsed times. Build and test before measuring; do not overlap timing with compilation.
Use small nested-query pilots before increasing count. Accounted memory categories describe query-owned or
borrowed resources, not process RSS; compiled code is a separate process-wide cost.
