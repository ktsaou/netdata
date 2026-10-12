# PromQL design decisions

The approved expression-engine decisions moved with the canonical Rust implementation to
[the pinned library design document](https://github.com/netdata-labs/promql-engine/blob/93f9c2c37b080efa9341431c237d2003ace4c2e4/tests/promql-conformance/DESIGN.md).
The complete original document and qualification identities remain in the library's imported history.

Netdata owns contexts metadata, dbengine page formats and retained-data projection, temporary backing,
HTTP and transport integration. The [operator guide](../../src/web/api/promql/README.md) describes the public API.
Generic expression qualification and Agent integration qualification establish different contracts.
Alertmanager remains outside this work.

## Source and build ownership

- The independent engine is consumed through the pinned `src/crates/promql-engine` Git submodule. Its C header,
  Cargo lock, notices and generic conformance harness travel with that source authority.
- Netdata supplies the typed storage and output callbacks. ABI 4 tables reject mismatched versions and layouts.
- PromQL is enabled by default when DBengine is enabled. Custom CMake or installer builds can explicitly opt out.
- Rust 1.91 is the minimum. Cargo builds use the lockfile and an independent unwind profile, including MinSizeRel.
  Ordinary Rust panics are contained at the C boundary; an actual allocator OOM can remain process-fatal.
- The pinned engine targets the retained Go AMD64/FMA-on arithmetic route on all supported devices. The library
  design records numerical tolerance, deterministic ordering and explicit upstream-bug exceptions.

## Metadata belongs to contexts

- DBengine stores time series by UUID. The existing host/context/instance/metric hierarchy supplies metadata.
  There is no second persistent PromQL metadata catalogue or independent SQL-label cache.
- Native context, instance and metric handles remain acquired while selected. Host lifetime guards protect
  query owners from removal. Queries wait for the existing initial-load attempt, with cancellation checks;
  an unavailable initial load returns an error rather than a successful empty catalogue.
- Archived labels use the existing on-demand contexts loader. Successful loads remain cached in the instance.
- Immutable `STRING` references retain original names and label bytes. `string_dup()` increments a reference
  count and returns the same pointer; it does not copy those bytes. Individual string references are necessary
  because a retained container can replace its fields during collection.
- Generated underscore aliases and escaped label keys require owned bytes. Per-query descriptors, reference
  arrays and selected-series indexes are accounted. Independently owned results can contain transformed labels.
- Each descriptor retains its selected metadata and tier spans for query replay. This is neither historical
  label storage nor an atomic snapshot of all hosts and concurrently changing sample pages.

## Series naming and units

- Every dimension becomes a series named by its exact context and, where different, its underscore alias.
  There is no metric-name prefix. Both aliases have distinct logical identities and share one physical reader.
- Eight built-in `netdata_*` labels identify host, context, chart, dimension and units. Host labels use
  `netdata_host_label_`. Chart keys beginning with `netdata_`, and `__name__`, use `netdata_chart_label_`.
- Values retain Netdata's native unit magnitude. KiB remains KiB; counter integration converts retained rates
  back to their corresponding cumulative unit. Gauges retain their decoded stored values.
- The live provider emits float samples. Language support for native histograms in the engine does not change
  Netdata's storage model.

## Counter pages and migration

- Only counters use the new page formats. Ordinary gauge and legacy page encodings remain readable.
- Each counter page at every tier has one binary64 anchor before its first retained contribution: 8 bytes per
  page, with no per-point absolute counter field. Tier0 keeps its existing compact sample codec.
- Higher-tier counter records are 24 bytes: three float32 statistics, valid/anomalous u32 counts and one u32
  flags word. Reset and gap-presence flags survive aggregation. Unknown legacy provenance stays explicitly unknown.
- Positive counter amounts are stored positive; visual negation is a presentation flag. Existing interpolation,
  collection-side gap decisions, streaming and replay protocols are unchanged.
- Storage reads return the retained calculated view and reconstructed cumulative view together. Missing records
  remain gaps. For a reset-bearing contribution, reset the cumulative value before adding that contribution.
- Higher-tier reconstruction integrates the retained mean over the record interval. Coarse buckets lose the
  exact order of resets, gap positions and original timing; this is an approved approximation.
- Ordinary historical incremental data has no anchor. Its cumulative reconstruction starts synthetically and
  uses surviving reset flags. The `absolute` query option normalizes legacy negative counter inputs before
  reconstruction; it does not rewrite gauges or expression results. Already lost coarse signed cancellation
  cannot be recovered.
- The required query history establishes one starting basis per host/UUID, shared by selectors, aliases and
  replay. A legacy start remains legacy. An anchored start carries its accumulator across later pages and tiers;
  later anchors never replace it. Only retained resets restart the accumulator.
- A metric that starts in ordinary gauge/mixed data uses the legacy basis throughout that query. Algorithm
  transitions do not introduce duplicate records at the same timestamp.
- Queries with different required histories can reconstruct different levels at the same timestamp. Expression
  compatibility on identical samples does not promise recovery of original Prometheus scrape histories.

## Paged reads, accounting and backing

- Demand cursors acquire one page at a time and decode into bounded batches. They reuse DBengine's shared page
  and extent LRUs rather than retaining all raw pages across a time range or all selected series.
- Forward evaluation reuses the current page; computed selectors retain bounded streaming state where supported.
  Other replay paths can repeat storage work, which the query observations report.
- Provider callbacks receive explicit query-owned allocation handles, including cleanup. This captures temporary
  C allocation peaks as well as retained memory. Shared borrowed views have separate observations.
- Accounting distinguishes engine-owned memory, provider workspaces, private pages, shared views, result state,
  temporary bytes and concurrent queries. Regex dependency memory is explicitly estimated. Native global caches
  and SQLite's compiler allocations are not an exact whole-process allocation census.
- Implicit resource caps are disabled pending device-adaptive policy. Explicit deadlines and cancellation remain
  active. Aggregation, matching, metadata references and result indexes can still grow with cardinality.
- Provider-managed temporary storage supports external computations and large outputs. The C adapter places
  backing under the configured cache directory or explicit PromQL directory. Unavailable/full backing returns
  an error; the engine does not silently materialize the complete dataset in RAM.

## API and response ownership

- Five v1 routes expose instant/range evaluation and labels/values/series discovery through the existing metrics
  ACL and anonymous-data permission. Base routes include retained fleet hosts; explicit host/node prefixes scope
  one host. GET and form POST are supported, except label values is GET-only.
- Original URI and body bytes are parsed independently of the dashboard query parser. Form body precedence,
  escaped separators, embedded bytes and singular/repeated fields follow the pinned API contract.
- Successful bodies, annotations and optional statistics are completed and sealed before publication. A failed
  query publishes its error envelope, rather than a partial successful body.
- Response lengths and offsets are 64-bit. Large bodies use temporary backing and fixed windows. Local HTTP
  retains bounded gzip/send state; pooling releases query-owned response memory immediately.
- ACLK sends one exact MQTT PUBLISH, retaining its bounded source through QoS1 acknowledgement or discard.
  Broker and MQTT packet ceilings return an explicit oversized-result error. This does not establish distributed
  Cloud expression execution or a live Cloud deployment qualification.
- WebRTC retains the response on a serial channel worker and sends UTF-8 PLAIN frames only when its SDK buffer
  drains. Callback wakeups carry copied identifiers; close joins workers and releases owners before SDK cleanup.
- Query operations retain owned controls and copied IDs. The one-second cancellation watcher observes socket
  retirement without retaining web-client or SSL pointers; registry retirement precedes descriptor reuse.
  Caller and explicit URI deadlines preserve the earlier deadline. Blocking native SQL is not claimed to have
  fixed cancellation latency.
- Shutdown closes admission, cancels outstanding work and drains transport/query owners before DBengine closes.

## Qualification boundaries

The [integration runners](../promql-integration/README.md) exercise real storage, collected data, contexts,
routed HTTP and loopback WebRTC independently of the library's same-input PromQL corpus. Receipts bind source,
library pin and actual binary. Controlled MQTT/ACLK tests establish bounded source ownership and framing; they
do not substitute for a live Cloud connection. Distributed Cloud execution and a separate rule evaluation engine
remain subsequent targets.
