# Completed discovery for grouped counts

Application-disappearance queries often compare a historical population with the currently present population:
`count by(host, app)(count_over_time(m[1h])) unless on(host, app) count by(host, app)(m)`.
Both sides return counts, but repeatedly discovering their group membership added work even when the existing
set-operation machinery already held a complete, validated discovery result.

The grouped discovery admission now accepts an ordinary `count(selector)` and, for a multi-clock range,
`count(count_over_time(selector[range]))`. It reuses the existing set membership owner. Selected values still
evaluate their original operands, with required final reads, errors, cancellation and cleanup preserved.
There is no new raw-history cache, provider capability, implicit limit, arithmetic rule or storage policy.

Leading parentheses now reach the grouped admission check. The check receives the original expression node,
so every wrapper still passes its original clock, boundary and preprocessing checks. This also corrects the
same missed admission for existing parenthesized grouped Sum expressions.

Admission remains deliberately bounded: one aggregate, the existing selector/window proof and exact matching
clocks. Fixed or ranked Count, nested aggregates, Count over `abs`, derived matrices and other window reducers
retain ordinary evaluation. Single-clock Count windows also retain ordinary evaluation. These are optimization
boundaries, not language exclusions. Final discovery still requires unique Float result rows; native input is
evaluated and validated before a Count result can supply that proof.

## Measured cost

[Qualification](count-set-qualification.json) and [compressed observations](count-set-observations.jsonl.gz)
bind the measured sources, executables, complete outcomes and counters. The fixture has 32 groups, two inputs
per group, 31 evaluation times, a one-hour window and 1,024-entry provider pages. It includes disappeared,
stale/resumed, continuously present and never-collected populations, including ordinary NaNs.

| Operation | Accounted work before → after | Release time ratio | Release-min time ratio |
| --- | ---: | ---: | ---: |
| Unless | 5,545,308 → 4,040,278 | 0.956 | 0.867 |
| And | 5,425,673 → 3,994,411 | 0.965 | 0.857 |
| Or | 7,383,134 → 4,527,760 | 0.878 | 0.736 |

Each set expression still fetches 644,752 sample entries and makes 16,319 temporary-backing reads.
Unless/And workspace peak changes from 163,984 to 164,264 bytes; Or changes from 168,872 to 169,152 bytes.
The other seven tracked memory-category peaks are unchanged. These category counters are not process RSS.
Normal Netdata executable text grows by 1,192 bytes, from 36,709,579 to 36,710,771; data and BSS are unchanged.

Timing uses five alternating baseline/candidate blocks, three repeats per binary and two profiles: 300 queries
including warmups. Repeat zero is excluded within each binary/block. The standalone operand controls have
unchanged accounted work, but are 3.1–4.4% slower in release and 3.7–4.5% faster in release-min in this run.
The query fixture uses `Control::default()`, so these timings have no query deadline polling. HTTP requests
exercise a different control path. These shared-host measurements exclude fixture transport and do not
establish live concurrent throughput.
The work reduction is larger than the measured speedup and must not be presented as equivalent to it.

## Qualification and reproduction

- Full Rust library suites: 1,110 passed and two ignored in each profile.
- Twenty analytic cases per measured binary, with independently calculated complete counts and membership.
- 486 complete outcomes per baseline/candidate profile: three page sizes, both name-removal modes, three time
  scopes, all three set operators, Float/native/mixed histories and distinct output names sharing matching keys.
- Actual admission boundaries, native output and cancellation tests, plus 48 required final-acquisition failures.
- Eight native GCC/Clang C/C++ consumers, 468 lifecycle/error cases each.
- Normal Netdata: eight storage phases and 922 observations. Actual HTTP qualification covers both codecs,
  including all three grouped-count set operations across collected hosts.

### Standalone C qualification correction — 2026-10-29

The original eight standalone C consumers linked older top-level static libraries. Building the Rust fixture
binary updated its dependencies but did not republish `target/<profile>/libnetdata_promql_engine.a`.
The source snapshots recorded beside those C executions therefore did not establish their actual library cohort.
Those original executions qualify the older library, not this Count optimization.

The replacement [C qualification](count-set-c-qualification-correction.json) explicitly builds `--lib` in each
profile before linking and running all eight consumers. It preserves the original observations and identifies
their earlier library provenance. The Rust fixture/unit results and Netdata storage/HTTP qualification remain
valid: their actual build logs bind the fresh Rust binaries or explicitly build and link the library.

From the crate directory, build `promql-query-fixture` with the `fixtures` feature. Then pipe
`python tests/promql-conformance/count_set_cases.py` into that executable. The generator defaults to 8/32 groups
and pages 17/1024; `--groups` and `--pages` select another fixture size. Pipe output through gzip when retaining
it. The module's `expected()` and `normalized()` functions provide the separate analytic result check.
The published generator was checked against the original private generator and all 60 retained analytic results.

Run the three `grouped_count_set_discovery` unit tests and the full library suite in both profiles.
The actual C lifecycle panel is `comparison_sets.c`; the collected HTTP panel is Netdata's
`tests/promql-integration/live.py` with an internal-check Agent. Existing upstream qualification remains
historically attributed: this cohort compares the unchanged engine against the candidate rather than claiming
a new exhaustive Prometheus run or new ARM execution coverage.

Live concurrent and long-history qualification remains a separate deployment step. This optimization does not
resolve the histogram representation decision or change which retained storage tier supplies samples.
