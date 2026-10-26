# Netdata PromQL dashboards for Grafana

These four importable dashboards use the Netdata PromQL API and the metrics collected by Linux Agents and Parents.
They are also representative integration workloads: selectors, aggregation, ranking, arithmetic, vector matching,
boolean comparisons, rates, temporal functions and subqueries all run against retained DBengine data.

| File | Dashboard | Panels |
|---|---|---:|
| [fleet-overview.json](fleet-overview.json) | Fleet Overview: CPU, RAM, load and sample freshness | 8 |
| [host-resources.json](host-resources.json) | Host Resources: CPU modes, memory, load, temporal CPU summaries and context switches | 8 |
| [storage-network.json](storage-network.json) | Storage & Network: filesystems, disk activity and interface traffic | 10 |
| [applications.json](applications.json) | Applications: CPU/RSS rankings, processes, threads, I/O and page faults | 10 |

## Import and use

1. Install the PromQL-enabled branch using [the installation guide](../GETTING_STARTED.md).
2. Add a Prometheus datasource pointing to the Agent or Parent. Use POST and the collection interval of the source
   data; the playground uses 1s. For counters with historical negative presentation, add `options=absolute` to the
   datasource's custom query parameters. Provisioned datasources require the same setting under
   `jsonData.customQueryParameters` in their provisioning configuration.
3. In Grafana, choose **Dashboards → New → Import**, upload one JSON file, and select its destination folder.
4. Choose your datasource in the dashboard's **Data source** dropdown. Its saved default UID is `netdata-promql`.
   Choose a host from **Host**; the saved single-host default is `promql`, which must be changed if absent locally.
5. Select application groups, devices, interfaces or mount points where applicable. The host selector lists hosts
   with retained `system.cpu` data in the selected time range; it is not a complete inventory of every retained host.

The files use Grafana's [Classic JSON model](https://grafana.com/docs/grafana/latest/visualizations/dashboards/build-dashboards/view-dashboard-json-model/)
with standard Prometheus query variables. They were exercised on Grafana 13.2.4. They can also be created through
[the dashboard resource API](https://grafana.com/docs/grafana/latest/developer-resources/api-reference/http-api/dashboard/)
by placing the JSON in the resource's `spec`. Each file has a distinct stable UID; imports should not overwrite
unrelated dashboards. The cross-dashboard menu selects dashboards tagged `promql-playground` and carries the current
variables and time range. Label selectors use datasource-default `${variable}` interpolation, which handles both
PromQL string escaping and multi-value regex escaping. Do not replace it with generic `${variable:regex}` formatting:
for example, a selected `/` mount otherwise becomes an invalid PromQL string escape.

The default is 15 minutes, with a 30-second minimum panel interval and 1-minute refresh. These are dashboard settings,
not new engine resource limits. Empty series remain empty; no expression substitutes zero for unavailable data.
Native MiB, GiB, KiB/s and decimal kbit/s magnitudes are retained without additional byte conversions.

## Metric semantics

| Metrics | Query interpretation |
|---|---|
| `system_cpu`, `system_ram`, `system_load`, `disk_space`, `disk_util` | Gauges; query directly |
| `system_ctxt`, `system_net`, `net_net`, `net_packets`, `disk_io`, `disk_ops` | Incremental dimensions; use `rate(...[$__rate_interval])` |
| `app_cpu_utilization` | Gauge; sum user/system dimensions. 100% means one CPU core, not the entire host |
| `app_mem_usage`, `app_processes`, `app_threads` | Gauges; application RSS totals can count shared pages more than once |
| `app_disk_physical_io`, `app_mem_page_faults` | Already-computed rate gauges; do not apply `rate()` again |

Application I/O writes are presentation-negated gauges, so their panel uses `abs()` for positive display. Historical
network/disk counters need `options=absolute` **before counter reconstruction and rate evaluation**. Applying
`abs(rate(...))` afterwards cannot repair false counter-reset detection. This option leaves gauges unchanged.
New counter pages already reconstruct positive values. See [storage semantics](../../../src/web/api/promql/README.md#storage-semantics)
for the remaining limitations of historical and coarse-tier reconstruction.

Fleet network totals are not formed by summing physical and virtual interface charts together. Host and chart identity
remain in aggregation keys where required; selectors use only underscore aliases to avoid duplicating dotted aliases.
Host filters match names: if multiple hosts share a name, the selection includes each host, with host IDs retained in
query grouping. Legends show the names.

The CPU peak and percentile panels use explicitly sampled subqueries: 30-second samples over 5 minutes and 1-minute samples
over 30 minutes. They do not claim to capture every one-second spike. CPU busy includes I/O wait because it is 100 minus idle.
Application trends select the top ten at the **end of the selected range**, using `@ end()`, and show their available
history throughout that range. Changing rank no longer creates holes in a line. Actual source gaps remain visible;
points are neither connected across gaps nor replaced with zero. Applications absent at range end are excluded.
Membership is recalculated when the range changes or refreshes. CPU and RSS rank application groups; I/O and faults
rank individual dimension streams. Rankings span the selected hosts and applications. The CPU/RSS summary bars use
the same aligned endpoint as their trend selection. They request invariant `@ end()` range values and reduce
them to one value per application before sorting the bars. Instant requests use a different timestamp in Grafana
and can otherwise disagree with the trend ranking. Summary totals retain their existing instant-query behavior.
Freshness only describes hosts whose samples remain within lookback; absence does not mean healthy. Threshold counts
are examples of expressions, not installed alert rules.

## Qualification and findings

The initial live serial cohort on Netdata `7f0a1e1a3a` exercised all 36 panels at 15 minutes and 18 range panels at 6 hours,
plus five scoped metadata operations. Explicit counter normalization was used for direct API qualification.

- 57 of 59 requests succeeded with nonempty results.
- Two 6-hour application top-10 CPU/RSS trend requests exceeded an explicit 25-second deadline. The corresponding
  15-minute queries succeeded in about 8–11 seconds. These are retained measurements of the original per-step ranking queries; the fixed-membership revision below
  is qualified separately.
- The 6-hour fleet RAM-ratio query took about 19 seconds. These are live observations during collection/replication,
  not isolated benchmarks or latency guarantees.
- Scoped host/application/device values, label names and series discovery took about 0.16–0.23 seconds in that cohort.
  This does not qualify unfiltered fleet-wide label discovery.
- HTTP `stats=all` currently exposes timings and sample counts, but not exact query memory, provider reads or active
  PromQL execution counts. Process RSS and shared DBengine charts cannot provide precise per-query attribution.

A separate four-request concurrent Grafana-backend cohort passed all 26 panels in the Fleet, Host Resources and
Applications dashboards at 15 minutes, with nonempty frames. Whole-process Netdata RSS ranged from about 2,171 to
2,180 MiB during a 60-second observation window with collection and replication active. That change includes shared
caches and other activity; it is not a measurement of query-owned allocations.

The dashboards preserve these expressions so they remain useful regression workloads. A successful dashboard does
not establish full PromQL compatibility, complete retained history or production capacity for every query shape.

After activating the documented datasource option, all ten Storage & Network panels passed Grafana-backend checks.
Browser testing also exposed and corrected generic variable-formatting errors for mount paths; direct API substitutions
alone did not exercise that layer. The four dashboards use the Prometheus datasource's default variable interpolation.


## Fixed-ranking deployment

The VM runs the engine from `00e7704947`, installed through the existing helper with configuration and retained data
preserved. Both services are active; 24 hosts remain available and local/replicated timestamps advance. The saved
Applications dashboard now shows fixed end-of-range top-ten histories, with CPU/RSS bars using the same aligned
endpoint. The host/application filters, native units and real source gaps are preserved.

- All eight direct trend checks (four panels at 15 minutes and 6 hours) match the complete unranked histories filtered
  by the endpoint ranking. Fifteen-minute requests at 30-second steps took 2.68–5.28 seconds; six-hour requests at
  five-minute steps took 7.46–13.13 seconds. The previous fixed-ranking expressions hit 25-second deadlines.
- All ten panels pass the four-request concurrent Grafana backend check. Browser checks pass with all applications
  and with two dotted application names selected. CPU/RSS bars are sorted by their endpoint values.
- The actual six-hour browser request uses **one-minute steps**. Its four concurrent trends completed in
  **15.64–28.73 seconds**. Longer-range latency remains a limitation; the coarser serial measurements above are not
  representative of this heavier browser workload. These are live observations during collection, not guarantees.
- Separate Grafana endpoint checks at 15 minutes and 6 hours show exact CPU/RSS bar/trend label and value equality.
  Small selections use bounded bar text/height and a zero baseline for readable physical-value comparisons.

The core's [qualification record](../../../src/crates/promql-engine/tests/promql-conformance/fixed-ranking-qualification.json)
binds both optimized profiles, 2,784 reference comparisons, read-failure checks and native C/C++ callers. Its synthetic
read/time savings and query-memory measurements do not measure live per-query RAM. No storage format, collection,
interpolation or configured query limit changed.
