#!/usr/bin/env bash
# Common helpers for sonarqube-audit scripts.
# Sourced from the per-action scripts; not executed directly.

set -euo pipefail

# IMPORTANT: define with $'...' so the variables contain real ESC bytes,
# not the literal four-character string "\033". This way both `echo -e
# "${SQ_RED}..."` and `printf '%s' "${SQ_RED}..."` render correctly --
# without forcing every printf format string to be the variable itself
# (which trips shellcheck SC2059) or %b (which adds inconsistency).
#
# Color vars are referenced by sourcing scripts; shellcheck cannot see that.
# shellcheck disable=SC2034
SQ_RED=$'\033[0;31m'
# shellcheck disable=SC2034
SQ_GREEN=$'\033[0;32m'
# shellcheck disable=SC2034
SQ_YELLOW=$'\033[1;33m'
# shellcheck disable=SC2034
SQ_GRAY=$'\033[0;90m'
# shellcheck disable=SC2034
SQ_NC=$'\033[0m'

sq_repo_root() {
    git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel
}

sq_load_env() {
    local root env
    root="$(sq_repo_root)"
    env="${root}/.env"
    if [[ ! -r "${env}" ]]; then
        echo -e "${SQ_RED}[ERROR]${SQ_NC} Missing ${env}. See SKILL.md for the .env template." >&2
        return 1
    fi
    set -a
    # shellcheck disable=SC1090
    source "${env}"
    set +a

    : "${SONAR_TOKEN:?SONAR_TOKEN is empty in .env}"
    : "${SONAR_HOST_URL:=https://sonarcloud.io}"
    : "${SONAR_PROJECT:?SONAR_PROJECT is empty in .env (e.g. netdata_netdata)}"
    # SONAR_ORG is optional today -- the existing scripts don't pass it to
    # the API, but qualityprofile management calls (documented in SKILL.md)
    # require it. Default empty; consumers should fail loudly if they need it.
    : "${SONAR_ORG:=}"
    export SONAR_TOKEN SONAR_HOST_URL SONAR_PROJECT SONAR_ORG
}

sq_audit_dir() {
    local root dir
    root="$(sq_repo_root)"
    dir="${root}/.local/audits/sonarqube"
    mkdir -p "${dir}"
    echo "${dir}"
}

# Cloudflare in front of api.sonarcloud.io rejects non-ASCII bodies.
# Fail before the network round-trip rather than debug a 403 challenge.
#
# `tr -d '\000-\177'` deletes ALL ASCII bytes; anything left is non-ASCII.
# This is portable across GNU and BSD/macOS (unlike `grep -P`, which is GNU-only).
sq_require_ascii() {
    local s="$1"
    if LC_ALL=C printf '%s' "${s}" | LC_ALL=C tr -d '\000-\177' | grep -q .; then
        echo -e "${SQ_RED}[ERROR]${SQ_NC} Comment contains non-ASCII characters. Cloudflare blocks them. Replace em-dashes with '--' and curly quotes with straight quotes." >&2
        return 1
    fi
}

# Print a curl invocation with the token masked (for transparency without leaking).
# In SONAR_DRY_RUN mode the command is printed but not executed -- this only
# affects calls routed through sq_run, which is the WRITE path (api_post).
# Read-only API calls (issue/hotspot search used to enumerate findings) still
# run in dry-run so the caller can see what would be acted on.
sq_run() {
    local arg
    # Color vars contain real ESC bytes (defined with $'...'), so '%s' is
    # both safe (no SC2059) and correctly renders the colors.
    printf >&2 '%s> %s' "${SQ_GRAY}" "${SQ_YELLOW}"
    for arg in "$@"; do
        if [[ "${arg}" == "${SONAR_TOKEN}:" ]]; then
            printf >&2 '%q ' '<TOKEN>:'
        else
            printf >&2 '%q ' "${arg}"
        fi
    done
    printf >&2 '%s\n' "${SQ_NC}"
    if [[ "${SONAR_DRY_RUN:-0}" == "1" ]]; then
        return 0
    fi
    "$@"
}

