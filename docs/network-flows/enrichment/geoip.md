<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/enrichment/geoip.md"
sidebar_label: "GeoIP"
learn_status: "Published"
learn_rel_path: "Network Flows/Enrichment"
keywords: ['geoip', 'mmdb', 'maxmind', 'dbip', 'iptoasn', 'enrichment']
endmeta-->

# GeoIP enrichment

GeoIP enrichment adds country, state, city, coordinates, and AS-name labels to every flow record by looking up the source and destination IPs against IP-intelligence databases on disk. The dashboard uses these fields for the country / state / city maps, the globe view, and the AS-name columns in tables and Sankey diagrams.

## What it populates

| Field | Source |
|---|---|
| `SRC_COUNTRY` / `DST_COUNTRY` | Geo database |
| `SRC_GEO_STATE` / `DST_GEO_STATE` | Geo database (subdivisions) |
| `SRC_GEO_CITY` / `DST_GEO_CITY` | Geo database |
| `SRC_GEO_LATITUDE` / `DST_GEO_LATITUDE` | Geo database |
| `SRC_GEO_LONGITUDE` / `DST_GEO_LONGITUDE` | Geo database |
| `SRC_AS` / `DST_AS` (when populated by the chain) | ASN database (see [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md)) |
| `SRC_AS_NAME` / `DST_AS_NAME` | ASN database (always — irrespective of the provider chain) |

City, latitude, and longitude are not preserved into the rollup tiers (1m / 5m / 1h). Country and state are. See [Retention and Querying](/docs/network-flows/retention-querying.md) for the full list.

## Database format

Netdata reads the **MMDB** format. MMDB is the binary format MaxMind invented; it is also used by other providers (DB-IP, IPInfo, custom internal builds). The plugin does not require MaxMind specifically — any provider that ships an MMDB compatible with the standard schema (`country.iso_code`, `city.names.en`, `subdivisions[].iso_code`, `location.latitude`, `location.longitude`, `autonomous_system_number`, `autonomous_system_organization`) works.

Two databases, with different roles:

- **`asn_database`** — feeds AS number and AS name lookups.
- **`geo_database`** — feeds country / state / city / latitude / longitude.

Either can be enabled independently. Either can be a list — when you list multiple, later entries overwrite earlier ones for non-empty fields.

## What ships out of the box

Netdata's native packages (DEB, RPM) include a stock MMDB payload built from public-domain sources:

- **DB-IP `dbip-asn-lite`** — for AS resolution
- **DB-IP `dbip-city-lite`** — for geographic resolution

The stock files live at `/usr/share/netdata/topology-ip-intel/topology-ip-{asn,geo}.mmdb`. They are loaded automatically — you do not need to configure anything.

These are NOT MaxMind databases. They are DB-IP's free-tier databases in MMDB format. Quality is comparable to MaxMind GeoLite2 for most regions; coverage of newly-allocated prefixes can lag MaxMind by a few days.

## Refreshing the databases

Netdata ships a binary at `/usr/sbin/topology-ip-intel-downloader` that fetches and rebuilds the local cache copy of the databases. The plugin auto-detects the cache copy first, falling back to the stock copy.

```bash
sudo /usr/sbin/topology-ip-intel-downloader
```

The downloader writes to `/var/cache/netdata/topology-ip-intel/`:

- `topology-ip-asn.mmdb`
- `topology-ip-geo.mmdb`
- `topology-ip-intel.json` (metadata about the run)

Atomic publish via `rename()` — readers never see a half-written file.

The downloader is **not scheduled by Netdata**. You decide how often it runs. A typical setup is a weekly cron or systemd timer:

```bash
# /etc/cron.d/netdata-topology-ip-intel
30 4 * * 0 root /usr/sbin/topology-ip-intel-downloader
```

DB-IP releases new MMDBs monthly. IPtoASN releases ASN data daily. Anything between weekly and monthly is fine.

### Source builds don't include the stock files

If you built Netdata from source, the stock MMDB files are NOT bundled. Run the downloader once to populate the cache directory before flow data starts arriving — otherwise no GeoIP enrichment will happen.

```bash
sudo /usr/sbin/topology-ip-intel-downloader
```

## Available providers and artifacts

The downloader supports two built-in providers:

| Provider | Artifact | Family | Format | Source |
|---|---|---|---|---|
| `dbip` | `asn-lite` | ASN | MMDB, CSV | DB-IP free tier |
| `dbip` | `country-lite` | Geo | MMDB, CSV | DB-IP free tier |
| `dbip` | `city-lite` | Geo | MMDB, CSV | DB-IP free tier |
| `iptoasn` | `combined` | ASN | TSV | iptoasn.com |

You can mix providers per family (use one for ASN, another for Geo) and provide multiple sources per family — the first source wins on overlap.

### Selecting providers

```bash
# Default: DB-IP for both
/usr/sbin/topology-ip-intel-downloader

# Use IPtoASN for ASN, DB-IP for Geo
/usr/sbin/topology-ip-intel-downloader \
  --asn iptoasn:combined \
  --geo dbip:city-lite

# Country-only Geo (smaller, faster lookups)
/usr/sbin/topology-ip-intel-downloader --geo dbip:country-lite

# Disable Geo entirely
/usr/sbin/topology-ip-intel-downloader --no-geo
```

### Custom databases

You can point the plugin at any MMDB file you trust — including MaxMind GeoLite2/GeoIP2 (BYO license), IPInfo, or your own internal build:

```yaml
enrichment:
  geoip:
    asn_database:
      - /usr/share/GeoIP/GeoLite2-ASN.mmdb
    geo_database:
      - /usr/share/GeoIP/GeoLite2-City.mmdb
    optional: false
```

The MMDB schema must expose at least the standard fields the plugin reads. Vendor-specific extra fields are ignored. URLs and remote fetches are not supported on the plugin side — the downloader has its own URL/path support if you want a managed pipeline; otherwise drop the file in place yourself.

## Configuration

```yaml
enrichment:
  geoip:
    asn_database: []          # list of MMDB paths; empty = auto-detect
    geo_database: []          # list of MMDB paths; empty = auto-detect
    optional: false           # whether missing files are fatal at startup
```

`optional: false` (the default for explicit-path configs) makes the plugin fail to start when a configured database is missing or unreadable. `optional: true` reduces it to a warning. Auto-detected databases get `optional: true` automatically — disappearing stock or cache files don't crash the plugin.

The aliases `asn-database`, `geo-database`, and `country-database` (legacy) are accepted for the keys.

## Hot reload

The plugin polls the database files every 30 seconds and reloads automatically when the file mtime or size changes. There is no need to restart the agent after refreshing the MMDBs — the downloader's atomic publish + the plugin's polling means new data picks up within ~30 seconds.

If a hot-reload fails (corrupted file, parse error), the plugin keeps the previous reader and logs a warning. The retry window stays open, so the next reload attempt happens at the next polling tick.

## Internal IPs and the "random country" trap

The plugin does NOT skip GeoIP lookups for private IPs (RFC 1918 / RFC 6598 / link-local / ULA). It just hands the IP to the MMDB and uses whatever comes back.

For the **stock DB-IP files**, the downloader pre-tags private and loopback ranges with a `netdata.ip_class = "private"` flag, which the plugin uses to render `AS_NAME` as `AS0 Private IP Address Space` instead of `AS0 Unknown ASN`. Country / city / lat / lon for these ranges are typically empty.

For **third-party MMDBs**, behaviour depends on the database. Some MaxMind GeoLite2 builds return spurious country data for RFC 1918 addresses. If you see "internal traffic appearing in random countries" on the maps, this is the cause.

The fix is in [Static metadata](/docs/network-flows/enrichment/static-metadata.md): declare your internal CIDRs explicitly under `enrichment.networks`, and the network attributes you set there override whatever GeoIP returns.

## Reading the AS-name fallbacks

| Resolved AS | `*_AS_NAME` value |
|---|---|
| `0`, IP marked `private` by the MMDB | `AS0 Private IP Address Space` |
| `0`, no other info | `AS0 Unknown ASN` |
| Non-zero, MMDB has a name | `AS{n} {name}` (e.g. `AS15169 Google LLC`) |
| Non-zero, MMDB has no name | `AS{n}` |

These strings exist so the dashboard never shows blank cells for the AS-name field.

## What can go wrong

- **No MMDB found at startup.** Auto-detect failed and you have no explicit paths. The plugin starts but no GeoIP enrichment happens — country, city, AS name fields are empty. Check `journalctl -u netdata | grep geoip` for an "auto-detected" or "no databases" line.
- **MMDB exists but is the wrong schema.** Lookups silently return empty. Check the MMDB with `mmdblookup` to confirm the standard fields are present.
- **Stale MMDB.** No built-in alert. Plugin doesn't tell you. Set up your own monitoring on the file mtime if your data freshness matters — for example, alert if the file is older than 60 days.
- **Internal IPs showing wrong country.** The MMDB returned data for a private range. Declare the range under `enrichment.networks` to override.
- **Hot-reload didn't pick up new data.** Wait 30 seconds. If still not loaded, check the journal for a "reload skipped" message.

## What's next

- [Static metadata](/docs/network-flows/enrichment/static-metadata.md) — Declare internal networks and override per-prefix attributes.
- [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md) — How AS numbers and names get resolved across the provider chain.
- [Validation and Data Quality](/docs/network-flows/validation.md) — How to confirm enrichment is working.
