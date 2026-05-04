# SOW-0004 - learn-site-structure private skill

## Status

Status: open

Sub-state: stub. Originally bundled with `integrations-lifecycle/` in a single "doc-pipeline" SOW; split out on 2026-05-04 because documentation and integrations are separate concerns. The integrations skill is now tracked by SOW-0007. Depends on SOW-0002 (skill format convention) so the skill follows the established `<name>/SKILL.md` shape. Implementation does not start until SOW-0002 closes.

## Requirements

### Purpose

Build a **private developer skill** that captures the operational
knowledge for Netdata's documentation pipeline:

**`learn-site-structure/`** -- explains how content in this repo
(and adjacent repos) controls the structure of the Netdata learn
site (env-keyed `learn.netdata.cloud`): directory conventions,
frontmatter, navigation/sidebar, the export/sync flow into the
website repo, the website generator (Hugo / static-site builder),
and the deployment surface.

The skill is private (`<repo>/.agents/skills/learn-site-structure/`)
-- it exists for Netdata maintainers writing or updating docs, not
for end users.

### User Request

> "We also need more private skills:
> 1. how documentation this repo controls learn.netdata.cloud
>    site structure
> ..."
>
> Follow-up (2026-05-04): "split them please. documentation and
> integrations are not the same thing"

### Assistant Understanding

Facts (not yet verified -- this is a stub):

- Documentation source files in `<repo>/docs/` and per-component
  README-style docs feed the learn site (env-keyed
  `learn.netdata.cloud`) via a sync/export flow. The user has separate repos at
  `${NETDATA_REPOS_DIR}/netdata` (source) and
  `${NETDATA_REPOS_DIR}/website/content` (rendered content).

Inferences:

- The website generator and the learn-site sync flow are a
  bounded surface that can be documented independently from the
  integrations pipeline. The integrations pipeline shares the
  same downstream website but is driven by `metadata.yaml`, not
  by `docs/`, so it lives in its own skill (SOW-0007).

Unknowns (to be resolved during stage-2a investigation):

- Whether the website generator is Hugo, Astro, or something
  else; where its config lives; how it picks up Netdata's
  generated artifacts.
- The exact directory conventions on the learn site (sections,
  sidebars, indexes).
- Whether there is an existing developer-facing doc that
  partially covers this (e.g. a CONTRIBUTING note on adding a
  doc) that the skill should reference rather than duplicate.
- The exact sync/export commands and the cadence at which they
  run.

### Acceptance Criteria

- `<repo>/.agents/skills/learn-site-structure/SKILL.md` exists
  with frontmatter triggers covering "learn site",
  "learn.netdata.cloud", "docs site structure", "site sidebar",
  "docs sync", "website generator".
- `<repo>/.agents/skills/learn-site-structure/` includes:
  - The end-to-end flow diagram (text-based) from a doc file
    in this repo to a published page on the learn site
    (env-keyed `learn.netdata.cloud`).
  - The directory conventions (where to put a doc, when to use
    a section index, how the sidebar is computed).
  - The export/sync command(s) and what they do.
  - Known gotchas (frontmatter fields that silently break
    rendering, broken-link pitfalls, etc.).
- AGENTS.md "Project Skills Index" section adds a one-line entry
  for `.agents/skills/learn-site-structure/`.
- Skill follows the format convention established by SOW-0002.

## Analysis

Sources to consult during stage-2a investigation (not yet read):

- `<repo>/docs/` (source docs).
- `${NETDATA_REPOS_DIR}/website/` (rendered site).
- `${NETDATA_REPOS_DIR}/netdata/docs/` and any other docs source repos
  the user maintains.
- Existing CONTRIBUTING / docs-author notes in this repo.
- Any sync scripts under `<repo>/packaging/` or
  `${NETDATA_REPOS_DIR}/website/scripts/`.

Risks:

- Investigation may reveal that the doc sync flow has
  undocumented edge cases (e.g. links across repos that get
  silently rewritten). Document the divergences explicitly
  rather than papering over them.
- Some content may be authored in the website repo directly
  rather than in this repo. Make the boundary explicit so
  maintainers know where to edit a given page.

## Pre-Implementation Gate

Status: not-started

To be filled during stage 2a when investigation begins. Likely
no user decisions are needed beyond confirming the scope
boundary with `integrations-lifecycle` (SOW-0007): if a doc
flows through `metadata.yaml`, that's SOW-0007's territory; if
it flows through `<repo>/docs/`, it's this skill.

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
2. Stage 2a: investigate the docs sync flow and the website
   generator. Capture evidence in the SOW.
3. Stage 2b: fill the Pre-Implementation Gate and present
   decisions to the user (if any).
4. Stage 2c: write the skill.
5. Validate by walking a real "update a doc page" example
   end-to-end and confirming the skill's instructions match
   what the maintainer actually does.
6. Close.

## Execution Log

### 2026-05-03

- Created as a stub during the 4-SOW split (originally bundled
  with `integrations-lifecycle/`).

### 2026-05-04

- Split: `integrations-lifecycle/` moved to SOW-0007. This SOW
  is now scoped to `learn-site-structure/` only. Filename
  changed from `SOW-0004-20260503-doc-pipeline-skills.md` to
  `SOW-0004-20260503-learn-site-structure-skill.md`.

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
