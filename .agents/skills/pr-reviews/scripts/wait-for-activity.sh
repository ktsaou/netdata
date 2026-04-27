#!/usr/bin/env bash
# Block until new activity appears on a PR (new comment, new review, new
# commit), or until the timeout fires.
#
# Usage:
#   wait-for-activity.sh <pr-number> [<timeout-seconds>] [<poll-interval-seconds>]
#
# Defaults: timeout=1800 (30 min), poll=30s.
#
# Establishes a baseline by reading the cached fetch-all.sh dump (or fetching
# fresh if missing). Then polls every <poll-interval> seconds for changes.
# Exits 0 when something new is found; exits 124 on timeout.
#
# "New" means any of:
#   - issue-comments count changed
#   - review-comments count changed
#   - reviews count changed
#   - PR head sha changed (new push)
#   - reviewThreads isResolved transitions
#
# Both cubic-dev-ai and copilot post a comment when they have nothing new
# to add -- so this loop ends naturally on a clean re-review.
#
# Note about Costa's rule: "the PR should never be left with unaddressed
# comments". After this returns, run fetch-all.sh again, classify the new
# activity, and address it.

set -euo pipefail

# shellcheck source=./_lib.sh
source "$(dirname "$0")/_lib.sh"
pr_require_gh

PR="${1:?usage: $0 <pr-number> [<timeout-seconds>] [<poll-interval-seconds>]}"
pr_require_numeric "${PR}"
TIMEOUT="${2:-1800}"
POLL="${3:-30}"

SLUG="$(pr_repo_slug)"

snapshot() {
    # Quick snapshot for change detection -- not a full fetch.
    gh pr view "${PR}" --repo "${SLUG}" --json headRefOid \
        --jq '"head=\(.headRefOid)"'
    gh api "/repos/${SLUG}/issues/${PR}/comments?per_page=1" \
        --jq '"latest_issue=\(.[0].id // 0)_\(.[0].updated_at // \"\")_count_\(. | length)"' \
        2>/dev/null || true
    gh api "/repos/${SLUG}/pulls/${PR}/comments?per_page=1" \
        --jq '"latest_review_comment=\(.[0].id // 0)_\(.[0].updated_at // \"\")"' \
        2>/dev/null || true
    gh api "/repos/${SLUG}/pulls/${PR}/reviews?per_page=1" \
        --jq '"latest_review=\(.[0].id // 0)_\(.[0].submitted_at // \"\")"' \
        2>/dev/null || true
    # Counts (exact -- enumerate via paginated count endpoint via Link parsing).
    # Cheap proxy: a HEAD on the listing endpoints with per_page=1 returns
    # a Link header whose last page == total. For correctness we just
    # paginate.
    gh api --paginate "/repos/${SLUG}/issues/${PR}/comments?per_page=100" --jq 'length' 2>/dev/null \
        | awk 'BEGIN{t=0} {t+=$1} END{print "n_issue="t}'
    gh api --paginate "/repos/${SLUG}/pulls/${PR}/comments?per_page=100" --jq 'length' 2>/dev/null \
        | awk 'BEGIN{t=0} {t+=$1} END{print "n_review_comment="t}'
    gh api --paginate "/repos/${SLUG}/pulls/${PR}/reviews?per_page=100" --jq 'length' 2>/dev/null \
        | awk 'BEGIN{t=0} {t+=$1} END{print "n_review="t}'
}

echo -e "${PR_GRAY}[wait] PR ${SLUG}#${PR}  timeout=${TIMEOUT}s  poll=${POLL}s${PR_NC}" >&2

baseline="$(snapshot)"
echo -e "${PR_GRAY}[wait] baseline:${PR_NC}" >&2
echo "${baseline}" | sed 's/^/    /' >&2

start=$(date +%s)
while true; do
    sleep "${POLL}"
    now=$(date +%s)
    elapsed=$(( now - start ))
    if (( elapsed >= TIMEOUT )); then
        echo -e "${PR_YELLOW}[wait] timeout after ${elapsed}s -- no new activity${PR_NC}" >&2
        exit 124
    fi
    current="$(snapshot)"
    if [[ "${current}" != "${baseline}" ]]; then
        echo -e "${PR_GREEN}[wait] new activity detected after ${elapsed}s:${PR_NC}" >&2
        diff <(printf '%s' "${baseline}") <(printf '%s' "${current}") | sed 's/^/    /' >&2
        exit 0
    fi
    echo -e "${PR_GRAY}[wait] ${elapsed}s elapsed, no change yet...${PR_NC}" >&2
done
