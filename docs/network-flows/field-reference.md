<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/field-reference.md"
sidebar_label: "Field Reference"
learn_status: "Published"
learn_rel_path: "Network Flows"
keywords: ['fields', 'flow record', 'schema', 'reference']
endmeta-->

# Field Reference

Each flow record carries up to 91 fields. Some come straight from the exporter, others are added by enrichment after decode. This page is the canonical list — what each field means, where it comes from, and which protocols populate it.

In the dashboard, fields appear by their canonical name (uppercase, e.g., `SRC_AS_NAME`). The dashboard is case-insensitive when typing into the filter ribbon.

## How to read the protocol columns

| Symbol | Meaning |
|---|---|
| ✓ | Always populated by this protocol when the data is available |
| ◐ | Populated only when the exporter includes the relevant Information Element in its template (v9 / IPFIX) or the relevant record type (sFlow) |
| — | Never populated by this protocol; expect this field to be empty |

Enrichment-only fields are marked **enrichment** — the decoder never fills them; they come from configured GeoIP databases, static metadata, classifiers, or routing sources.

## Counters and sampling

The four most-used fields. Read these first.

| Field | Type | Description |
|---|---|---|
| `BYTES` | uint64 | Bytes in the flow, **already multiplied by `SAMPLING_RATE`** at ingest. The dashboard's volume numbers come from this. |
| `PACKETS` | uint64 | Packets in the flow, already multiplied by `SAMPLING_RATE`. |
| `RAW_BYTES` | uint64 | Bytes the exporter actually reported, before scaling. Use when sampling is uniform across all your exporters and you want exact counts. |
| `RAW_PACKETS` | uint64 | Packets the exporter actually reported, before scaling. |
| `FLOWS` | uint64 | Number of flows aggregated into this record. Always 1 for raw records. |
| `SAMPLING_RATE` | uint64 | Packets-per-sample reported by the exporter. `1` means unsampled. Used as the multiplier for BYTES and PACKETS. |

Every protocol populates these. sFlow always sends a sampling rate (per-sample). NetFlow v5 reads a header rate. NetFlow v7 has no rate field and is treated as unsampled. NetFlow v9 and IPFIX may include the rate per-record or via Sampling Options.

## Identity — who and what

| Field | Type | v5 | v7 | v9 | IPFIX | sFlow | Description |
|---|---|---|---|---|---|---|---|
| `FLOW_VERSION` | string | ✓ | ✓ | ✓ | ✓ | ✓ | One of `v5`, `v7`, `v9`, `ipfix`, `sflow`. |
| `EXPORTER_IP` | IP | ✓ | ✓ | ✓ | ✓ | ✓ | The device that sent this flow. For sFlow, the agent address takes precedence over the UDP source IP. |
| `EXPORTER_PORT` | uint16 | ✓ | ✓ | ✓ | ✓ | ✓ | Source UDP port of the exporter. |
| `SRC_ADDR` | IP | ✓ | ✓ | ◐ | ◐ | ◐ | Source IP. v9/IPFIX from IE 8/27, sFlow from sampled header or `SampledIPv4`/`SampledIPv6`. |
| `DST_ADDR` | IP | ✓ | ✓ | ◐ | ◐ | ◐ | Destination IP. |
| `SRC_PORT` | uint16 | ✓ | ✓ | ◐ | ◐ | ◐ | Source L4 port. |
| `DST_PORT` | uint16 | ✓ | ✓ | ◐ | ◐ | ◐ | Destination L4 port. |
| `PROTOCOL` | uint8 | ✓ | ✓ | ✓ | ✓ | ◐ | IP protocol number. TCP=6, UDP=17, ICMP=1, ICMPv6=58, GRE=47, ESP=50. |
| `ETYPE` | uint16 | ✓ (IPv4) | ✓ (IPv4) | ◐ | ◐ | ◐ | EtherType. 2048 = IPv4, 34525 = IPv6. |
| `DIRECTION` | enum | — | — | ◐ | ◐ | — | `ingress`, `egress`, or `undefined`. |

NetFlow v5 and v7 are IPv4-only. For v9, IPFIX, and sFlow, IPv6 fields populate when the exporter sends them.

## Routing — addresses and AS

| Field | Type | Source | Description |
|---|---|---|---|
| `SRC_PREFIX` | IP | decoder + enrichment | Source network prefix. |
| `DST_PREFIX` | IP | decoder + enrichment | Destination network prefix. |
| `SRC_MASK` | uint8 | decoder + enrichment | Source prefix length in bits. |
| `DST_MASK` | uint8 | decoder + enrichment | Destination prefix length in bits. |
| `NEXT_HOP` | IP | decoder | BGP next-hop or RIB next-hop, depending on the exporter. |
| `SRC_AS` | uint32 | decoder + enrichment | Source autonomous system. |
| `DST_AS` | uint32 | decoder + enrichment | Destination autonomous system. |
| `SRC_AS_NAME` | string | **enrichment** | Friendly AS name (e.g., `AS15169 Google LLC`). |
| `DST_AS_NAME` | string | **enrichment** | Friendly AS name. |
| `DST_AS_PATH` | string | sFlow `ExtendedGateway` / BGP enrichment | BGP AS path as comma-separated ASNs. |
| `DST_COMMUNITIES` | string | sFlow `ExtendedGateway` / BGP enrichment | BGP communities. |
| `DST_LARGE_COMMUNITIES` | string | BGP enrichment | RFC 8092 large communities. |

Static-network configuration can override `SRC_MASK` / `DST_MASK` and `SRC_AS` / `DST_AS` with more specific values from your CIDR-to-attribute map.

## Interfaces

| Field | Type | Source | Description |
|---|---|---|---|
| `IN_IF` | uint32 | decoder | Ingress SNMP ifIndex. |
| `OUT_IF` | uint32 | decoder | Egress SNMP ifIndex. |
| `IN_IF_NAME` | string | **enrichment** | Friendly name. |
| `OUT_IF_NAME` | string | **enrichment** | Friendly name. |
| `IN_IF_DESCRIPTION` | string | **enrichment** | SNMP `ifDescr` or your label. |
| `OUT_IF_DESCRIPTION` | string | **enrichment** | SNMP `ifDescr` or your label. |
| `IN_IF_SPEED` | uint64 | **enrichment** | Interface speed in bps. |
| `OUT_IF_SPEED` | uint64 | **enrichment** | Interface speed in bps. |
| `IN_IF_PROVIDER` | string | **enrichment** | Your transit provider tag (e.g., `Cogent`, `Lumen`). |
| `OUT_IF_PROVIDER` | string | **enrichment** | Same. |
| `IN_IF_CONNECTIVITY` | string | **enrichment** | Connectivity type tag (`transit`, `peering`, `customer`, `cdn`, ...). |
| `OUT_IF_CONNECTIVITY` | string | **enrichment** | Same. |
| `IN_IF_BOUNDARY` | uint8 | **enrichment** | `1` = External (Internet-facing), `2` = Internal (LAN/private). |
| `OUT_IF_BOUNDARY` | uint8 | **enrichment** | Same. |

`*_BOUNDARY` is counter-intuitive: 1 means "external" (the Internet side). It's defined that way so that filtering for `IN_IF_BOUNDARY=1` cleanly gives you "traffic that came in from the Internet".

## Layer 2

| Field | Type | v5 | v7 | v9 | IPFIX | sFlow | Description |
|---|---|---|---|---|---|---|---|
| `SRC_MAC` | MAC | — | — | ◐ | ◐ | ◐ | Source MAC. v9 IE 56, IPFIX IE 56/81. sFlow from `SampledHeader` or `SampledEthernet`. |
| `DST_MAC` | MAC | — | — | ◐ | ◐ | ◐ | Destination MAC. v9 IE 80, IPFIX IE 80/57. |
| `SRC_VLAN` | uint16 | — | — | ◐ | ◐ | ◐ | Source VLAN. v9 IE 58, IPFIX IE 58/243. **For sFlow, only from `ExtendedSwitch` records — NOT from 802.1Q tags inside a sampled packet header.** |
| `DST_VLAN` | uint16 | — | — | ◐ | ◐ | ◐ | Destination VLAN. |
| `MPLS_LABELS` | string | — | — | ◐ | ◐ | ◐ | MPLS label stack as comma-separated decimal label values (label only, not EXP/S/TTL). |

## NAT

| Field | Type | v5/v7 | v9 | IPFIX | sFlow | Description |
|---|---|---|---|---|---|---|
| `SRC_ADDR_NAT` | IP | — | ◐ | ◐ | — | Post-NAT source address. v9 IE 225, IPFIX IE 225/281. |
| `DST_ADDR_NAT` | IP | — | ◐ | ◐ | — | Post-NAT destination address. |
| `SRC_PORT_NAT` | uint16 | — | ◐ | ◐ | — | Post-NAT source port. |
| `DST_PORT_NAT` | uint16 | — | ◐ | ◐ | — | Post-NAT destination port. |

## Protocol metadata

| Field | Type | Description |
|---|---|---|
| `IPTTL` | uint8 | IP TTL. v9 uses Min/MaxTtl; IPFIX uses IE 192/52. |
| `IPTOS` | uint8 | IP Type of Service / DSCP byte. |
| `IPV6_FLOW_LABEL` | uint32 | IPv6 flow label (20-bit). v9/IPFIX only. |
| `TCP_FLAGS` | uint8 | OR of all TCP control bits seen in the flow (SYN/ACK/FIN/RST/PSH/URG). |
| `IP_FRAGMENT_ID` | uint32 | IPv4 ident or IPv6 fragment ID. |
| `IP_FRAGMENT_OFFSET` | uint16 | Non-zero means fragmented. |
| `ICMPV4_TYPE` | uint8 | ICMPv4 type. |
| `ICMPV4_CODE` | uint8 | ICMPv4 code. |
| `ICMPV6_TYPE` | uint8 | ICMPv6 type. |
| `ICMPV6_CODE` | uint8 | ICMPv6 code. |
| `FORWARDING_STATUS` | uint8 | RFC 7270 outcome code: `64..127` = forwarded, `128..191` = dropped, `192..255` = consumed. |

## Timestamps

| Field | Type | Description |
|---|---|---|
| `FLOW_START_USEC` | uint64 | Microseconds since epoch. From v5/v7 first-switched + sysUptime; from v9 first-switched normalised against system init time; from IPFIX `flowStartMicroseconds` family. Not populated for sFlow. |
| `FLOW_END_USEC` | uint64 | Microseconds since epoch. Same sources. Not populated for sFlow. |
| `OBSERVATION_TIME_MILLIS` | uint64 | IPFIX observation time (`observationTimeMilliseconds`). |

## Geolocation (enrichment-only)

| Field | Type | Description |
|---|---|---|
| `SRC_COUNTRY` | string | ISO 3166 country code. |
| `DST_COUNTRY` | string | ISO 3166 country code. |
| `SRC_GEO_STATE` | string | State / province. |
| `DST_GEO_STATE` | string | State / province. |
| `SRC_GEO_CITY` | string | City. |
| `DST_GEO_CITY` | string | City. |
| `SRC_GEO_LATITUDE` | string | Decimal latitude (string-encoded). Hidden in tables by default. |
| `DST_GEO_LATITUDE` | string | Decimal latitude. |
| `SRC_GEO_LONGITUDE` | string | Decimal longitude. |
| `DST_GEO_LONGITUDE` | string | Decimal longitude. |

City, latitude, and longitude are **not preserved in the rollup tiers** (1m, 5m, 1h). Aggregating on them forces the query to tier 0 (raw). Country and state survive into rollups.

## Network labels (enrichment-only)

These are the labels you assign to your own networks via static-metadata or network-sources configuration. The decoder never fills them.

| Field | Type | Description |
|---|---|---|
| `SRC_NET_NAME` | string | Friendly name for the source network. |
| `DST_NET_NAME` | string | Friendly name for the destination network. |
| `SRC_NET_ROLE` | string | Role tag (e.g., `dmz`, `office`, `printing`, `iot`). |
| `DST_NET_ROLE` | string | Role tag. |
| `SRC_NET_SITE` | string | Physical site (e.g., `dc-fra1`). |
| `DST_NET_SITE` | string | Physical site. |
| `SRC_NET_REGION` | string | Region (e.g., `eu`, `us-east`). |
| `DST_NET_REGION` | string | Region. |
| `SRC_NET_TENANT` | string | Tenant (multi-tenant deployments). |
| `DST_NET_TENANT` | string | Tenant. |

## Exporter labels (enrichment-only)

Labels you attach to your exporters via static-metadata or classifiers.

| Field | Type | Description |
|---|---|---|
| `EXPORTER_NAME` | string | Friendly name. Falls back to an IP-derived string if no enrichment match. |
| `EXPORTER_GROUP` | string | Group tag. |
| `EXPORTER_ROLE` | string | Role tag (e.g., `edge`, `core`, `wan`). |
| `EXPORTER_SITE` | string | Site tag. |
| `EXPORTER_REGION` | string | Region tag. |
| `EXPORTER_TENANT` | string | Tenant tag. |

## Per-protocol availability summary

For exporter-derived fields (not enrichment), the protocols differ. The shortest version:

- **NetFlow v5**: IPv4 5-tuple, AS, interfaces, next-hop, IPTOS, TCP flags, bytes, packets, sampling rate (header), first/last switched timestamps. No IPv6, MAC, VLAN, NAT, ICMP, MPLS.
- **NetFlow v7**: same as v5 minus the sampling rate.
- **NetFlow v9**: depends on the template. Theoretically all the IEs Netdata maps (see [the IPFIX/v9 IE map](#what-ies-are-mapped) below). IPv6 supported.
- **IPFIX**: superset of v9. Adds biflow (initiator/responder counters and `reverseInformationElement` IEs). Wider IE coverage. ICMP type and code as separate IEs.
- **sFlow v5**: depends on which sFlow record types the agent emits. From `SampledHeader` you get most fields after parsing the truncated packet (Ethernet/IPv4/IPv6/TCP/UDP/ICMP/MPLS). VLANs come only from `ExtendedSwitch`. AS path and BGP communities come from `ExtendedGateway`. Counter samples are dropped.

## What IEs are mapped

For NetFlow v9 and IPFIX, only specific Information Elements end up in flow-record fields. The rest of the template is parsed (so the decoder can walk past them) but the values are dropped.

The mapped IEs cover the standard set: identity (8/12/27/28, 7/11), counters (1/2/23/24/231/232/298/299), interfaces (10/14/252/253), protocol (4/5/6), ToS/DSCP (5/55), TTL (52/192), VLANs (58/59/243/254), MACs (56/80/57/81), NAT (225/226/281/282/227/228), AS (16/17), prefixes (44/45), masks (9/13/29/30), MPLS (70-79), ICMP (32/176-179, 139), fragmentation (54/88), IPv6 flow label (31), forwarding status (89), direction (61/239), sampling (34/50/305/306), timestamps (21/22/152/153/322 and the seconds/microseconds variants), and the data-link section for decapsulation (315).

Vendor enterprise IEs are recognised only for one Juniper case (PEN 2636 `commonPropertiesId`) used to surface forwarding status. Cisco AVC, Cisco NEL/NSEL NAT events, and similar vendor-private fields are parsed (so the decoder doesn't fail) but their values are not exposed in flow records.

If you need a specific IE mapped, open an issue with sample fixtures.

## Filtering and aggregation hints

Some fields are queryable but not aggregatable:

- `BYTES`, `PACKETS`, `FLOWS`, `RAW_BYTES`, `RAW_PACKETS`, `SAMPLING_RATE` — these are sums in tables and sankeys; you cannot filter or group-by them.
- `FLOW_START_USEC`, `FLOW_END_USEC`, `OBSERVATION_TIME_MILLIS` — timestamps, used by the time-range picker; not used as facets.
- The four geo-coordinate fields (`SRC_GEO_LATITUDE/LONGITUDE`, `DST_GEO_LATITUDE/LONGITUDE`) are stored but hidden in the table by default and not exposed as facets.

The dashboard also exposes two **virtual facets** that don't exist in the canonical schema:

- `ICMPV4` — a synthesised string from `ICMPV4_TYPE` and `ICMPV4_CODE`, useful for filtering ICMPv4 messages by their named type/code combination (e.g., "echo-request").
- `ICMPV6` — same for ICMPv6.

Filtering on either of these virtual fields runs against the underlying `*_TYPE` and `*_CODE` fields.

## A note on field counts

You may see "89 fields" or "91 fields" in different parts of the codebase. The current canonical list has **91 entries**. The schema has grown over time and not every reference has caught up. The list above is exhaustive for the current release.

## What's next

- [Configuration](/docs/network-flows/configuration.md) — `netflow.yaml` reference.
- [Retention and Querying](/docs/network-flows/retention-querying.md) — How the four tiers store data and which fields they preserve.
- [Visualisation](/docs/network-flows/visualization/summary-sankey.md) — Reading the dashboard.
- [Validation and Data Quality](/docs/network-flows/validation.md) — How to know your data is right.
