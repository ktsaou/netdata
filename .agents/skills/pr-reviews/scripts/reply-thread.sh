#!/usr/bin/env bash
# Reply inside an existing review-comment thread.
#
# Usage:
#   reply-thread.sh <pr-number> <comment-id> <body-text>
#   reply-thread.sh <pr-number> <comment-id> @<file>
#
# <comment-id> is the REST databaseId of any comment in the thread (e.g. one
# of `databaseId` from review-threads.json -> .comments.nodes[]). The new
# reply is anchored under that thread automatically.
#
# To resolve the thread after replying, call resolve-thread.sh with the
# thread's GraphQL node id.

set -euo pipefail

# shellcheck source=./_lib.sh
source "$(dirname "$0")/_lib.sh"
pr_require_gh

PR="${1:?usage: $0 <pr-number> <comment-id> <body>}"
pr_require_numeric "${PR}"
COMMENT_ID="${2:?usage}"
BODY_ARG="${3:?usage}"

if [[ "${BODY_ARG}" == @* ]]; then
    BODY="$(cat "${BODY_ARG#@}")"
else
    BODY="${BODY_ARG}"
fi

if [[ -z "${BODY// /}" ]]; then
    echo -e "${PR_RED}[ERROR]${PR_NC} Empty body." >&2
    exit 2
fi

SLUG="$(pr_repo_slug)"
echo -e "${PR_GRAY}[reply-thread] PR ${SLUG}#${PR} reply to comment ${COMMENT_ID}${PR_NC}" >&2

gh api --method POST "/repos/${SLUG}/pulls/${PR}/comments/${COMMENT_ID}/replies" \
    -f body="${BODY}" \
    --jq '"posted reply id=\(.id) url=\(.html_url)"'
