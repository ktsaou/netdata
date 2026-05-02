---
name: project-writing-collectors
description: Best practices and orientation for AI assistants authoring or modifying Netdata data-collection plugins or modules in any language. Read before adding a new collector, modifying an existing one, working on NetFlow/sFlow/IPFIX, OTEL ingestion, topology, SNMP profiles, or interactive Functions. Covers the mindset, the plugin landscape, task routing, per-topic DOs and DON'Ts, pointers to canonical docs, and a pre-PR self-check.
type: project
---

# Writing Netdata data collection plugins and modules

## What this skill is

You are about to add or modify data collection in the Netdata Agent. This document is a manifesto and a routing map: it tells you the mindset, where things live, the bad practices that have bitten previous collectors, and which canonical docs to open when you need depth. It is deliberately not a tutorial — the deep references already exist in the repo. Your job is to know they exist, pick the right one, and keep your work consistent with conventions the maintainers will accept.

## When a collector request arrives

A repeatable workflow:

1. **Classify the task** (new collector / modify / SNMP profile / Function / topology / NetFlow or OTEL / internal C / external plugin) — see *Where to start*.
2. **Pick the closest existing implementation** and read its source. Mirroring an adjacent collector is the universal pattern across all frameworks.
3. **Read the routing docs** for the chosen path before coding.
4. **Design metrics with NIDL first** — before writing any collection code.
5. **List required artifacts** (`metadata.yaml`, `config_schema.json`, stock conf, `health.d/`, `README.md`, function shape) and decide what changes.
6. **Define validation** — what fixtures, what tests, what real-use evidence will prove this works.

## The Netdata data-collection mindset

**Quick terminology.** A *plugin* is a process or daemon thread that publishes metrics to the Agent. A *module* is one collector inside a plugin orchestrator (`go.d.plugin`, `ibm.d.plugin`). A *job* is a configured instance of a module (one MySQL job per MySQL server). *Charts* group *dimensions*; *labels* annotate nodes, charts and instances. The data-model mnemonic is **NIDL** — Nodes, Instances, Dimensions, Labels — and it shapes every dashboard.

**Frequent collection by default.** The Agent is built for 1-second collection on >1.5M daily installs across every kind of platform; many collectors set higher defaults (e.g., `ping` 5s, SNMP 10s) when the data source warrants it. Code that allocates, reconnects, retries, or logs inside the collection cycle is multiplied by that count. Hot-path discipline is the entry ticket, not an optimization.

**Gaps are data.** When a value cannot be measured, emit nothing. The dashboard renders the gap and the user knows collection is broken. A zero fabricates a working state and hides the bug.

**Metric design is dashboard design.** The way you group dimensions into charts and attach labels *is* the dashboard the user sees. Read NIDL before designing metrics.

**IDs are public contracts.** Chart `context` (V1: `Ctx`; V2: `context_namespace` + chart context), chart IDs, dimension IDs, instance labels — once shipped, they are bound by health alerts, dashboards, exports, anomaly detection, and ML jobs. Renaming any of them silently breaks downstream consumers. Treat them as permanent.

**Configuration is layered.** Per-job source priority: `stock < discovered < user < dyncfg`, matched by job identity. A higher-priority source replaces a lower-priority job with the same identity; non-colliding jobs continue to load.

**Mirror an existing collector.** The repo has 132 go.d modules and 24 internal C plugins; the maintainers' patterns live there, not in any prose doc. Pick the closest existing collector by domain and read its source. Caveat for go.d: only 5 modules use V2 — see *Frameworks*.

**Remote-monitoring collectors live behind vnodes.** When one collector talks to N targets (SNMP, remote DBs, cloud APIs), each target is a vnode so its metrics, alerts and RBAC behave as if it were a separate node in Netdata Cloud.

**Your metrics travel.** Every metric flows through streaming to parents and to Netdata Cloud. Cardinality, dimension count, and instance churn cost compute and bandwidth on every hop — not just on the local node.

## The plugin landscape

| Family | Lang | Platforms | Where in repo | Scope |
|---|---|---|---|---|
| `proc.plugin` | C | Linux | `src/collectors/proc.plugin/` | Kernel `/proc` and `/sys` (CPU, memory, disk, net) |
| `apps.plugin` | C | Linux/FreeBSD/macOS/Windows | `src/collectors/apps.plugin/` | Per-process and per-user/group resource usage; `processes` Function |
| `cgroups.plugin` | C | Linux | `src/collectors/cgroups.plugin/` | Containers and control groups |
| `ebpf.plugin` | C + eBPF | Linux | `src/collectors/ebpf.plugin/` | Kernel function tracing via eBPF |
| `network-viewer.plugin` | C | Linux | `src/collectors/network-viewer.plugin/` | L3/L4 sockets; `topology:` Functions (not metrics) |
| `systemd-journal.plugin` / `windows-events.plugin` | C | Linux/Windows | `src/collectors/systemd-journal.plugin/`, `src/collectors/windows-events.plugin/` | Log/event explorers via Functions |
| `systemd-units.plugin` | C | Linux | `src/collectors/systemd-units.plugin/` | systemd unit state |
| `windows.plugin` | C | Windows | `src/collectors/windows.plugin/` | Windows performance counters |
| `freebsd.plugin` / `macos.plugin` | C | platform-specific | `src/collectors/{freebsd,macos}.plugin/` | OS analogs of `proc.plugin` |
| `freeipmi.plugin` / `nfacct.plugin` / `tc.plugin` / `xenstat.plugin` / `debugfs.plugin` / `diskspace.plugin` | C | Linux | `src/collectors/<name>.plugin/` | Specialty kernel/host data |
| Niche C plugins (`statsd`, `slabinfo`, `idlejitter`, `timex`, `cups`, `ioping`, `perf`) | C | various | `src/collectors/<name>.plugin/` | Rarely a starting point for new work |
| `go.d.plugin` | Go (no CGO) | All | `src/go/plugin/go.d/` | 132 application integrations |
| `ibm.d.plugin` | Go + CGO | Linux, IBM i | `src/go/plugin/ibm.d/modules/` | IBM workloads (DB2, IBM i / AS-400, IBM MQ, WebSphere) |
| `netflow-plugin` | Rust | Linux | `src/crates/netflow-plugin/` | NetFlow v5/v9, IPFIX, sFlow |
| `netdata-otel` | Rust | Linux | `src/crates/netdata-otel/otel-plugin/` | OpenTelemetry ingestion |
| `netdata-log-viewer` | Rust | Linux | `src/crates/netdata-log-viewer/` | OTEL signal viewer + journal Function backend |
| `charts.d.plugin` / `python.d.plugin` | Bash / Python | All | `src/collectors/{charts,python}.d.plugin/` | **Legacy** — do not add new modules |

**Path conventions.** Internal C plugins → `src/collectors/<name>.plugin/`. Go orchestrators → `src/go/plugin/{go.d,ibm.d}/`. Rust plugins → `src/crates/<name>/`.

`go.d.plugin` modules are organized by category, not enumerated here: databases, web servers and reverse proxies, message queues, DNS/DHCP, container/orchestration, search and analytics, application monitoring backends, network appliances and SNMP, enterprise hardware, system services. `ibm.d.plugin` modules: IBM i (AS/400), DB2, IBM MQ, WebSphere PMI.

## Where to start (routing by task)

**Which plugin family for monitoring X?**

- HTTP/API/database/MQ/cloud SaaS, no CGO → `go.d.plugin`
- IBM workload requiring vendor drivers → `ibm.d.plugin`
- Heavy binary protocol parsing (NetFlow, sFlow, IPFIX, OTEL) → Rust plugin under `src/crates/`
- Kernel `/proc` or `/sys` data, eBPF, OS-level integration → internal C plugin under `src/collectors/`
- Anything new in Bash/Python → no — both orchestrators are legacy

| If you are doing… | Start with |
|---|---|
| New off-the-shelf application integration (no CGO) | `src/go/plugin/go.d/docs/how-to-write-a-collector.md`; V2 reference: `src/go/plugin/go.d/collector/ping/` |
| New IBM workload integration (CGO) | `src/go/plugin/ibm.d/AGENTS.md`, `src/go/plugin/ibm.d/framework/README.md` |
| New Rust plugin (NetFlow, OTEL, log/journal) | SDK at `src/crates/netdata-plugin/` (code-only docs in `lib.rs`); reference: `src/crates/netflow-plugin/` |
| New SNMP profile (no code change) | `src/go/plugin/go.d/collector/snmp/profile-format.md`; stock profiles at `src/go/plugin/go.d/config/go.d/snmp.profiles/default/` |
| New interactive Function | `src/go/plugin/framework/functions/README.md`, `src/plugins.d/FUNCTION_UI_SCHEMA.json`, `src/plugins.d/FUNCTION_UI_DEVELOPER_GUIDE.md` |
| Topology work | `src/go/pkg/topology/`, `src/go/plugin/go.d/collector/snmp_topology/`, `src/collectors/network-viewer.plugin/`, `src/streaming/README.md`. Note: `snmp_topology` builds on top of SNMP profiles — extending profiles is usually the right starting point. |
| Auto-discovery for a new go.d module | rules: `src/go/plugin/go.d/config/go.d/sd/{net_listeners,docker,snmp,http}.conf`. Engine: `src/go/plugin/agent/discovery/`. Schemas: `src/go/plugin/go.d/discovery/sdext/config_schema_*.json` |
| OTEL ingestion or log viewer | `src/crates/netdata-otel/otel-plugin/`, `src/crates/netdata-log-viewer/` |
| New external plugin in any language (PLUGINSD) | `src/plugins.d/README.md` |
| New internal C plugin | `src/collectors/README.md`; `src/libnetdata/`; mirror an adjacent collector; shared metric definitions at `src/collectors/common-contexts/` |
| Cross-plugin data enrichment | `src/libnetdata/netipc/` (C), `src/go/pkg/netipc/` (Go), `src/crates/netipc/` (Rust) |
| Privileged operations (sudo wrapper) | `src/collectors/utils/ndsudo.c` |
| Credentials in config | `src/collectors/SECRETS.md` (`${env:}/${file:}/${cmd:}/${store:}`) |
| Build / install / dev loop | go.d unit tests: `cd src/go && go test ./plugin/go.d/collector/<name>/...`. Single-module dev run: `go run ./cmd/godplugin -m <name> -d`. Rust: `cargo test -p <crate>`. Whole-project install: `./netdata-installer.sh`. New Rust crate goes into the `src/crates/Cargo.toml` workspace. |

## Topic guidance

For each topic: a short DO, a short DON'T, and a pointer to where to read more.

### Metrics design (NIDL)

**DO.** Read `docs/NIDL-Framework.md` before designing metrics. Group dimensions into charts that answer one operational question; use labels for instances and contexts. Pick the right **chart type** — `line`, `area`, `stacked`, `heatmap` (defined in `src/database/rrdset-type.h`). Pick the right **dimension algorithm** — `absolute` for gauges, `incremental` for monotonic counters, `percentage-of-incremental-row` and `percentage-of-absolute-row` for proportional dimensions (defined in `src/database/rrd-algorithm.h`, documented in `src/plugins.d/README.md`). For C plugins, reuse shared metric definitions from `src/collectors/common-contexts/`.

**DON'T.** Mirror upstream data structures one-to-one — that produces a chart per metric. Don't ship unbounded high-cardinality labels. Don't use `absolute` on a counter (the most common bug); don't use `line` when `stacked` is the right shape (CPU states, disk-time breakdown). Don't pick chart priorities arbitrarily — see `src/collectors/all.h` for the convention.

**Pointer.** `docs/NIDL-Framework.md`; chart-template engine: `src/go/plugin/framework/charttpl/README.md`; metric store: `src/go/pkg/metrix/README.md`; chart-type/algorithm definitions: `src/database/rrdset-type.h`, `src/database/rrd-algorithm.h`.

### Hot-path performance

**DO.** Allocate buffers, maps and slices once at `Init()` and reuse them across `Collect()` cycles. Hold persistent connections; reconnect only on failure with backoff. Cache anything stable between iterations (schema, capabilities, profile selections). Right pattern: keep reusable state on the collector struct (or build it in `Init()`) and reset it at the top of `Collect()` if needed; see `ping/collect.go` for a V2 reference.

**DON'T.** Don't allocate a fresh structure per `Collect()` cycle (search `src/go/plugin/go.d/collector/ap/collect.go` for `mx := make(map[string]int64)` per Collect — that pattern is what new code should not repeat). Don't reconnect every cycle.

**Pointer.** `src/go/BEST-PRACTICES.md`, `src/go/COLLECTOR-LIFECYCLE.md` (both V1 — see Frameworks for the V1/V2 split).

### Error handling

**DO.** Every error log answers three questions: *what operation*, *what target*, *what was expected vs observed*. Wrap errors with context (Go: `fmt.Errorf("...: %w", err)`); preserve the cause; check return codes from system calls and library functions.

**DON'T.** Don't return a bare `err` with no context. Don't log "failed". Don't ignore syscall returns or library NULLs.

### Logging discipline

**DO.** Log at `debug` inside the collection loop. For known-recoverable conditions, log at `warn` or `error` *once* per condition (gate by an internal flag). Use `info`/`notice` for once-at-startup events. Reserve `error` for operator-actionable issues.

**DON'T.** Don't emit per-collection log lines (a past `ebpf.plugin` regression flooded logs because the collection loop logged every PID allocation). Don't use `error` severity for transient conditions; users will lose trust in the severity.

### Gaps are data

**DO.** When you cannot measure a value this iteration, emit nothing for that dimension.

**DON'T.** Don't default missing values to `0`. Past pain documented in `src/collectors/proc.plugin/proc_net_dev.c` — search the file for the comment `shouldn't use 0 value, but NULL` to see the bug acknowledged in code.

### Cardinality and dynamic entities

**DO.** When a collector emits one chart per discovered entity (a process, a connection, a profile target), bound the count, allow `selector` controls in config to scope which entities are tracked, and **obsolete** charts when entities disappear so they don't accumulate forever. See `src/go/BEST-PRACTICES.md` (search "max" and "obsolete") for the V1 pattern.

**DON'T.** Don't ship a collector that creates a new chart per request, per PID, or per ephemeral connection without bounding and obsoleting.

### Configuration

**DO.** Tunables belong in `config_schema.json` and `metadata.yaml` (both must be complete). The stock `.conf` should show safe, representative examples — not necessarily every tunable. Source priority is `stock < discovered < user < dyncfg`, matched by job identity. IaC users configure via files in `/etc/netdata`; dashboard users configure via DYNCFG; both must work for the same collector.

**DON'T.** Don't hardcode timeouts, paths, ports or credentials. Don't let stock conf and schema contradict each other.

**Pointer.** `src/plugins.d/DYNCFG.md`; `docs/developer-and-contributor-corner/dyncfg.md`; `src/health/alert-configuration-ordering.md`.

### Frameworks

**Reality check.** Only **5 of 132** `go.d` collectors use V2 today: `ping`, `mysql`, `azure_monitor`, `powerstore`, `powervault`. The two large reference docs — `src/go/BEST-PRACTICES.md` and `src/go/COLLECTOR-LIFECYCLE.md` — describe the V1 lifecycle (`Init/Check/Charts/Collect/Cleanup` + `map[string]int64` output). The V2 building blocks have framework READMEs (`src/go/plugin/framework/charttpl/README.md`, `src/go/plugin/framework/chartengine/README.md`, `src/go/pkg/metrix/README.md`) describing the *mechanics*; there is no end-to-end V2 collector authoring tutorial beyond `how-to-write-a-collector.md` plus the `ping/` source.

**For a new go.d module: use V2.** Mirror `src/go/plugin/go.d/collector/ping/` (or `mysql/` for a V2 module that also uses Functions). Copying any other module mirrors V1 and the maintainers will ask you to migrate. V2 imports: `github.com/netdata/netdata/go/plugins/plugin/framework/collectorapi` and `.../pkg/metrix`. The `CollectorV2` interface lives at `src/go/plugin/framework/collectorapi/collector.go`.

**Lifecycle semantics.** `Init()` is one-time setup — failure disables the job permanently. `Check()` is auto-detection probe — failure disables but can be retried later. `Collect()` is the hot path, runs every `update_every` seconds. `Cleanup()` is guaranteed on shutdown — close connections, release resources, prevent leaks.

**Silent-failure trap (go.d).** A new go.d module compiles and its tests pass even when it is not loaded by the plugin at runtime. Loading requires four wiring steps from `how-to-write-a-collector.md`: import in `src/go/plugin/go.d/collector/init.go`, `modules:` toggle in `src/go/plugin/go.d/config/go.d.conf`, stock job config at `src/go/plugin/go.d/config/go.d/<name>.conf`, and an entry in `src/go/plugin/go.d/README.md`. The same trap applies to `ibm.d`.

**DO.** For ibm.d (CGO, IBM-vendor workloads) use the ibm.d framework with `go generate` after touching `contexts.yaml`. For Rust, use the SDK at `src/crates/netdata-plugin/` (`bridge/`, `protocol/`, `rt/`, `charts-derive/`, `schema/`, `types/`, `error/`); SDK docs are in `lib.rs` doc-comments only — there is no README. Reference impl: `src/crates/netflow-plugin/`. For internal C plugins, mirror an adjacent collector under `src/collectors/<name>.plugin/` and lean on `src/libnetdata/`. For external plugins in any language, follow the PLUGINSD protocol.

**DON'T.** Don't write new go.d modules against framework v1. Don't add modules to `charts.d.plugin` or `python.d.plugin`. Don't reach for `ibm.d` for non-IBM CGO needs — that framework is shaped around vendor drivers; CGO outside the IBM ecosystem is a design discussion. Don't run `go generate` for go.d (it has no `//go:generate` directives — it uses `//go:embed`). Don't add new third-party Go module or system-library dependencies casually — they ship to every Netdata install; check with maintainers if non-trivial.

**Pointer.** `src/go/plugin/go.d/docs/how-to-write-a-collector.md`; framework packages under `src/go/plugin/framework/`; shared packages under `src/go/pkg/`; `src/go/plugin/ibm.d/AGENTS.md`; `src/go/plugin/ibm.d/framework/README.md`; Rust SDK at `src/crates/netdata-plugin/`; `src/plugins.d/README.md`.

### Generated artifacts

Several artifacts are *generated* and must not be hand-edited:

- `integrations/<name>.md` (every collector) — generated from `metadata.yaml`. Banner: "DO NOT EDIT THIS FILE DIRECTLY".
- `ibm.d` modules — generated `README.md`, `metadata.yaml`, `config.go`, and `zz_generated_*.go` files; produced by `go generate` from `contexts.yaml`. See `src/go/plugin/ibm.d/AGENTS.md` and `src/go/plugin/ibm.d/framework/README.md`.
- Rust plugins — chart definitions are derived at compile time via the `charts-derive` proc-macro (no separate command).

When a generated file looks wrong, find the source of truth (`metadata.yaml`, `contexts.yaml`, or a Rust derive macro input) and regenerate.

### SNMP profiles

**DO.** SNMP collection is profile-driven. Adding or extending SNMP coverage means writing or extending a profile; read `profile-format.md` first. The SNMP topology collector (`snmp_topology`) builds on top of SNMP profiles — extending profiles is usually the right starting point for topology work too.

**DON'T.** Don't hardcode OID-to-metric mappings inside a custom collector or vendor branch. Past pain: SNMP topology code that pre-dated profiles required per-vendor branches that were painful to maintain.

**Pointer.** Spec: `src/go/plugin/go.d/collector/snmp/profile-format.md` (~2000 lines). Stock profiles ship from `src/go/plugin/go.d/config/go.d/snmp.profiles/default/` (installed at runtime under `/usr/lib/netdata/conf.d/go.d/snmp.profiles/default/`).

### Cross-plugin enrichment via netipc

**DO.** When a collector needs data from another collector, use netipc. In-tree libraries: C at `src/libnetdata/netipc/`, Go at `src/go/pkg/netipc/`, Rust at `src/crates/netipc/`. Both clients and servers exist in all three languages.

**DON'T.** Don't shell out to another plugin. Don't open private sockets. Don't poll log files. Don't reinvent IPC.

**Pointer.** Upstream spec, tests, benchmarks, fuzzy/chaos suite: <https://github.com/netdata/plugin-ipc>. Changes to the netipc libraries propagate upstream.

### vnodes for remote targets

**DO.** Every collector that talks to remote targets emits each target as a vnode. The go.d and ibm.d frameworks support it: set `Vnode` in the job config and respect it in `Init()` and DYNCFG handlers. Vnodes are the unit Netdata Cloud uses for rooms, alert routing and RBAC.

**DON'T.** Don't collapse multiple targets into one logical node. Past pain: an older refactor had to retroactively split job-name validation per vnode/domain because earlier collectors hadn't accounted for it.

**Pointer.** `src/go/plugin/framework/vnodes/`; `src/go/BEST-PRACTICES.md` (search for "Vnode").

### Documentation and configuration consistency

A new or modified collector ships these in sync, with framework-dependent variations:

- the code (Go, C, Rust, Bash, Python)
- `metadata.yaml` — drives integration pages on `learn.netdata.cloud`, `www.netdata.cloud`, in-app help; metric units, alert references, setup steps
- `config_schema.json` — DYNCFG schema; the dashboard renders it
- stock `.conf` — safe, representative example for `/etc/netdata`
- `health.d/*.conf` — alert templates; alerts bind to charts via the `context` field, which is a permanent contract
- `README.md` — concise narrative
- if the collector exposes a Function: response shape conforming to `src/plugins.d/FUNCTION_UI_SCHEMA.json`

Framework-dependent: go.d/ibm.d ship the full set; Rust plugins ship a subset; some internal C plugins don't carry every artifact — mirror the closest existing plugin.

**DO.** Treat these artifacts as one unit. When you change units in code, update `metadata.yaml` in the same commit.

**DON'T.** Don't ship metrics without `metadata.yaml`. Don't change a unit and leave the doc stale. Don't add a config knob without updating schema + stock conf + metadata together. Don't hand-edit any file under `integrations/` (or any `zz_generated_*.go` in ibm.d) — they are regenerated.

**Pointer.** `integrations/README.md`. Alerts reference: `src/health/REFERENCE.md`.

### Testing

**DO.** Use real protocol artifacts: production dumps, vendor SDK fixtures, captured packet traces. NetFlow keeps flow fixtures under `src/crates/netflow-plugin/testdata/flows/` with sourcing recorded in `testdata/ATTRIBUTION.md` (do the same for any new fixtures with redistribution-sensitive provenance). Where to find fixtures: vendor SDK samples, public packet captures, anonymized PR/issue captures, mirrored upstreams. Standard go.d test-function names: `Test_testDataIsValid`, `TestCollector_ConfigurationSerialize`, `TestCollector_Init`, `TestCollector_Check`, `TestCollector_Collect` — match the convention in adjacent collectors. Functions get a dedicated validator at `src/go/tools/functions-validation/` (E2E plus schema checks).

**DON'T.** Don't fabricate test data the parser passes by accident. Don't skip tests because "this protocol can't be tested locally" — that's exactly when fixtures matter most.

### Functions

**When to build one.** Functions complement metrics; they don't replace them. Build a Function when the answer is interactive/tabular live data — process list, log entries, network connections, FDB tables, topology snapshots, flows. If the answer is a numeric time series, that's a metric.

**DO.** Response shape is one of `info_response`, `data_response`, `topology_response`, `flows_response`, `error_response`, `not_modified_response` (defined in `FUNCTION_UI_SCHEMA.json`). For Go, use the builders in `src/go/pkg/funcapi/`. For Rust, implement the `FunctionHandler` trait from the SDK runtime (`src/crates/netdata-plugin/rt/`). Functions run concurrently with the collection loop. Validate with `src/go/tools/functions-validation/`.

**DON'T.** Don't emit ad-hoc JSON. Don't block the collection loop on Function work. Don't skip the schema check during development.

**Pointer.** Backend (Go): `src/go/plugin/framework/functions/README.md`. Backend (Rust): `src/crates/netdata-plugin/rt/src/lib.rs` (`FunctionHandler` trait). UI/protocol: `src/plugins.d/FUNCTION_UI_DEVELOPER_GUIDE.md`, `src/plugins.d/FUNCTION_UI_REFERENCE.md`, `src/plugins.d/FUNCTION_UI_SCHEMA.json`. Validator: `src/go/tools/functions-validation/README.md`. References: `src/collectors/network-viewer.plugin/` (topology), `src/collectors/systemd-journal.plugin/` (log explorer), `apps.plugin` (processes).

## Production-quality criteria

Distinct from the pre-PR self-check — a collector is *production-quality* when it satisfies all of:

- **Survives target unavailability for hours** without log floods, fd leaks, memory growth, or runaway retries.
- **Bounded memory under failure** — buffers do not grow on parse errors or stuck connections.
- **No fd / goroutine / thread leaks** across `Cleanup()` cycles or job reloads.
- **Cycle-latency budget respected** — at the configured `update_every`, a single `Collect()` finishes well under one cycle even on a slow target.
- **Graceful with partial / malformed upstream responses** — parser does not crash, log-flood, or skip downstream collection.
- **High-cardinality entities bounded and obsoleted** (see *Cardinality* topic).
- **IDs (chart context, chart ID, dimension ID, instance labels) are stable** — never renamed without a migration plan.

## Self-check before opening a PR

1. Do all metrics have units, chart families, and meaningful names? Did NIDL inform the grouping? Are chart types and dimension algorithms correct (`incremental` for counters, etc.)?
2. Are gaps preserved (no zero defaults for missing values)?
3. Does the collection cycle allocate, log per iteration, or reconnect every cycle?
4. Do error logs answer *what operation, what target, what was expected vs observed*?
5. Are config knobs in `config_schema.json` and `metadata.yaml`? Does the stock `.conf` show a representative example?
6. Are alerts present in `health.d/`?
7. Is `README.md` updated? (Not the generated `integrations/<name>.md`.)
8. For remote targets: is vnode wiring done?
9. For SNMP: did I extend a profile rather than hardcode OIDs?
10. For cross-plugin enrichment: am I using netipc?
11. For Functions: does the response conform to one of the six shapes? Non-blocking with respect to the collection loop? Schema-validated?
12. For ibm.d only: did I run `go generate` after touching `contexts.yaml`? (go.d uses `//go:embed`, no generation step.)
13. For new go.d modules: are all four wiring steps done (init.go, go.d.conf, stock conf, README)?
14. Tests use real fixtures (pcap, production samples, vendor data) — and would they catch the bug I just fixed?
15. High-cardinality labels / instances: bounded? rotating instances obsoleted?
16. Production-quality criteria above — survives hours of target outage without leaks or log floods?

## Canonical documentation pointers

| Topic | Open when | Path |
|---|---|---|
| NIDL framework | designing metrics, labels, charts | `docs/NIDL-Framework.md` |
| Chart types and dimension algorithms | choosing chart shape and metric algorithm | `src/database/rrdset-type.h`, `src/database/rrd-algorithm.h` |
| Chart priorities (C plugins) | dashboard ordering convention | `src/collectors/all.h` |
| Shared metric definitions (C) | reusing common contexts | `src/collectors/common-contexts/` |
| Plugin types and privileges | choosing where to add a collector | `src/collectors/README.md` |
| External plugin protocol (PLUGINSD) | writing a non-Go external plugin | `src/plugins.d/README.md` |
| go.d V2 authoring | adding a `go.d` module | `src/go/plugin/go.d/docs/how-to-write-a-collector.md` |
| go.d V2 collector interface | implementing `CollectorV2` | `src/go/plugin/framework/collectorapi/` |
| go.d best practices (V1) | implementing a V1 `go.d` collector | `src/go/BEST-PRACTICES.md` |
| go.d collector lifecycle (V1) | Init/Check/Collect/Cleanup details | `src/go/COLLECTOR-LIFECYCLE.md` |
| Chart template engine (V2) | V2 chart definition | `src/go/plugin/framework/charttpl/README.md` |
| Chart engine (V2) | V2 chart wiring | `src/go/plugin/framework/chartengine/README.md` |
| Metric store (V2) | V2 metric output (`metrix`) | `src/go/pkg/metrix/README.md` |
| Functions backend (Go) | implementing a Function in Go | `src/go/plugin/framework/functions/README.md` |
| Functions runtime (Rust) | implementing a Function in Rust | `src/crates/netdata-plugin/rt/src/lib.rs` (`FunctionHandler` trait) |
| Functions UI schema | function response shape | `src/plugins.d/FUNCTION_UI_SCHEMA.json` |
| Functions developer guide | function patterns | `src/plugins.d/FUNCTION_UI_DEVELOPER_GUIDE.md` |
| Functions reference | full field/visual vocabulary | `src/plugins.d/FUNCTION_UI_REFERENCE.md` |
| Functions validator | E2E + schema validation | `src/go/tools/functions-validation/README.md` |
| ibm.d quick checklist | starting `ibm.d` work | `src/go/plugin/ibm.d/AGENTS.md` |
| ibm.d framework | contexts, generators, layout | `src/go/plugin/ibm.d/framework/README.md` |
| Rust plugin SDK | new Rust plugin (code-only docs) | `src/crates/netdata-plugin/` (`rt/`, `protocol/`, `bridge/`, `charts-derive/`, `schema/`, `types/`, `error/`) |
| Rust NetFlow plugin (reference) | NetFlow / sFlow / IPFIX work | `src/crates/netflow-plugin/` |
| OTEL ingestion (Rust) | OpenTelemetry signals | `src/crates/netdata-otel/otel-plugin/` |
| Log viewer (Rust) | OTEL signal viewer / journal Function | `src/crates/netdata-log-viewer/` |
| SNMP profile format | adding/extending an SNMP profile | `src/go/plugin/go.d/collector/snmp/profile-format.md` |
| SNMP stock profiles | starting from a known device | `src/go/plugin/go.d/config/go.d/snmp.profiles/default/` |
| Auto-discovery engine | how jobs get auto-created | `src/go/plugin/agent/discovery/` |
| Auto-discovery rules (go.d) | adding service-detection rules | `src/go/plugin/go.d/config/go.d/sd/{net_listeners,docker,snmp,http}.conf` |
| Topology library (Go) | topology providers | `src/go/pkg/topology/` |
| SNMP topology collector | topology source code | `src/go/plugin/go.d/collector/snmp_topology/` |
| network-viewer plugin | L3/L4 sockets + topology Functions | `src/collectors/network-viewer.plugin/` |
| DYNCFG protocol | adding dynamic configuration | `src/plugins.d/DYNCFG.md` |
| DYNCFG (developer corner) | broader context | `docs/developer-and-contributor-corner/dyncfg.md` |
| Health alerts reference | authoring health alert templates | `src/health/REFERENCE.md` |
| Configuration override ordering | layered priority pattern | `src/health/alert-configuration-ordering.md` |
| Integrations pipeline | doc generation from `metadata.yaml` | `integrations/README.md` |
| Credentials in config | `${env:}/${file:}/${cmd:}/${store:}` | `src/collectors/SECRETS.md` |
| Privileged operations | restricted setuid helper | `src/collectors/utils/ndsudo.c` |
| netipc (C / Go / Rust) | cross-plugin enrichment | `src/libnetdata/netipc/`, `src/go/pkg/netipc/`, `src/crates/netipc/` |

## Maintaining this skill

This skill is **live**. When you find a gap, an outdated pointer, or a bad practice not yet captured, propose changes to this file in the same PR that exposed the issue. When fixing a wrong pointer, also record what was misleading about the prior text — future readers see both the corrected map and the failure mode that produced it. Mention the change in the PR description so it gets reviewed consciously rather than skimmed.
