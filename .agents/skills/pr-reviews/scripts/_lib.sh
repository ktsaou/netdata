#!/usr/bin/env bash
# Common helpers for pr-reviews scripts.
# Sourced from the per-action scripts; not executed directly.

set -euo pipefail

# Color vars are used by sourcing scripts; shellcheck cannot see that.
# shellcheck disable=SC2034
PR_RED='\033[0;31m'
# shellcheck disable=SC2034
PR_GREEN='\033[0;32m'
# shellcheck disable=SC2034
PR_YELLOW='\033[1;33m'
# shellcheck disable=SC2034
PR_GRAY='\033[0;90m'
# shellcheck disable=SC2034
PR_NC='\033[0m'

pr_repo_root() {
    git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel
}

# Owner/repo of the upstream remote (or origin if no upstream).
# Override with PR_REPO_SLUG=owner/repo for cross-repo work.
# Uses bash parameter expansion so repo names containing dots parse correctly.
pr_repo_slug() {
    if [[ -n "${PR_REPO_SLUG:-}" ]]; then
        echo "${PR_REPO_SLUG}"
        return
    fi
    local root url
    root="$(pr_repo_root)"
    url="$(git -C "${root}" config --get remote.upstream.url 2>/dev/null \
         || git -C "${root}" config --get remote.origin.url)"
    url="${url%.git}"               # strip trailing .git
    url="${url#*github.com[:/]}"    # strip everything up to and including github.com:/
    echo "${url}"
}

# Audit/state directory for pr-reviews artifacts.
pr_audit_dir() {
    local root dir
    root="$(pr_repo_root)"
    dir="${root}/.local/audits/pr-reviews"
    mkdir -p "${dir}"
    echo "${dir}"
}

# Per-PR working directory.
pr_state_dir() {
    local pr="${1:?usage: pr_state_dir <pr-number>}"
    local dir
    dir="$(pr_audit_dir)/pr-${pr}"
    mkdir -p "${dir}"
    echo "${dir}"
}

# Verify gh is available and authenticated. Bail loudly otherwise.
pr_require_gh() {
    if ! command -v gh >/dev/null; then
        echo -e "${PR_RED}[ERROR]${PR_NC} 'gh' CLI not installed. https://cli.github.com/" >&2
        return 1
    fi
    if ! gh auth status >/dev/null 2>&1; then
        echo -e "${PR_RED}[ERROR]${PR_NC} 'gh' is not authenticated. Run 'gh auth login'." >&2
        return 1
    fi
}

# Bot logins recognized by the skill. The first regex matches the AI reviewers
# the skill iterates with autonomously; the second matches CI/quality bots
# whose comments are informational (sonar quality gate, etc.).
PR_AI_BOT_RE='^(cubic-dev-ai|copilot|copilot-pull-request-reviewer|github-copilot)\[bot\]$'
PR_INFO_BOT_RE='^(sonarqubecloud|netdata-bot|github-actions|coderabbitai)\[bot\]$'

# Classify a login -> "ai_bot" | "info_bot" | "human".
pr_classify_author() {
    local login="$1"
    if [[ "${login}" =~ ${PR_AI_BOT_RE} ]]; then
        echo "ai_bot"
    elif [[ "${login}" =~ ${PR_INFO_BOT_RE} ]]; then
        echo "info_bot"
    else
        echo "human"
    fi
}

# Pretty timestamp in UTC.
pr_now_utc() {
    date -u +%Y-%m-%dT%H:%M:%SZ
}
