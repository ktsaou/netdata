# PromQL API

Netdata exposes PromQL instant queries, range queries and label discovery over its retained DBengine data.
The Rust expression engine reads the existing contexts catalogue and DBengine pages through Netdata's C adapter.
Agents and Parents use the same API; a Parent can query the hosts whose data it retains.

For branch installation and example queries over collected Linux data, see
[Install and try the PromQL branch](../../../../tests/promql-integration/GETTING_STARTED.md).

## Endpoints

| Endpoint | Methods | Result |
|---|---|---|
| `/api/v1/query` | GET, form POST | Instant scalar, string, vector or matrix |
| `/api/v1/query_range` | GET, form POST | Matrix evaluated at successive steps |
| `/api/v1/labels` | GET, form POST | Label names |
| `/api/v1/label/{name}/values` | GET | Values of one label |
| `/api/v1/series` | GET, form POST | Series label sets |

These Prometheus-shaped endpoints use v1 paths. Other new Netdata API integrations should use v3.
They inherit the existing metrics access rules and anonymous-data permission, including any configured bearer
requirement. They are available when the Agent is built with PromQL and DBengine enabled.

POST bodies must use `application/x-www-form-urlencoded`. Body parameters precede URL parameters; for a singular
parameter, its first occurrence wins. Repeated `match[]` parameters are combined. URL-encode expressions and label
values so that characters such as `+`, `&` and `=` survive form parsing.

## Host scope and series identity

An unprefixed request queries all retained hosts on the contacted Agent or Parent. Netdata's existing host and node
prefixes select a single host, for example `/host/{machine_guid}/api/v1/query` or `/host/localhost/api/v1/query`.
This scope is independent of the v3 `scope_nodes` parameter.

Each dimension is a separate series. Its metric name is the exact context name, with an additional underscore
alias when the spelling differs: `system.cpu` and `system_cpu`. There is no `netdata_` metric-name prefix.
Quoted selectors can address exact context names. The two aliases share physical data but have distinct
`__name__` labels, so selecting both includes both logical series.

Every series has these labels:

| Label | Meaning |
|---|---|
| `__name__` | Exact context name or its underscore alias |
| `netdata_host_id`, `netdata_host_name` | Host identity |
| `netdata_context` | Exact context name |
| `netdata_chart_id`, `netdata_chart_name` | Chart identity |
| `netdata_dimension_id`, `netdata_dimension_name` | Dimension identity |
| `netdata_units` | Netdata's stored unit magnitude |

Host labels use `netdata_host_label_` followed by their original key. Chart labels keep their key, except keys
starting with `netdata_` and the key `__name__`, which use `netdata_chart_label_` followed by the original key.
Labels come from the existing contexts system. They are the values selected during the query, rather than
historical labels for every sample; a fleet query does not promise an atomic metadata snapshot across hosts.

## Query parameters

| Parameter | Endpoint | Meaning |
|---|---|---|
| `query` | query, query_range | Required PromQL expression |
| `time` | query | Evaluation timestamp; defaults to now |
| `start`, `end`, `step` | query_range | Required inclusive evaluation interval and positive step |
| `timeout` | query, query_range | Explicit execution deadline; an earlier caller deadline still wins |
| `lookback_delta` | query, query_range | Positive lookback override; default is 5 minutes |
| `limit` | All five | Maximum returned series or discovery entries; omitted or zero means no output limit |
| `stats` | query, query_range | Optional query statistics; `all` requests the full statistics object |
| `options=absolute` | query, query_range | Normalize legacy presentation-negated counter inputs before reconstruction |
| `match[]` | Discovery | Repeated series selectors; at least one is required by series |
| `start`, `end` | Discovery | Optional retained-time overlap filter |

Timestamps accept Unix seconds or RFC3339. Durations accept seconds or PromQL duration syntax. A range query
requires a scalar or instant-vector expression. Discovery selectors must contain at least one matcher that does
not match an empty value. `options=absolute` leaves gauges and negative expression results unchanged.
Netdata dashboard shaping options are not supported by these endpoints.

Metric-name discovery (`/api/v1/label/__name__/values`) uses the existing contexts catalogue. With no selector,
or for each `match[]` containing only `__name__` predicates, it stops at the first metric in each context that has
retained DBengine data overlapping the requested time range. Both name aliases are still matched by the Rust
engine, and the usual union, sorting and limit rules apply. This path does not load or validate unrelated chart
labels, so an unrelated archived-label loading failure does not prevent listing a name. A selector that filters
other labels uses full series metadata. There is no separate metadata catalogue or persistent discovery cache.

## Results and errors

Successful JSON uses `status: success` and `data`. Query data contains `resultType` and `result`; discovery data
is an array. Sample timestamps are numeric Unix seconds, and float values are JSON strings, including `NaN`,
`+Inf` and `-Inf`. Successful responses can include `warnings`, `infos` and requested `stats`.

Errors use `status: error`, `errorType` and `error`. Typical HTTP statuses are:

- `400`: invalid parameters or expression syntax.
- `422`: expression evaluation failure.
- `499`: canceled query or shutdown admission refusal when a response can still be returned.
- `503`: expired deadline.
- `500`: metadata, storage, temporary backing or internal failure.
- `413`: an ACLK result exceeds the MQTT packet or broker ceiling.

A response is completed before publication. An execution failure does not publish a partial successful result.
Large responses support bounded local HTTP sending and gzip, ACLK delivery and paced WebRTC PLAIN frames.
An already disconnected caller may receive no error body.

## Retained-data behavior

PromQL evaluates Netdata's stored data. Existing fixed-step interpolation, retention tiers and native units remain
in use: a metric measured in KiB remains in KiB. This can differ from evaluating original Prometheus scrape samples.
The live adapter currently supplies floating-point samples; the standalone engine's histogram language support
does not add native histogram storage to Netdata.

Counter pages contain one 8-byte counter anchor per page at every tier. Higher-tier counter records retain aggregate
statistics and reset/gap flags. Counter levels are reconstructed page by page, with reset applied before adding
the flagged contribution. Coarse buckets do not retain exact reset order, missing-point positions or original
collection times, so reconstructed counters and expressions can differ from the original observations.

Older incremental data uses approximate cumulative reconstruction. A physical series keeps its starting basis
throughout the query: a legacy start remains legacy, and an anchored start carries its accumulator through later
page and tier transitions. Later anchors do not replace that basis. Different required query histories can therefore
produce different reconstructed levels at the same timestamp.

## Configuration and resources

The `[promql]` section in `netdata.conf` has these settings:

| Setting | Default |
|---|---|
| `lookback delta` | `5m` |
| `default subquery step` | `1m` |
| `temporary directory` | Netdata's configured cache directory |
| `experimental functions` | no |
| `binary fill modifiers` | no |
| `extended range selectors` | no |
| `start timestamps` | no |
| `delayed metric name removal` | no |

Duration expressions are enabled. The optional language switches follow the pinned engine's upstream feature
contracts. Query-owned memory, storage work and concurrent queries are accounted; implicit sample, memory, work,
concurrency and time limits are disabled. Explicit request and caller deadlines still apply. Native request-size
and transport packet ceilings remain in force.

Raw samples are read in bounded pages rather than loading a complete series or time-by-series dataset. Retained
metadata references, aggregation state and result indexes still grow with query cardinality. Shared DBengine LRU
memory is reported separately from query-owned allocations; accounting is not a measurement of total process RSS.

Temporary backing is required for large results and computations that need external storage. Keep the configured
directory writable with sufficient space. If backing is unavailable or full, the query returns an explicit error;
there is no fallback that loads the entire response into RAM. Small scalar results can complete without backing.
Completed single-series results with one sample kind read their point records through the existing 8 KiB output
window. Multiple-series and mixed-kind results retain exact-sized reads. This introduces no additional heap buffer;
[the integration measurements](../../../../tests/promql-integration/README.md#output-record-reading) record its
scope, real-account checks and measured costs. Float point serialization reuses that window for pinned sample
formatting and complete-record assembly, without another heap allocation. Its
[separate measurements](../../../../tests/promql-integration/README.md#float-point-serialization) retain complete
byte/account checks, timing distributions and finite qualification limits.
Counter reconstruction keeps 16 page-boundary checkpoints in RAM and spills longer histories into one query-owned
backing file. Such counter queries need backing even when their final result is small. Checkpoints preserve the
query's starting basis and exact accumulated bits; copy scratch stays within 8 KiB. DBengine page formats are unchanged.

## Building and scope

PromQL is enabled by default with DBengine. Source builds need Rust 1.91 or newer and the locked Cargo dependencies.
The complete engine source and conformance suite are included under `src/crates/promql-engine`; no separate PromQL
repository is required. Custom builds can opt out with
`--disable-promql` in `netdata-installer.sh` or `-DENABLE_PROMQL=Off` in CMake. The engine uses an unwind profile
so ordinary Rust panics can return an error through its C interface.

The [Agent integration design](../../../../tests/promql-conformance/DESIGN.md) explains storage and ownership.
The [library design](../../../crates/promql-engine/tests/promql-conformance/DESIGN.md) records expression
compatibility decisions. This API does not implement distributed Cloud query processing, a separate alert-rule
evaluation engine or Alertmanager.

### Optional evaluation resource observations

Exact `stats=all` adds `data.stats.netdata` to successful query/query_range responses. Ordinary responses and
`stats=true` retain their existing shape. The extension reuses existing accounting; it does not enable limits.

- `snapshot: "evaluation-before-output-commit"` means the execution snapshot precedes response sorting, JSON assembly
  and gzip. Compilation and initial request/output allocations use separate accounts. This is **not total HTTP-request
  memory**, process RSS or shared DBengine cache size.
- `memoryBytes` reports `current`, `peak`, `acquired`, `released`, and `overflowed` for each existing category:
  `compiled`, `workspace`, `result`, `providerHeld`, `borrowedView`, `regexEstimate`, `sharedCacheReference`, `accounting`.
  Engine-owned allocations are accounted; regex dependency memory is estimated, and borrowed/shared views are
  references rather than additional owned RAM. Peaks need not occur together: do not sum category peaks as total RAM.
- `samplesRead`, `providerCalls`, `sampleBatches`, `samplesFetched`, `work`, and `outputSamples` report execution work.
  `sampleBatches` counts returned provider batches, not disk reads. The existing standard `timings` and `samples`
  fields remain unchanged.
- `temporaryStorage` reports accounted extent bytes plus read/write calls and bytes. It describes query backing,
  not total filesystem usage.
- `storage.tiers` is ordered by tier index. Each entry reports `pageEncounters` and `points` across storage cursors;
  repeated encounters are counted and cache hits are not distinguished from physical I/O. `reconstructionPoints`
  reports additional counter reconstruction work. Overflow fields are numeric flags.

If execution started and its report is available, an error response requested with `stats=all` includes
`stats.netdata` at the top level, with `snapshot: "execution-report"`. This later snapshot includes any output work
performed before failure. Parse/parameter failures need not have execution statistics. Success and failure snapshots
have different lifecycle positions; compare those positions explicitly. Client concurrency and end-to-end response
latency must be measured by the caller.
