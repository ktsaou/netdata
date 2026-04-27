#!/usr/bin/env bash
# Common helpers for graphql-audit scripts.
# Sourced from the per-action scripts; not executed directly.

set -euo pipefail

GH_RED='\033[0;31m'
GH_GREEN='\033[0;32m'
GH_YELLOW='\033[1;33m'
GH_GRAY='\033[0;90m'
GH_NC='\033[0m'

gh_repo_root() {
    # Walk up from this _lib.sh; that's stable regardless of caller layout.
    git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel
}

gh_repo_slug() {
    # Owner/repo of the upstream remote (or origin if no upstream).
    local root url
    root="$(gh_repo_root)"
    url="$(git -C "${root}" config --get remote.upstream.url 2>/dev/null \
         || git -C "${root}" config --get remote.origin.url)"
    # Normalise SSH and HTTPS forms to "owner/repo".
    sed -E 's|^.*github\.com[:/]([^/]+/[^/.]+)(\.git)?$|\1|' <<< "${url}"
}

gh_audit_dir() {
    local root dir
    root="$(gh_repo_root)"
    dir="${root}/.local/audits/graphql"
    mkdir -p "${dir}"
    echo "${dir}"
}

# Run gh against the GitHub API. Authentication comes from `gh auth status`.
# No token is required in .env when using the gh CLI directly.
gh_api() {
    if ! command -v gh >/dev/null; then
        echo -e "${GH_RED}[ERROR]${GH_NC} 'gh' CLI is not installed. Install from https://cli.github.com/." >&2
        return 1
    fi
    gh "$@"
}
