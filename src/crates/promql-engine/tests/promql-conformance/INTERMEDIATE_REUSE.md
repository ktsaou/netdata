# Provider-backed completed intermediates

The engine can retain completed grouped Float values, evaluated selector points and successful subquery reader
progress in provider-owned temporary storage. This removes repeated computation without holding raw query history
in RAM. The optional storage capability uses the existing ABI4 callbacks; absence of that capability retains replay.
An error from an available backing store fails the query.

This developer guide is referenced by the `tests-promql-conformance` skill. The approved ownership and traversal
choices are in [DESIGN.md](DESIGN.md#completed-intermediate-values-and-reader-progress).

## Ownership and completion

- `computed_groups::Store` owns bounded pages and exact-key four-word cells. An absent result is distinct from
  an unfinished cell. Final generation tags prevent preparation values from satisfying fresh final reads.
- Selector preparation and final traversal have different layouts. Changing generation does not transpose data.
  Dirty resident pages must survive gap initialization, cache replacement and direct nonresident record access.
- A validated borrowed Float page can supply future evaluated points on its supported grid. It cannot trigger
  another source read. Work is bounded by the page's sample count, including skipped or already-known points.
  Pending Float credits become useful reads only on actual consumption; probes cannot admit them twice.
- `derived_progress` retains a fixed reader checkpoint per existing descriptor in provider backing. Only successful
  Float completion can publish progress; native, backward, mismatched or unfinished requests use ordinary traversal.
- A completed aggregate sample does not by itself establish its interval certificate. Direct single-Float range
  consumers preserve the aggregate's real terminal step after the ordinary kernel visitor. The reader stages at
  most one next action. A staged fetch must be returned before processing its pending flag, and a staged completion
  must be delivered before saving reader progress.
- Source cursor bounds can advance after an observed selector lookup. Netdata's query-wide counter origin and tier
  policy remain unchanged. The C provider must reconstruct the same absolute values for later cursor starts.

## Regression checks

Run the fixture suites from `src/crates/promql-engine`, using the installed toolchain:

```sh
cargo test --locked --features fixtures --release
cargo test --locked --features fixtures --profile release-min
```

The relevant tests live in `computed_groups`, `derived_progress`, `derived_matrix`, `instant_select`,
`storage_observed`, `c_abi::tests::temporary` and `execution::computed_group_tests`.

Check exact Float bits, labels, source clocks, stale barriers, native/mixed fallback, sparse series and windows,
both name-removal modes, complete diagnostics and logical sample totals. Exercise physical pages of 1, 17 and 1,024
samples. Test dirty backing, absent capability, real read/write errors, cancellation after callbacks and independent
cleanup ownership. A malformed returned page tail must fail before future-point publication.

The terminal tests distinguish ordinary endpoint retirement from the default completed-child handoff. Fallback
tests retain the original no-certificate assertions. Default tests require actual terminal completion, exact results,
linear cold input deliveries and interruption before endpoint publication. Reader tests separately preserve
visitor-before-terminal error order and execution of a staged next-clock fetch.

Mapped-window checks also compare both scalar operand orders, arithmetic, filtering and Boolean comparisons
against ordinary replay and pinned original results. Include reserved-name projection, ambiguous labels, nested
maps, fixed-time boundaries, sparse and nonfinite values, and mixed/native history. A native fallback assertion
must check both completed-window consumption and dry-progress counters: zero value-cache hits alone does not
prove that ordinary traversal was retained. Calibrate failures against actual final-phase acquisitions and exercise
cancellation after real mapped-window delivery; the direct aggregate terminal hook does not apply to mapped children.
Preparation-to-final changes from Float to native input must revoke mapped progress eligibility even when SUM
returns absence, COUNT returns a Float, or MAX ignores the native input. Keep existing unfinished and completed
cells intact when revoking eligibility.

Adjacent raw-window checks belong to `matrix_reader`. Compare ordinary traversal against resumed traversal for
equal-width and overlapping windows, including sparse/nonfinite values, EOF, exact sample credits and one-page
ownership. Native copy normalization depends on its first admission endpoint: preserve same-clock consumption
when a new native appears, then reject the following adjacent reuse. Exercise newly reached seek failures,
cancellation, geometry/owner/scope changes and checked clock overflow. The disjoint path remains a separate control.

Replay the retained original evaluation, SRE recording/alert, temporal, native-histogram, grouping and precise-time
panels in both profiles. Keep existing source-unspecified ordering classifications visible; they are not strict
original-output passes. Build and execute actual native, ARM64 and ARM32 C/C++ callers. Qualify the changed Netdata
binary through `tests/promql-integration/storage.py`, including late cursor starts for native/legacy counters,
reset/gap pages, mixed history and aliases.

## Performance checks

Measure complete queries as both source count and history grow. Report raw entries fetched, work, temporary I/O
bytes and calls, peak memory, and cumulative allocation traffic separately. Fewer raw reads alone are insufficient:
strided four-word accesses can otherwise transfer an entire backing page each time. Dense window reads need page
amortization; sparse window reads need bounded record transfers. Preserve failed experiments and compare the same
immutable inputs with alternating repeated runs before making throughput claims.

These checks qualify the engine and caller mechanics. Live Grafana latency, continued reception and historical
histogram representation remain separate integration checks; controlled results cannot substitute for them.

## Recorded qualification

[computed-intermediates-qualification.json](computed-intermediates-qualification.json) binds source, binaries,
inputs and retained reports. The broad run contains 128,652 builtin and 231,540 SRE comparisons with no result or
paired-bit differences, plus 18,192 temporal/temporary/count-values comparisons. Another 31,572 comparisons retain
their existing ordering and numerical classifications; they are not all strict original-output passes.

Both final profiles pass 1,080 unit tests (two ignored) and 1,976 focused original comparisons each. The receipt
distinguishes the broad-run binaries from the final rebuild, whose only changes are lint attributes and test-only
iteration cleanup. Actual native, ARM64 and ARM32 C/C++ consumers passed on the broad-run source. The Netdata
storage test passed all six array/Gorilla write/read/append phases, including later counter cursor starts.

The controlled timing fixture contains 32 source series, 16 groups and 29 output clocks. Six alternating measured
runs follow one excluded warmup. Medians compare the accepted pre-change fixture with the broad-run candidate:

| Expression | Before | After |
| --- | ---: | ---: |
| `sum by(id)(m)` | 6.339 ms | 6.739 ms |
| `sum by(id)(m)/100` | 12.442 ms | 8.470 ms |
| `max by(id)(timestamp(m))` | 8.995 ms | 9.641 ms |
| `time()-max by(id)(timestamp(m))` | 64.700 ms | 12.088 ms |
| `sum by(id)(m)/on() group_left sum(m)` | 66.671 ms | 14.617 ms |
| `avg_over_time((sum by(id)(m))[4s:1s])` | 520.696 ms | 15.546 ms |
| `quantile_over_time(0.95,(sum by(id)(m))[4s:1s])` | 2,506.115 ms | 18.165 ms |

The two bare controls become 0.400 ms and 0.646 ms slower. Each now uses a 29,696-byte backing extent and 1,930
temporary I/O callbacks; workspace peaks grow by 13,184 and 15,648 bytes respectively. Both still fetch far fewer
raw entries and perform less work. This fixture serves raw input from RAM but backing through file operations;
callback overhead is a plausible explanation, not measured attribution. These shared-workstation timings do not
establish live server capacity or universal speed improvement.

The native GCC C executable's total `size` report grows from 7,952,302 to 8,109,242 bytes in release (+1.97%) and
5,632,095 to 5,747,643 bytes in release-min (+2.05%). This is compiled footprint, not per-query allocation or RSS.
Runtime paging remains bounded per store; group metadata, outputs and the number of stores can still grow with
query cardinality. Provider backing also consumes storage and I/O outside those RAM pages.

### Scalar-mapped aggregate windows

[mapped-windows-qualification.json](mapped-windows-qualification.json) records the subsequent single-scalar-map
extension before the subsequent native-admission correction. Both profiles pass 1,083 unit tests (two ignored), 128,652 builtin comparisons combined, 3,952 retained
focused comparisons and 2,592 new mapped-window comparisons. All compared outcomes and numeric bits match the
pinned originals. The predecessor also passes the same new 1,296 cases separately. A later test-only lint edit is
explicitly distinguished from the frozen binaries; runtime source is unchanged.

The larger fixture has 36 source series, 18 groups and 361 output clocks across six hours, with a 30-minute
subquery window and one-minute step. Three alternating runs give these median times:

| Expression | Before | After |
| --- | ---: | ---: |
| `avg_over_time((avg by(id)(m))[30m:1m])` | 447.235 ms | 440.035 ms |
| `avg_over_time((100-avg by(id)(m))[30m:1m])` | 11,799.887 ms | 516.919 ms |
| `quantile_over_time(.95,(100-avg by(id)(m))[30m:1m])` | 63,156.602 ms | 1,252.868 ms |

Both mapped queries add 1,520 bytes to their workspace peak: average 85,808 → 87,328 bytes; quantile
85,872 → 87,392 bytes. Their cumulative workspace allocation falls from 677,842,910 → 30,812,110 bytes and
3,526,912,062 → 31,160,366 bytes respectively. The additional backing extent is 2,448 bytes, one 136-byte progress
record per group. Source entries fetched remain 28,152; the improvement removes repeated work on computed values.

The smaller four-group fixture's unchanged average control measures 3.363 → 3.361 ms; mapped average
17.640 → 3.979 ms and mapped quantile 78.244 → 4.949 ms. These are six measured runs after one warmup.
Measurements ran on the shared workstation alongside the size-optimized unit build/run; they do not establish
isolated throughput or VM latency. Review subsequently found that backing presence alone did not exclude native
inputs introduced during final execution. A store-local monotonic Float-input flag fixes that admission gap;
36 phase-changing cases compare ordinary traversal and require zero mapped-progress hits. No wrong expression
result was demonstrated by that finding. The receipt above remains bound to its original source and binaries.

### Adjacent raw Float windows and native admission

[adjacent-windows-qualification.json](adjacent-windows-qualification.json) binds the combined native-admission
correction and adjacent raw-window optimization. Both optimized profiles pass 1,087 unit tests each (two ignored).
The combined comparisons include 128,652 builtin, 231,540 SRE, 3,952 focused, 2,592 mapped-window, 4,380 overlapping
window and 18,192 temporal/temporary/count-values executions with exact outcomes and numeric bits. The separate
31,572-panel remainder retains 924 strict ordering disagreements and six copies of the established numerical
witness; these are not strict passes. Actual native, ARM64 and ARM32 C/C++ callers and six real DBengine storage
phases are separately source-bound in the receipt.

With a ten-second evaluation step, a completed Float window can seek directly to the next window's lower bound
instead of replaying every earlier endpoint. It still reads the current window chronologically and retains no raw
history. Six alternating measured repetitions after one warmup gave these release medians:

| Fixture | Query | Before | After | Fetched entries before → after |
| --- | --- | ---: | ---: | ---: |
| 1 series, 2,401 points | `rate(m_total[10s])` | 28.514 ms | 1.417 ms | 585,534 → 8,148 |
| Same | `rate(m_total[11s])` | 28.522 ms | 1.432 ms | 586,000 → 8,150 |
| Same | `count_over_time(m_total[300s])` | 29.084 ms | 7.659 ms | 590,792 → 138,336 |
| 8 series, 1,201 points | `rate(m_total[11s])` | 60.133 ms | 5.590 ms | 1,191,648 → 32,560 |

All eight measured memory-category peaks are unchanged in these release comparisons. The unchanged disjoint
control, `rate(m_total[9s])` over 1,201 points, is slower: 0.791 → 1.019 ms with 4,066 entries fetched in both.
Other qualification jobs overlapped these shared-workstation runs; they do not establish isolated capacity or
universal speed improvement. Wide windows still revisit their own samples, and dense computed-window backing I/O
remains a separate source of work. These measurements do not establish candidate deployment or live VM latency.

The C average fixture's old physical-read assertion failed because Float reads decreased from 48/64 to 34/36
for one-point/larger pages. An actual C diagnostic copy retained every other assertion and confirmed the new
counts; stale-native reads remained 96/128. The corrected exact expectations preserve values, labels, sample
credits, cursor counts, calibrated provider failures and cleanup checks. Both original failures remain evidence.

### Exact Count boundaries

Ordinary direct `count_over_time()` windows can reuse exact integer counts at their requested lower and upper
boundaries. For overlapping windows, the first scan records covered boundaries; subsequent adjacent clocks read
only the newly reached tail. Parent preparation can reuse already completed clocks. Count is the integer difference
between two prefixes, converted to Float only afterward. No raw-history array or expiration queue is retained.

The catalog owns a lazy, optional provider-backed table per admitted Count call. Its extent is
`rows * (5 + 2 * clocks) * 8` bytes: five header words per physical series and two boundary counts per query clock.
These are completed summaries in backing storage, not a RAM allocation of the full table. Transfers use bounded
buffers, including 32-word lower-boundary batches and one clean last-row header cache. Query accounting includes
owners, temporary extents, calls and transfers. Both full and compact Folded progress layouts remain unchanged.

Admission requires the same call, selector, physical series, precise range, query scope and checked clock geometry,
with `0 < step < range`. Equal-width/disjoint windows, fixed-time selectors, derived matrices, anchored/smoothed
inputs and statistics projection retain ordinary evaluation. Missing optional backing retains ordinary replay;
actual backing errors fail. A native allocation, including a stale native value, revokes this optimization for the
series through the rest of the query. Float NaNs count, stale Float samples do not, and zero samples produce absence.

Preparation and final execution use separate generations. The first final acquisition is fresh. Only successfully
committed headers make boundary cells readable; EOF and unconsumed page tails do not certify future clocks.
Backing failures during an ordinary source probe are retained until source validation finishes, so a later source
error still wins. Cancellation remains immediate. The current pending backing error is returned before native
fallback, logical credits or publication. Full-window logical sample credits remain distinct from newly read samples
and from Matrix cursor progress.

Qualification lives in `count_windows`, the typed-provenance `range_cache` tests, and the actual C
`completed_count_windows` panel. Cover uninitialized backing, boundary equality, sparse/empty/nonfinite data,
negative clocks, skipped clocks, fresh final data, native transitions, scope mismatch, absent backing, read/write
failures and cancellation. Include a full 32-word staging failure followed by a later source error, and a partial
staging failure before native fallback. Run pages 1, 17 and 1,024 and both name-removal modes. General Float reducers
must not inherit Count's integer subtraction identity.

[count-windows-qualification.json](count-windows-qualification.json) records 1,096 passing units per optimized
profile (two ignored), strict Clippy, 1,932 new Count comparisons, 128,652 builtin and 231,540 SRE comparisons,
and 18,192 retained temporal/temporary/count-values comparisons. Those comparisons match complete outcomes and
numeric bits. The separate 31,572-execution remainder retains 924 ordering disagreements and six copies of the
established numerical witness; these are not strict passes. Actual native, ARM64 and ARM32 C/C++ callers pass,
including 42 Count cases per C run, and the Netdata binary passes all six DBengine write/read/append phases.

The controlled fixture has 4,501 points per series, 31 output clocks, a 30-second step and 17-sample pages. Six
alternating measured repetitions follow one excluded warmup. Release medians compare the preceding adjacent-window
implementation with the Count boundary implementation:

| Sources | Expression | Before | After | Fetched entries before → after |
| --- | --- | ---: | ---: | ---: |
| 1 | `count_over_time(m[1h])` | 11.380 ms | 1.555 ms | 223,476 → 9,276 |
| 1 | `sum by(id)(count_over_time(m[1h]))` | 16.796 ms | 1.525 ms | 335,214 → 9,276 |
| 1 | `count_over_time(m[5m])` | 1.083 ms | 1.086 ms | 18,996 → 2,680 |
| 1 | `sum by(id)(count_over_time(m[5m]))` | 1.637 ms | 1.240 ms | 28,494 → 2,680 |
| 8 | `count_over_time(m[1h])` | 89.965 ms | 10.934 ms | 1,787,808 → 74,208 |
| 8 | `sum by(id)(count_over_time(m[1h]))` | 134.560 ms | 11.607 ms | 2,681,712 → 74,208 |
| 8 | `count_over_time(m[5m])` | 8.445 ms | 8.289 ms | 151,968 → 21,440 |
| 8 | `sum by(id)(count_over_time(m[5m]))` | 12.897 ms | 9.427 ms | 227,952 → 21,440 |

Admitted queries add 1,616 bytes to the measured workspace peak: the one-source bare query grows from 7,657 to
9,273 bytes, and eight-source grouped Count from 16,893 to 18,509 bytes. The other seven memory-category peaks
are unchanged; raw borrowed-page peak stays 4,624 bytes. Eight-source grouped one-hour Count reduces cumulative
workspace allocation from 11,092,668 to 1,277,708 bytes. Its backing extent is 4,288 bytes. Category peaks are not
a simultaneous total-RAM measurement; backing extent and compiled code are separate costs.

Equal-width Count and SUM controls retain their memory and fetched-entry counts. Their measured times remain
within about 0.02 ms of the predecessor. The first unbatched Count implementation regressed bare five-minute
queries by 8–14%; it is preserved separately. Batching lower boundaries and caching one clean header remove that
measured regression in the current repeated fixture. Tiny, empty or sparse queries can still pay store overhead;
these shared-workstation results establish neither universal speedup nor live VM latency.

The native release fixture's text grows by 56,048 bytes (1.24%), with data +632 and BSS +664 bytes. Release-min
text grows by 37,776 bytes (1.32%), data +384 and BSS +2,800 bytes. Total fixture sizes grow from 4,806,734 to
4,864,078 bytes and 3,123,185 to 3,164,145 bytes respectively. These are fixed compiled costs, not per-query RAM.
Historical receipts remain bound to their own binaries. The C harness output-roster correction happened after the
Count comparison runs; it changes no runtime source or imported comparison helper and is separately bound by
the final caller manifests. Wider practical remediation and live histogram projection remain open; historical
bucket-boundary repair is explicitly excluded by decision 71.
