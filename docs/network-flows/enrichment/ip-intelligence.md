<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/enrichment/ip-intelligence.md"
sidebar_label: "IP Intelligence"
learn_status: "Published"
learn_rel_path: "Network Flows/Flows Enrichment"
keywords: ['geoip', 'asn', 'ip intelligence', 'country', 'city', 'mmdb', 'enrichment']
endmeta-->

# IP Intelligence

IP intelligence is the family of post-decode lookups that turns raw IP addresses into operationally useful labels: countries, states, cities, coordinates, AS numbers, AS names. Two MaxMind-format MMDB files drive it — one ASN database, one geographic database. The plugin loads them once at startup and refreshes them in place every 30 seconds when the underlying files change.

The actual database content comes from a provider you configure: DB-IP (the default that Netdata installs out of the box), MaxMind GeoIP/GeoLite2, IPtoASN, or a custom MMDB build of your own. Each provider has its own integration card with installation steps and refresh cadence. This page covers the enrichment **mechanism** — how the lookups feed into flow records, what each MMDB contributes, and what happens when a database is missing or stale.

## Fields populated

Every flow record gets two passes — one against the ASN database, one against the geo database — for both the source and destination IP. Successful lookups populate:

| Field | Source MMDB | Notes |
|---|---|---|
| `SRC_AS_NAME`, `DST_AS_NAME` | ASN | Rendered as `AS{number} {organisation}`. Falls back to `AS0 Unknown ASN` (or `AS0 Private IP Address Space` when the database tagged the prefix as private) when the lookup misses. The AS *number* itself comes from the [ASN provider chain](/docs/network-flows/enrichment/asn-resolution.md), not directly from the MMDB. |
| `SRC_COUNTRY`, `DST_COUNTRY` | Geo | ISO 3166 country code. Overridable per-prefix via [static metadata](/docs/network-flows/enrichment/static-metadata.md) `enrichment.networks.<cidr>.country`. |
| `SRC_GEO_STATE`, `DST_GEO_STATE` | Geo | State / province / subdivision. Preserved into all four journal tiers. |
| `SRC_GEO_CITY`, `DST_GEO_CITY` | Geo | City. **Raw tier only** — the rollup tiers drop city to keep cardinality manageable. |
| `SRC_GEO_LATITUDE`, `DST_GEO_LATITUDE` | Geo | Decimal latitude. **Raw tier only.** Hidden in the default table view. |
| `SRC_GEO_LONGITUDE`, `DST_GEO_LONGITUDE` | Geo | Decimal longitude. **Raw tier only.** Hidden in the default table view. |

The country / state pair survives into rollups because cardinality is bounded (~250 countries, low thousands of subdivisions). City and coordinates explode cardinality and only live in the raw tier, which is why the city map and globe queries automatically constrain themselves to the raw tier.

## Configuration

```yaml
enrichment:
  geoip:
    asn_database:
      - /var/cache/netdata/topology-ip-intel/topology-ip-asn.mmdb
    geo_database:
      - /var/cache/netdata/topology-ip-intel/topology-ip-geo.mmdb
    optional: true
```

| Key | Type | Notes |
|---|---|---|
| `asn_database` | list of paths | One or more ASN MMDB files. Multiple files compose: each lookup tries every database, last match wins per field. |
| `geo_database` | list of paths | One or more geographic MMDB files. Same composition rule. Aliases: `geo-database`, `country-database`. |
| `optional` | bool, default `false` | When `true`, missing or unreadable files at startup are tolerated (the resolver starts with no databases). When `false`, a missing file aborts startup. The auto-detected files described below are always treated as `optional: true`. |

If you provide nothing, the plugin auto-detects the MMDB files at startup:

1. `<cache_dir>/topology-ip-intel/topology-ip-asn.mmdb` and `topology-ip-geo.mmdb` (where `cache_dir` is the parent of the configured `journal.journal_dir`, or `NETDATA_CACHE_DIR`, typically `/var/cache/netdata`).
2. `<stock_data_dir>/topology-ip-intel/...` as a secondary location.

A stock Netdata install populates the cache directory with DB-IP MMDB files. See the [DB-IP IP Intelligence](/src/crates/netflow-plugin/integrations/db-ip_ip_intelligence.md) integration card for the installer and refresh schedule.

If you want a different provider, configure `asn_database` / `geo_database` explicitly. The plugin reads MaxMind format (`.mmdb`); GeoLite2 and DB-IP both ship in this format. See:

- [DB-IP IP Intelligence](/src/crates/netflow-plugin/integrations/db-ip_ip_intelligence.md) — the default.
- [MaxMind GeoIP / GeoLite2](/src/crates/netflow-plugin/integrations/maxmind_geoip_-_geolite2.md) — commercial / free with attribution.
- [IPtoASN](/src/crates/netflow-plugin/integrations/iptoasn.md) — public ASN-only feed.
- [Custom MMDB Database](/src/crates/netflow-plugin/integrations/custom_mmdb_database.md) — your own MMDB build.

## Refresh

The resolver checks the file signature (size + mtime per configured path) every 30 seconds. If the signature changed, it re-loads the readers in place. Only successful re-loads swap the active databases — a transient read error keeps the previous readers serving lookups.

This means you can update the MMDB files on disk without restarting the plugin. The provider integration cards' refresh scripts rely on this: a cron job downloads the new file, atomically replaces the existing one, and the plugin picks it up within 30 seconds.

## Lookup order

Per flow record, per IP (source and destination):

1. **ASN database scan** — for every configured ASN MMDB, the plugin runs the lookup and updates `asn`, `asn_name`, and the private/reserved flag (`ip_class`) from the matching record. Multiple ASN databases compose: the last successful match wins per field.
2. **Geo database scan** — same procedure for the geographic databases. Country, state, city, coordinates updated.
3. **Result merging** — the resulting attributes feed into the broader [ASN resolution chain](/docs/network-flows/enrichment/asn-resolution.md) (which decides whether to use the MMDB AS number, the flow record's AS number, or BGP-derived data) and the [static metadata / network sources](/docs/network-flows/enrichment/static-metadata.md) overlay (which can override country, AS number, and other fields per CIDR).

The ASN MMDB also populates the AS *name* at the very end of the chain, regardless of which provider gave the AS *number*. So even if your `asn_providers` chain prefers BGP over the MMDB for the number, the friendly name still comes from the ASN database.

## Things to know

### Private IPs render as `AS0 Private IP Address Space`

The DB-IP-built ASN database tags RFC 1918 / RFC 6598 / link-local ranges with a private flag. The plugin reads that flag and renders the AS name accordingly. Public IPs that the database doesn't recognise render as `AS0 Unknown ASN` instead. This is purely a labelling distinction; both come out as AS number `0`.

### IPv6 lookups against an IPv4-only database are skipped

The resolver inspects each database's metadata. An IPv6 lookup against an IPv4-only file is silently skipped (no warning, no error). If you mix IPv4 and IPv6 traffic, configure both an IPv4 ASN/geo file and a separate IPv6 file, or use a database that covers both address families.

### Database staleness drift over weeks

ASN ownership data changes — companies merge, prefixes get reassigned, new ASNs appear. A database older than a quarter or two will start labelling reassigned prefixes with the previous owner. There's no in-process signal for "your MMDB is too old"; refresh on the cadence the provider recommends (DB-IP and GeoLite2 ship monthly).

### Geographic accuracy is best-effort

City-level GeoIP is accurate for the public IP space but can be wildly wrong for VPNs, mobile carriers, and cloud-provider egress IPs (which often resolve to the cloud region's central city, not the actual user). Use country and state for trends; use city only when you've validated it for the prefix you care about.

## Failure modes

| Symptom | Cause | Fix |
|---|---|---|
| Plugin won't start: "failed to load enrichment.geoip.\<asn or geo\>_database" | Configured path missing or unreadable; `optional: false` | Set `optional: true`, fix the path, or unset the database to fall back to auto-detect. |
| Country / city columns empty for everything | No geo database loaded | Check the plugin logs at startup; verify the MMDB file exists and is readable by the netdata user. |
| `AS0 Unknown ASN` for known public IPs | Stale ASN MMDB; or the configured database doesn't cover that prefix | Refresh the MMDB; or add a database that covers the missing prefix. |
| AS name shows `AS{n}` with no organisation | The ASN MMDB doesn't have a name string for that AS number | Use a richer ASN MMDB (the GeoLite2 ASN feed has more coverage than the IPtoASN minimal feed). |
| Map renders empty over a long window | City / lat / lng are raw-tier-only; the query auto-fell-back to a rollup tier | Narrow the time range so the query fits the raw tier, or use the country / state map (those survive into rollups). |

## What's next

- [DB-IP IP Intelligence](/src/crates/netflow-plugin/integrations/db-ip_ip_intelligence.md), [MaxMind GeoIP / GeoLite2](/src/crates/netflow-plugin/integrations/maxmind_geoip_-_geolite2.md), [IPtoASN](/src/crates/netflow-plugin/integrations/iptoasn.md), [Custom MMDB Database](/src/crates/netflow-plugin/integrations/custom_mmdb_database.md) — provider integration cards with installation and refresh steps.
- [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md) — the chain that decides whether the AS number comes from the MMDB, the flow record itself, or BGP.
- [Static metadata](/docs/network-flows/enrichment/static-metadata.md) — per-CIDR overrides for country, AS number, and other labels.
- [Network identity](/docs/network-flows/enrichment/network-identity.md) — labelling your own networks on top of IP intelligence.
