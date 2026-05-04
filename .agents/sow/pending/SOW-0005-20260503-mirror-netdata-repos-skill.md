# SOW-0005 - mirror-netdata-repos private skill

## Status

Status: open

Sub-state: stub. Smallest of the post-SOW-0002 skill stubs. Depends on SOW-0002 (skill format convention) so the skill follows the established `<name>/SKILL.md` shape. Independent of SOW-0003, SOW-0004, and SOW-0007; can ship in any order after SOW-0002.

## Requirements

### Purpose

Build a **private developer skill** that documents how Netdata
maintainers (and AI assistants helping them) sync all Netdata
organization repositories into `${NETDATA_REPOS_DIR}/` for cross-repo
code review, evaluation, and grep-across-the-org workflows.

The user already has the working script `${NETDATA_REPOS_DIR}/sync-all.sh`.
This skill captures the operational knowledge around it: when to
run it, what it does, how to extend it (add a new repo), how it
interacts with any wider observability-repo mirror the user
maintains, and the gotchas to avoid (e.g. accidentally
committing into a sub-repo, conflicts with active dev branches).

### User Request

> "3. how to sync all netdata repos into [env-keyed:
> ${NETDATA_REPOS_DIR}], so that netdata devs can have a local
> copy of all the organization repo for cross repo code reviews
> and evaluations (I have a script for that in [env-keyed:
> ${NETDATA_REPOS_DIR}/sync-all.sh])"
>
> (Quoted with literal absolute paths replaced by their `.env`
> keys per `<repo>/.agents/sow/specs/sensitive-data-discipline.md`.)

### Assistant Understanding

Facts (to be verified during stage 2):

- A working sync script exists at `${NETDATA_REPOS_DIR}/sync-all.sh`.
- The target directory `${NETDATA_REPOS_DIR}/` is the user's
  cross-org mirror.
- A separate, larger observability-projects mirror exists on
  the user's workstation (covers thousands of repos across many
  platforms; documented by the user's global `mirrored-repos`
  skill).

Inferences:

- The two mirrors serve different purposes:
  `${NETDATA_REPOS_DIR}/` = active dev mirror of Netdata-org
  repos; the larger observability-projects mirror = read-only
  research mirror across the broader ecosystem.

Unknowns:

- Whether `sync-all.sh` covers public repos only, or also
  private Netdata repos requiring SSH credentials.
- Whether the script handles repos that are forks vs origin
  repos.
- The frequency at which the user typically runs it.
- Whether any of the synced repos have a "do not modify
  outside this branch" rule that the skill should warn about.

### Acceptance Criteria

- `<repo>/.agents/skills/mirror-netdata-repos/SKILL.md` exists with
  frontmatter triggers covering "sync netdata repos",
  "cross-repo review", "all netdata repos", "sync-all.sh".
- `<repo>/.agents/skills/mirror-netdata-repos/` includes:
  - A short overview of what the script does.
  - The exact command to run.
  - The list of repos it touches (or a pointer to the
    authoritative list inside the script).
  - When to run it (before a cross-repo grep / review,
    typically).
  - How to add a new repo (edit the script, run it, commit).
  - How to handle repos that have local in-progress work
    (don't blow them away).
  - Cross-references to the user's global `mirrored-repos`
    skill for the bigger research mirror (without hardcoding
    the user's global skills path).
- AGENTS.md "Project Skills Index" section adds a one-line
  entry for `.agents/skills/mirror-netdata-repos/`.
- Skill follows the format convention established by
  SOW-0002.

## Analysis

Sources to consult during stage 2:

- `${NETDATA_REPOS_DIR}/sync-all.sh` (the script the skill
  documents).
- `${NETDATA_REPOS_DIR}/` directory listing (current mirror
  state).
- The user's global `mirrored-repos` skill (related; documents
  the larger observability-projects mirror).

Risks:

- Low. The skill is documentation around an existing script.
  No code changes; no risk to running infrastructure.
- One non-obvious risk: if a maintainer runs `sync-all.sh`
  blindly while in-progress work is uncommitted in a sub-repo,
  the script could overwrite uncommitted changes. The skill
  must call this out clearly.

## Pre-Implementation Gate

Status: not-started

To be filled during stage 2 when the script is read. Likely
no user decisions are needed beyond "is the existing script
the source of truth, or do we also propose changes to it?"

Sensitive data handling plan:

- This SOW (and every committed artifact it produces) follows
  the spec at
  `<repo>/.agents/sow/specs/sensitive-data-discipline.md`. No
  literal absolute paths, usernames, hostnames, or identifiers
  in any committed file. Every reference uses an env-key
  placeholder (`${KEY_NAME}`) defined in `.env`.
- Specifically required `.env` keys for this SOW:
  `NETDATA_REPOS_DIR` (added if not already present).
- Pre-commit verification grep (from the spec) runs on every
  staged change before commit.

## Implications And Decisions

No user decisions required at this stub stage.

## Plan

1. **Wait for SOW-0002 to close** so the skill format
   convention is locked.
2. Read `${NETDATA_REPOS_DIR}/sync-all.sh`.
3. Walk through one real run on the user's workstation and
   capture the actual behavior.
4. Write the skill.
5. Validate by walking the "add a new repo" recipe end-to-end.
6. Close.

## Execution Log

### 2026-05-03

- Created as a stub during the 4-SOW split.

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
