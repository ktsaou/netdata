---
name: coverity-audit
description: Triage Coverity Scan defects (https://scan.coverity.com) for this project — fetch outstanding/dismissed/fixed defects, run multi-model analysis, finalize verdicts (Bug / FalsePositive / Intentional) on the upstream tracker. Use when the user asks to "review Coverity defects", "triage Coverity findings", "fetch Coverity outstanding", or anything mentioning Coverity Scan, CIDs, or the scan.coverity.com web UI.
---

# Coverity Scan triage skill

This skill drives the Coverity Scan unofficial JSON API to fetch defect lists,
fetch per-defect details, and apply triage decisions (classification, severity,
action, comment). The Coverity Scan **public** site has no documented API; the
scripts here mimic what the browser does.

The skill operates on the project configured in `.env` (see Setup). Scripts
auto-detect the repo root and write all artifacts under `<repo-root>/.local/`.

## MANDATORY — startup sequence when this skill is invoked

Do these steps in this order. Skipping any step costs the user their session.

### Step 1 — ask the user for a fresh cookie

Coverity Scan auth is cookie-based and tied to a live browser session. Give
the user the exact recipe (verbatim):

> 1. Open https://scan.coverity.com/projects/<owner>-<repo>?tab=overview
>    (replace `<owner>-<repo>` with the project slug; e.g. `netdata-netdata`).
> 2. Click **"View Defects"** at the top right -- a new tab opens to
>    https://scan4.scan.coverity.com/# (or `scanN.scan.coverity.com` --
>    Coverity load-balances across hosts; use whichever URL you land on as
>    `COVERITY_HOST` in `.env`).
> 3. **Keep that tab open the whole time we work.** Closing it kills the
>    session immediately.
> 4. Press **F12** to open DevTools, switch to the **Network** tab.
> 5. Click any defect in the list -- 3-4 requests appear in the Network tab.
> 6. Right-click any of those requests, select **Copy > Copy as cURL**.
> 7. Paste the entire curl command back here.

If the curl the user pastes does NOT contain a `-b 'cookie=...'` line, ask
again -- they likely picked "Copy as fetch" or "Copy as Node.js fetch"
instead of "Copy as cURL".

### Step 2 — save the cookie in `.env`

Extract the value of the `-b` argument from the user's curl and write/update
`<repo-root>/.env` with:

```bash
COVERITY_COOKIE='<paste-the-entire-cookie-blob-here>'
COVERITY_PROJECT_ID=<numeric-projectId-from-the-URL>
COVERITY_VIEW_OUTSTANDING=<viewId-of-the-Outstanding-view>
COVERITY_HOST=https://scan4.scan.coverity.com
```

`.env` is gitignored. The cookie blob must include both `COVJSESSIONID-build`
and `XSRF-TOKEN`. The scripts extract `XSRF-TOKEN` automatically.

### Step 3 — start the keepalive (background)

```
Bash tool with run_in_background=true,
command="bash .agents/skills/coverity-audit/scripts/keepalive.sh"
```

The keepalive **exits non-zero the moment a ping fails** (session expired,
browser tab closed, cookie went bad). The orchestrator's background-task
notification fires immediately so you know to ask the user for a fresh cookie
and restart from Step 1.

**Stop the keepalive at the end of the triage session** by killing its
background task.

### Step 4 — proceed with the actual triage work

Now and only now is it safe to fetch tables, fetch details, run analyzers,
finalize defects.

## MANDATORY — keep this skill alive

**If you (the agent) discover a new pattern, gotcha, working flow, correction,
or any piece of knowledge while running this skill — update this `SKILL.md`
AND commit it BEFORE proceeding. Knowledge that isn't committed is lost.**

Examples of things to capture:
- New verdict mapping discovered
- New API endpoint/parameter that was undocumented
- New failure mode (Cloudflare quirks, session-expiry signals, rate limits)
- New scrape pattern for displayImpact, owner, history, etc.

## Setup

### 1. Browser tab — must stay open

Coverity authentication is cookie-based and tied to a live browser session.
**The Coverity Scan tab must remain open in the user's browser for the
entire triage run.** Closing the tab kills the cookie, no amount of
keepalive pings can revive it.

### 2. Capture the cookie

Ask the user (Costa) to:

1. Open https://scan.coverity.com and navigate to the project's defects view.
2. Open DevTools → Network panel.
3. Click any request to `*.scan.coverity.com` (e.g. the `table.json` XHR).
4. Right-click → **Copy → Copy as cURL**.
5. Paste the entire curl command somewhere the agent can read it (the chat,
   a temp file). The agent will extract the `-b 'cookie=...'` value.

The agent then writes `<repo-root>/.env` (gitignored) with:

```bash
# Coverity Scan
COVERITY_COOKIE='<paste the entire single-quoted cookie string from the curl -b argument>'
COVERITY_PROJECT_ID=<numeric projectId — find it in the URL or table.json query>
COVERITY_VIEW_OUTSTANDING=<viewId of the "Outstanding" view, optional but recommended>
# COVERITY_HOST defaults to https://scan4.scan.coverity.com — override only if Coverity routed you elsewhere
```

The cookie blob must include both the session id (`COVJSESSIONID-build=...`)
and the CSRF token (`XSRF-TOKEN=...`). The scripts extract `XSRF-TOKEN` from
the cookie automatically.

### 3. Start the keepalive (background)

Run `keepalive.sh` as a background task at the start of the triage session:

```
Bash tool with run_in_background=true,
command="bash .agents/skills/coverity-audit/scripts/keepalive.sh"
```

It pings every 5 minutes (`PING_INTERVAL=300`) and exits non-zero the moment
the session goes bad — that's the signal to ask the user for a fresh cookie.

**Stop** the keepalive at the end of the session.

## Verdict enum

These are the only valid verdicts. The mapping to Coverity attributes lives
in `scripts/finalize-defect.sh`:

| Verdict                          | Coverity classification | Action          |
|----------------------------------|-------------------------|-----------------|
| TRUE_BUG_MEMORY_CORRUPTION       | Bug (24)                | Fix Submitted (3) |
| TRUE_BUG_CRASH                   | Bug (24)                | Fix Submitted (3) |
| TRUE_BUG_RESOURCE_LEAK           | Bug (24)                | Fix Submitted (3) |
| TRUE_BUG_LOGIC                   | Bug (24)                | Fix Submitted (3) |
| TRUE_BUG_UB                      | Bug (24)                | Fix Submitted (3) |
| FALSE_POSITIVE_GUARD_EXISTS      | False Positive (22)     | Ignore (5)        |
| FALSE_POSITIVE_UNREACHABLE       | False Positive (22)     | Ignore (5)        |
| FALSE_POSITIVE_TRUSTED_INPUT     | False Positive (22)     | Ignore (5)        |
| FALSE_POSITIVE_TOOL_MODEL        | False Positive (22)     | Ignore (5)        |
| IMPOSSIBLE_CONDITIONS            | False Positive (22)     | Ignore (5)        |
| COSMETIC                         | Intentional (23)        | Ignore (5)        |
| NEEDS_HUMAN                      | (no-op — flag for the user) |
| CODE_GONE                        | (no-op when refactor removed the path; map to FalsePositive+Ignore in pipeline) |

Severity (Coverity's displayImpact -> attribute 1):
- High -> 11 (Major), Medium -> 12 (Moderate), Low -> 13 (Minor),
  unknown -> 10 (Unspecified)

## Phases (queue scopes)

When walking the defect tracker, group work by phase:

1. **Outstanding** — the live queue. Default; verdicts always applied.
2. **Old / dismissed** — previously closed. Apply a verdict only if it
   *disagrees* with the existing classification.
3. **Fixed** — already fixed in code. Verify, usually no-op.
4. **Unclassified non-outstanding** — corner cases.
5. **Re-runs** — anything we previously triaged that has resurfaced.

## Workflow

### Step 1 — fetch the table (per phase)

Inspect the Coverity UI, count pages for the view, then:

```
bash .agents/skills/coverity-audit/scripts/fetch-table.sh \
    "${COVERITY_VIEW_OUTSTANDING}" 7 .local/audits/coverity/raw/outstanding
```

Produces `.local/audits/coverity/raw/outstanding-page1.json` ... and a
combined flat array at `.local/audits/coverity/raw/outstanding-all.json`.

### Step 2 — fetch per-defect details

```
bash .agents/skills/coverity-audit/scripts/fetch-details.sh \
    .local/audits/coverity/raw/outstanding-all.json \
    .local/audits/coverity/details
```

One file per CID at `.local/audits/coverity/details/cid-<CID>.json`.

### Step 3 — triage one defect

For each CID, the orchestrator drives a multi-model verdict. Suggested layout
(matches the established pipeline):

```
.local/audits/coverity/triage/<phase>/cid-<CID>/
    analyzers/        # per-model output (glm, kimi, qwen, ...)
    decision.json     # consensus or override decision
    comment.txt       # ASCII-only comment posted to Coverity
    fix.commit        # if TRUE_BUG_*: SHA of the fix commit
```

### Step 4 — finalize on Coverity

Once verdict + comment + (optional) fix commit are in place:

```
bash .agents/skills/coverity-audit/scripts/finalize-defect.sh \
    <CID> <VERDICT> <PHASE> .local/audits/coverity/triage/<phase>/cid-<CID>/comment.txt \
    [<commit-sha>]
```

This:
- Skips silently for `NEEDS_HUMAN` and `CODE_GONE`.
- Reads `displayImpact` from the table-fetch artifacts to derive severity.
- Appends `Fix commit: <sha>` to the comment when a SHA is given.
- Posts the JSON to `/sourcebrowser/updatedefecttriage.json`.

## ASCII-only comments — non-negotiable

Coverity Scan sits behind Cloudflare. The WAF rejects bodies containing
non-ASCII bytes (em-dashes, smart quotes, accented letters) with a 403
Cloudflare challenge that looks like an expired-session error but isn't.

- Use `--` instead of em-dash (U+2014).
- Use straight quotes `"` `'` instead of smart quotes.
- The scripts reject non-ASCII before the network round-trip.

## Failure modes — quick diagnosis

| Symptom                                            | Likely cause                                  |
|----------------------------------------------------|-----------------------------------------------|
| HTTP 401 / 403 / 302 → HTML response               | Session expired. Recapture cookie from browser. |
| HTTP 403 with Cloudflare challenge HTML            | Either non-ASCII in comment, or browser tab closed. |
| keepalive.sh exits non-zero                        | Same as above. Ask user for a fresh cookie.   |
| HTTP 200 but `defectStatus` empty                  | XSRF token stale. Recapture cookie.            |
| `Could not parse session from .env`                | Wrong cookie format. Paste the FULL `-b 'k=v; ...'` string. |

## Recurring tips

- The `lastDefectInstanceId` field, NOT `cid`, is what `defectdetails.json` wants.
- Pages appear paginated server-side. The `pageSize` is fixed by the view; use
  `totalCount / pageSize` for `<pages>` in `fetch-table.sh`.
- Coverity caches results aggressively; if a CID disappears from "Outstanding"
  immediately after finalize, a fresh fetch can take a few seconds to reflect.
- Idempotence: `fetch-details.sh` skips files already present, so partial runs
  are safe to re-invoke.
