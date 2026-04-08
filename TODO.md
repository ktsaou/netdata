# TODO: SNMP BGP Split PR

## Purpose

Publish the SNMP side of the mixed BGP monitoring work as its own focused PR.

This PR covers only:
- the generic `snmp` collector changes needed for BGP
- SNMP profile-engine enhancements needed by those profiles
- BGP SNMP profiles and fixtures
- stock SNMP BGP alerts

This PR must not absorb native daemon collector work.

## Reference Source

- Mixed source worktree: `/home/costa/src/PRs/netdata-bgp-monitoring`
- Mixed source TODO: `/home/costa/src/PRs/netdata-bgp-monitoring/TODO-BGP-MONITORING.md`
- If anything in this split feels unclear or underexplained, check the mixed worktree first before inventing new behavior or reshaping scope.

## TL;DR

- This PR extends the existing generic `snmp` collector to monitor BGP truthfully from standard and vendor MIBs.
- It adds the collector/profile capabilities required to model BGP peers, route counts, alerts, and per-vendor context without pretending that all devices expose the same data.

## Analysis

- This is not a new dedicated SNMP BGP collector. It is an extension of the existing generic `snmp` collector and its `ddsnmp` profile engine.
- The split contains:
  - BGP profile-engine enhancements such as `format: hex`, `format: uint32`, value-based `lookup_symbol`, `index_transform.drop_right`, raw-index tag extraction, richer `ip_address` handling, virtual-metric grouping/tag emission, and related tests
  - standard `BGP4-MIB` expansion
  - vendor-native BGP coverage for Cisco, Juniper, Nokia/TiMOS, Huawei, Arista, Dell, Cumulus, and Alcatel
  - end-to-end SNMP alert-surface validation with copied external fixtures
  - stock SNMP BGP alerts in `src/health/health.d/snmp_bgp.conf`
- Current public metric naming still follows the generic SNMP collector model:
  - contexts are emitted as `snmp.device_prof_*`
  - this is implementation truth today, not a finalized BGP product namespace
- The SNMP side is intentionally truth-preserving:
  - standard `BGP4-MIB` cannot promise route counts
  - vendor-native route-count semantics differ by vendor
  - Huawei/Nokia totals must not be misrepresented as the same kind of gauge as Cisco/Juniper/Arista/Dell

## Decisions

### Imported Product Decisions

1. This split covers Tier 1 SNMP monitoring only; BMP remains outside this PR.
2. SNMP promises must remain vendor-truthful. Do not claim data that device MIBs do not expose.
3. Customer-fit for this batch is SNMP-first for device/router monitoring: peer inventory, status, uptime, route counts where honest, route-change visibility, alerts, and cleared-alert history.
4. Real-time per-route inspection is explicitly out of scope here and remains deferred to BMP.
5. Public BGP metric/chart contracts must eventually follow NIDL rules before final contract freeze.

### Split-PR Decision

1. This PR must stay limited to the `snmp` collector, SNMP profiles, SNMP fixtures/tests, and `snmp_bgp.conf`.
2. Native `bgp` collector files, native alerts, and native daemon docs belong in the separate `bgp-daemons` PR.

## Pending

- Decide whether the SNMP BGP customer-facing namespace should remain `snmp.device_prof_*` or be normalized into a clearer BGP-specific namespace such as `snmp.bgp.*`.
- Complete the final NIDL/public-context cleanup for the SNMP side before final public contract freeze.
- Finalize and document what stays in generic `BGP4-MIB` versus vendor-specific extensions.
- Keep adding metrics only where the underlying profile exposes them honestly; do not flatten unlike vendor semantics into one fake universal contract.
- Cisco evidence gap remains:
  - there is still no copied public raw `cbgpPeer3Table` walk
  - current peer3 coverage is validated by official MIB semantics, independent external implementations, and Netdata tests, but not by a copied raw peer3 fixture
- SNMP customer-fit documentation is still incomplete:
  - per-context/per-vendor capability documentation needs to be turned into user-facing docs
  - route-count semantics and route-change semantics need explicit operator-facing explanation
- Customer-fit follow-up remains pending against the Darling-style requirements:
  - SNMP must be described per collector context and per vendor capability
  - peer FSM substate coverage should be explicitly confirmed in the final operator docs, not treated as incidental
- After this PR and the native-daemon PR settle enough to stop backend churn, the next mandatory follow-up remains BMP.

## Plan

1. Keep this PR scoped to SNMP collector and profile work only.
2. Preserve all current copied-fixture and profile-driven alert validation in this split.
3. Publish with explicit reviewer guidance that this PR is about SNMP-managed devices, not Linux-native daemons.
4. Use the mixed worktree TODO as the source of truth for any pending item that needs clarification during review.

## Implied Decisions

- It is better to expose uneven vendor truth honestly than to fabricate a uniform SNMP BGP surface that lies.
- Shared alert contracts are acceptable when backed by real per-vendor mappings.
- Operator-facing documentation must explain capability differences by vendor, not only by raw metric names.

## Testing Requirements

- `go test ./plugin/go.d/collector/snmp/...`
- Preserve profile-engine regression coverage for:
  - `lookup_symbol`
  - `drop_right`
  - `format: hex`
  - `format: uint32`
  - `ip_address` parsing, including zone-scoped and textual/octet forms
  - virtual-metric grouping and visible tag emission
- Preserve end-to-end fixture coverage for:
  - Cisco
  - Juniper
  - Nokia/TiMOS
  - Huawei
  - Arista
  - Dell
  - Cumulus
  - generic `BGP4-MIB`

## Documentation Updates Required

- User-facing SNMP BGP guide framed for enterprise operators.
- Clear vendor capability matrix tied to the emitted collector contexts.
- Explicit documentation of what standard `BGP4-MIB` can and cannot provide.
- Clear explanation that live per-route inspection is not provided by SNMP in this batch and is deferred to BMP.
