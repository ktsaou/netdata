# Control checks during label comparisons

The shared byte comparison and equality helpers checked cancellation/deadlines on entry and again immediately
before comparing their first 1,024-byte chunk. Only constant-time slice setup and, for equality, the length
comparison separated those checks. The second observation did not bracket any byte processing or callback.

Both helpers now retain the entry check and checks before every subsequent chunk. Empty inputs and unequal
lengths still observe the entry check. Accounting, byte comparison order and partial-work reporting are unchanged.
This introduces no allocation, cache, background timer, public control option or change to C page validation.

## Qualification

The source-bound [report](label-comparison-qualification.json) and
[compressed observations](label-comparison-observations.jsonl.gz) retain paired whole-query timings, exact resource
counters, binary identities and both Rust build profiles. Each measured C query verifies every output label,
timestamp and Float bit, plus annotations, provider releases and engine ownership after cleanup.

- Rust units cover empty inputs, unequal lengths, last-byte differences, lengths around 1,024 bytes,
  expired/cancelled entry and interruption between later chunks. Existing shared-helper users run in the full suite.
- The C fixture generates 32 samples per series through bounded pages; it does not retain a raw input matrix.
- Expressions exercise a plain selector, `and on(zone)`, arithmetic `+ on(zone)` and grouped Count/set composition.
- Labels have either 16 or 1,025 value bytes. Queries run with no deadline and with an explicit one-hour deadline.
- Timing uses six alternating baseline/candidate blocks. Block zero is warmup; five measured pairs remain per case.
  Compilation and full test suites finish before the timing panel begins.
- Direct expressions use four and 64 series. The grouped Count timing uses four series. An earlier 64-series
  exploration used older pre-Count libraries; its 40–186-second results are historical observations, not
  current-source qualification. The fixture does not supply optional completed-value backing.

Shared-host timings are observations, not a guarantee of live concurrent throughput. Deadline-present and
deadline-absent cases must be reported separately. Accounted work does not count clock reads, so unchanged work
is expected even when the elapsed time improves. Memory-category totals are not process RSS.

## Measured results

Both Rust profiles pass 1,112 tests with two ignored. The existing C++ embedding suite passes in each profile.
All 672 C benchmark executions validate their complete outputs and cleanup; every paired work, fetched-sample,
provider-call and memory observation is identical. This leaves 280 measured pairs after warmup exclusion.

Representative release-profile cases use 64 series and an explicit deadline:

| Operation | Label value bytes | Baseline median ms | Candidate median ms | Median paired ratio |
| --- | ---: | ---: | ---: | ---: |
| Set intersection | 16 | 411.37 | 392.95 | 0.956 |
| Arithmetic join | 16 | 378.27 | 345.01 | 0.913 |
| Set intersection | 1,025 | 449.24 | 430.43 | 0.942 |
| Arithmetic join | 1,025 | 410.39 | 390.88 | 0.941 |

Results are mixed outside these cases. Release-min with 64 series and 1,025-byte values has paired ratios of
0.999 for intersection and 1.014 for arithmetic; its four-series wide-label Count case is 1.022.
Short controls vary substantially, and medians of paired ratios differ from ratios of independent medians.
The full report retains every case rather than claiming a universal improvement or a live-workload speedup.

The measured C caller's text grows by 112 bytes in release (7,972,189 to 7,972,301) and 144 bytes in release-min
(5,526,020 to 5,526,164). Data and BSS are unchanged. No tracked query-memory increase was observed.

## Reproduction

Build the crate with `cargo build --locked --features fixtures --release`, or use `--profile release-min`.
Compile `label_comparison.c` as C11 with `-O2 -Wall -Wextra -Werror`, the crate's `include/` directory and the
resulting `libnetdata_promql_engine.a`, linking `dl`, `pthread` and `m`. Use the existing Cargo target directory.
Build `--lib` explicitly before linking: a `--bin` build can leave that top-level archive stale. The initial
comparison caught such a mismatch through unequal sample/work counters; those observations were discarded as
evidence for this change. The published comparison uses freshly built committed-source baseline libraries and
the verified candidate binaries. The associated historical Count C qualification is corrected in
[COUNT_SET_DISCOVERY.md](COUNT_SET_DISCOVERY.md#standalone-c-qualification-correction--2026-10-29).

The executable takes six positional arguments:

| Argument | Meaning |
| --- | --- |
| series | 1–128 unique series |
| width | 16–4,097 bytes per label value |
| deadline | 0 for none; 1 for one hour |
| operation | 0 selector; 1 set intersection; 2 arithmetic join; 3 grouped Count intersection |
| repeats | 1–20 complete collected queries |
| page | Provider page capacity: 1, 17 or 1,024 |

The measured panel uses page capacity 17 and one query per process. Preserve frozen baseline and candidate
executables, alternate their order, and compare all non-timing fields exactly. Run the large grouped Count case
only deliberately; it is a slow fallback workload, not a prerequisite for repeated timing of this byte-level change.
