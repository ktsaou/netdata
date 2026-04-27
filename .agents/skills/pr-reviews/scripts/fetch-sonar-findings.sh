#!/usr/bin/env bash
# Fetch SonarCloud findings introduced by a specific pull request.
#
# Usage:
#   fetch-sonar-findings.sh <pr-number>
#
# SonarCloud does NOT post per-finding inline comments on the GitHub PR --
# only a QualityGate summary comment is delivered. The actual issue list
# lives behind /api/issues/search?pullRequest=<N> and /api/hotspots/search.
# This script pulls both, so the PR-reviews loop can address them.
#
# Outputs (under .local/audits/pr-reviews/pr-<N>/):
#   sonar-issues.json    -- all open issues this PR introduced
#   sonar-hotspots.json  -- all open security hotspots this PR introduced
#
# Reads SONAR_TOKEN, SONAR_HOST_URL, SONAR_PROJECT from <repo-root>/.env --
# the same .env entries the sonarqube-audit skill uses. If they're missing,
# this script prints what's needed and exits 1.

set -euo pipefail

# shellcheck source=./_lib.sh
source "$(dirname "$0")/_lib.sh"

PR="${1:?usage: $0 <pr-number>}"

ROOT="$(pr_repo_root)"
ENV="${ROOT}/.env"

if [[ ! -r "${ENV}" ]]; then
    echo -e "${PR_RED}[ERROR]${PR_NC} Missing ${ENV}." >&2
    echo "Add to ${ENV}:" >&2
    echo "  SONAR_TOKEN='<token from https://sonarcloud.io/account/security>'" >&2
    echo "  SONAR_HOST_URL=https://sonarcloud.io" >&2
    echo "  SONAR_PROJECT=<project_key>" >&2
    exit 1
fi
set -a
# shellcheck disable=SC1090
source "${ENV}"
set +a
: "${SONAR_TOKEN:?SONAR_TOKEN not set in .env -- see sonarqube-audit/SKILL.md}"
: "${SONAR_HOST_URL:=https://sonarcloud.io}"
: "${SONAR_PROJECT:?SONAR_PROJECT not set in .env}"

DIR="$(pr_state_dir "${PR}")"

# Paginate through Sonar's issue search filtered by pullRequest. ps=500 is
# the max per page. Stop when paging.total is reached or a page returns 0.
fetch_paginated() {
    local kind="$1" path_with_filter="$2" out="$3"
    local page=1 total=0 fetched=0
    echo '[]' > "${out}"
    while :; do
        local resp
        resp="$(curl --fail --silent --show-error -u "${SONAR_TOKEN}:" \
            "${SONAR_HOST_URL}${path_with_filter}&ps=500&p=${page}")"
        local items
        items="$(jq ".${kind}" <<< "${resp}")"
        local in_page
        in_page="$(jq 'length' <<< "${items}")"
        total="$(jq -r '.paging.total' <<< "${resp}")"
        fetched=$(( fetched + in_page ))

        # Append to running file
        jq -s '.[0] + .[1]' "${out}" <(printf '%s' "${items}") > "${out}.merged"
        mv "${out}.merged" "${out}"

        echo -e "${PR_GRAY}[fetch-sonar] ${kind}: page ${page} +${in_page} (running ${fetched}/${total})${PR_NC}" >&2
        (( fetched >= total || in_page == 0 )) && break
        page=$(( page + 1 ))
    done
}

echo -e "${PR_GRAY}[fetch-sonar] PR ${PR} -> ${DIR}${PR_NC}" >&2

# 1) Issues introduced by the PR (resolved=false: still actionable).
fetch_paginated "issues" \
    "/api/issues/search?componentKeys=${SONAR_PROJECT}&pullRequest=${PR}&resolved=false" \
    "${DIR}/sonar-issues.json"

# 2) Security Hotspots introduced by the PR.
fetch_paginated "hotspots" \
    "/api/hotspots/search?projectKey=${SONAR_PROJECT}&pullRequest=${PR}&status=TO_REVIEW" \
    "${DIR}/sonar-hotspots.json"

# Summary
n_issues="$(jq 'length' "${DIR}/sonar-issues.json")"
n_hotspots="$(jq 'length' "${DIR}/sonar-hotspots.json")"

echo
echo "SonarCloud findings on PR ${PR}:"
echo "  issues:   ${n_issues}"
echo "  hotspots: ${n_hotspots}"
echo
if (( n_issues > 0 )); then
    echo "Issues by severity / rule:"
    jq -r 'group_by(.severity + " " + .rule) | .[] | "  \(.[0].severity)  \(.[0].rule)  x\(length)"' \
        "${DIR}/sonar-issues.json"
    echo
    echo "Issue details:"
    jq -r '.[] | "  \(.key)  \(.severity)  \(.rule)  \(.component | sub("^[^:]+:"; ""))\(if .line then ":" + (.line | tostring) else "" end)\n    \(.message)"' \
        "${DIR}/sonar-issues.json"
fi
if (( n_hotspots > 0 )); then
    echo
    echo "Hotspots:"
    jq -r '.[] | "  \(.key)  \(.vulnerabilityProbability)  \(.ruleKey)  \(.component | sub("^[^:]+:"; ""))\(if .line then ":" + (.line | tostring) else "" end)\n    \(.message)"' \
        "${DIR}/sonar-hotspots.json"
fi
