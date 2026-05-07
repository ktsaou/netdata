# SNMP Profile Projection

## Purpose

SNMP profiles are one catalog with explicit projections for their consumers.
Regular SNMP metric collection, SNMP topology, and SNMP licensing use the same
profile loading, matching, inheritance, metadata, and tag machinery, but they
consume different profile views.

## Consumers

The supported profile consumers are:

- `metrics` - regular SNMP charted metrics and virtual metrics.
- `topology` - SNMP topology observations.
- `licensing` - typed SNMP network-device license rows.

Profile metadata fields and top-level `metric_tags` default to all consumers.
They may narrow their visibility with:

```yaml
consumers: [metrics]
consumers: [topology]
consumers: [licensing]
```

Metric rows under top-level `metrics:` are regular metric rows. They are
metrics-only.

Topology rows live under top-level `topology:` and must declare a closed
`kind`.

Licensing rows live under top-level `licensing:` and emit typed license rows,
not chart metrics and not hidden underscore-prefixed metrics.

## Topology Rows

Topology rows reuse the regular `MetricsConfig` scalar/table shape:

```yaml
topology:
  - kind: lldp_rem
    table:
      OID: 1.0.8802.1.1.2.1.4.1
      name: lldpRemTable
    symbols:
      - OID: 1.0.8802.1.1.2.1.4.1.1.6
        name: lldp_rem
    metric_tags:
      - tag: lldp_loc_port_num
        index: 2
```

Topology row symbol names must not be underscore-prefixed. The historical
`_topology_*` naming convention is not a classifier.

Topology rows must not set regular metric chart/export fields on the row value
symbol: `chart_meta`, `metric_type`, `mapping`, `transform`, `scale_factor`,
`format`, or `constant_value_one`.

`systemUptime` remains in `metrics:` for regular SNMP collection. It is not a
topology kind. Topology-specific uptime acquisition is collector code, not
profile topology schema.

## Topology Kinds

The closed topology kind set is:

- `lldp_loc_port`
- `lldp_loc_man_addr`
- `lldp_rem`
- `lldp_rem_man_addr`
- `lldp_rem_man_addr_compat`
- `cdp_cache`
- `if_name`
- `if_status`
- `if_duplex`
- `ip_if_index`
- `bridge_port_if_index`
- `fdb_entry`
- `qbridge_fdb_entry`
- `qbridge_vlan_entry`
- `stp_port`
- `vtp_vlan`
- `arp_entry`
- `arp_legacy_entry`

## Licensing Rows

Licensing rows are row-centric because one license row can aggregate identity,
descriptors, state, timers, and usage signals:

```yaml
licensing:
  - id: vendor-license-group
    table:
      OID: 1.3.6.1.4.1.example.1
      name: vendorLicenseTable
    identity:
      id: { index: 1 }
      name:
        symbol:
          OID: 1.3.6.1.4.1.example.1.2
          name: vendorLicenseName
    state:
      symbol:
        OID: 1.3.6.1.4.1.example.1.3
        name: vendorLicenseState
      mapping:
        1: healthy
        2: degraded
        3: broken
    signals:
      expiry:
        timestamp:
          symbol:
            OID: 1.3.6.1.4.1.example.1.4
            name: vendorLicenseExpiry
          format: snmp_dateandtime
```

Scalar-only licensing rows are allowed. If a scalar row combines multiple
scalar signal OIDs into one license row, it must declare an explicit `id:` so
the grouping is stable. Otherwise scalar structural identity defaults to the
single scalar signal OID.

`from: <oid>` lets a typed licensing value read a sibling OID directly. For
table rows, `from` must be a peer column under the same table OID. For scalar
rows, it must be another scalar in the same profile or explicit scalar group.
Cross-profile `from` references are invalid.

Supported licensing signal fields are:

- state severity: `state`
- timers: `expiry`, `authorization`, `certificate`, `grace`
- usage: `used`, `capacity`, `available`, `percent`

Timer signals may declare either the shorthand timer value, `timestamp`, or
`remaining`. Sentinel policies are closed names and are evaluated by the typed
licensing producer before runtime consumers see the row.

## Resolve And Projection

`ddsnmp.Catalog.Resolve()` resolves profiles by `sysObjectID`, `sysDescr`, and
manual profile policy. The regular SNMP collector uses manual-profile fallback
semantics. The topology collector uses manual-profile augment semantics.

`ResolvedProfileSet.Project(metrics)` returns the regular metrics view:

- keeps `metrics`;
- keeps `virtual_metrics`;
- drops `topology`;
- filters metadata and top-level metric tags by `consumers`.

`ResolvedProfileSet.Project(topology)` returns the topology view:

- keeps `topology`;
- drops regular `metrics`;
- drops `virtual_metrics`;
- filters metadata and top-level metric tags by `consumers`.

`ResolvedProfileSet.Project(licensing)` returns the licensing view:

- keeps `licensing`;
- drops regular `metrics`;
- drops `topology`;
- drops `virtual_metrics`;
- filters metadata and top-level metric tags by `consumers`.

The regular SNMP collector uses an explicit combined metrics+licensing view so
one SNMP pass can produce charted metrics and typed license rows. Single-consumer
projections remain pure.

`ProjectedView.FilterByKind()` is a topology view filter. VLAN-context topology
uses it with the VLAN-scopable kind set instead of hardcoded topology mixin
filenames.

## Inheritance And Merge Rules

Profile inheritance must merge `topology:` and `licensing:` rows in addition to
`metrics:`, `virtual_metrics`, metadata, global metric tags, and static tags.

Topology row identity is:

```text
kind + table identity + symbol name
```

The table identity is the table name when set, otherwise the table OID. Scalar
topology rows use kind plus scalar symbol name and OID.

When a derived topology row overrides an inherited row with the same identity,
the derived row wins. Conflicting topology kinds for the same table/symbol
identity are load errors.

Cross-profile deduplication runs after profile matching because it depends on
matched-profile specificity. It must deduplicate both regular metrics and
topology/licensing rows in the resolved matched set.

Licensing structural identity is:

```text
origin profile id + table OID + row index
origin profile id + scalar signal OID
origin profile id + explicit scalar group id
```

`origin profile id` is the logical profile file that declared the licensing row,
including mixin-origin rows after `extends:` merge. It is not the root matched
profile name and not an absolute workstation path. Repeated `(structural
identity, signal kind)` entries are load errors unless a valid inheritance
override handles them.

## Delivery

Regular metrics are emitted through `ProfileMetrics.Metrics`.

Topology rows are emitted through `ProfileMetrics.TopologyMetrics` and carry
`Metric.TopologyKind`.

Licensing rows are emitted through `ProfileMetrics.LicenseRows` and carry
typed grouped fields for identity, descriptors, state, timers, usage, tags,
origin profile id, table OID, row key, and structural id.

`ProfileMetrics.HiddenMetrics` remains a generic delivery container for
underscore-prefixed non-topology and non-licensing metrics. SNMP topology and
SNMP licensing must not depend on hidden metrics.

Top-level `metric_tags` on topology projections are profile/device labels. They
are applied through topology profile-tag ingestion and are not topology row
dispatch keys.

## Validation Guarantees

Profile validation rejects:

- unknown topology kinds;
- underscore-prefixed topology row value symbol names;
- regular metric chart/export-only fields on topology row value symbols;
- unknown licensing signal, sentinel, and state policy names;
- licensing rows without state or signals;
- scalar licensing rows that group multiple scalar signal OIDs without an
  explicit `id`;
- repeated licensing signal kinds for the same structural identity;
- licensing table `from` OIDs outside the row table;
- underscore-prefixed licensing value names;
- regular metric chart/export-only fields, transforms, scale factors, and
  constant-value hacks on licensing row value symbols;
- unsupported licensing value formats;
- invalid `consumers` values;
- virtual metrics whose sources resolve to topology rows.
