# SOW-0003 - query-agent-events private skill

## Status

Status: open

Sub-state: stub. Depends on SOW-0002 (Netdata query skills infrastructure) closing first. Stage-1 investigation already inherited from old SOW-0002 (.env+skill pattern, status-file v28 schema, systemd-journal Function shape, dual-transport architecture). Implementation cannot start until SOW-0002 ships the `query-netdata-agents/scripts/` library this skill consumes.

## Requirements

### Purpose

Build a **private developer skill** that lets a Netdata maintainer (or
an AI assistant helping one) query, fetch, and analyze received
agent-events submissions on the Netdata-operated ingestion server.

This skill is intentionally NOT public. It lives at
`<repo>/.agents/skills/query-agent-events/` only -- not under
`docs/netdata-ai/skills/`. Reasons:

- The data is operator-sensitive (machine GUIDs, claim IDs, cloud
  metadata, hardware DMI fields) and the user does not want a
  public "how to scrape Netdata's agent-events" doc.
- The journalctl-via-ssh path included in the skill requires
  privileged shell access to a specific host whose name lives only
  in `.env`.
- The skill is a maintainer triage tool, not a user-facing feature.

The fit-for-purpose use cases the skill must support:

- "What crashes is the fleet seeing in the last 24 hours, grouped
  by exit cause?"
- "Show me events from a specific agent (by `AE_AGENT_ID`)."
- "What's the distribution of `AE_AGENT_HEALTH` values among
  stable releases over the last 30 days?"
- "Fetch a specific event by timestamp + machine GUID and pretty-
  print its `AE_FATAL_*` fields."
- "Compare crash counts across `AE_AGENT_VERSION` for v2.8+ to
  spot regressions."

### User Request

> "the query-agent-events skill should not be public in
> docs/netdata-ai. It should live in .agents/skills/ since this is
> a developer tool, not an end-user tool. The skill should
> document the direct journalctl method via ssh, but it the
> destination IP and any other private info should be in .env.
>
> the journalctl method is documented in
> .local/agent-events-journals.md - this is untrusted document,
> not to be copied to a skill as-is. So, you need to review it,
> you can try it..."

The user added these `.env` keys (values stay in `.env`):

- `AGENT_EVENTS_NC_SPACE`
- `AGENT_EVENTS_HOSTNAME`
- `AGENT_EVENTS_MACHINE_GUID`
- `AGENT_EVENTS_NODE_ID`

### Assistant Understanding

Facts:

- The Netdata Agent serializes a status document (schema v28) and
  POSTs it to a public ingestion endpoint. The receiving service
  persists each submission as a systemd journal entry in a
  dedicated journal namespace whose name is documented in
  `.local/agent-events-journals.md` (an untrusted draft).
- Each status JSON dot-path is converted into a journal field
  prefixed with `AE_` and the dots replaced with underscores
  (per the untrusted draft, to be verified against
  `src/libnetdata/log/log2journal/` and the producer-side
  serialization).
- Three transports are in scope:
  1. Cloud-proxied: uses
     `query-netdata-cloud/query-logs.md` and
     `query-netdata-cloud/query-functions.md` (delivered by
     SOW-0002).
  2. Direct agent: uses
     `query-netdata-agents/scripts/_lib.sh::agents_call_function`
     (delivered by SOW-0002), with bearer auto-mint/refresh.
  3. journalctl-via-ssh: uses standard ssh + the journalctl
     namespace flag. Destination host comes from `.env`.

Inferences:

- The journal field map and enum values in
  `.local/agent-events-journals.md` are likely correct in spirit
  but must be verified field-by-field against the producer code
  and a sampled response before the skill encodes them as truth.
- The default query the skill ships should be conservative
  (last 24h, narrow facets) so a single `query` call does not
  flood the ingestion server.

Unknowns:

- Whether `.local/agent-events-journals.md` is fully accurate
  on every field-name mapping (especially edge cases:
  array indices, deeply-nested paths, omitted-on-graceful-exit
  fields).
- Whether SOW-0002 ships a Cloud-proxied logs query helper ready
  to consume (decision 1 of SOW-0002 directly governs this).
- Whether the journalctl path returns the same field set as the
  Netdata-via-Cloud path; small drift is likely (the journal
  output includes systemd `_*` fields that the Function may
  filter out).

### Acceptance Criteria

- `<repo>/.agents/skills/query-agent-events/SKILL.md` exists
  with frontmatter triggers covering "agent events",
  "agent-events", "status file", "crash reports", "fleet
  crashes", "ingestion server".
- `<repo>/.agents/skills/query-agent-events/scripts/` ships:
  - `_lib.sh` (mirrors the legacy skill helper shape, prefix
    `agentevents_`; sources `.env`, depends on the helpers from
    `query-netdata-agents/scripts/_lib.sh`)
  - `query-events.sh` -- thin wrapper around the `systemd-
    journal` Function with flags `--via {cloud|agent|ssh}`,
    `--last N`, `--after T`, `--before T`, `--query STR`,
    `--source SEL`, `--facets a,b,c`, `--histogram FIELD`.
  - `fetch-event.sh` -- single-event fetch by `AE_AGENT_ID` +
    timestamp anchor.
  - `summarize.sh` -- jq-driven facet/histogram pretty-print.
- `<repo>/.agents/skills/query-agent-events/AE_FIELDS.md` exists
  and documents the verified `AE_*` field map (and known
  divergences from `.local/agent-events-journals.md`).
- All raw outputs land under
  `<repo>/.local/audits/query-agent-events/` (gitignored).
- A small acceptance fixture: at least one real round-trip via
  the Cloud transport and one via ssh, each producing a small
  bundle that includes a `crash-*` event for a stable
  (`AE_AGENT_VERSION` matches `^v2\.([89]|\d\d)\.`) release.
- AGENTS.md "Project Skills Index" section adds a one-line entry
  for `.agents/skills/query-agent-events/`.
- Sensitive-data gate: the SOW, the SKILL.md, and every
  committed script contain zero raw values for any of
  `AGENT_EVENTS_*`, no machine GUIDs, no claim IDs, no public-
  facing host names except those already in the open-source
  code. Verified by pre-commit grep.

## Analysis

Sources to consult during stage 2 (already mostly read at stage 1):

- `<repo>/src/daemon/status-file.{c,h}` (schema v28).
- `<repo>/src/daemon/status-file-io.{c,h}`.
- `<repo>/src/daemon/status-file-dmi.{c,h}`.
- `<repo>/src/libnetdata/exit/exit_initiated.h` (exit_reason
  enum).
- `<repo>/src/libnetdata/log/log2journal/` (journal field naming
  conventions; verify the `AE_` prefix story).
- `<repo>/.local/agent-events-journals.md` (untrusted draft;
  treat each claim as a hypothesis until cross-verified).
- `<repo>/src/collectors/systemd-journal.plugin/...` (Function
  shape).
- The output of `query-netdata-agents/scripts/_lib.sh` from
  SOW-0002 (consumer side).

Risks:

- Privacy: every fetched event carries identifying fields. The
  skill MUST NOT copy real values into committed artifacts. A
  redaction filter (`redact-events.sh`) ships as an opt-in
  filter, not a default.
- Untrusted-doc risk: copying field names verbatim from
  `.local/agent-events-journals.md` without verification is
  unsafe. Stage 2 must spot-check at least the top-20 most-used
  fields and the all-the-enums against producer source.
- Volume: a fleet of 1.5M+ daily Netdata installs producing
  events at non-zero rates means naive "fetch all" calls would
  return enormous payloads. Default queries must be narrow.
- Schema drift: STATUS_FILE_VERSION will keep moving. Treat
  unknown fields as opaque pass-through; key analyzers off the
  documented field paths only.
- Producer vs consumer endpoint confusion: never point at the
  producer ingest URL (a `const char *` in
  `src/daemon/status-file.c:988` -- the agent POSTs there).
  Always point at the consumer endpoint resolved from
  `${AGENT_EVENTS_HOSTNAME}` and the Cloud space.

## Pre-Implementation Gate

Status: blocked-on-prereq

This SOW cannot begin implementation until SOW-0002 closes. The
gate sections (problem/root-cause, evidence, plan, validation,
artifact impact, decisions) will be filled in once SOW-0002
delivers the `query-netdata-agents/scripts/_lib.sh` helpers and
the Cloud-side function-call shape is locked.

Sensitive data handling plan:

- This SOW (and every committed artifact it produces) follows
  the spec at
  `<repo>/.agents/sow/specs/sensitive-data-discipline.md`. No
  literal IPs, hostnames, UUID-shaped IDs, tokens, absolute
  install/user paths, usernames, tenant names, or secrets in
  any committed file. Every reference uses an env-key
  placeholder (`${KEY_NAME}`) defined in `.env`.
- Specifically required `.env` keys for this SOW:
  `NETDATA_CLOUD_TOKEN`, `AGENT_EVENTS_NC_SPACE`,
  `AGENT_EVENTS_HOSTNAME` (used in four roles: cloud room
  name, ssh-able host, direct-HTTP host, journalctl
  namespace -- value happens to be the same string today),
  `AGENT_EVENTS_MACHINE_GUID`, `AGENT_EVENTS_NODE_ID`. No
  new agent-events keys are needed; the existing four cover
  the skill's needs.
- Pre-commit verification grep (from the spec) runs on every
  staged change before commit.

Holding-pattern decisions to record now (so they are not lost):

- Privacy policy: default is "store raw under
  `.local/audits/query-agent-events/`, never share". Opt-in
  redact filter ships in stage 2.
- Initial script set: `query-events.sh` + `fetch-event.sh` +
  `summarize.sh` (the trio recommended at stage-1 follow-up).
- `AE_FIELDS.md` shape: cross-reference table -- producer
  source path | journal field name | enum values (if any) |
  notes.

## Implications And Decisions

No new user decisions required at this stub stage. All
infrastructure decisions blocking this SOW are recorded in
SOW-0002 (decisions 1-3). Once SOW-0002 closes, this SOW will
add its own decision list covering:

- The exact `__logs_sources` value or `query=` predicate that
  scopes the journal query to agent-events on the ingestion-
  server agent.
- Whether the skill should also accept a literal namespace
  string (passed through to the Function) in `.env` as
  `AGENT_EVENTS_JOURNAL_NAMESPACE`.
- Whether the journalctl-via-ssh path is mandatory in stage 2
  or can ship in a follow-up.

## Plan

1. **Wait for SOW-0002 to close.**
2. Fill in this SOW's Pre-Implementation Gate, Decisions, and
   Implementation Plan based on the SOW-0002 deliverables.
3. Move to `current/` as `Status: in-progress`.
4. Implement, validate, close.

## Execution Log

### 2026-05-03

- Created as a stub during the 4-SOW split.

## Validation

Pending. All sub-fields will be filled at close.

## Outcome

Pending.

## Lessons Extracted

Pending.

## Followup

None yet.

## Regression Log

None yet.

Append regression entries here only after this SOW was completed or closed and later testing or use found broken behavior. Use a dated `## Regression - YYYY-MM-DD` heading at the end of the file. Never prepend regression content above the original SOW narrative.
