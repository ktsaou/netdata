#!/usr/bin/env bash
# Resolve a review thread on a PR (marks the conversation as done in the UI).
#
# Usage:
#   resolve-thread.sh <thread-node-id>
#
# <thread-node-id> is the GraphQL id from review-threads.json -> .[].id.
# It looks like "PRRT_kwDO..." -- not the numeric REST id.
#
# Resolving a thread does NOT require a reply, but the convention is to reply
# first then resolve. See SKILL.md.

set -euo pipefail

# shellcheck source=./_lib.sh
source "$(dirname "$0")/_lib.sh"
pr_require_gh

THREAD_ID="${1:?usage: $0 <thread-node-id>}"

gh api graphql -F threadId="${THREAD_ID}" -f query='
    mutation($threadId:ID!) {
        resolveReviewThread(input: {threadId: $threadId}) {
            thread { id isResolved }
        }
    }
' --jq '.data.resolveReviewThread.thread | "resolved=\(.isResolved) id=\(.id)"'
