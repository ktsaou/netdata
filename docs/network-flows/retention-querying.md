<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/retention-querying.md"
sidebar_label: "Retention and Tiers"
learn_status: "Published"
learn_rel_path: "Network Flows"
keywords: ['retention', 'tiers', 'rollup', 'tier selection']
endmeta-->

# Retention and Tiers

Netdata stores flow data in four tiers. The tier model is transparent — you do not pick a tier when you query, the dashboard picks for you. Understanding how it picks helps you interpret what you're seeing and avoid surprises when older data isn't there.

For the configuration surface (per-tier `size_of_journal_files` and `duration_of_journal_files`), see [Configuration → Per-tier retention](/docs/network-flows/configuration.md#per-tier-retention). For the query semantics (group-by limits, full-text search, URL sharing, dashboard query parameters), see [Visualization → Overview](/docs/network-flows/visualization/overview.md).

## The four tiers

| Tier | Bucket | On-disk dir | YAML key |
|---|---|---|---|
| Raw | per-flow | `flows/raw/` | `raw` |
| 1-minute | 60 s | `flows/1m/` | `minute_1` |
| 5-minute | 300 s | `flows/5m/` | `minute_5` |
| 1-hour | 3600 s | `flows/1h/` | `hour_1` |

The raw tier stores every flow record as it arrived. The other three are rollup tiers — they aggregate raw flows into time-bucketed groups by identity (exporter, ASN, country, ports — see below).

## What survives the rollup

Rollup tiers (1m, 5m, 1h) deliberately drop a few fields to keep cardinality manageable. **The dropped fields are: `SRC_ADDR`, `DST_ADDR`, `SRC_PORT`, `DST_PORT`, `SRC_GEO_CITY`, `DST_GEO_CITY`, `SRC_GEO_LATITUDE`, `DST_GEO_LATITUDE`, `SRC_GEO_LONGITUDE`, `DST_GEO_LONGITUDE`.**

Everything else survives — country, state, ASN, AS path, BGP communities, exporter and interface labels, protocol, TCP flags, ToS/DSCP, ICMP type/code, MPLS labels, VLANs, MACs, next-hop, post-NAT addresses, and the bytes/packets sums. So rollups are perfectly fine for most country / ASN / interface / protocol questions, but useless if you need to ask "which IP".

This is why filtering or grouping by IP/port/city/lat/lon forces the query to the raw tier — there is no other tier that has those fields.

For the per-field tier-preservation matrix, see [Field Reference](/docs/network-flows/field-reference.md).

## How the dashboard picks a tier

For every query the dashboard sends to the plugin, the planner makes a single decision: which tier (or tiers) can satisfy this?

**Rules:**

1. **Any IP/port/city/lat/lon filter or group-by → raw tier.** No exception. The rollup tiers don't have those fields.
2. **A non-empty full-text search → raw tier.** Full-text search runs as a regex against the raw journal payload, which only the raw tier carries.
3. **Otherwise, pick the coarsest tier that satisfies the time range and bucket-count requirement.**
   - Time-Series view needs at least 100 buckets in the window. So:
     - under 100 minutes → 1-minute tier
     - 100 minutes to 8h20m → 5-minute tier
     - 8h20m and longer → 1-hour tier
   - Table / Sankey / Maps have no bucket-count constraint; tier alignment alone drives the choice.

When the planner picks a tier and the time range crosses tier-aligned boundaries, the query is **stitched** — head fragment in a finer tier, aligned middle in the chosen tier, tail fragment in a finer tier. You don't see this; the results merge cleanly. It exists so wide windows that don't quite align to one-hour boundaries still work.

The plugin reports the chosen tier in the response stats (`query_tier` = `0`, `1`, `5`, or `60`). The dashboard uses this for diagnostic banners.

## What "no data" actually means

If you ask for a 30-day window with an IP filter and raw-tier retention is 24 hours, you get an empty response. No error, no banner reading "data has expired" — just an empty result set. The dashboard renders this as "No data".

The reason is a layered fallback in the planner: if a span asks for the raw tier and the files for that span have been rotated out, the planner tries the smaller tiers (1m, 5m, 1h), but those don't have IP fields, so they cannot satisfy a query that filters on IP. Result: the span returns no flows.

Other spans within the same query that don't need raw data may still return flows. So it's also possible to see partial coverage — half the time range filled, half empty.

For Time-Series, "no data" appears as zero values in the affected buckets, not as a special "missing" indicator. The chart still draws; the empty regions are flat lines at zero.

## What forces the raw tier in practice

Quick reference for "why is my query slow / showing less time?":

- Adding `SRC_ADDR`, `DST_ADDR`, `SRC_PORT`, or `DST_PORT` as a filter
- Adding any of those fields to the group-by
- Switching to the city map (it uses `SRC_GEO_CITY`/`DST_GEO_CITY` plus latitudes/longitudes)
- Typing anything into the global search ribbon

If you see the time depth in your dashboard suddenly shrink after you applied a filter, you've hit the raw-tier limit.

## Default retention and the most common misconfiguration

Each tier has its own `size_of_journal_files` and `duration_of_journal_files`. The built-in defaults are uniform — `10GB` and `7d` on every tier. That is rarely what you want; the whole point of having rollup tiers is to keep them around longer than raw.

A more useful production profile:

```yaml
journal:
  tiers:
    raw:
      size_of_journal_files: 200GB
      duration_of_journal_files: 24h
    minute_1:
      size_of_journal_files: 20GB
      duration_of_journal_files: 14d
    minute_5:
      size_of_journal_files: 20GB
      duration_of_journal_files: 30d
    hour_1:
      size_of_journal_files: 20GB
      duration_of_journal_files: 365d
```

This gives you 24 hours of full-detail forensics, 14 days of 1-minute trends, 30 days of 5-minute snapshots, and a year of hourly aggregates.

See [Configuration → Per-tier retention](/docs/network-flows/configuration.md#per-tier-retention) for the full schema and [Sizing and Capacity Planning](/docs/network-flows/sizing-capacity.md) for how to estimate the actual disk footprint per tier from your flow rate.

## Things that surprise people

- **An IP filter shrinks the time depth.** This is correct behaviour, but the dashboard doesn't always make it obvious. If your time range is wider than raw-tier retention, drop the IP filter to see the broader rollup data.
- **The city map can't go back as far as the country map.** The city map needs the city/lat/lon fields (raw-only); the country map only needs `SRC_COUNTRY`/`DST_COUNTRY` (preserved in rollups).
- **Tier files use short names** (`1m`, `5m`, `1h` on disk) but YAML uses the explicit names (`minute_1`, `minute_5`, `hour_1`). Mind the difference.

## What's next

- [Configuration → Per-tier retention](/docs/network-flows/configuration.md#per-tier-retention) — `netflow.yaml` schema for per-tier retention.
- [Sizing and Capacity Planning](/docs/network-flows/sizing-capacity.md) — Disk and CPU estimates from your flow rate.
- [Field Reference](/docs/network-flows/field-reference.md) — Which fields exist and which survive into rollups.
- [Visualization → Overview](/docs/network-flows/visualization/overview.md) — How the dashboard sends queries; group-by limits; full-text search; URL sharing.
