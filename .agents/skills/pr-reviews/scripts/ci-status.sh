#!/usr/bin/env bash
# Report current CI status for a PR head commit.
#
# Usage:
#   ci-status.sh <pr-number>          # human-readable summary
#   ci-status.sh <pr-number> --json   # raw JSON
#
# This is read-only. It is the precondition before any push: if a CI run is
# in progress or just finished, the existing results are the ones we want to
# capture before pushing again -- otherwise the next push restarts CI and we
# lose the prior signal.

set -euo pipefail

# shellcheck source=./_lib.sh
source "$(dirname "$0")/_lib.sh"
pr_require_gh

PR="${1:?usage: $0 <pr-number> [--json]}"
pr_require_numeric "${PR}"
JSON=0
[[ "${2:-}" == "--json" ]] && JSON=1

SLUG="$(pr_repo_slug)"

# pr view --json statusCheckRollup gives the per-check breakdown.
data="$(gh pr view "${PR}" --repo "${SLUG}" --json statusCheckRollup,headRefOid,mergeStateStatus)"

if (( JSON )); then
    printf '%s\n' "${data}"
    exit 0
fi

head_sha="$(jq -r '.headRefOid' <<< "${data}" | head -c 10)"
merge_state="$(jq -r '.mergeStateStatus' <<< "${data}")"

echo "Head: ${head_sha}   Merge state: ${merge_state}"
echo

# Group checks by conclusion / status.
jq -r '
    .statusCheckRollup
    | map(. + {key: ((.conclusion // .status) // "PENDING")})
    | group_by(.key)
    | map({key: .[0].key, count: length, names: [.[].name // .[].context]})
    | sort_by(.key)
    | .[]
    | "\(.key)\t\(.count)\t\((.names | sort | unique | join(", ")[0:200]))"
' <<< "${data}" | column -t -s $'\t'

echo
total=$(jq '.statusCheckRollup | length' <<< "${data}")
fail=$(jq '[.statusCheckRollup[] | select((.conclusion // "")|test("FAILURE|TIMED_OUT|CANCELLED"))] | length' <<< "${data}")
running=$(jq '[.statusCheckRollup[] | select((.status // "")=="IN_PROGRESS" or (.status // "")=="QUEUED")] | length' <<< "${data}")
echo "Total checks: ${total}   Failing: ${fail}   Running: ${running}"

if (( running > 0 )); then
    echo -e "${PR_YELLOW}WARNING: ${running} check(s) still running. Pushing now will lose those results.${PR_NC}" >&2
    exit 2
fi
if (( fail > 0 )); then
    echo -e "${PR_RED}WARNING: ${fail} check(s) failing. Address these BEFORE pushing.${PR_NC}" >&2
    exit 3
fi
exit 0
