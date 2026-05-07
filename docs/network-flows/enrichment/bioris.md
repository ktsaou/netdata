<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/enrichment/bioris.md"
sidebar_label: "BioRIS"
learn_status: "Published"
learn_rel_path: "Network Flows/Enrichment"
keywords: ['bioris', 'bio-rd', 'ripe ris', 'bgp', 'enrichment']
endmeta-->

# BioRIS enrichment

BioRIS lets Netdata consume BGP routing data from a [bio-rd](https://github.com/bio-routing/bio-rd) `cmd/ris/` daemon over gRPC. bio-rd is a Go-based BGP daemon that can peer with the [RIPE RIS](https://www.ripe.net/analyse/internet-measurements/routing-information-service-ris) Route Collectors (or any BGP/BMP source) and expose the resulting RIB through a gRPC interface. Netdata is a client of that interface.

This is the path to use when you want a third-party view of the BGP routing table — RIPE RIS's view, for example — without running a BGP session yourself or deploying BMP across your network.

:::warning Integration test gap
The proto and route-conversion logic is unit-tested. The gRPC client path — connecting to bio-rd, consuming the streaming `DumpRIB` and `ObserveRIB` RPCs, retry/backoff — is **not** integration-tested. The feature ships because the parsing layer is solid and the gRPC client is built on standard tonic primitives, but it has not been validated end-to-end against a real bio-rd daemon in this repository's tests. Treat configuration changes as production-impacting and validate against your own setup before relying on the data.
:::

## What it populates

BioRIS feeds the same in-memory routing trie that BMP populates. Both sources contribute to the same fields:

| Field | Side | Notes |
|---|---|---|
| `SRC_AS` / `DST_AS` | both | When the `routing` provider chain reaches BioRIS data |
| `SRC_MASK` / `DST_MASK` | both | Same |
| `NEXT_HOP` | dest only | BGP next-hop from the destination route |
| `DST_AS_PATH` | dest only | BGP AS path (CSV of ASNs) |
| `DST_COMMUNITIES` | dest only | Standard BGP communities |
| `DST_LARGE_COMMUNITIES` | dest only | RFC 8092 large communities |

Same caveats as BMP:

- AS names come from the GeoIP/ASN MMDB, not from BioRIS.
- Source-side AS path and communities are not surfaced.

If both BMP and BioRIS are enabled, they share one trie. Routes from either source contribute equally; lookups pick the best-matching route across both. There is no preference between the two.

## How it works

```
RIPE RIS (rrc00..rrcNN) ──BGP/BMP──▶ bio-rd cmd/ris/ daemon ──gRPC──▶ Netdata netflow-plugin
                                                                             │
                                                                             ▼
                                                          (shared in-memory routing trie)
```

You run bio-rd's `cmd/ris/` daemon yourself. It peers with RIPE RIS (or any BGP/BMP source you have access to) and exposes a gRPC service. Netdata connects to that service and consumes:

1. **`GetRouters`** — discovers which routers the daemon is tracking.
2. **`DumpRIB`** — for each router and address family, streams a one-shot baseline of the RIB.
3. **`ObserveRIB`** — keeps a long-lived stream open for incremental updates.

The full refresh runs at the configured `refresh` interval (default 30 minutes). `ObserveRIB` runs continuously between refreshes for real-time updates. After each refresh, BioRIS re-spawns observe streams for any new routers and cancels streams for routers that have disappeared.

## Setting up bio-rd

bio-rd is a separate project. The plugin only consumes its gRPC interface; it does NOT bundle bio-rd. You install bio-rd yourself.

```bash
# Install Go (≥1.20), then:
git clone https://github.com/bio-routing/bio-rd.git
cd bio-rd/cmd/ris
go build -o /usr/local/bin/ris .
```

Configure `ris` to peer with one or more RIPE RIS route collectors (or your own BGP/BMP sources). Refer to [bio-rd's documentation](https://github.com/bio-routing/bio-rd) for the BGP/BMP peering setup — the configuration format and capabilities are bio-rd's, not Netdata's.

Run `ris` so it listens on a gRPC port:

```bash
/usr/local/bin/ris --grpc_port 50051 --config.file /etc/bio-rd.yml
```

The bio-rd `ris` binary binds gRPC on `0.0.0.0:<grpc_port>` — there is no flag today to restrict to loopback. If you need that, use a firewall rule or a network namespace.

Then point Netdata at it.

## Configuration

```yaml
enrichment:
  routing_dynamic:
    bioris:
      enabled: true                                # default false
      timeout: 200ms                               # connect + per-RPC timeout
      refresh: 30m                                 # full RIB re-dump interval
      refresh_timeout: 10s                         # per-DumpRIB request timeout
      ris_instances:
        - grpc_addr: "127.0.0.1:50051"
          grpc_secure: false
          vrf: ""
          vrf_id: 0
        - grpc_addr: "ris.example.internal:50051"
          grpc_secure: true
          vrf: "global"
```

| Key | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | Master switch |
| `timeout` | duration | `200ms` | Connect timeout and unary-RPC timeout |
| `refresh` | duration | `30m` | How often to re-dump every router's RIB from scratch |
| `refresh_timeout` | duration | `10s` | Per-`DumpRIB` request timeout and per-message stream timeout |
| `ris_instances` | list | `[]` | One or more bio-rd endpoints |

Per-instance:

| Key | Type | Default | Notes |
|---|---|---|---|
| `grpc_addr` | string | `""` | Required. `host:port` or full URI |
| `grpc_secure` | bool | `false` | When `true`, uses TLS with the system CA bundle |
| `vrf` | string | `""` | bio-rd VRF name to scope the RIB |
| `vrf_id` | u64 | `0` | bio-rd VRF numeric ID |

When `grpc_secure: false`, the connection is plain HTTP/2 (h2c). When `true`, native TLS — system CA bundle only, no custom CA config and no client-cert/mTLS support.

There is **no authentication**. Anyone who can reach the gRPC port can read the RIB. Restrict access at the firewall, or run bio-rd on the same host as Netdata and bind it to localhost.

## Multiple instances are additive, not failover

When you list more than one entry under `ris_instances`, Netdata fetches from **all of them in parallel** and merges their routes into the shared trie. There is no primary/backup logic — both feeds contribute, both can return the same prefix from different angles. Lookups pick the best-matching route from the merged set.

This is appropriate for "I want both rrc00 and rrc12's view" but not for "fall back to rrc12 if rrc00 is unreachable". For failover semantics, put the two endpoints behind a single gRPC proxy that handles failover upstream, and configure Netdata with one instance.

## Provider chain integration

Same as BMP — BioRIS-derived routes flow through the `routing` provider in `asn_providers` / `net_providers`. See [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md). With the default chain `[flow, routing, geoip]`, the exporter's view wins; reorder to `[routing, flow, geoip]` to prefer BioRIS.

## Things that go wrong

- **The default 200ms timeout is aggressive.** Over the public internet to RIPE RIS, you may need 2-5 seconds. If you see "deadline exceeded" errors in the journal, increase `timeout`.
- **Initial dump takes minutes for full feeds.** A full IPv4+IPv6 RIB from a route collector is millions of prefixes. The first refresh takes time. Subsequent observe streams are incremental.
- **Memory growth without bound.** Same as BMP — a full feed adds a permanent set of prefixes to the trie. Plan for several hundred MB per RIPE RIS RRC.
- **bio-rd not running or unreachable.** Netdata logs warnings and retries with exponential back-off. Enrichment from BioRIS is empty until the connection recovers.
- **VRF mismatch.** If your bio-rd config exposes a specific VRF and you don't set `vrf` / `vrf_id` on the Netdata side, Netdata's `GetRouters` may return nothing. Match the VRF identity that bio-rd advertises.
- **The integration test gap.** The fetch path has not been exercised end-to-end in tests. Validate behaviour against your own setup before depending on it for capacity or security decisions.

## What's next

- [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md) — Provider chain that picks AS numbers.
- [BMP routing](/docs/network-flows/enrichment/bmp-routing.md) — The local alternative to BioRIS.
- [bio-rd documentation](https://github.com/bio-routing/bio-rd) — Setting up the daemon that BioRIS consumes.
