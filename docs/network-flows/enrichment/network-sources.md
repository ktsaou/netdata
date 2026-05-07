<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/enrichment/network-sources.md"
sidebar_label: "Network Sources"
learn_status: "Published"
learn_rel_path: "Network Flows/Enrichment"
keywords: ['network sources', 'enrichment', 'http fetch', 'ipam', 'aws', 'gcp', 'azure']
endmeta-->

# Network sources

Network sources are HTTP-fetched feeds that produce the same kind of network labels as the [static `networks` block](/docs/network-flows/enrichment/static-metadata.md) — but pulled from a remote endpoint at a configurable interval, and shaped through a `jq` transform so the response can match many input formats.

Use this when:

- You have an IPAM (NetBox, BlueCat, Infoblox, an internal service) that already knows your prefixes and their attributes.
- You want to consume cloud-provider published IP ranges (AWS, GCP, Azure) to label traffic to/from those clouds without maintaining static prefix lists.
- You have a federated configuration source — multiple Netdata Agents pulling from one IPAM endpoint stays consistent automatically.

:::warning Integration test gap
The jq transform layer and the YAML configuration are unit-tested. The HTTP fetch loop — connecting to the remote endpoint, handling 4xx/5xx responses, parsing malformed JSON, retry/backoff — is **not** integration-tested. The feature ships because the transform is well-tested and the HTTP client is standard reqwest, but the fetch path has not been validated end-to-end against a panel of real endpoints in this repository's tests.
:::

## What it populates

Same field set as static `networks`:

`SRC_NET_NAME` / `DST_NET_NAME`, `SRC_NET_ROLE` / `DST_NET_ROLE`, `SRC_NET_SITE` / `DST_NET_SITE`, `SRC_NET_REGION` / `DST_NET_REGION`, `SRC_NET_TENANT` / `DST_NET_TENANT`, `SRC_COUNTRY` / `DST_COUNTRY`, `SRC_GEO_STATE` / `DST_GEO_STATE`, `SRC_GEO_CITY` / `DST_GEO_CITY`.

Plus, when supplied: `SRC_AS` / `DST_AS` overrides via the per-row `asn` field, and `SRC_AS_NAME` / `DST_AS_NAME` via the per-row `asn_name` field.

What network sources cannot set: `SRC_GEO_LATITUDE` / `DST_GEO_LATITUDE`, `SRC_GEO_LONGITUDE` / `DST_GEO_LONGITUDE`. Coordinates are static-only — use the `networks` block for those.

## How a fetch works

For each configured source:

1. The plugin issues an HTTP request (default GET, or POST if configured) at the `interval` cadence.
2. Headers configured under `headers:` are added to the request, plus an automatic `Accept: application/json`.
3. The response body is parsed as JSON.
4. The configured `transform` (a [jaq](https://github.com/01mf02/jaq) jq-equivalent expression) runs over the parsed JSON.
5. The transform must produce a stream of objects, each with a `prefix` field (a CIDR string) and any of the optional attribute fields.
6. The records are merged into the network-attributes trie.

Each source runs in its own task. Multiple sources fetch in parallel; within a source, only one fetch is in flight at a time.

On any failure (HTTP error, JSON parse error, jq runtime error, empty result), the source backs off exponentially (starting at `interval / 10`, doubling up to `interval`) and retries. On success it resets to the configured `interval`.

## Configuration

```yaml
enrichment:
  network_sources:
    aws_ip_ranges:
      url: "https://ip-ranges.amazonaws.com/ip-ranges.json"
      method: GET
      interval: 24h
      timeout: 60s
      transform: |
        (.prefixes + .ipv6_prefixes)[] | {
          prefix: (.ip_prefix // .ipv6_prefix),
          tenant: "amazon",
          region: .region,
          role: (.service | ascii_downcase)
        }
      headers: {}
      proxy: true
      tls:
        enable: false
        ca_file: ""
        cert_file: ""
        key_file: ""
```

Per-source keys:

| Key | Type | Default | Notes |
|---|---|---|---|
| `url` | string | (required) | HTTP/HTTPS endpoint |
| `method` | string | `GET` | `GET` or `POST` only |
| `interval` | duration | `60s` | Fetch cadence (the loop floors at 60s) |
| `timeout` | duration | `60s` | Per-request timeout |
| `transform` | string (jq) | `"."` | jaq expression applied to the parsed body |
| `headers` | map<string,string> | `{}` | Extra request headers |
| `proxy` | bool | `true` | When `false`, ignores `HTTP(S)_PROXY` env vars |
| `tls.enable` | bool | `false` | Enable custom TLS settings (custom CA, mTLS) |
| `tls.ca_file` | path | `""` | PEM bundle for verifying the server certificate |
| `tls.cert_file` | path | `""` | PEM client certificate for mTLS |
| `tls.key_file` | path | `""` | PEM private key for mTLS (or empty to read from `cert_file`) |

### TLS verification cannot be disabled

The configuration accepts the legacy keys `tls.verify` and `tls.skip_verify` for compatibility, but the validation layer **rejects** any attempt to disable verification (`tls.verify: false` or `tls.skip_verify: true`). Self-signed or internal CAs must be supplied via `tls.ca_file`. There is no override.

This is deliberate. Network-source data flows directly into enrichment that affects security investigations and capacity decisions — silently accepting MITM-able responses would corrupt every downstream analysis.

### Single page only

The fetch is one-shot per cycle. There is no pagination, no cursor handling, no `Link: rel=next` following. If your IPAM exposes paginated endpoints, either:

- Expose a separate "all prefixes" bulk endpoint (most IPAMs have one).
- Wrap with a server-side script that aggregates all pages and serves the result at one URL.

### Authentication

Set the necessary headers explicitly:

```yaml
headers:
  Authorization: "Token abc123"
```

The plugin does not have built-in OAuth flow, basic-auth helpers, or token refresh. If your endpoint needs short-lived tokens, refresh them outside Netdata and put the current valid token in the headers config (and reload).

## The transform

The `transform` is a jq expression compiled by [jaq](https://github.com/01mf02/jaq). It receives the entire parsed JSON body and must produce a **stream of objects**.

Each output object should look like:

```json
{
  "prefix": "10.0.0.0/8",
  "name": "internal",
  "role": "lan",
  "site": "fra1",
  "region": "eu-central",
  "country": "DE",
  "state": "HE",
  "city": "Frankfurt",
  "tenant": "tenant-a",
  "asn": 64500,
  "asn_name": "Internal AS"
}
```

Required: `prefix`. All other fields are optional and default to empty / 0.

The `asn` field accepts an integer (`64500`), a string (`"64500"`), or the AS notation (`"AS64500"`).

If the transform produces nothing (empty result), the cycle is treated as a failure and triggers backoff. The same applies to non-object rows — every output element must be an object.

If the transform fails to compile at startup (or at source spawn), the source is disabled with a warning in the journal. The plugin continues without it.

## Examples

### NetBox IPAM

```yaml
enrichment:
  network_sources:
    netbox:
      url: "https://netbox.example.internal/api/ipam/prefixes/?limit=1000"
      headers:
        Authorization: "Token abcdef0123"
      interval: 5m
      timeout: 30s
      transform: |
        .results[] | {
          prefix: .prefix,
          tenant: (.tenant.name // ""),
          site: (.site.name // ""),
          role: (.role.name // ""),
          name: .description
        }
```

### AWS published IP ranges

```yaml
enrichment:
  network_sources:
    aws:
      url: "https://ip-ranges.amazonaws.com/ip-ranges.json"
      interval: 24h
      transform: |
        (.prefixes + .ipv6_prefixes)[] | {
          prefix: (.ip_prefix // .ipv6_prefix),
          tenant: "amazon",
          region: .region,
          role: (.service | ascii_downcase)
        }
```

### GCP published IP ranges

```yaml
enrichment:
  network_sources:
    gcp:
      url: "https://www.gstatic.com/ipranges/cloud.json"
      interval: 24h
      transform: |
        .prefixes[] | {
          prefix: (.ipv4Prefix // .ipv6Prefix),
          tenant: "gcp",
          region: .scope,
          role: .service
        }
```

### Internal IPAM with mTLS

```yaml
enrichment:
  network_sources:
    corp_ipam:
      url: "https://ipam.corp/api/networks"
      tls:
        enable: true
        ca_file: /etc/netdata/ssl/corp-ca.pem
        cert_file: /etc/netdata/ssl/netdata.crt
        key_file: /etc/netdata/ssl/netdata.key
      interval: 10m
```

## Lookup priority

In the network-attributes resolution merge order:

1. **GeoIP** seeds the base layer.
2. **Network sources** merge on top — at each prefix length (least-specific to most-specific).
3. **Static `networks` config** merges last — at each prefix length, **after** network sources.

So when a prefix is defined in both a network source and the static config, the static config wins on any non-empty field. This is intentional: explicit operator configuration overrides imported data.

## Things that go wrong

- **Endpoint is paginated.** Only the first page is fetched. Use a bulk endpoint or wrap with a server-side script.
- **Default interval is 60 s.** Fast for an IPAM, slow for AWS/GCP ranges. Tune per source.
- **TLS verify cannot be disabled.** Use `tls.ca_file` for internal CAs.
- **Empty result from the transform** is treated as failure. If your endpoint returns no prefixes (legitimate state for a quiet IPAM), the source backs off as if it errored. Workaround: have the upstream return at least one synthetic prefix.
- **Authorization header must be in `headers:`**, not in the URL. URLs with embedded credentials (`https://user:pass@host`) are not specially handled.
- **JSON parse errors are silent in the dashboard.** Watch the Netdata journal (`journalctl -u netdata | grep network_sources`) for warnings.

## What's next

- [Static metadata](/docs/network-flows/enrichment/static-metadata.md) — Static `networks` block (overrides network sources at the same prefix length).
- [GeoIP](/docs/network-flows/enrichment/geoip.md) — The base layer that network sources merge on top of.
- [ASN resolution](/docs/network-flows/enrichment/asn-resolution.md) — How the per-row `asn` field plugs in.
