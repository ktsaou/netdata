# Practical PromQL workload findings — 2026-10-26

The expanded suite has **12 dashboards and 136 panel expressions**, including cross-node bandwidth, application CPU/RSS,
instance comparisons, ratios, disappearance detection and alert conditions. It now measures real evaluation resource
counters as well as HTTP latency. The main finding is repeated work under common aggregate wrappers and joins;
long-range performance is not yet suitable for these workloads under a 25-second request deadline.

This is a baseline and an optimization priority list, not a claim that all dashboards are fast or that the live adapter
supports every PromQL input type. Alert-expression testing does not install an alert scheduler or notification system.

The `fleet4` dashboard cohort honors panel settings: 120 range and 16 instant requests. Earlier references to
its “range cohort” mean this mixed panel workload. The separate instant cohort evaluates all 136 targets instantly.

## Follow-up: label-comparison control checks — 2026-10-29

Runtime `2c49941745` is installed. Shared byte comparisons now avoid an immediate duplicate entry check while
retaining entry and subsequent-chunk interruption. The [controlled qualification](../../../src/crates/promql-engine/tests/promql-conformance/LABEL_COMPARISON.md)
finds 4–9% improvements in representative release-profile joins, mixed results elsewhere, and no tracked query-memory
increase. Both profiles pass 1,112 unit tests; all 672 measured C executions preserve outputs and resource counters.

The [live comparison](label-comparison-live.json) has 348 successes and the same 12 timeouts in each 360-query run.
Two concurrent fleet panels and ten long-history requests time out. Fleet batch time changes from 173.89 to 178.90
seconds; the median successful-pair ratio is 0.990. This does not establish an overall live speedup. The unchanged
engine already had these 12 failures in the fresh baseline, compared with nine in the earlier capture.

All successful labels, timestamps and counts agree; all 360 warning/info arrays agree. Unordered results match in
345 of 348 successful pairs. The three averages differ at 33 points by at most two floating-point units in the last
place. Unchanged inner values in each run's independently observed host order reproduce every point exactly,
including the additional instant fleet CPU average. The instant expression does not enter the multistep ordering
boundary. As before, restart-mediated metadata loading is an inferred explanation for the observed order change.

Installation and configuration comparisons pass. Grafana returns 79 frames, and all 18 common observed CPU hosts
advance their reception timestamps. Both services are active with 41 GiB free at deployment verification.
The long-history/concurrency findings and histogram design decision remain open.

## Follow-up: grouped Count set discovery — 2026-10-28

Runtime `02f2343863` was installed for this comparison. Grouped Count set operations now reuse completed membership discovery while
preserving fresh selected values and final validation. The [controlled qualification](../../../src/crates/promql-engine/tests/promql-conformance/COUNT_SET_DISCOVERY.md)
covers ordinary selectors, count windows, native inputs, wrapper clocks and actual C/HTTP callers.
The standalone C-library source attribution was corrected on October 29; all eight consumers were rerun after
explicit library builds. Rust fixture/unit and Netdata storage/HTTP evidence were unaffected. See the linked
qualification's correction for original and replacement artifacts.

The [complete live comparison](count-set-live.json) records 351 successes and the same nine timeouts in each
360-query run. The application-availability range query improves from 22.00 to 18.19 seconds; accounted work
decreases from 912,901,876 to 650,063,444 with 13,294,907 fetched sample entries unchanged. Its instant counterpart
changes from 6.25 to 6.69 seconds. Fleet panel batch time changes from 107.66 to 112.14 seconds, and the median
elapsed ratio across successful pairs is 1.047. These observations do not establish an overall speedup.

All successful pairs retain result types, labels, timestamps and counts; all 360 warning/info arrays agree.
Unordered values match in 349 of 351 successful pairs, including all 17 Count/unless pairs. The remaining two
averages differ at 26 points by at most two floating-point units in the last place. Unchanged inner values,
combined in each run's independently observed host order, reproduce every one of their 6,087 and 31 points
exactly. Provider order is preserved under [decision 9](../../../src/crates/promql-engine/tests/promql-conformance/DESIGN.md#9-define-deterministic-order-only-where-upstream-leaves-it-unspecified);
restart-induced metadata loading remains an inference, not a recorded nested-callback trace.

Installation and persistent configuration comparisons pass. Grafana returns 79 frames, and all 18 common
observed CPU hosts advance their reception timestamps. The remaining failures are application CPU sums and
service CPU rates at 6 hours, 24 hours and 7 days, plus three 7-day fleet/history/alert expressions. Historical
bucket-boundary changes remain outside scope, and cumulative histogram exposure still awaits a design decision.

## Follow-up: direct DBengine allocation handles — 2026-10-27

Runtime `c48c575224` was installed for this comparison. DBengine read leases now own their existing Rust allocation handles directly,
removing a C wrapper and its extra accounting handle. The [controlled qualification](../allocation-handles-qualification.json)
covers ownership, resizing, refusal cleanup and handles retained beyond query/report lifetimes. Both build modes
pass storage/API checks; the corrected metadata fixture passes 148 HTTP checks and 24 host-readiness cases.

The [complete live comparison](allocation-handles-live.json) still has ten timeouts out of 360 requests on each
version. Seven-day bandwidth now completes in 21.77 seconds; a concurrent application-disappearance query changes
from 23.36 seconds to a timeout. Fleet panel batch time is 109.14 seconds before and 110.85 seconds after.
The median elapsed ratio across 349 successful pairs is 1.001. This does not establish an overall speedup.

All successful pairs retain labels, timestamps, result counts and complete diagnostics. Unordered results match
exactly in 347 pairs. The other two averages differ at 13 points by one or two floating-point units in the last
place. Unlike earlier captures, these runs retain complete compressed responses. Their inner values are unchanged;
applying each run's independently observed host order reproduces all 6,087 application-average points and all
31 bandwidth-average points exactly. Provider order is preserved by [decision 9](../../../src/crates/promql-engine/tests/promql-conformance/DESIGN.md#9-define-deterministic-order-only-where-upstream-leaves-it-unspecified).
Restart-mediated metadata loading is an inferred cause of the changed host order, not a directly recorded trace.

The live resource comparison is mixed: accounting-allocation traffic decreases overall among the 336 pairs with
identical sample counts, while workspace and borrowed-view traffic increase overall. These are cumulative
allocation/exposure counters, not simultaneous RAM. Cache residency and background work are uncontrolled; the
per-lease allocation reduction is established by the controlled C regression, not by a universal live-memory claim.

Installation and both persistent configuration comparisons pass. Netdata and Grafana remain active with 48 GiB
free. Grafana returns 79 frames in 1.62 seconds, and all 18 observed hosts advance. Remaining priorities include
the common disappearance query's repeated grouped-Count discovery, long-history latency and histogram decision 73.

## Earlier follow-up: overlapping journal lookup — 2026-10-27

Runtime `e02563e86b` was installed at this checkpoint. DBengine's journal index orders end times; a journal that starts after a
query cannot prove that every later journal also misses it. Both lookup modes now continue searching.
The [controlled regression](../journal-order-qualification.json) reproduces missing data with the unchanged
reader on both codecs and passes with the correction. This adds no reader state or history buffers; necessary
additional journal scanning is a correctness cost, not a claimed optimization.

The [full before/after replay](journal-order-live.json) completes 351 of 360 requests on both versions.
The same nine requests time out: two at six hours, two at one day, and five at seven days. All 136 fleet
panels, 136 instant requests and 64 selected short/concurrent requests complete. Fleet panel batch time is
103.39 seconds before and 111.35 seconds after; live caches, data and background work are uncontrolled.

All successful pairs preserve series/point and diagnostic counts. Complete row hashes match in 278 pairs
in emitted order and 348 after removing order differences. Application CPU-average and bandwidth-average
produce three remaining content differences across range/instant cohorts. This harness retains hashes and
counts, not the complete response bodies, so those historical differences cannot be reconstructed from this
capture. Their cause is unproved; no full live-value equivalence or overall speedup is claimed.

Installation completes in 205.73 seconds with 3.1 GiB peak build memory and both configuration comparisons
passing. The running binary/process remain unchanged during replay, Netdata and Grafana stay active, and
49 GiB remains free. Grafana returns 79 frames in 1.65 seconds; all 18 reporting CPU hosts advance.
Long-history latency and histogram representation remain open; historical histogram boundary repair remains
excluded by decision 71.

## Earlier follow-up: C page publication polling — 2026-10-27

Runtime `72f69b6fd0` was installed at this checkpoint. The C adapter no longer walks every validated sample solely to poll control
and count work. Complete page validation and the final control check remain. The
[controlled qualification](../adapter-page-polling-qualification.json) records unchanged measured memory and
I/O, both profiles with 1,107 passing tests, actual GCC/Clang C/C++ callers, and independent review.
Across 72 controlled C-query configurations, median elapsed ratios are 0.850 with deadlines and 0.964 without.
Three initial slower cases remain recorded; none reproduces in 15 longer paired blocks.

The [fresh before/after live replay](adapter-page-polling-live.json) covers all 360 requests:

| Cohort | Before timeouts | After timeouts |
|---|---:|---:|
| Fleet panels, four clients | 0/136 | 1/136 |
| Fleet instants | 0/136 | 0/136 |
| Selected short ranges and eight-client cases | 0/64 | 0/64 |
| Six hours | 2/8 | 2/8 |
| One day | 2/8 | 2/8 |
| Seven days | 6/8 | 5/8 |
| Total | 10/360 | 10/360 |

Seven-day fleet bandwidth completes in 21.61 seconds instead of timing out. The concurrent application physical-I/O
panel now times out. The fleet panel batch takes 108.13 seconds versus 105.68 seconds before. These live results do
not establish an overall workload improvement; caches, retained data and background work are uncontrolled.

All 349 successful pairs retain their series/point and diagnostic counts. Complete row hashes match in 280 pairs
in emitted order and 347 after removing ordering differences. Application CPU-average and bandwidth-average panels
have two remaining value-level digest differences; their before/after value equivalence is not established by this
capture. These are the same query families investigated in the earlier live-comparison analysis, not proof that this
pair has the same cause. Original observations remain in both compressed captures.

Installation succeeds in 250.71 seconds with 3.1 GiB peak build memory; both persistent configuration comparisons
pass. Netdata and Grafana remain active with approximately 50 GiB free. Grafana returns 79 frames in 1.62 seconds,
all 18 reporting CPU hosts advance, and the installed binary/process remain unchanged throughout the replay.
Long-history and concurrent-query failures remain open. Histogram representation remains under decision 73 discussion;
historical bucket-boundary repair remains excluded by decision 71.

## Earlier follow-up: buffered metadata key comparisons — 2026-10-27

Runtime `58f4be2276` was installed at this checkpoint. Stored key comparisons reuse the existing 8 KiB scratch window across fields,
reducing small backing reads during duplicate discovery. Contexts ownership, lexical output, native units and
paged storage are unchanged. No extra heap buffer or persistent metadata catalogue is added.
[Controlled qualification](../metadata-buffered-keys-qualification.json) covers both builds, key boundaries,
allocation cleanup, cancellation and 142 HTTP checks. Independent implementation and evidence reviews passed.

[Live measurements](metadata-buffered-keys-live.json), five serial repetitions after warmup:

| Request | Before median | After median |
|---|---:|---:|
| Whole catalogue | 2.484 s | 0.755 s |
| Unfiltered `limit=1` | 2.441 s | 0.726 s |
| Unfiltered `limit=0` | 0.103 s | 0.104 s |
| Exact `system_cpu` metadata | 6.98 ms | 7.36 ms |

Full-catalogue and one-name medians improve about 70%. Four concurrent full-catalogue requests finish as a batch
in 1.05 seconds versus 2.76 seconds before. Serial response digest sets match for every case, including all
4,964 names and 4,972 descriptions. Caches and live data are uncontrolled; these measurements do not establish
cold-cache performance or universal throughput. Positive limits still traverse and sort the complete catalogue.

Installation completed in 209.53 seconds with 3 GiB peak build memory and both configuration comparisons passing.
The deployment observer initially stopped after systemd collected the completed transient unit; retained journal
and installed binary checks establish success. Read-only measurements resumed without another install or restart.
Netdata and Grafana remain active with approximately 50 GiB free. Grafana CPU history returns 18 frames in
0.51 seconds, all 18 reporting CPU hosts advance, and the process/binary remain unchanged after checks.

This metadata-only step does not repeat the full 360-query workload. Long-history and concurrent expression latency
remain open; histogram representation remains under discussion and historical boundary repair is excluded by 71.

## Earlier follow-up: zero-limit metadata preparation — 2026-10-27

Runtime `049817dc89` was installed at this checkpoint. A zero name limit now discards descriptions before building intermediate
records/indexes or sorting. It retains contexts traversal, parameter checks, unavailable-host errors and cancellation.
[Controlled qualification](../metadata-zero-limit-qualification.json) covers internal and normal builds plus 142 HTTP
checks. [Live evidence](metadata-zero-limit-live.json) records five serial repetitions per case, after one warmup:

| Request | Before median | After median |
|---|---:|---:|
| Unfiltered `limit=0` | 2.347 s | 0.091 s |
| Unfiltered `limit=1` | 2.447 s | 2.461 s |
| Whole catalogue | 2.431 s | 2.435 s |
| Exact `system_cpu` metadata | 7.36 ms | 6.85 ms |

The zero-limit reduction is 96.1%; positive limits and full-catalogue preparation are unchanged. The serial response
digest sets match for every case, including the full 4,964-name / 4,972-description catalogue. Four concurrent zero-limit
requests finish in 0.47–0.51 seconds each, versus a 2.82-second batch before. These are warm, uncontrolled live
observations; they establish neither cold-cache performance nor universal throughput.

Installation succeeds in 196.79 seconds with both configuration comparisons passing. Netdata and Grafana remain
active with approximately 50 GiB free. Grafana's CPU-history request returns 18 frames in 0.52 seconds; all 18
reporting CPU hosts advance. The full 360-query workload is not repeated for this metadata-only change.
Full-catalogue preparation, positive limits, long-history and concurrent-query findings remain open. Histogram
representation remains under discussion, with historical boundary repair excluded by decision 71.

## Earlier follow-up: metric descriptions through Netdata and Grafana — 2026-10-27

Runtime `c46a8fbe6a` was installed at this checkpoint. GET `/api/v1/metadata` now returns context-owned instance titles, native units
and counter/gauge types under exact and underscore names. Both direct and Grafana-proxy routes previously returned
404. [Live evidence](metadata-descriptions-live.json) records the implemented endpoint and its remaining cost:

- Exact CPU descriptions: **4.8–9.9 ms** across direct and proxy aliases.
- Whole catalogue: **4,964 names / 4,972 descriptions**, **706,579 JSON bytes**, **2.39–2.43 seconds**.
- Four simultaneous catalogue requests: **2.42–2.59 seconds each**, all successful.
- Unfiltered `limit=0` and `limit=1`: **2.32–2.37 seconds**. They still build the full catalogue before limiting
  output. This is measured unnecessary work and remains an optimization target.
- Grafana's real CPU-history query returns **18 frames in 0.50 seconds**. All **18 reporting CPU hosts advance**.

These are repeated warm live observations, not a cold-cache or universal throughput guarantee. The initial short
reception check saw 17/18 advancing; a later check and the full repeated capture both saw 18/18. Its stopped timing
capture is retained rather than presented as a successful measurement.

The first normal build found a test-helper compile-guard mismatch missed by internal-check qualification. The
corrected revision passes a normal build and six normal storage/API phases, with independent review. Installation
then succeeds in 199.22 seconds; both config comparisons pass, both services remain active and 51 GiB is free.
[Qualification](../metadata-descriptions-qualification.json) preserves the failed link and separate build modes.
This endpoint adds no persistent metadata catalogue and reads no metric samples. Long-history and concurrent-query
findings below remain open; histogram representation remains under discussion.

## Earlier follow-up: completed mapped aggregate intervals — 2026-10-27

Engine `e1c932c7ce` was installed for this checkpoint. Eligible fixed maps can reuse completed aggregate intervals across output groups.
[Controlled qualification](../../../src/crates/promql-engine/tests/promql-conformance/MAPPED_GROUP_INTERVALS.md)
records lower work and allocation traffic without retaining raw history. The full
[360-request live replay](mapped-group-intervals-live.json) preserves the preceding run's windows, scopes and deadlines.

- The selected CPU-history query improves from **13.72 to 3.02 seconds at six hours**. At 24 hours it completes in
  **9.55 seconds**, versus a 25-second timeout previously. Its seven-day case still times out.
- Fleet panels at four clients: **2/136 timeouts**, previously zero; cohort **106.81 to 106.34 seconds**.
  Application physical I/O and disappearance detection time out in this run.
- All 136 instant requests and selected short-range cohorts complete without timeouts.
- Selected 6h/24h/7d: **2/8, 2/8 and 6/8 timeouts**, previously 2/8, 3/8 and 5/8.
- Total: **12/360 timeouts**, previously 10. This is a targeted improvement, not an overall workload success.

Live caches and data are uncontrolled. Among 347 successful pairs, 291 ordered result hashes match and 56 differ;
all series/point counts match. The baseline lacks the newer row-order-independent hashes, so the remaining historical
value uncertainty is not resolved by this replay. Controlled same-input qualification remains separate.

Grafana application share returns 79 frames in 1.73 seconds; all 18 reporting CPU hosts advance. Both services remain
active, configuration comparisons passed, the installed binary remains unchanged after the replay, and approximately
51 GiB is free. Help/type metadata is separate unfinished work and is not included in this deployment. Live histogram
projection remains under discussion; historical bucket-boundary repair stays excluded by decision 71.

## Earlier follow-up: resolved aggregate inputs — 2026-10-27

Engine `4fdd8413bb` was installed for this checkpoint. Direct selector, rate-family and Count inputs can reuse an aggregate's resolved
position and labels with multiple grouping keys. [Controlled qualification](../../../src/crates/promql-engine/tests/promql-conformance/RESOLVED_GROUP_INPUTS.md)
records 20–26% faster wide-label disappearance queries, lower allocation traffic and unchanged measured memory peaks.

[The full live replay](resolved-group-inputs-live.json) repeats all 360 requests against a fresh unchanged-engine
baseline, preserving windows, steps, scopes and deadlines:

- Fleet panels at four clients: **0/136 timeouts**, previously 1; cohort **119.53→106.81 seconds**.
- Disappearance detection completes in **23.09 seconds**. This remains close to its 25-second deadline.
- All 136 instant requests and selected short-range cohorts complete without timeouts.
- Selected 6h/24h/7d: **2/8, 3/8 and 5/8 timeouts**; the preceding seven-day result was 6/8.
- Total: **10/360 timeouts**, previously 12. Seven-day bandwidth now completes in 22.79 seconds.

Among 348 successful pairs, 270 raw-result hashes match and 78 differ, with unchanged series/point counts.
There are **76 instant and two range differences**; the latter are application averages and mean outbound bandwidth.
The bandwidth range has one series, so result-array order alone cannot explain every difference. Context traversal
can change across restart and alter sequential floating-point rounding, but that remains a hypothesis for these
historical differences. Complete historical bodies were not captured, so **live value equivalence is unverified**.

The affected current queries repeat stably. Captured inner-reduction values produce identical results in the old
and new engines in 72 fixed-input pairs, covering both profiles, three page sizes and reversed source order.
These checks do not reconstruct missing historical bodies. Future measurements retain both the original ordered
hash and a separate complete-row order-independent hash; neither drops duplicate rows or sample fields.

Grafana application share returns 79 frames in 1.40 seconds; all 18 reporting CPU hosts advance. Both services are
active, configuration checks passed, data was preserved and approximately 51 GiB remains free. Long-history latency,
the historical comparison uncertainty, live histogram representation and help/type metadata remain open. Historical
histogram bucket-boundary repair stays excluded by decision 71.

## Earlier follow-up: Count summaries from validated page tails — 2026-10-27

Engine `73313c6f69` was installed for this checkpoint. It reuses bounded integer summaries from already validated pages, without keeping
raw history. [Controlled qualification](../../../src/crates/promql-engine/tests/promql-conformance/INTERMEDIATE_REUSE.md#count-summaries-from-validated-page-tails)
records 70.4% fewer fetched entries and 31.8–34.1% faster grouped Count sums in the large-page fixture, with a
32-byte query-workspace increase. [Live measurements](count-page-prefixes-live.json) repeat all 136 fleet panel
requests at four clients (120 range and 16 instant), then four focused serial Count requests. Windows, steps and deadlines are unchanged.

- Grouped Count under four clients: **21.37→13.37 seconds**.
- Serial grouped Count: **19.34/15.96→9.83/9.70 seconds**.
- Serial disappearance: a timeout plus 24.51 seconds previously; **20.36/20.78 seconds**, both successful now.
- Fleet cohort: **2/136 timeouts**, versus 1 previously; total time **129.26 versus 126.96 seconds**. Disappearance
  still times out under load, and the application physical-I/O panel also times out in this run.

The I/O expression does not use Count and has varied across earlier replays. This single uncontrolled run does not
establish its cause. Successful serial results match their earlier successful-result digests; a previous error is
not a comparable complete result. The aggregate fleet timing does not establish an overall throughput improvement.

Grafana application share returns 79 frames in 1.14 seconds; all 18 reporting CPU hosts advance. Netdata and Grafana
are active, configuration checks passed, retained data was preserved, and approximately 52 GiB remains free.
This is **not another full 360-request replay**: instant and long-history results below retain their earlier source
attribution. Long-history and concurrent-query performance, live histogram representation and help/type metadata
remain limitations. Historical histogram boundary repair is excluded by decision 71.

## Earlier follow-up: direct Count aggregate labels — 2026-10-27

Engine `947ef6c78f` was installed for this checkpoint. Eligible Count aggregates reuse existing group labels after fresh source matching
and completed evaluation. The [controlled qualification](../../../src/crates/promql-engine/tests/promql-conformance/COUNT_GROUP_LABELS.md)
shows targeted improvements without increasing measured memory peaks. The [full live replay](count-group-labels-live.json)
retains all 360 requests and their unchanged windows, resolution and deadlines.

- Fleet panel cohort: **1/136 timeout**, previously 3; cohort time 126.96 seconds, previously 135.94.
- Fleet instant: **0/136 timeouts**. Selected short-range and eight-client cohorts also have none.
- Selected 6h/24h/7d: **2/8, 3/8 and 6/8 timeouts**, unchanged from the previous full replay.
- Total: **12/360 timeouts**, previously 14. Grouped Count now completes in 21.37 seconds under four clients;
  disappearance still times out. The previously failing application I/O trend also completes in this run.

This full comparison includes the intervening single-clock C change. Live caches and data are uncontrolled, so it
cannot isolate either change. A separate four-request serial comparison against that immediately preceding build
records grouped Count at 19.34/15.96 seconds, versus 21.05/17.88 seconds; successful results have identical digests.
Disappearance timed out once and completed once in 24.51 seconds. This remains too close to the deadline to call fixed.

Grafana application share returns 79 frames in 1.57 seconds, all 18 reporting CPU hosts advance, both services are
active and approximately 52 GiB remains free. Configuration and retained data were preserved. Historical histogram
boundary repair remains excluded by decision 71; remaining performance and live representation work remains open.
The later page-summary deployment is recorded above.

## Earlier follow-up: single-clock C control snapshots — 2026-10-27

Engine `be36099f3b` was installed for this checkpoint. Its C control-state interface shares one clock reading for status and remaining
time, preserving all polling locations and deadline settings. The [component qualification](../../../src/crates/promql-engine/tests/promql-conformance/CONTROL_POLLING.md)
shows lower polling cost; the [focused live results](control-poll-live.json) do **not establish a whole-query speedup**.

Two fixed-window fleet Count expressions were run serially twice before and after deployment. Grouped Count took
20.01/19.18 seconds before and 21.05/17.88 seconds after; both successful results match their baseline digests.
The disappearance expression still timed out at 25 seconds in every run. These are four requests per revision,
not a new full-suite replay, and live caches/data are uncontrolled. The preceding full replay is recorded below.

Grafana application share returned 79 frames in 1.78 seconds; all 18 reporting CPU hosts advanced. Both services
are active, both configuration comparisons passed, retained data was preserved and approximately 53 GiB remains
free. The remaining performance and representation findings remain open. The subsequent Count label change is recorded in the newer checkpoint above.

## Earlier follow-up: exact Count boundaries — 2026-10-27

Engine `916d412696` was installed for this checkpoint. The [same-window results](count-windows-live.json)
and [compressed measurements](count-windows-measurements.jsonl.gz) repeat all 360 requests with unchanged
scopes, time windows, steps and 25-second deadlines. Count now reuses exact completed boundary summaries;
controlled fixture gains have **not resolved the two fleet Count timeouts**.

| Cohort | Previous deployment timeouts | Current deployment timeouts |
| --- | ---: | ---: |
| Fleet, 136 panel requests, 4 clients | 2 | 3 |
| Fleet, 136 instant queries, 4 clients | 0 | 0 |
| Selected 6-hour queries, 8 requests | 2 | 2 |
| Selected 24-hour queries, 8 requests | 3 | 3 |
| Selected 7-day queries, 8 requests | 7 | 6 |

The fleet range cohort took 135.94 seconds, versus 134.81 seconds previously. The one-hour application
absence query and grouped five-minute sample-count query still timed out. The application I/O trend also
hit the deadline in this replay; one live run does not establish the cause. Seven-day CPU availability
completed this time. Overall, 14 of 360 requests timed out, unchanged from the previous deployment.

Before timing out, the one-hour Count request fetched 16.48 million entries and read 6.21 million samples;
the five-minute Count request fetched 25.90 million and read 2.84 million. Both now use temporary backing.
These are partial executions with different amounts of completed work, not throughput comparisons.
Source traversal, backing callback traffic and remaining long-history costs need further investigation.

The application-share request returned 79 Grafana frames in 1.58 seconds. All 18 reporting CPU hosts advanced,
Netdata and Grafana remain active, and approximately 53 GiB remains free. Both installation configuration
comparisons passed and retained data was preserved. Live data and cache state remain uncontrolled; exact-input
core qualification is documented separately in the [Count guide](../../../src/crates/promql-engine/tests/promql-conformance/INTERMEDIATE_REUSE.md#exact-count-boundaries).
Historical histogram bucket-boundary repair is excluded by decision 71; other live histogram representation
and measured performance gaps remain open.

## Earlier follow-up: mapped and adjacent-window reuse — 2026-10-27

Engine `019cc2bffa` is installed on the playground. The new
[same-window results](adjacent-windows-live.json) and [compressed measurements](adjacent-windows-measurements.jsonl.gz)
repeat all 360 requests after adding scalar-mapped aggregate reuse and forward traversal for overlapping Float
windows. The 25-second deadline, scopes, time windows and resolutions are unchanged.

| Cohort | Before either optimization | Previous deployment | Current deployment |
| --- | ---: | ---: | ---: |
| Fleet, 136 panel requests, 4 clients | 28 timeouts | 2 | 2 |
| Fleet, 136 instant queries, 4 clients | 4 timeouts | 0 | 0 |
| Selected 6-hour queries, 8 requests | 7 timeouts | 6 | 2 |
| Selected 24-hour queries, 8 requests | 7 timeouts | 6 | 3 |
| Selected 7-day queries, 8 requests | 8 timeouts | 8 | 7 |

The six-hour bandwidth query now finishes in 5.91 seconds and the mapped history-quantile query in 11.89 seconds;
both timed out previously. The fleet range cohort takes 134.8 seconds, versus 149.4 seconds on the previous
deployment and 298.3 seconds at baseline. The same two-host application-share request returns 79 Grafana frames
in 1.54 seconds. All 18 reporting CPU hosts advanced their timestamps, both services are active, and approximately
53 GiB remains free. Configuration comparisons passed and retained data was preserved.

The two short-range failures are still the one-hour application disappearance query and five-minute application
sample-count query. They time out before producing output and report no temporary backing use. Six-hour application
CPU totals and service-instance history also remain too slow; most seven-day cases still time out. Wide Count
windows still rescan their current samples, and dense computed-window reads still incur excess backing traffic.
These remain remediation work, together with live histogram representation. Empty alert predicates are successful
requests, not proof of healthy infrastructure. Live input data and caches are not frozen, so these observations
cannot substitute for the core's separate exact-input qualification.

## Earlier follow-up: completed intermediate reuse — 2026-10-27

Engine `306f6e4c71` was installed for this earlier checkpoint. The
[same-window follow-up](intermediate-reuse-live.json) and [compressed measurements](intermediate-reuse-measurements.jsonl.gz)
repeat all 360 main requests with the original scopes, steps, concurrency and 25-second deadline.

| Cohort | Previous timeouts | Current timeouts |
| --- | ---: | ---: |
| Fleet, 136 panel requests, 4 clients | 28 | 2 |
| Fleet, 136 instant queries, 4 clients | 4 | 0 |
| Selected fleet queries, serial, 16 requests | 6 | 0 |
| Selected fleet queries, 8 clients, 16 requests | 6 | 0 |
| Selected 6-hour queries, 8 requests | 7 | 6 |
| Selected 24-hour queries, 8 requests | 7 | 6 |
| Selected 7-day queries, 8 requests | 8 | 8 |

The 136-query range cohort completes in 149.4 seconds versus 298.3 seconds previously. The same two-host application
share request now returns 79 frames through Grafana's actual datasource backend in 1.55 seconds; it previously failed
at 30 seconds. All 18 reporting CPU hosts advanced their timestamps during verification. Both services remain active,
configuration checks passed, retained data was preserved, and approximately 54 GiB remains free.

These results show substantial short-range progress, not completion. Application sample counts and disappearance
queries still time out in the range cohort; most long-history cases remain too slow. Source tracing identifies
overlapping raw-window prefix replay and mapped aggregate subqueries that miss completed-window reuse. Histogram
representation remains unresolved. Live data and caches can change between runs; identical request parameters do not
make this an immutable-data numerical comparison. The core's separate qualification is in
[the intermediate-reuse guide](../../../src/crates/promql-engine/tests/promql-conformance/INTERMEDIATE_REUSE.md).

The sections below preserve the original baseline and its outstanding findings.

## Delivered and deployed

- Eight new dashboards supplement the original four. Their queries are generated by [workloads.py](workloads.py),
  referenced by [the catalogue](workload-catalogue.json), and consumed directly by [measure.py](measure.py).
- Optional `stats=all` now exposes existing execution memory, fetched samples, work, backing and DBengine page/point
  observations. Ordinary responses and `stats=true` retain their previous shape. See the
  [snapshot contract](../../../src/web/api/promql/README.md#optional-evaluation-resource-observations).
- The playground runs Netdata `4b2fd852b8`, with its configuration and retained data preserved. Subsequent commits change
  dashboard/test/documentation files, not that installed C/Rust implementation. Dashboard expressions are bound to
  `6260bb1db5`, with readable legends from `ba45ca8ff4`.
- After deployment, both services are active; 24 hosts are retained and all 18 currently reporting CPU data advance their
  sample timestamps. About 56 GiB remains free on the 100 GiB SSD. The VM has 8 vCPUs and 16 GiB RAM.

## Measurements

The [qualification summary](practical-qualification.json) binds **360 main measurements**, plus 13 focused diagnostic
requests, three metadata requests and input-presence checks. The [compressed records](practical-measurements.jsonl.gz)
retain per-request counters and expression templates without returned identities or values. Read the gzip through EOF
and compare its compressed/uncompressed hashes with the qualification summary when reusing it.

| Cohort | Requests | Nonempty | Empty | Timed out |
|---|---:|---:|---:|---:|
| Fleet, 15-minute ranges, 30-second steps, 4 clients | 136 | 100 | 8 | 28 |
| Fleet, instant expressions, 4 clients | 136 | 121 | 11 | 4 |
| Eight selected queries, one host, serial, two repetitions | 16 | 10 | 6 | 0 |
| Same selection, two hosts, serial, two repetitions | 16 | 12 | 4 | 0 |
| Same selection, fleet, serial, two repetitions | 16 | 10 | 0 | 6 |
| Same selection, fleet, 8 clients, two repetitions | 16 | 10 | 0 | 6 |
| Same selection, fleet, 6 hours / 60-second steps, serial | 8 | 1 | 0 | 7 |
| Same selection, fleet, 24 hours / 300-second steps, serial | 8 | 1 | 0 | 7 |
| Same selection, fleet, 7 days / 1,800-second steps, serial | 8 | 0 | 0 | 8 |

These use an explicit **25-second benchmark request deadline**, not a new configured engine limit. Ingestion and
replication continue, and caches are uncontrolled. These are operational observations, not isolated capacity guarantees.
The seven-day requests do not establish seven days of complete data availability. The selected long-range cases are a
focused sample, not every panel or every combination of range, resolution and scope.

“Nonempty” means a returned series, not proven numerical correctness, finite values at every point, complete input
history or healthy infrastructure. The 11 empty instant cases had nine distinct input selectors; each had matching
current data at that cohort timestamp. This rules out complete current-selector absence for those checks, but does not
prove complete temporal input coverage or that an alert condition should be false.

The first 136-range cohort retained two incorrect dashboard offset templates. Both were subsequently corrected and
rerun successfully with nonempty results; their original empty records are preserved, not rewritten. An earlier
118-query discovery pass also caught a malformed generated expression. It is not counted in the main table.

## Ranked work to do next

### 1. Stop reevaluating aggregates under ordinary arithmetic and matching joins

This is the strongest concrete first optimization target. The following serial comparisons use the same fixed time
window and source selection within each pair:

| Expression shape | Time | Fetched sample entries | Workspace peak |
|---|---:|---:|---:|
| Application CPU SUM by application, one host, 30-second steps | 2.232 s | 4,153,388 | 798,642 B |
| Same SUM followed by `/ 100` | 3.601 s | 8,159,104 | 700,837 B |
| Grouped `max(timestamp(cpu))`, fleet, 120-second steps | 1.687 s | 2,667,010 | 1,716,924 B |
| `time() -` the same grouped maximum | 12.849 s | 24,160,270 | 1,720,444 B |

At 30-second steps, the grouped timestamp maximum completes in 5.035 seconds, while the enclosing freshness subtraction
hits 25 seconds. Its interrupted execution has already fetched 52,543,568 sample entries. The workspace difference in
the completed 120-second comparison is only **3,520 bytes**; that memory delta does not explain the extra work.

The application CPU-share ratio also times out in the instant fleet cohort and errors in the two-node Grafana dashboard.
It combines grouped application totals with a fleet total through `on() group_left`. It should be part of the next
optimization cohort alongside scalar arithmetic and freshness subtraction; fixing only one spelling would miss this
common operational shape.

Source analysis identifies mechanisms consistent with the measurements:

- Generic per-timestamp preparation creates a new evaluator machine in
  [execution.rs](../../../src/crates/promql-engine/src/execution.rs#L4015).
- Ordinary selector evaluation starts from the query range's first lookup in
  [evaluator.rs](../../../src/crates/promql-engine/src/evaluator.rs#L1826), and
  [instant_select.rs](../../../src/crates/promql-engine/src/instant_select.rs#L140) advances through that prefix again.
- Contributor traversal admits float pointwise calls but excludes `timestamp()` in
  [execution.rs](../../../src/crates/promql-engine/src/execution.rs#L1337). A surrounding wrapper can therefore lose
  reuse available to a direct aggregate. The measurements isolate the expensive wrapper; they do not assign every
  fetched entry to one internal mechanism.

**Recommendation, long-term-best:** extend reuse through common aggregate projections and joins while preserving
arithmetic order, labels, timestamps, errors, paged input and bounded live state. Reuse computed results/cursor progress;
do not solve this by retaining the raw query dataset. The implementation design and qualification belong to the next
optimization stage.

### 2. Make temporal subqueries and longer histories practical

The two-window CPU condition and both 30-minute CPU-percentile examples time out even as instant fleet queries. Almost
all selected 6-hour and 24-hour histories time out, and all eight selected seven-day histories do. Successful short-range
queries therefore cannot establish acceptable long-range behavior.

Keep the same expressions and measured resolutions when optimizing. Coarsening a chart, shortening its window or
substituting another expression may make it render, but does not fix the measured workload. Focused scope comparisons
show why single-host qualification is insufficient:

| Application CPU cores by application, 15-minute range | Median time | Fetched entries per repetition |
|---|---:|---:|
| One host | 4.725 s | 9,571,626 |
| Two hosts | 11.942 s | 25,439,916 |
| Fleet | 25-second deadline | 59,267,405 / 59,932,292 before interruption |

These are separate cohorts from the controlled arithmetic pair above and must not be combined into a before/after
optimization claim. The corresponding workspace peaks are 788,649 B, 1,967,179 B and 13,836,808 B.

### 3. Close the live histogram representation gap

The inspected Prometheus/OTel collector histogram contexts expose disjoint range buckets without `le`. Copying their
bounds into `le` does not create cumulative classic buckets. The live suite therefore exercises honest bucket distributions
and histogram count rates, while marking classic cumulative-histogram projection **unqualified**. Native histogram
storage is also absent from the float-only live adapter.

This is an integration gap, distinct from the Rust evaluator's controlled histogram conformance tests. Classic histogram
queries are production-relevant and should remain on the completion plan. The representation/projection design needs its
own decision; this milestone does not invent new storage, collection or metadata ownership behavior.

**Decision 71:** changes to histogram bucket boundaries retain Netdata's existing undefined historical behavior.
Repairing that limitation is explicitly outside this PromQL work: no schema-versioned chart identities or metadata
history will be introduced for it. This accepted limitation does not qualify the missing cumulative-bucket projection
or native-histogram ingestion.

## Browser and metadata checks

- Two-node Fleet Bandwidth: all 14 panels return nonempty results through Grafana. Its actual 30-second step and rendered
  31-second rate window agree with the runner's one-second datasource scrape-interval assumption.
- Two-node Applications Across Nodes: 13 panels return data; the application CPU-share panel returns Grafana HTTP 400
  with datasource status 502. The browser error message was not retained, so the exact browser failure cause is not
  asserted. The same expression independently exceeds the direct-query deadline.
- Fleet Service Instances: all 12 requests succeed; the failed-state-duration panel is empty at the checked time.
- Two-node Alert Expression Workloads: all 12 requests succeed; empty condition results remain visible as no data.
  Historical legend values are explicitly not presented as current alert state.
- Metadata requests for metric names, CPU-host values and application-group values complete in 0.22–0.26 seconds,
  returning 4,958 aliases, 18 host names and 309 groups. This does not qualify every unfiltered metadata scan.

Browser testing exposed overly long automatic legends. The generated dashboards now show readable host/application/
instance labels and named totals, while retaining full identity in query results. Automatic refresh is disabled on the
expanded dashboards so slow broad queries do not repeatedly overlap.

## Corrections and validation

- Fixed a packet-ratio denominator that counted multicast in addition to received packets. A controlled numerical
  fixture gives 0.1 for 15 drops / (100 received + 50 sent), with 25 multicast already included in received.
- Fixed historical-comparison construction that accidentally inserted `offset 1h` inside the host-variable matcher.
  Two numerical fixture checks now produce the expected 30-percentage-point change; live offset and explicit-past
  selectors agree, and the two corrected range expressions return data.
- All 136 expressions pass parser/type checks on an empty fixture. This is a syntax check, not function coverage.
  Six catalogue/interpolation/numerical tests pass, including the two input-driven regressions above.
- The HTTP extension passes ordinary/true/all/deprecated statistics, empty-result, parse-error, timeout, execution-error,
  gzip and concurrent-use checks, with existing ownership checks passing. The unchanged output-record fixture passes
  29 cases. Independent implementation review found no production defect in the statistics extension.
- Evaluation memory categories are separate: regex dependency memory is estimated, borrowed/shared-cache observations
  are references, and category peaks need not coincide. Neither their sum nor process RSS is exact per-query RAM.
  Fetched entries, returned sample batches and DBengine page encounters are not physical disk-read counts.
- This work adds measurements and representative workloads. It does not claim a new engine speedup, full current-SRE
  configuration qualification, native histogram storage, rule scheduling or distributed Cloud execution.
