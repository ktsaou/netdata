# SOW-0001 - project-writing-collectors skill

## Status

Status: in-progress

Sub-state: structure decisions resolved (1A, 2A, 3A, 4A, 5A, 6A, 8B; 7 — manifesto framing, prose, summary DOs/DONTs per topic, 300-500 lines target with no lengthy code examples); first draft of SKILL.md authored for user review at `.agents/skills/project-writing-collectors/SKILL.md` (242 lines); FUNCTION_UI_SCHEMA.json added as required artifact in the consistency-sync set.

## Requirements

### Purpose

Create a runtime project skill `project-writing-collectors` that orients an AI assistant arriving cold at any Netdata data-collection task: tells the assistant *what canonical documents already exist*, *when to read each*, and *what is at stake* if collector authoring conventions are violated.

The skill is a gateway. It must never duplicate canonical documentation. The repo already owns the deep references (NIDL framework, plugin frameworks, profile format, plugin protocol, DYNCFG, functions, streaming, integrations pipeline) — the skill points to them.

The skill exists because, without it, every new collector task forces the user to manually re-teach: where docs live, what NIDL is, what frameworks v1/v2 are, what is mandatory in metadata.yaml / config_schema.json / health.d, that vnodes exist for remotely-monitored systems, that SNMP uses profiles, that no metric should default to zero, that hot paths must not log or allocate, etc. Each repetition is a tax on the user's time and a risk for incorrect output.

### User Request

Quoted from chat:

> "I want us to work on a new skill: project-writing-collectors with description: best practices for Netdata collectors - read this before adding data collection plugins or modules to Netdata"

> "let an assistant understand what it is dealing with"

> "We need a balance. The assistants must get enough information to understand if and when they need to read additional documents, and grasp what is at stake when working with collectors. Best practices, Bad practices, Pointers for additional documentation."

Constraints from the user:

- General rules for ALL plugins (per-plugin rules go elsewhere — possibly per-plugin skills later).
- Not an inventory (so go.d module names are out — categories only).
- Not a comprehensive guide (the repo already has those).
- The skill is *live* and may be updated when gaps are found, with user permission.

### Assistant Understanding

Facts:

- The repo already owns substantial collector documentation: `docs/NIDL-Framework.md` (442 lines), `src/go/BEST-PRACTICES.md` (387), `src/go/COLLECTOR-LIFECYCLE.md` (1209), `src/plugins.d/README.md` (909), `src/plugins.d/DYNCFG.md` (468), `src/plugins.d/FUNCTION_UI_REFERENCE.md` (1714), `src/go/plugin/go.d/collector/snmp/profile-format.md` (2046), `src/go/plugin/ibm.d/framework/README.md` (153), plus a 56-line `src/go/plugin/go.d/docs/how-to-write-a-collector.md` for go.d.
- A landing-page pattern already exists in the repo: `src/go/AGENTS.md` and `src/go/CLAUDE.md` (17 lines each, content-equal, scoped to IBM.D plugin) — short router with 5 rules. `src/go/plugin/ibm.d/AGENTS.md` (145 lines) is a deeper IBM.D checklist.
- AGENTS.md mandates collector consistency between code, metadata.yaml, config_schema.json, stock conf, health.d, README.
- ~30 plugins exist across C, Go, Rust, Python, Bash, eBPF.
- 12 recurring bad-practice patterns identified with file:line evidence (see Analysis).
- Existing project skills under `.agents/skills/` range from 162 (graphql-audit) to 508 (pr-reviews) lines; average ~330.
- SOW directories were empty at SOW-creation time; this is SOW-0001 in numbering despite AGENTS.md text mentioning SOW-0003.

Inferences:

- A single SKILL.md is appropriate based on existing skill sizes and the orientation-only scope.
- Updating-this-skill must be an explicit footer rule, since the user wants it to be live.
- Audit rules and authoring rules largely overlap — splitting into two skills risks duplication; embedding both in one skill keeps the merge gate close to the authoring guidance.

Unknowns:

- Whether a separate `project-auditing-collectors` skill is desirable, or audit lives inside this skill (decision needed).
- Whether to create companion docs in the skill dir for true gaps (vnodes, error handling, labels, logging conventions), or leave gaps as 1-2 line inline summaries plus follow-up SOWs (decision needed).

### Acceptance Criteria

- An assistant given a new collector task and only this skill must be able to:
  - identify which plugin tree the work belongs in;
  - locate every canonical doc relevant to that work;
  - know the non-negotiable rules (no zero defaults, no log spam, no per-iteration alloc/reconnect, no missing metadata.yaml/health.d, vnodes for remote, profiles for SNMP);
  - know the audit checklist before merge.
- Verification: walk through 4 self-tests with the skill content visible:
  1. New go.d module → routes to BEST-PRACTICES.md, COLLECTOR-LIFECYCLE.md, NIDL, how-to-write-a-collector.md, integrations/templates.
  2. New SNMP profile → routes to profile-format.md.
  3. New C external plugin → routes to plugins.d/README.md, src/collectors/README.md.
  4. New ibm.d module → routes to ibm.d/framework/README.md, ibm.d/AGENTS.md.
- Each of the 12 bad-practice patterns must be flagged either in the "non-negotiables" or "audit checklist" sections of the skill.
- AGENTS.md "Project Skills Index" must list the new skill.

## Analysis

### Canonical Documents That Already Exist

(verified to exist via Bash on 2026-05-02)

| Topic | Path | Lines |
|---|---|---|
| NIDL framework | docs/NIDL-Framework.md | 442 |
| go.d collector authoring (V2) | src/go/plugin/go.d/docs/how-to-write-a-collector.md | 56 |
| go.d best practices | src/go/BEST-PRACTICES.md | 387 |
| go.d collector lifecycle | src/go/COLLECTOR-LIFECYCLE.md | 1209 |
| go.d plugin overview | src/go/plugin/go.d/README.md | (verified, lines unread) |
| ibm.d framework | src/go/plugin/ibm.d/framework/README.md | 153 |
| ibm.d landing/checklist | src/go/plugin/ibm.d/AGENTS.md | 145 |
| ibm.d plugin overview | src/go/plugin/ibm.d/README.md | (verified) |
| go.d/ibm.d landing | src/go/AGENTS.md = src/go/CLAUDE.md | 17 |
| go.d agent framework | src/go/plugin/agent/README.md | (verified) |
| External plugin protocol (PLUGINSD) | src/plugins.d/README.md | 909 |
| Collector privileges/types | src/collectors/README.md | (verified) |
| SNMP profile format | src/go/plugin/go.d/collector/snmp/profile-format.md | 2046 |
| DYNCFG protocol | src/plugins.d/DYNCFG.md | 468 |
| DYNCFG (developer corner) | docs/developer-and-contributor-corner/dyncfg.md | (verified) |
| Functions reference | src/plugins.d/FUNCTION_UI_REFERENCE.md | 1714 |
| Functions developer guide | src/plugins.d/FUNCTION_UI_DEVELOPER_GUIDE.md | (verified) |
| Streaming & replication | src/streaming/README.md | (verified) |
| Streaming parent clusters | src/streaming/PARENT-CLUSTERS.md | (verified) |
| Health alerts (reference) | src/health/REFERENCE.md | (verified) |
| Health alerts (overview) | src/health/README.md | (verified) |
| Alert config ordering | src/health/alert-configuration-ordering.md | (verified) |
| Overriding stock alerts | src/health/overriding-stock-alerts.md | (verified) |
| Claim / registration | src/claim/README.md | (verified) |
| Integrations pipeline | integrations/README.md | (verified) |
| Integration templates | integrations/templates/README.md | (verified) |
| Dynamic-configuration UI | docs/netdata-agent/configuration/dynamic-configuration.md | (verified) |
| Replication of past samples | docs/observability-centralization-points/metrics-centralization-points/replication-of-past-samples.md | (verified) |
| charts.d.plugin (legacy) | src/collectors/charts.d.plugin/README.md | (legacy) |
| python.d.plugin (legacy) | src/collectors/python.d.plugin/README.md | (legacy) |

### Documentation Gaps

Topics where the repo lacks a canonical doc and the skill must either inline guidance or track a follow-up SOW:

- Netdata labels (host/chart/instance) — no canonical reference.
- Configuration override hierarchy (DYNCFG > /etc/netdata > stock > internal defaults) — scattered.
- Memory allocation conventions (mallocz/freez/strdupz) — only in libnetdata code comments.
- nd_log conventions (levels, throttling) — scattered across libnetdata.
- Error handling conventions for collectors — none unified.
- Vnode registration — code only, no public doc.
- Chart/dimension definitions — examples in collectors only.
- Plugin update interval / cadence guidance — scattered.
- Testing patterns for collectors — no unified doc.
- netipc library — canonical lives at github.com/netdata/plugin-ipc, not linked from this repo.

### Recurring Bad Practices (file:line evidence)

| # | Pattern | Example |
|---|---|---|
| 1 | Default-to-zero on missing data | src/collectors/proc.plugin/proc_net_dev.c:782 (TODO comment admits the bug) |
| 2 | Log spam in iteration loops | ebpf.plugin (commit bde8262e33) |
| 3 | Allocations in collection loop | src/go/plugin/go.d/collector/ap/collect.go:53 |
| 4 | Reconnects per iteration | SNMP topology pre-hardening |
| 5 | Vague error context | many go.d collectors return raw err with no wrap |
| 6 | Silent fallbacks | src/go/plugin/go.d/collector/mysql/mysqlfunc/error_info.go (fallbackTable) |
| 7 | Hardcoded options without DYNCFG | many SNMP/timeout defaults |
| 8 | Missing vnode support | refactor commit 4245df367f |
| 9 | Metrics shipped without metadata.yaml/health.d | 4 metadata gaps across 133 collectors |
| 10 | Ignored syscall return codes | systemd-journal NULL guard commit b455bbe1c |
| 11 | SNMP collectors without profiles | older topology code |
| 12 | Blocking inside collection loop | apps.plugin commit 6084e3f98b |

### External Pattern Reference

- Telegraf input plugin guide (189 lines): minimal interface spec, convention over walkthrough.
- OTel Collector CONTRIBUTING.md (470 lines): prescriptive PR shape, named audiences, defers RFC detail.
- Datadog integrations README (58 lines): defers all detail to external docs site.
- Prometheus exporter guides (~686 lines combined): hands-on, example-first.
- Synthesis: lead with role/audience, links table, do/don't, defer deep detail.

### Existing Project Skills (size reference)

| Skill | Lines |
|---|---|
| graphql-audit | 162 |
| sonarqube-audit | 190 |
| coverity-audit | 474 |
| pr-reviews | 508 |

## Pre-Implementation Gate

Status: needs-user-decision

Problem / root-cause model:

- AI assistants approaching Netdata data collection do not know which canonical docs exist (NIDL, frameworks, profiles, plugins.d protocol, DYNCFG, functions, streaming, integrations pipeline). Without orientation they: invent metric grouping, miss vnodes, log-spam in hot paths, default missing data to zero, miss metadata.yaml/health.d/config_schema, hardcode options, write SNMP without profiles, allocate per-iteration, reconnect per iteration, ignore syscall return codes. Evidence: 12 recurring patterns documented with file:line above. Without a router skill the user must reteach all of this in every new session.

Evidence reviewed:

- 25+ canonical in-repo docs verified to exist (Analysis section).
- src/go/AGENTS.md / src/go/CLAUDE.md — existing 17-line landing page pattern, IBM.D-scoped.
- src/go/plugin/go.d/docs/how-to-write-a-collector.md — go.d-specific 56-line guide.
- 12 bad-practice patterns from subagent investigation.
- External patterns from Telegraf, OTel, Datadog, Prometheus.
- Existing project skills (sizing reference).

Affected contracts and surfaces:

- New file: `.agents/skills/project-writing-collectors/SKILL.md`.
- AGENTS.md: add entry under "Runtime input project skills" in the Project Skills Index.
- No code changes; no spec changes; no metadata.yaml/config_schema changes.
- The skill will reference canonical docs by relative path; renames in the future require skill update.

Existing patterns to reuse:

- Frontmatter format from existing project skills (`name`, `description`, `type`).
- Router shape from src/go/AGENTS.md (short, numbered rules, links table).
- Length range from existing project skills (160-510 lines).
- File:line evidence style from coverity-audit / sonarqube-audit (for the bad-practices section).

Risk and blast radius:

- Skill bloat: tries to cover everything; assistants stop reading. Mitigation: hard cap on length, defer all deep content to canonical docs.
- Drift: if BEST-PRACTICES.md, NIDL-Framework.md, or profile-format.md change paths, skill links break. Mitigation: explicit footer rule that PRs touching collectors must update the skill if conventions or doc paths change.
- Coverage illusion: a 12-item checklist does not guarantee an audit catches a bug. Mitigation: each bad-practice row carries file:line evidence so reviewers verify by example, not by checkbox.
- Stale plugin landscape: plugins added/removed without updating skill. Mitigation: include the skill in the collector-consistency rule already in AGENTS.md.

Sensitive data handling plan:

- The skill ships in a public repository. It must contain no customer names, no private endpoints, no credentials, no internal tooling references. Bad-practice file:line evidence cites public source code only. No issue.

Implementation plan:

(Awaiting user decisions before finalizing.)

1. Author SKILL.md based on the structure decided in "Open decisions".
2. Register the skill in AGENTS.md → Project Skills Index → Runtime input skills.
3. Self-test with the 4 routing scenarios listed in Acceptance Criteria.
4. Run the 12 bad-practice patterns against the SKILL.md to confirm each is flagged.
5. Open a follow-up SOW (or follow-ups, plural) for each canonical-doc gap that the user wants the skill to point at but no canonical doc yet exists.

Validation plan:

- 4 self-test routing walkthroughs (above).
- 12-pattern coverage verification (above).
- User review of structure before implementation begins.
- After implementation, re-walkthrough with a fresh subagent that has not seen the design conversation: does it route correctly?

Artifact impact plan:

- AGENTS.md: add new skill entry under Runtime input project skills (single trigger line + 2-3 lines of "use when").
- Runtime project skills: this SOW *creates* the skill.
- Specs: no spec change expected (not a behavioral change).
- End-user/operator docs: none affected.
- End-user/operator skills: none affected.
- SOW lifecycle: this SOW transitions pending→current after decisions, current→done on completion. Gap follow-ups (vnodes doc, error-handling doc, labels doc, logging conventions doc, etc.) tracked as separate SOWs in pending/.

Open decisions: resolved 2026-05-02.

1. Single skill, audit checklist embedded (1A).
2. Full plugin coverage in v1 (2A).
3. Gap topics handled inline as 1-2 line guidance with pointers; canonical doc gaps tracked as follow-up SOWs (3A).
4. Plugin landscape embedded in SKILL.md (4A).
5. Legacy plugins (charts.d, python.d) included with "do not add new modules" marker (5A).
6. Bad-practice file:line evidence kept as nudges, not enforcement (6A); reframed away from "audit gate" toward "past pain looked like this".
7. Length target 300-500 lines; manifesto framing (prose, trust the reader, no MUST/MANDATORY/NEVER), summary DOs and DON'Ts per topic, no lengthy code examples. Draft came in at 242 lines — lean by design, will expand on user request if specific sections feel thin.
8. Skill description tightened to "best practices + orientation" framing (8B), with broad trigger keywords for collector/plugin/module/integration/data-collection work.

Late addition by user: a function schema JSON file (`src/plugins.d/FUNCTION_UI_SCHEMA.json`) is the contract for any collector that exposes a function. Added to the consistency-sync set in the Documentation section of the skill, and called out in the Functions topic with pointers to FUNCTION_UI_DEVELOPER_GUIDE.md and FUNCTION_UI_REFERENCE.md.

## Implications And Decisions

Pending user response to Open Decisions 1-7.

## Plan

(Pending Pre-Implementation Gate close.)

## Execution Log

### 2026-05-02

- Investigation completed via 4 parallel subagents (in-repo doc inventory, plugin landscape, recurring bad-practice patterns, external authoring-guide structures).
- 25+ canonical docs verified to exist on disk.
- Critical missed doc found: `src/go/plugin/go.d/docs/how-to-write-a-collector.md` (subagent A miss, recovered by direct grep).
- 12 bad-practice patterns collected with file:line evidence.
- 4 external project authoring guides analyzed for structure inspiration.
- SOW filed; awaiting structure decisions.

## Validation

Pending implementation.

## Outcome

Pending.

## Lessons Extracted

Pending.

## Followup

(To be filled at implementation; expected: per-gap-doc follow-up SOWs for vnodes / error handling / labels / logging conventions / netipc cross-link.)
