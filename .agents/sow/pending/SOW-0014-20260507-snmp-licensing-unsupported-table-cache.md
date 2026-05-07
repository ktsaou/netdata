# SOW-0014 - SNMP licensing unsupported table cache

## Status

Status: open

Sub-state: follow-up created from SOW-0013 close-out review.

## Requirements

### Purpose

Avoid repeated SNMP licensing table-root walks on Cisco and other devices that explicitly report unsupported licensing tables, without reintroducing broad missing-OID poisoning for ordinary metric tables or empty-but-valid tables.

### User Request

User accepted broad Cisco licensing coverage through dedicated mixins and agreed that nonexistent licensing table OID caching should be tracked as the solution for unsupported devices.

### Assistant Understanding

Facts:

- `cisco.yaml` extends `_cisco-licensing-traditional.yaml` and `_cisco-licensing-smart.yaml`.
- Those mixins add two licensing table roots: `clmgmtLicenseInfoTable` and `ciscoSlaEntitlementInfoTable`.
- SOW-0013 intentionally removed the old empty-walk table missing cache write because empty tables and unsupported tables must not be conflated.
- Explicit `NoSuchObject` / `NoSuchInstance` table-root responses are still a reasonable candidate for a narrowly scoped licensing unsupported-table cache.

Inferences:

- The follow-up should distinguish unsupported table roots from empty tables with zero rows.
- The cache should be scoped to typed licensing table roots and must not suppress ordinary metric table OIDs.

Unknowns:

- Exact `gosnmp` walk response shapes for explicit table-root `NoSuchObject` / `NoSuchInstance` need fixture or mock confirmation before implementation.

### Acceptance Criteria

- Typed licensing table walks that receive explicit no-such table-root responses are skipped on later collection cycles.
- Empty valid tables with zero rows are not cached as unsupported.
- Ordinary metrics and topology table collection are unaffected.
- Tests cover unsupported table root, empty table, and ordinary metric table behavior.

## Analysis

Sources checked:

- `src/go/plugin/go.d/collector/snmp/ddsnmp/ddsnmpcollector/collector_licensing.go`
- `src/go/plugin/go.d/collector/snmp/ddsnmp/ddsnmpcollector/collector_table.go`
- `src/go/plugin/go.d/config/go.d/snmp.profiles/default/cisco.yaml`
- `src/go/plugin/go.d/config/go.d/snmp.profiles/default/_cisco-licensing-traditional.yaml`
- `src/go/plugin/go.d/config/go.d/snmp.profiles/default/_cisco-licensing-smart.yaml`
- `.agents/sow/current/SOW-0013-20260507-snmp-licensing-projection.md`

Current state:

- Missing scalar OIDs are cached exactly.
- Licensing table roots with no returned PDUs are not cached, by design, because zero rows may mean an empty valid table.
- Broad Cisco licensing can therefore continue walking unsupported licensing table roots each cycle until this follow-up is implemented.

Risks:

- Over-broad caching would hide licensing data that appears later or suppress unrelated regular metrics.
- Under-scoped caching leaves avoidable per-cycle Cisco probe cost.

## Pre-Implementation Gate

Status: blocked

Problem / root-cause model:

- Broad Cisco licensing coverage intentionally trades initial probe breadth for profile simplicity. Unsupported table roots need a precise negative-cache path keyed to explicit no-such table-root responses, not to empty walk results.

Evidence reviewed:

- `collector_licensing.go` walks licensing table roots on cache misses.
- `collector_table.go` no longer marks zero-PDU walks missing.
- SOW-0013 execution log records this as a follow-up requirement.

Affected contracts and surfaces:

- ddsnmpcollector licensing table collection.
- Collection stats for missing OIDs and table walks.
- Cisco SNMP collection overhead on unsupported devices.

Existing patterns to reuse:

- Shared exact `missingOIDs` scalar cache.
- Existing table cache and SNMP walk helpers.
- Existing mock SNMP tests under `ddsnmpcollector`.

Risk and blast radius:

- Narrow runtime collector behavior change.
- Must not alter profile schema.
- Must not change ordinary table metric missing behavior.

Sensitive data handling plan:

- Use synthetic SNMP mock PDUs only. Do not add real device walks or raw MIB content.

Implementation plan:

1. Model explicit table-root no-such responses in tests.
2. Add a licensing-scoped unsupported table-root cache or equivalent filter.
3. Prove empty valid tables are still retried.

Validation plan:

- `go test -count=1 ./collector/snmp/ddsnmp/ddsnmpcollector`
- `go test -count=1 ./collector/snmp/ddsnmp/...`
- `go test -count=1 ./collector/snmp/...`

Artifact impact plan:

- AGENTS.md: no expected update.
- Runtime project skills: no expected update unless a new authoring rule emerges.
- Specs: update `.agents/sow/specs/snmp-profile-projection.md` if the cache becomes a durable projection behavior.
- End-user/operator docs: no expected update.
- End-user/operator skills: no expected update.
- SOW lifecycle: track this as the SOW-0013 follow-up.

Open-source reference evidence:

- None. This is collector runtime behavior with synthetic tests.

Open decisions:

- None for planning. Implementation details should be validated against actual `gosnmp` no-such walk behavior before patching.

## Implications And Decisions

- Follow-up created to satisfy SOW-0013 follow-up discipline; no implementation is part of SOW-0013.

## Plan

1. Add tests for unsupported licensing table-root no-such handling.
2. Implement narrowly scoped cache/filter behavior.
3. Validate narrow and shared SNMP suites.

## Execution Log

### 2026-05-07

- Created as follow-up from SOW-0013 final review.

## Validation

Acceptance criteria evidence:

- Pending implementation.

Tests or equivalent validation:

- Pending implementation.

Real-use evidence:

- Pending implementation.

Reviewer findings:

- Pending implementation.

Same-failure scan:

- Pending implementation.

Sensitive data gate:

- Pending implementation.

Artifact maintenance gate:

- AGENTS.md: pending implementation outcome.
- Runtime project skills: pending implementation outcome.
- Specs: pending implementation outcome.
- End-user/operator docs: pending implementation outcome.
- End-user/operator skills: pending implementation outcome.
- SOW lifecycle: pending implementation outcome.

Specs update:

- Pending implementation.

Project skills update:

- Pending implementation.

End-user/operator docs update:

- Pending implementation.

End-user/operator skills update:

- Pending implementation.

Lessons:

- Pending implementation.

Follow-up mapping:

- Pending implementation.

## Outcome

Pending.

## Lessons Extracted

Pending.

## Followup

None yet.

## Regression Log

None yet.
