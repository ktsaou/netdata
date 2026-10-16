# Live PromQL integration checks

For a hands-on installation and real query examples, use [Install and try the PromQL branch](GETTING_STARTED.md).

The runners use private directories under the repository's ignored `.tmp/` and preserve storage files, process
identities and logs. They install nothing and change no existing service. Use an existing Agent with DBengine,
PromQL and `NETDATA_INTERNAL_CHECKS` explicitly enabled. Debug is useful for correctness checks; performance
measurements require an optimized Release build. The HTTP runners also use a private runtime directory and
command pipe, filter inherited `NETDATA_*` settings, and retire only their verified instance through `/exit`.

| Runner | Exercised path |
|---|---|
| `storage.py --binary BUILD/netdata` | Five-tier storage, native provider/API/dispatcher and controlled delivery |
| `storage.py --binary BUILD/netdata --rtc-peers` | The same checks plus actual loopback WebRTC peers |
| `live.py --binary BUILD/netdata` | Ordinary HTTP server over real collected DBengine data |
| `performance.py --binary BUILD/netdata` | Fixed warm HTTP/DBengine latency, two-client completion and Linux process memory |
| `backing.py` | Native backing I/O with sparse offsets above 2 GiB |
| `checkpoints.py` | Exact bounded checkpoint ownership, shared backing and controlled I/O failures |

Both storage codecs run by default; `--codecs array` or `--codecs gorilla` selects one. Actual peer qualification
also requires the pinned libdatachannel build. Each runner requires its exact closed case roster, rejects duplicate
or missing cases and nonzero errors, and binds the source cohort and executable. Fixture timeouts enable no product
query cap.

## Optimized live API measurements

The Linux measurement runner reuses `live.py` startup, instance identity, all 29 HTTP checks, 12 host-readiness
checks and graceful shutdown for each codec. Its fixed panel covers scalar, fleet gauge, local counter rate,
fleet average and six-group histogram expressions. Separate plain/gzip 100,000-point `vector(1)` controls
exercise evaluation and delivery with zero DBengine samples. Every measured result is checked completely.

Use the Release options from the [installation guide](GETTING_STARTED.md#build-and-install-a-private-agent),
plus explicit internal checks in both compiler flags. A build-only invocation is:

```bash
cmake -S . -B .tmp/promql-live-release -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DDEFAULT_FEATURE_STATE=Off \
  -DENABLE_PLUGIN_SCRIPTS=Off -DENABLE_DASHBOARD=Off \
  -DENABLE_DBENGINE=On -DENABLE_PROMQL=On -DENABLE_BUNDLED_PROTOBUF=Off \
  -DCMAKE_C_FLAGS=-DNETDATA_INTERNAL_CHECKS=1 \
  -DCMAKE_CXX_FLAGS=-DNETDATA_INTERNAL_CHECKS=1
cmake --build .tmp/promql-live-release --parallel 2 --target netdata
python3 tests/promql-integration/performance.py --binary .tmp/promql-live-release/netdata
```

Each query gets one warmup, seven serial samples and four two-client batches. The receipt retains all timings,
headers, byte counts, complete compressed response artifacts, existing `stats=all` fields and owned-process
RSS/lifetime high-water readings. Request duration starts before connecting and ends after reading the final
wire bytes; client gzip decoding, JSON checks and evidence compression are excluded. Batch spans and overlaps
come from those same wire timestamps. Client overlap does not prove simultaneous Rust evaluation.
The six server timing fields are captured before final output commit, so they omit its completed JSON assembly
and compression. Use wire duration for the complete client-visible request.

This is a small warm integration baseline: two hosts, one gauge and one incremental counter each, fixed-clock
collection, one storage tier and 16 MiB page/extent caches. It is not a cold-disk or high-cardinality throughput
measurement. Internal checks remain enabled in the optimized binary. HTTP sample statistics are logical counts,
not fetched pages or allocation bytes. RSS covers the whole Agent; its high-water also includes startup and the
earlier HTTP qualification. It excludes kernel file-cache memory and temporary-file size. These observations do
not isolate Rust memory or establish IoT/large-parent sizing.

The [recorded Release baseline](live-release-qualification.json) retains the earlier binary and results. The
[direct set qualification](../../src/crates/promql-engine/tests/promql-conformance/instant-set-qualification.json)
binds its Rust engine and containing Release build, with 82 HTTP/readiness checks and
224 complete measured responses. Its histogram-replay measurements are retained below; the later C output
measurements follow under [output record reading](#output-record-reading). Seven-sample median wire times were:

| Query | Raw before → current (ms) | Gorilla before → current (ms) |
|---|---:|---:|
| Scalar | 0.352 → 0.341 | 0.341 → 0.351 |
| Fleet gauge | 0.992 → 0.657 | 0.851 → 0.783 |
| Local counter rate | 1.127 → 0.785 | 0.846 → 0.666 |
| Fleet average | 1.517 → 0.805 | 0.994 → 0.817 |
| Six-group histogram | 2434.536 → 18.388 | 2344.758 → 19.054 |
| 100,000-point delivery, plain | 235.925 → 237.216 | 226.310 → 234.433 |
| 100,000-point delivery, gzip | 261.891 → 247.216 | 240.830 → 247.050 |

The histogram expression improves about 123–132 times in this fixture. A matched generic-engine panel independently
removes the repeated discovery: the identical expression over two stored points fetches 99,952→390 entries.
Its simplified labels/history differ from the live provider, so the two panels do not measure a C/provider cost
ratio. Generic tracked workspace rises 336 bytes for this expression; some tiny controls use additional metadata
and have slower medians. Whole-Agent RSS remains a separate observation. The expression still accounts 12 logical
inputs; its live sample-unit peak changes 7→5, which is not a RAM-byte measurement.

The live before/current cohorts ran at different times on a shared host. Do not attribute every control's timing
change to this set optimization or infer maximum throughput. All serial outliers, two-client spans and whole-process
readings remain retained. The earlier baseline also preserves the caller's corrected `data.stats` parsing failure.
Both Agents in that cohort exit normally; shutdown response reads time out while native cleanup finishes.
That cohort changed private Rust replay behavior; storage, provider, API and resource-limit contracts remain.

## Output record reading

Final C response assembly reuses its existing 8 KiB workspace when one series has a single sample kind.
Those point records are physically consecutive. Multiple-series and mixed float/histogram results retain
exact-sized reads. Reader metadata is local to one chain, so sorting, labels and annotations can reuse the same
scratch buffer. Large points consume the cached prefix before refilling. This adds no heap buffer or retained
per-series field; generic body/backing and transport behavior are unchanged.

[Output record qualification](output-record-qualification.json) retains the original, rejected prototype and final
source/binary identities. The original 100,000-point chain makes 199,680 file reads; the final reader makes 390,
with the same 3,194,878 bytes read. Seven alternating measured pairs after one warmup pair give median isolated
assembly times of 118.474 → 4.181 ms. The probe uses controlled allocation/polling; it does not measure actual
Agent accounting or end-to-end latency. The first prototype over-read sparse, bursty and large histogram records;
the final ownership guard and partial-window consumption pass the corresponding byte bounds.

The fixed live API panel uses the same Rust archive in the before and current optimized Agents. Both codecs
pass all 82 HTTP/readiness checks and 224 complete measured responses, with normal zero exits. Seven-sample
median wire times from the two cohorts are:

| Query | Raw before → current (ms) | Gorilla before → current (ms) |
|---|---:|---:|
| Scalar | 0.341 → 0.348 | 0.351 → 0.350 |
| Fleet gauge | 0.657 → 0.796 | 0.783 → 0.822 |
| Local counter rate | 0.785 → 0.909 | 0.666 → 0.823 |
| Fleet average | 0.805 → 1.053 | 0.817 → 0.829 |
| Six-group histogram | 18.388 → 17.762 | 19.054 → 18.102 |
| 100,000-point delivery, plain | 237.216 → 118.003 | 234.433 → 117.476 |
| 100,000-point delivery, gzip | 247.216 → 134.889 | 247.050 → 128.644 |

Plain delivery improves about 50%; gzip improves about 45–48% in this fixture. Live cohorts ran at different
times on a shared host; small-control changes cannot all be attributed to this reader. Seven of the 16 paired
normal probe controls have slower medians: the largest relative increase is 11.1% (180 → 200 ns), and the largest
absolute increase is about 6 µs for mixed chains. Probe executable text grows 480 bytes. The measured containing
Agent adds 2,052 text bytes and 32 data bytes. All costs and raw samples remain in the report; these results do not
establish saturation throughput, cold-disk or high-cardinality scaling.

Both actual Agent builds also pass 672 storage/provider/API/delivery records each. All 16 output records, including
their real Result/Workspace peaks, match exactly between builds. Plain 2,049-point and 100,000-point results each
peak at 20,212 Result bytes and 14,592 Workspace bytes. Gzip has additional bounded codec allocations. These are
query-account categories; they are not total process or system memory.

Run the focused Linux/GNU-linker probe from the repository root:

```sh
python3 tests/promql-integration/output_records.py
python3 tests/promql-integration/output_records.py --cc arm-linux-gnueabihf-gcc-14 --runner 'qemu-arm -L /usr/arm-linux-gnueabihf'
python3 tests/promql-integration/output_records.py --cc 'gcc -fsanitize=address,undefined -fno-omit-frame-pointer'
```

It compiles the actual output/body/backing code, checks 29 complete cases and retains source copies, commands,
PIDs, statuses and counters in a new private scratch directory. Coverage includes inline/spill boundaries,
interleaved and mixed chains, a point over 8 KiB, sparse offsets above 2 GiB, short/EINTR/error I/O, cancellation
on cached data, bounded read counts/bytes and cleanup. Native, ARM32, ARM64 and native ASan/UBSan pass.
The allocation/control shim does not qualify daemon accounts, numerical formatting, gzip or transports; the
separate real Agent panel does. `--output-source` accepts an in-repository captured output.c for before/current
comparison. `--allow-legacy-reads` only disables the old large-case read-count bound; other checks remain active.

## Caller checks

The launch, lifecycle and measurement-envelope checks run without starting an Agent:

```bash
python3 -m unittest discover -s tests/promql-integration -p 'test_*.py' -v
```

They retain their private fixture directories under `.tmp/`, like the integration runners.

`backing.py` compiles the exact production I/O owner with a small allocation/control shim. It checks sparse
3 GiB sizing, bounded reads and writes across the 2 GiB boundary, and offset overflow. Use `--cc COMPILER`
and `--runner 'EMULATOR OPTIONS'` for an installed cross compiler and emulator. Its receipts do not qualify
daemon memory accounting.

`checkpoints.py` compiles the actual checkpoint and backing owners with allocation/control and I/O wrappers.
Its closed cases cover exact NaN/zero/integer bits, interleaved metrics, 65,537 checkpoints, before/exact-first
and last fast paths, failed allocation/reservation/copy/write, interrupted publication, short I/O and sparse
offsets above 3 GiB. It accepts the same `--cc` and `--runner` options. These are module/shim observations;
the storage runner below measures the real daemon's query-owned allocation handles separately.

## Counter checkpoint memory

Each physical metric retains a short 16-record prefix and exact first/last records. Longer histories use one
query-owned anonymous file, shared across metrics and aliases, and a lazy copy window capped at 8 KiB. Successful
growth reserves a whole region before copying/appending and publishes after its final control check. Failed growth
leaves the earlier records valid; unreachable regions remain until query cleanup. No raw history is held in RAM.

The real provider fixture forces four samples per DBengine page, then compares complete native/legacy counter sums
and release accounting at 16/17/33/129/513/2,049 pages. In the recorded array baseline and both current codecs,
tracked provider peak at 2,049 pages falls from 135,097 to 45,097 bytes. A 16-page query grows from 37,017 to
37,081 bytes. These are source-bound fixture observations, not whole-process RSS or general throughput guarantees.
Checkpoint state is bounded in history length; metric metadata and reducer state still grow with cardinality.

The runner also exercises real forward/backward/ended seeks, alias-shared basis, two metrics sharing one file,
failed-open reference/account balance, sticky failed seek/next, reset-bearing gaps, and negative/absolute legacy
views after spill. Its callback cases use the existing output callback's borrowed account and close all descendants
before returning. Short no-backing queries succeed; the longer counter case returns a specific provider error.
Large counter histories can therefore require writable backing even when their output is tiny. Checkpoint disk
traffic remains separate from Rust temporary-float-store observations.
The [checkpoint qualification](checkpoint-qualification.json) binds the source, executable, complete storage/live
receipts and actual module execution on AMD64/ARM64/ARM32. Existing Rust expression qualification retains its
unchanged source/binary identity; these C-only changes do not reattribute or rerun the full expression corpus.

Natural DBengine page boundaries depend on chart alignment, including its process-local pointer hash. A probe can
fetch a whole page before a replay seeks, so fetched-sample totals may vary across processes with identical inputs.
The retained quantile case has identical outputs and 410 page callbacks but different probe overfetch. Use the
forced four-sample pages for stable checkpoint read-count comparisons; retain natural-page counts as observations.

The Rust adapter now accesses a validated borrowed C page without a second sample-descriptor array. Its
[owned qualification](../../src/crates/promql-engine/tests/promql-conformance/borrowed-page-qualification.json)
records exact payload/lifetime/control checks, standalone target execution and matched fixture memory/time.
New storage and live API cohorts qualify the complete containing Agent separately; standalone foreign targets
remain engine/caller evidence. The Netdata metadata, page formats and checkpoint backing owners are unchanged.

The [completed join qualification](../../src/crates/promql-engine/tests/promql-conformance/binary-replay-qualification.json)
separately binds the evaluator change and a rebuilt Agent's storage/API checks. Actual C caller cases cover a
later operand error before a second output error, selected-pair read failure after reuse, and complete resource
release. Their controlled fixture callbacks supplement the live storage and HTTP checks.

The [instant-fold qualification](../../src/crates/promql-engine/tests/promql-conformance/instant-fold-qualification.json)
binds reuse of completed instant numeric range results to both optimized profiles and fresh storage/API cohorts.
Actual C callers retain final initialization errors after the first output. Matched quantile fixtures roughly
halve sample reads with unchanged tracked workspace peaks; these observations are separate from live disk throughput.

## Storage and native components

The [range-head qualification](../../src/crates/promql-engine/tests/promql-conformance/range-head-qualification.json)
binds the removal of redundant per-series preparation scans to fresh containing storage/API checks. Generated
512-series numeric ranges use about 43% less recorded work and run about 1.4× faster, with unchanged outputs/reads
and a 96-byte workspace reduction. These are synthetic query measurements; storage and provider contracts are unchanged.

The [range uniqueness qualification](../../src/crates/promql-engine/tests/promql-conformance/range-uniqueness-qualification.json)
binds indexed preparation duplicate checks to fresh containing storage/API cohorts. Generated 512-series instant
range reductions use about 25% less tracked work, run about 1.1× faster and reduce peak workspace by 2 KiB,
with unchanged results and reads. The measurements cover synthetic queries; raw paging and provider contracts remain unchanged.

The [projected-label qualification](../../src/crates/promql-engine/tests/promql-conformance/projected-label-qualification.json)
binds compact join-label construction to fresh containing storage/API cohorts. Generated label-heavy joins use
about 21% less tracked query workspace with unchanged outputs and reads. Contexts references, provider metadata,
storage readers and raw paging retain their existing ownership; synthetic timings show no consistent speed gain.

The [rate-join qualification](../../src/crates/promql-engine/tests/promql-conformance/rate-join-qualification.json)
binds completed direct rate-operand discovery to the optimized engine and a freshly rebuilt Agent's storage/API
checks. Generated joins reduce repeated metadata work with unchanged fetched samples and peak query workspace.
Actual C callers preserve first-pass failure priority, cancellation, output failure and cleanup. Synthetic timing
improvements are separate from live DBengine throughput.

The fixture uses real storage writers and demand-reader APIs at all five tiers. Separate write, cold-read and append
processes exercise both tier0 codec families, counter-page anchors, coarse32-bit counts, positive counter storage
with negative presentation, reset-bearing gaps, all-gap pages, forced page boundaries, cadence changes,
ordinary/counter format transitions and rejected repeated writes. Every decoded point is compared with the
fixture's timestamp, format, calculated statistics, flags and independently calculated counter endpoint.

The cold-read process also checks twelve reader cases: warm/cold success, cancellation before opening and while
decoding, refused workspace/page/view accounting, worker cancellation, and a pinned failed-cache collision that
requires a real private page. Errors retain their first cause. Every case waits for its owned libuv work to retire
and requires zero remaining allocation handles. These artificial refusals enable no query limit.

Twenty-five collection checks per codec use real chart/dimension owners. They cover mixed gauge/counter buckets,
saved provenance after an algorithm change, partial/all-gap reset metadata, varied negative extrema at all four
higher tiers, raw incremental collection with a reset and cadence change, and legacy fine/coarse backfill.
Ordinary tier0 assertions use its decoded retained value, including existing codec rounding.

The cold-read process also exercises the live contexts bridge, alias/origin sharing, tier seams and replay,
discovery bounds, original metadata pointer reuse through field replacement, archived label loading/cache reuse,
form parsing, native handlers and ordinary API dispatch. Output checks cover bounded plain/gzip backing, refused
backing, native send windows and immediate pooled-client release. Controlled MQTT/ACLK sources cover short writes,
packet ceilings, acknowledgement/discard ownership and errors; WebRTC frame checks cover UTF-8, negotiated sizes
and stopped/send-failure cases. Operation checks cover socket retirement, copied IDs, descriptor reuse, deadline
precedence, watcher retirement and shutdown.

## Actual loopback WebRTC peers

With `--rtc-peers`, the fixture creates one SDK client and uses Netdata's real answerer and channel worker.
Both peers bind loopback with no external ICE server. Six cases check connection, live gauge, full HTTP form POST,
a 1,000,000-point range, remote close during sending and shutdown.

The receiver compares the 17,000,087-byte range body using fixed windows, validates consecutive frame headers
and UTF-8, and pauses a callback to observe native SDK buffering. Frames must stay below the negotiated 2,049-byte
size. Close/shutdown require retired owners, zero active operations and released query allocations.

## Actual HTTP server

`live.py` starts an isolated foreground Agent with a kernel-selected loopback HTTP port, private configuration,
claim state, caches and logs. Unrelated collection, streaming, ML, health and RTC activity is disabled. Two real
hosts are seeded with gauge and incremental dimensions through Netdata's fixed-clock collection path.

Ordinary HTTP requests check fleet/host scope, exact/underscore names, gauge sums, instant/range averages,
counter resets/rates, form precedence, discovery, plain/gzip 100,000-point ranges, syntax errors and explicit
timeouts. Native initial-load
checks use private SQLite preparation and partial-scan failures, successful creator ownership and concurrent
query references. A failed initial attempt returns the specific unavailable error and is not replayed. Actual API waiters also cover cancellation while UNLOADED or
LOADING, completion of the one original attempt, and abandonment of a queued load.

The average cases query constant local/child gauges collected into real DBengine pages. They check three scoped
instant averages and a fleet range average with exact HTTP results for both page codecs. The current average-engine
cohort records 66 combined HTTP/readiness checks and 672 storage records in
[the library qualification](../../src/crates/promql-engine/tests/promql-conformance/average-read-reuse-qualification.json).

The histogram constructor cohort adds three scoped instant quantiles and a fleet range quantile. It uses
`label_replace` to derive `le="1"` and `le="+Inf"` buckets from the same collected constant gauges. Each group's
quantile is exactly 0.5 and their fleet sum is 1. Both codecs pass all 74 combined HTTP/readiness checks and
672 storage records on the freshly rebuilt Agent, recorded in
[histogram key qualification](../../src/crates/promql-engine/tests/promql-conformance/histogram-key-qualification.json).

The runner requests the fixture Agent's native `/exit` endpoint and waits for that exact child to exit normally.
It sends no signal to a running Agent or unrelated process; its PID, command, exit status and logs are retained.

## Evidence and limits

Receipts bind the actual executable and the source files used by each check. Storage receipts also record the
Netdata revision and include the contained Rust engine in their source hashes. Historical receipts retain their
original separate-library revision and source hashes. Allocation callbacks measure owned workspace, private pages
and borrowed cache/pool exposure separately and must balance after owned work retires.
They are not measurements of total process RSS.

Provider fixture `cursor_points` and `provider_folds` count cursor reads and provider reconstruction only.
Native page-prefix decoding is excluded; these diagnostics do not measure total physical work.

The library's separate conformance corpus establishes same-input expression behavior. These fixtures qualify
the retained-data bridge and the stated API/delivery paths. They do not prove original scrape timing recovery,
general collector behavior, a live Cloud connection, distributed Cloud execution or an alert-rule engine.
Controlled ACLK/MQTT cases must be reported separately from actual HTTP and loopback peer checks.

The active-distribution cohort retains 672 storage records and expands live HTTP/readiness checks to 82.
Four added expressions per codec derive `1`/`+Inf` bucket labels from the already collected gauges and project
three distinct groups per host. Fleet queries exercise six histogram groups and return a quantile sum of `3`;
local/child queries return `1.5`, and the three-tick fleet range returns `3` at every tick. This exercises the
indexed Rust path through the existing API, contexts and DBengine without changing collectors or stored metrics.

At the active-distribution source, that deeper range expression exceeded the ordinary 30s test-client allowance
with both compared Debug Agents. Those partial runs remain separate performance evidence. That cohort's final
functional checks used 180s for the one range request and passed under both codecs. The historical allowance
changed only the test client; it was not a production query-timeout change or a throughput fix.

The subsequent completed set-input discovery route retains one current-scope active bitmap and preserves the
original first discovery, operand rereads and raw paging. Its live fixture retires the exceptional wait and again
uses 30s for every request, including the unchanged exact six-group range. Refer to the contained engine's
conformance guide for source-bound qualification and measured resource costs.

The source-bound set-discovery cohort passes 672 storage records and 82 live HTTP/readiness checks with normal
owned Agent exits. Its exact six-group range no longer needs the exceptional client allowance. Separate matched
Rust engine measurements show approximately 13× faster deep-query evaluation, adding 1,240 bytes of tracked
workspace. This is finite generated-data timing, not a production disk or process-RSS measurement.

The subsequent completed-prefix route omits previously successful nonselected set operand acquisitions while
retaining fresh requested/unverified reads, first discovery, matching and cleanup. A freshly linked Debug Agent
again passes 672 storage records and 82 live HTTP/readiness checks under both codecs, using ordinary 30s client
reads and normal exits. The source-bound engine cohort measures the same deep query about 16× faster than the
preceding discovery-only build, with unchanged 189,077-byte tracked workspace; these remain synthetic Rust engine
measurements. See the contained conformance guide's completed set-operand prefix section for exact sources,
small-query tradeoffs and the current serial same-Plan ownership requirement.

The subsequent canonical-hash route reuses the already computed descriptor hash for verified skipped operands.
Requested and unverified values retain their acquisition, label validation and hashing. Its freshly linked Debug
Agent passes the same 672 storage records and 82 HTTP/readiness checks with both codecs and ordinary owned exits.
Separate engine measurements preserve every observed read and workspace peak while reducing metadata work; the
contained conformance guide records complete outcomes, small-query costs and shared-host timing limits.

The subsequent sparse-membership route enumerates the existing set bitmap instead of scanning inactive descriptors.
Its freshly linked Debug Agent passes 672 storage records and 82 HTTP/readiness checks under both codecs, using
ordinary reads and normal exits. Separate complete engine queries preserve measured reads and workspace peaks;
sparse cases use roughly 8–10% less work. The contained conformance guide records dense/tiny tradeoffs and the
finite source-bound measurements. Storage, contexts metadata, provider ownership and API contracts are unchanged.

The subsequent copied-grouping route constructs only selected grouping fields while retaining contexts/provider
ownership and full input/ranking labels. Its freshly linked Debug Agent passes 672 storage records and 82
HTTP/readiness checks under both codecs, using ordinary client reads and normal owned exits. Separate matched
Rust engine queries reduce quantile/nested-group peak workspace and copied allocation volume. Raw transfer-path
and top-k peaks remain unchanged. The contained conformance guide records original slower timings and their
separate follow-up; these are finite generated-data measurements, not production disk or process-RSS claims.

The subsequent sort-publication route replaces large pairwise duplicate-hash checks with a temporary position
lookup after the original sorting callback completes. A freshly linked Debug Agent passes 672 storage records
and 82 HTTP/readiness checks under both codecs, with ordinary client reads and normal owned exits. The contained
conformance guide records exact order/error/cache/control obligations and the allocation/timing tradeoff. Storage,
contexts ownership, API contracts and paged raw data remain unchanged; these checks do not measure live disk throughput.

The subsequent completed-sort-delivery route reuses only an exact snapshot's successful full operand delivery.
Eligible bare direct-float range queries still acquire fresh selected values; instant, nested/computed and
uncertified shapes keep their ordinary reads. A freshly rebuilt/relinked Debug Agent passes the canonical
672 storage records and 82 HTTP/readiness checks with both codecs and normal owned exits. A separately bound,
private additive HTTP helper passes 88 checks/readiness, including complete two-host `sort()` and `sort_desc()`
range results compared with the same Agent's raw DBengine query. It leaves the canonical helper unchanged.
See the contained conformance guide's completed vector-sort emission section for source-bound memory and timing
tradeoffs. These generated-data checks preserve contexts, raw paging and API contracts; they do not establish
production disk throughput or process RSS.
