# Live PromQL integration checks

For a hands-on installation and real query examples, use [Install and try the PromQL branch](GETTING_STARTED.md).

The runners use private directories under the repository's ignored `.tmp/` and preserve storage files, process
identities and logs. They install nothing and change no existing service. Use an existing Debug Agent with
DBengine and PromQL enabled and `NETDATA_INTERNAL_CHECKS` explicitly defined.

| Runner | Exercised path |
|---|---|
| `storage.py --binary BUILD/netdata` | Five-tier storage, native provider/API/dispatcher and controlled delivery |
| `storage.py --binary BUILD/netdata --rtc-peers` | The same checks plus actual loopback WebRTC peers |
| `live.py --binary BUILD/netdata` | Ordinary HTTP server over real collected DBengine data |
| `backing.py` | Native backing I/O with sparse offsets above 2 GiB |
| `checkpoints.py` | Exact bounded checkpoint ownership, shared backing and controlled I/O failures |

Both storage codecs run by default; `--codecs array` or `--codecs gorilla` selects one. Actual peer qualification
also requires the pinned libdatachannel build. Each runner requires its exact closed case roster, rejects duplicate
or missing cases and nonzero errors, and binds the source cohort and executable. Fixture timeouts enable no product
query cap.

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

## Storage and native components

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

Ordinary HTTP requests check fleet/host scope, exact/underscore names, gauge sums, counter resets/rates, form
precedence, discovery, plain/gzip 100,000-point ranges, syntax errors and explicit timeouts. Native initial-load
checks use private SQLite preparation and partial-scan failures, successful creator ownership and concurrent
query references. A failed initial attempt returns the specific unavailable error and is not replayed. Actual API waiters also cover cancellation while UNLOADED or
LOADING, completion of the one original attempt, and abandonment of a queued load.

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
