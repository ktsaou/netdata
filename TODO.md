# TODO: Native BGP Daemons Split PR

## Purpose

Publish the native BGP daemon collector as its own focused PR.

This PR covers only:
- the dedicated `bgp` go.d collector
- its four backends: `frr`, `bird`, `gobgp`, `openbgpd`
- native collector charts, alerts, config, docs, and tests

This PR must not absorb SNMP profile-engine or SNMP BGP profile work.

## Reference Source

- Mixed source worktree: `/home/costa/src/PRs/netdata-bgp-monitoring`
- Mixed source TODO: `/home/costa/src/PRs/netdata-bgp-monitoring/TODO-BGP-MONITORING.md`
- If any behavior, limitation, or product boundary is unclear, check the mixed worktree and original TODO before changing direction.

## TL;DR

- This PR adds one dedicated native `bgp` collector with four backend implementations, all normalized into one shared internal BGP model and one `bgp.*` metric surface.
- The product scope is operational monitoring, not route analytics: peer health, churn, prefixes, summaries, bounded correctness, RPKI cache state, and useful alerts.

## Analysis

- The split contains one new collector module: `src/go/plugin/go.d/collector/bgp`.
- The four supported backends are:
  - FRR
  - BIRD
  - GoBGP
  - OpenBGPD
- They share one internal model for:
  - family summaries
  - peer views
  - neighbor/session detail
  - EVPN VNIs
  - RPKI cache and inventory
- Public contexts live under `bgp.*`, intentionally separate from the SNMP collector namespace.
- The collector is already heavily validated with:
  - unit tests
  - parser tests
  - replay/socket tests
  - copied fixtures
  - optional real-daemon integration harnesses

## Decisions

### Imported Product Decisions

1. This PR covers Tier 2 native daemon monitoring only.
2. The native BGP metric namespace stays separate from SNMP and uses `bgp.*`.
3. v1 remains operational monitoring, not BGP analytics.
4. No per-prefix time-series in this PR.
5. Correctness is included only when the daemon exposes it natively and only as bounded aggregated summaries.
6. Explicit configuration must override autodetection.
7. Multiple daemons on one node must remain representable as separate jobs.
8. Final public metric/chart contract still needs NIDL cleanup before it is considered frozen.

### Split-PR Decision

1. This PR must stay limited to the dedicated native `bgp` collector, its native alerts, config, docs, and tests.
2. SNMP collector/profile changes belong in the separate `snmp-bgp` PR.

## Pending

- FRR limitation remains:
  - withdraw-specific session churn is still unavailable on the current daemon JSON path
  - this is documented as a daemon limitation, not a collector bug
- FRR deep fallback is still not broadly live-proven on modern healthy peers:
  - fallback activation is covered by fixtures/replay and cold-scrape degraded tests
  - current FRR 10.6 healthy peers usually satisfy the collector from cheap counters first
- OpenBGPD boundaries remain:
  - filtered `/rib` support is limited to `ipv4`, `ipv6`, `vpnv4`, and `vpnv6`
  - active EVPN and FlowSpec route exchange are still not fully live-proven on the current Linux portable runtime
  - current Linux portable harnesses cannot prove a non-empty VPN RIB because the daemon lacks the needed l3vpn / mpe plumbing on that platform
- GoBGP boundary remains:
  - VRF correctness is still absent because upstream `ListPath(TABLE_TYPE_VRF)` does not expose validation state
- Product hardening still pending on the native side:
  - separate dashboard/template artifact work
  - enterprise-first native BGP monitoring guide
  - final TODO cleanup once split review feedback settles
- Customer-fit follow-up remains pending:
  - map the current backend boundaries and product-hardening items against real customer needs before starting BMP
- After this PR and the SNMP PR settle enough to stop backend churn, the next mandatory follow-up remains BMP.

## Plan

1. Keep this PR limited to the native `bgp` collector and its dependencies.
2. Preserve the shared internal model and unified `bgp.*` chart contract across all four backends.
3. Publish with explicit reviewer guidance that this PR is the Linux/native daemon path, separate from SNMP-managed devices.
4. Use the mixed worktree TODO as the source of truth for any subtle daemon limitation or research trail reviewers question.

## Implied Decisions

- Session-level truth is better than fake family-level precision when the daemon only exposes session-scoped cheap counters.
- High-cardinality paths must stay deliberately gated through `max*` / `select*` controls and optional deep queries.
- Operator ergonomics matter as much as parser correctness: chart ordering, labels, boundedness, and troubleshooting notes are part of the product.

## Testing Requirements

- `go test ./plugin/go.d/collector/bgp`
- Preserve native collector regression coverage for:
  - selection and bounded-cardinality behavior
  - shared chart layout and chart labels
  - connection reuse and reconnect behavior
  - FRR deep fallback behavior
  - RPKI cache and inventory paths
  - OpenBGPD filtered-RIB behavior
  - GoBGP family and VRF handling
- Preserve optional real-daemon integration coverage where available for:
  - FRR
  - BIRD
  - GoBGP
  - OpenBGPD

## Documentation Updates Required

- Enterprise-first native BGP monitoring guide.
- Final operator-facing explanation of current daemon-specific boundaries:
  - FRR withdraw limitation
  - OpenBGPD filtered-RIB and Linux runtime limitations
  - GoBGP VRF correctness limitation
- Separate dashboard/template work still pending after this PR.
