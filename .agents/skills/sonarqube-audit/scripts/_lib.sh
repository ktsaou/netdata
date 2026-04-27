#!/usr/bin/env bash
# Common helpers for sonarqube-audit scripts.
# Sourced from the per-action scripts; not executed directly.

set -euo pipefail

SQ_RED='\033[0;31m'
SQ_GREEN='\033[0;32m'
SQ_YELLOW='\033[1;33m'
SQ_GRAY='\033[0;90m'
SQ_NC='\033[0m'

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
    # shellcheck disable=SC1090
    set -a; source "${env}"; set +a

    : "${SONAR_TOKEN:?SONAR_TOKEN is empty in .env}"
    : "${SONAR_HOST_URL:=https://sonarcloud.io}"
    : "${SONAR_PROJECT:?SONAR_PROJECT is empty in .env (e.g. netdata_netdata)}"
    : "${SONAR_ORG:?SONAR_ORG is empty in .env (e.g. netdata)}"
    export SONAR_TOKEN SONAR_HOST_URL SONAR_PROJECT SONAR_ORG
}

sq_audit_dir() {
    local root
    root="$(sq_repo_root)"
    echo "${root}/.local/audits/sonarqube"
}

# Cloudflare in front of api.sonarcloud.io rejects non-ASCII bodies.
# Fail before the network round-trip rather than debug a 403 challenge.
sq_require_ascii() {
    local s="$1"
    if LC_ALL=C grep -qP '[^\x00-\x7F]' <<< "${s}"; then
        echo -e "${SQ_RED}[ERROR]${SQ_NC} Comment contains non-ASCII characters. Cloudflare blocks them. Replace em-dashes with '--' and curly quotes with straight quotes." >&2
        return 1
    fi
}

# Print a curl invocation with the token masked (for transparency without leaking).
sq_run() {
    local arg
    printf >&2 "${SQ_GRAY}> ${SQ_YELLOW}"
    for arg in "$@"; do
        if [[ "${arg}" == "${SONAR_TOKEN}:" ]]; then
            printf >&2 '%q ' '<TOKEN>:'
        else
            printf >&2 '%q ' "${arg}"
        fi
    done
    printf >&2 "${SQ_NC}\n"
    if [[ "${SONAR_DRY_RUN:-0}" == "1" ]]; then
        return 0
    fi
    "$@"
}
