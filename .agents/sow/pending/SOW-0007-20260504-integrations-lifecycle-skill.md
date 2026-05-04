# SOW-0007 - integrations-lifecycle private skill

## Status

Status: open

Sub-state: stub. Created 2026-05-04 by splitting the original "doc-pipeline" SOW (SOW-0004) into two: documentation goes to SOW-0004 (`learn-site-structure`); integrations land here. Depends on SOW-0002 (skill format convention) so the skill follows the established `<name>/SKILL.md` shape. Independent of SOW-0003, SOW-0004, and SOW-0005; can ship in any order after SOW-0002.

## Requirements

### Purpose

Build a **private developer skill** that captures the operational
knowledge for Netdata's integrations pipeline:

**`integrations-lifecycle/`** -- explains how `metadata.yaml`
files in this repo drive the integrations pages everywhere they
appear: the learn site (env-keyed `learn.netdata.cloud`), the
marketing site (env-keyed `netdata.cloud`), and the in-app
integrations page in the dashboard. Documents the entire
lifecycle: every supported option in `metadata.yaml`; the scripts
that consume it; the intermediate transformations; and the final
pages produced on each surface.

The skill is private (`<repo>/.agents/skills/integrations-lifecycle/`)
-- it exists for Netdata maintainers writing collectors and
integrations docs, not for end users.

### User Request

> "We also need more private skills:
> 2. how integrations work and how metadata.yaml controls
>    integrations pages in learn, www, in-app - this should
>    explain the entire lifecycle, from the options supported in
>    metadata.yaml to which scripts are run, what they do, and
>    how the final integrations pages are created."
>
> Follow-up (2026-05-04): "split them please. documentation and
> integrations are not the same thing"
>
> Earlier reminder (preserved verbatim from SOW-0001
> Lessons): "If a tunable changes, the schema, the stock conf,
> the metadata, the alerts, and the README must change in the
> same commit." This skill must surface that consistency
> requirement prominently.

### Assistant Understanding

Facts (not yet verified -- this is a stub):

- `metadata.yaml` files live next to collectors in
  `<repo>/src/go/plugin/go.d/modules/*/metadata.yaml` and at
  similar paths for other plugins (Python, internal C plugins,
  Rust crates).
- Production scripts under `<repo>/integrations/` consume them.
- `${NETDATA_REPOS_DIR}/website/` consumes the produced output for
  the marketing and learn surfaces; the in-app dashboard consumes
  generated artifacts shipped with the agent.
- For ibm.d modules, `metadata.yaml` is **generated** from
  `contexts.yaml` via `go generate` (per the
  project-writing-collectors skill). For go.d, it is hand-written.
- Collector consistency rule (from AGENTS.md): if any one of
  metadata.yaml / config_schema.json / stock conf / health.d
  alerts / README / code changes, the others MUST be updated in
  the same commit.

Inferences:

- The integrations pipeline is a bounded, well-defined surface
  separate from the docs sync flow, even though both feed the
  same downstream website. They diverge at the source: one is
  driven by `<repo>/docs/`, the other by `<repo>/src/**/metadata.yaml`.
- The skill must be exhaustive on the schema (every field, every
  enum) because that is the contract maintainers rely on to ship
  a correct collector.

Unknowns (to be resolved during stage-2a investigation):

- The exact set of scripts under `<repo>/integrations/` and
  what each one does (generators, validators, renderers).
- The exact `metadata.yaml` schema (every field, every nested
  block, every enum). Likely living in
  `<repo>/integrations/schemas/` or similar; needs a JSON
  Schema or equivalent reference.
- How alert metadata, dashboard metadata, and other
  collector-adjacent metadata files plug into the same
  pipeline (or whether they are independent).
- The boundary between the in-app integrations page (rendered by
  the agent / cloud-frontend) and the website-rendered pages.
- Whether all three downstream surfaces (learn, www, in-app)
  consume the same intermediate artifact, or each consumes a
  different artifact.
- Whether there is an existing developer-facing doc that
  partially covers this (e.g. `<repo>/integrations/README.md`)
  that the skill should reference rather than duplicate.

### Acceptance Criteria

- `<repo>/.agents/skills/integrations-lifecycle/SKILL.md` exists
  with frontmatter triggers covering "metadata.yaml",
  "integrations page", "integrations lifecycle", "collector
  metadata", "integrations build", "integration page rendering".
- `<repo>/.agents/skills/integrations-lifecycle/` includes:
  - The full `metadata.yaml` schema reference (every option
    documented with type, semantics, example, surfaces it
    affects).
  - The lifecycle: where `metadata.yaml` is read, what each
    consuming script does, what intermediate artifacts are
    produced, how each downstream surface (learn / www /
    in-app) renders the result.
  - The minimal "add or update a collector integration" recipe
    a developer should follow, including the consistency
    requirement (metadata + schema + stock conf + alerts +
    README must move together).
  - Cross-references to:
    - the `project-writing-collectors` skill (for the broader
      collector-authoring context) and
    - the `learn-site-structure` skill (SOW-0004) for the
      docs-driven surfaces.
- AGENTS.md "Project Skills Index" section adds a one-line entry
  for `.agents/skills/integrations-lifecycle/`.
- Skill follows the format convention established by SOW-0002.

## Analysis

Sources to consult during stage-2a investigation (not yet read):

- `<repo>/integrations/` (scripts and schemas).
- `<repo>/integrations/README.md` if present.
- `<repo>/src/go/plugin/go.d/modules/<one>/metadata.yaml`
  (a representative example).
- A representative `contexts.yaml` under `<repo>/src/go/plugin/ibm.d/`
  to capture the ibm.d generation flow.
- Any JSON Schema or YAML Schema files validating
  `metadata.yaml`.
- `${NETDATA_REPOS_DIR}/website/` rendering pipeline (how it
  consumes the agent's generated integrations artifact).
- The in-app integrations page source under
  `${NETDATA_REPOS_DIR}/dashboard/cloud-frontend/` or similar.

Risks:

- Scope is wide. The Pre-Implementation Gate of this SOW must
  decide whether all three downstream surfaces (learn, www,
  in-app) are in scope from the start, or whether the first
  cut covers only learn + in-app and the www surface ships in
  a follow-up.
- `metadata.yaml` schema is large; documenting every option
  exhaustively may stretch the SOW. A staged approach (most-used
  options first, exhaustive reference second) may be better.
- Investigation may reveal that the integrations pipeline is
  not uniform across the three surfaces. If so, document the
  divergences explicitly rather than papering over them.

## Pre-Implementation Gate

Status: not-started

To be filled during stage 2a when investigation begins. Will
need user decisions on:

- Scope: all three surfaces from the start, or staged?
- Whether to inline the `metadata.yaml` schema reference into
  the skill, or to reference an external schema document.
- Whether to bundle the consistency-requirement enforcement
  (e.g. a maintainer-facing pre-commit check) into this SOW or
  defer to a follow-up.

Sensitive data handling plan:

- This SOW (and every committed artifact it produces) follows
  the spec at
  `<repo>/.agents/sow/specs/sensitive-data-discipline.md`. No
  literal hostnames (including the learn / www domains),
  absolute install/user paths, usernames, tokens, or
  identifiers in any committed file. Every reference uses an
  env-key placeholder (`${KEY_NAME}`) defined in `.env`.
- Specifically required `.env` keys for this SOW:
  `NETDATA_REPOS_DIR` (already present from SOW-0002). Public
  site hostnames (learn, marketing) are documented as literals
  per the spec; this fork's checkout root is found via
  `git rev-parse --show-toplevel`.
- Pre-commit verification grep (from the spec) runs on every
  staged change before commit.

## Implications And Decisions

None yet at this stub stage. Will be added when investigation
starts.

## Plan

1. **Wait for SOW-0002 to close** (already complete).
2. Stage 2a: investigate the integrations pipeline end-to-end.
   Capture evidence in the SOW.
3. Stage 2b: fill the Pre-Implementation Gate and present
   decisions to the user (scope, schema-inline vs reference,
   consistency-check tooling).
4. Stage 2c: write the skill.
5. Validate by walking a real "add a new collector integration"
   example end-to-end and confirming the skill's instructions
   match what the maintainer actually does.
6. Close.

## Execution Log

### 2026-05-04

- Created when the user split the original doc-pipeline SOW
  (SOW-0004) into documentation (SOW-0004 keeps the slot, scoped
  to `learn-site-structure`) and integrations (this SOW).

## Validation

Pending.

## Outcome

Pending.

## Lessons Extracted

Pending.

## Followup

None yet.

## Regression Log

None yet.

Append regression entries here only after this SOW was completed or closed and later testing or use found broken behavior. Use a dated `## Regression - YYYY-MM-DD` heading at the end of the file. Never prepend regression content above the original SOW narrative.
