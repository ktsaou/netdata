# SOW-0005 - Cato Networks live validation

## Status

Status: open

Sub-state: blocked until a Cato tenant, vendor sandbox, or sanitized response captures are available.

## Requirements

### Purpose

Verify the shipped Cato Networks collector against real Cato API payloads and dashboard topology rendering before claiming production coverage of optional fields, BGP behavior, or real topology layouts.

### User Request

Follow-up created from SOW-0004 because no live Cato tenant credentials or vendor sandbox were available during implementation.

### Assistant Understanding

Facts:

- SOW-0004 implemented `cato_networks` collection and standalone topology using `github.com/catonetworks/cato-go-sdk@v0.2.5`.
- SOW-0004 unit tests cover SDK-typed fixtures and synthetic payloads, but not live Cato responses.
- `siteBgpStatus` is vendor-documented as Beta and needs live validation before broad production confidence.

Inferences:

- Live validation may require a read-only Cato API key, account ID, and possibly a regional API endpoint.
- Sanitized payload captures would materially improve future regression tests.

Unknowns:

- Exact real-tenant shape of optional site, interface, event, and BGP fields.
- Dashboard renderer behavior for real Cato hub-and-spoke topology layouts.

### Acceptance Criteria

- A live Cato job collects non-empty site connectivity and at least one site/interface throughput chart.
- `eventsFeed` marker behavior is verified without losing events across `Check()` and `Collect()`.
- `siteBgpStatus` is verified against at least one site with BGP configured, or explicitly documented as unavailable in the test tenant.
- `topology:cato_networks` returns valid topology JSON and renders acceptably in the dashboard.
- Sanitized response captures are added to collector tests if permission allows.

## Analysis

Sources checked:

- `.agents/sow/done/SOW-0004-20260501-cato-networks-collector.md` after SOW-0004 closes.
- `src/go/plugin/go.d/collector/cato_networks/`
- `.agents/sow/specs/cato-networks-collector.md`

Current state:

- Implementation is present but only locally validated.
- Live credentials are unavailable in this workspace.

Risks:

- Credentials and captured payloads may contain sensitive customer/account data and must be sanitized before storage.
- Live API calls share Cato's per-query, per-account rate limits with other integrations.

## Pre-Implementation Gate

Status: blocked

Problem / root-cause model:

- Local fixtures cannot prove real optional payload fields, BGP Beta behavior, or dashboard topology layout.

Evidence reviewed:

- SOW-0004 validation record and collector tests.

Affected contracts and surfaces:

- `src/go/plugin/go.d/collector/cato_networks/`
- `src/go/plugin/go.d/collector/cato_networks/testdata/` if sanitized captures are allowed.
- Operator docs if live behavior differs from the current documented contract.

Existing patterns to reuse:

- Existing collector tests in `src/go/plugin/go.d/collector/cato_networks/collector_test.go`.
- Netdata dashboard topology function path used by `topology:snmp`.

Risk and blast radius:

- No code should change until live validation shows a specific discrepancy.
- Never commit secrets, account IDs, site names, IPs, or customer-identifying data.

Implementation plan:

1. Obtain read-only tenant credentials or sanitized captures through an approved channel.
2. Run one configured `cato_networks` job with conservative defaults.
3. Verify metrics, EventsFeed marker behavior, BGP, and topology.
4. If payload differences are found, patch the collector and add sanitized fixtures.

Validation plan:

- Record exact commands, config redactions, charts observed, topology response shape, and any sanitized fixtures added.

Artifact impact plan:

- AGENTS.md: likely unaffected.
- Runtime project skills: likely unaffected.
- Specs: update `.agents/sow/specs/cato-networks-collector.md` if live behavior changes the contract.
- End-user/operator docs: update README/metadata/config docs if live behavior changes setup or troubleshooting.
- End-user/operator skills: likely unaffected.
- SOW lifecycle: close this SOW only after live validation is recorded or explicitly rejected by the user.

Open decisions:

- Credentials/sandbox availability is required before implementation.

## Implications And Decisions

None yet.

## Plan

1. Wait for approved Cato tenant/sandbox access or sanitized captures.
2. Run live validation with conservative defaults.
3. Patch and document any real-payload discrepancies.

## Execution Log

### 2026-05-01

- Follow-up SOW created from SOW-0004 close because live validation could not be run without credentials or sandbox access.

## Validation

Pending.

## Outcome

Pending.

## Lessons Extracted

Pending.

## Followup

None yet.
