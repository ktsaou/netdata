<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/enrichment/bmp-routing.md"
sidebar_label: "BMP Routing"
learn_status: "Published"
learn_rel_path: "Network Flows/Enrichment"
keywords: ['bmp', 'bgp', 'routing', 'rfc 7854', 'enrichment']
endmeta-->

# BMP routing enrichment

BMP (BGP Monitoring Protocol, [RFC 7854](https://www.rfc-editor.org/rfc/rfc7854)) lets a router push its BGP route updates to a passive collector. Netdata can act as that collector — it listens for BMP TCP connections from your routers, parses the BGP UPDATE messages, and builds an in-memory routing table that flow enrichment then reads from.

The result: every flow gets accurate AS numbers, AS paths, communities, and next-hop information from your real-time BGP table — not from a stale GeoIP database or from whatever the exporter happened to send in the flow record.

:::warning Integration test gap
BMP message parsing is unit-tested. The runtime path — the TCP listener, the framed-decode loop, the trie apply path, and the per-router cleanup — is **not** integration-tested. The feature is shipped because the parsing layer is solid and the runtime is built on standard tokio + netgauze primitives, but it has not been validated against a panel of real-world routers in this repository's tests. Treat configuration changes as production-impacting and validate against your own gear before relying on the data.
:::

## What it populates

When BMP enrichment is enabled and a flow's source or destination IP matches a learned BGP route:

| Field | Side | Notes |
|---|---|---|
| `SRC_AS` / `DST_AS` | both | When the `routing` provider in `asn_providers` chain reaches BMP |
| `SRC_MASK` / `DST_MASK` | both | When the `routing` provider in `net_providers` chain reaches BMP |
| `NEXT_HOP` | dest only | BGP next-hop from the destination route |
| `DST_AS_PATH` | dest only | Full BGP AS path (CSV of ASNs) |
| `DST_COMMUNITIES` | dest only | Standard BGP communities (CSV of u32) |
| `DST_LARGE_COMMUNITIES` | dest only | RFC 8092 large communities |

Notes:

- AS *names* (`*_AS_NAME`) come from the GeoIP/ASN MMDB, not BMP. See [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md).
- There are **no source-side AS path or communities** in flow records. Even when BMP knows them, they are not surfaced for the source IP. This is intentional — BGP path attributes are most meaningful for the destination of the traffic.

## What gets parsed and what doesn't

| BMP message | Handled? |
|---|---|
| Initiation | yes — required first message; if absent, the session is closed |
| Termination | yes — closes the session |
| Route Monitoring (BGP UPDATE) | yes — Update messages parsed; routes added to or removed from the trie |
| Peer Down Notification | yes — clears all routes for the affected peer |
| Peer Up Notification | accepted on the wire, ignored |
| Statistics Report | accepted on the wire, ignored |
| Route Mirroring | accepted on the wire, ignored |

NLRI types decoded: IPv4 unicast, IPv6 unicast, MPLS-labelled IPv4/IPv6, VPNv4 / VPNv6 with Route Distinguishers, L2VPN EVPN IP-prefix routes.

Withdrawal handling: `MP_UNREACH` (multi-protocol withdraw), legacy IPv4 `withdraw_routes`, and `Peer Down` events all remove routes from the trie. After a TCP disconnect, routes for that session linger for a configurable `keep` interval (default 5 minutes) before being purged — this prevents brief flaps from blanking the routing table.

## Listener configuration

Default listen address: `0.0.0.0:10179`. This is **not** the IANA-registered BMP port (7854) — Netdata follows the Akvorado convention of 10179.

```yaml
enrichment:
  routing_dynamic:
    bmp:
      enabled: true                          # default false
      listen: "0.0.0.0:10179"
      keep: 5m
      max_consecutive_decode_errors: 8
      receive_buffer: 0                      # 0 = OS default
      collect_asns: true
      collect_as_paths: true
      collect_communities: true
      rds: []                                # empty = accept all RDs
```

| Key | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | Master switch |
| `listen` | `host:port` | `0.0.0.0:10179` | TCP bind address |
| `keep` | duration | `5m` | Grace window after disconnect before purging that session's routes |
| `max_consecutive_decode_errors` | usize | `8` | Close the session after N consecutive decode errors |
| `receive_buffer` | usize | `0` | Optional `SO_RCVBUF` per connection in bytes; `0` = OS default |
| `collect_asns` | bool | `true` | If `false`, the AS number is forced to 0 for routes from this listener |
| `collect_as_paths` | bool | `true` | If `false`, AS paths are dropped before storage |
| `collect_communities` | bool | `true` | If `false`, communities and large communities are dropped |
| `rds` | list | `[]` | Whitelist of accepted Route Distinguishers; empty = accept all |

**Plain TCP only.** No TLS. No authentication. The listener accepts whatever connects to it. If your BMP router is in a different network, restrict access at the firewall and on a dedicated management interface — do not expose the port to the public internet.

**Multi-peer.** A single listener accepts simultaneous connections from many routers. Each connection is tracked independently.

### RD whitelist syntax

When you want to accept BMP from a router that runs L3VPNs and you only want certain VRFs in your enrichment table, use the `rds` whitelist. Accepted formats:

- `"0"` — the global table (no Route Distinguisher)
- `"65000:100"` — type 0 RD: `<2-byte ASN>:<4-byte index>`
- `"4200000000:100"` — type 2 RD: `<4-byte ASN>:<2-byte index>`
- `"192.0.2.1:42"` — type 1 RD: `<IPv4>:<2-byte index>`

Empty list = accept all RDs.

### How the routing data is structured

Internally, BMP and BioRIS share a single in-memory routing trie keyed by IP prefix. Each prefix entry holds a list of routes, one per `(peer, route_key)` tuple — so multipath BGP and multiple BGP peers contributing the same prefix coexist cleanly. Lookups prefer routes whose exporter or next-hop matches the flow being enriched, falling back to longest-prefix-match.

There is **no time-based eviction** of routes in the trie. Memory usage scales with the size of the BGP table you feed in. A full IPv4 + IPv6 BGP table from one peer is roughly 1.2M prefixes today — expect several hundred MB of resident memory per peer for a full feed. Plan accordingly.

## Configuration on the router side

These configurations are starting points based on each vendor's BMP CLI documentation. The plugin is netgauze-based and consumes RFC 7854 BMP v3 from any spec-compliant exporter, but specific vendor compatibility has not been validated by tests in this repository — verify against your platform's reference manual before deploying.

### Cisco IOS-XR

In IOS-XR the `bmp server` block is **global configuration**, not part of `router bgp`. Activate it on each neighbor (or neighbor-group, where supported):

```
bmp server 1
 host 10.0.0.10 port 10179
 description "Netdata BMP collector"
 initial-delay 5
 stats-reporting-period 60
 initial-refresh delay 30 spread 2
!
router bgp 65000
 neighbor 192.0.2.1
  bmp-activate server 1
```

### Juniper JunOS

```
set routing-options bmp station netdata station-address 10.0.0.10
set routing-options bmp station netdata station-port 10179
set routing-options bmp station netdata connection-mode active
set routing-options bmp station netdata local-address 10.0.0.1
set routing-options bmp station netdata statistics-timeout 60
set routing-options bmp station netdata route-monitoring pre-policy
```

`local-address` is recommended on multi-homed routers so the BMP TCP connection sources from a stable address. `statistics-timeout` controls the cadence of BMP Stats Report messages — without it, the station receives no statistics.

### FRR (`bgpd`)

FRR's BMP support is a **runtime module** that must be loaded explicitly. In `/etc/frr/daemons`:

```
bgpd_options=" -A 127.0.0.1 -M bmp"
```

Then restart FRR (`sudo systemctl restart frr`) and apply:

```
router bgp 65000
 bmp targets netdata
  bmp connect 10.0.0.10 port 10179 min-retry 5000 max-retry 60000
  bmp stats interval 60000
  bmp monitor ipv4 unicast pre-policy
  bmp monitor ipv6 unicast pre-policy
 exit
```

Without `-M bmp` in `bgpd_options`, every `bmp` command in the config is silently rejected as unknown — verify `vtysh -c 'show modules'` lists `bmp` after restart.

### Arista EOS

The exact CLI for BMP on EOS varies by version and is documented behind authenticated content on Arista's portal. The structure below matches the Ansible `arista.eos.eos_bgp_global` module's parameter mapping:

```
router bgp 65000
   bgp monitoring
   monitoring station netdata
      connection address 10.0.0.10 port 10179
      export-policy received routes pre-policy
```

Verify the exact keyword ordering against your EOS version's documentation or `?` help — specifically whether your version uses `monitoring received routes pre-policy` directly under `router bgp` or scoped inside the `monitoring station` block as shown.

## Provider chain integration

BMP-derived routes contribute to flow enrichment via the `routing` entry in the [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md) provider chain:

```yaml
enrichment:
  asn_providers: [flow, routing, geoip]    # default
  net_providers: [flow, routing]            # default
```

With the defaults, an exporter-supplied AS number wins over BMP. To prefer BMP over the exporter (useful when your BMP feed is more accurate than the exporter's view), reorder:

```yaml
enrichment:
  asn_providers: [routing, flow, geoip]
```

`bmp` is accepted as an alias for `routing` in the provider list, for backward compatibility and discoverability.

## Things that go wrong

- **Listener not receiving connections.** Check the firewall, and verify the router actually opened the BMP session (`show bmp` on Cisco / `show bmp connections` on Juniper / `show bmp targets` on FRR). The plugin doesn't proactively retry — it waits for the router to dial in.
- **Routes appear, then vanish after maintenance.** If BMP disconnects (router restart, link bounce), routes for that session are purged after `keep` (default 5 min). Increase `keep` if you accept brief gaps as the cost of avoiding flapping.
- **Memory growth without bound.** A full BGP table grows the trie permanently. Without route withdrawal, memory only goes up. Plan capacity for the worst case (full IPv4+IPv6 from every peer).
- **Plugin restart wipes the BMP table.** The trie is not persisted. Routes are re-learned as the routers re-send Initiation + Update messages — depending on your routers, full convergence can take seconds (FRR) to minutes (Cisco IOS-XR). Until then, BMP-derived enrichment is incomplete.
- **AS path inconsistent with the exporter's view.** Different vantage points see different paths. This is normal in BGP. If your exporter and your BMP-feeding router are different boxes with different routing tables, expect divergence.

## What's next

- [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md) — How BMP plugs into the provider chain.
- [BioRIS](/docs/network-flows/enrichment/bioris.md) — An alternative source of BGP routes via gRPC, useful when you can't run BMP.
- [Static metadata](/docs/network-flows/enrichment/static-metadata.md) — Per-prefix overrides that win over BMP.
