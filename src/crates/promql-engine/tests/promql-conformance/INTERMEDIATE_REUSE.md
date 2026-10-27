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
