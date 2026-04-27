#!/usr/bin/env bash
# Search SonarCloud findings for the configured project.
#
# Usage:
#   sonar-search.sh issues   [--rule RULE_ID] [--resolved=false|true] [--ps=500]
#   sonar-search.sh hotspots [--status=TO_REVIEW|REVIEWED] [--ps=500]
#   sonar-search.sh summary                                            # rule + count for open issues + hotspots
#
# Output: raw JSON (issues / hotspots) to stdout.
#
# This is a READ-ONLY script — it does not mutate Sonar state. Safe to run
# anytime to inspect what's outstanding.

set -euo pipefail

# shellcheck source=./_lib.sh
source "$(dirname "$0")/_lib.sh"
sq_load_env

cmd="${1:-summary}"; shift || true

case "${cmd}" in
    issues)
        rule=""
        resolved="false"
        ps="500"
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --rule) rule="$2"; shift 2 ;;
                --resolved=*) resolved="${1#*=}"; shift ;;
                --ps=*) ps="${1#*=}"; shift ;;
                *) echo "Unknown arg: $1" >&2; exit 2 ;;
            esac
        done
        url="${SONAR_HOST_URL}/api/issues/search?componentKeys=${SONAR_PROJECT}&resolved=${resolved}&ps=${ps}"
        [[ -n "${rule}" ]] && url="${url}&rules=${rule}"
        sq_run curl --fail --silent --show-error -u "${SONAR_TOKEN}:" "${url}"
        ;;

    hotspots)
        status="TO_REVIEW"
        ps="500"
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --status=*) status="${1#*=}"; shift ;;
                --ps=*) ps="${1#*=}"; shift ;;
                *) echo "Unknown arg: $1" >&2; exit 2 ;;
            esac
        done
        url="${SONAR_HOST_URL}/api/hotspots/search?projectKey=${SONAR_PROJECT}&status=${status}&ps=${ps}"
        sq_run curl --fail --silent --show-error -u "${SONAR_TOKEN}:" "${url}"
        ;;

    summary)
        echo "=== Open issues by rule ===" >&2
        curl --fail --silent --show-error -u "${SONAR_TOKEN}:" \
            "${SONAR_HOST_URL}/api/issues/search?componentKeys=${SONAR_PROJECT}&resolved=false&ps=500&facets=rules" \
            | jq -r '.facets[] | select(.property=="rules") | .values[] | "  \(.val) (\(.count))"' \
            | sort -k2 -t'(' -nr | head -30

        echo >&2
        echo "=== Open hotspots by rule ===" >&2
        curl --fail --silent --show-error -u "${SONAR_TOKEN}:" \
            "${SONAR_HOST_URL}/api/hotspots/search?projectKey=${SONAR_PROJECT}&status=TO_REVIEW&ps=500" \
            | jq -r '[.hotspots[] | .ruleKey] | group_by(.) | map({rule: .[0], count: length}) | sort_by(-.count) | .[] | "  \(.rule) (\(.count))"' \
            | head -30
        ;;

    "")
        echo "usage: $0 issues|hotspots|summary [args...]" >&2
        exit 2
        ;;
    *)
        echo "Unknown command: ${cmd}" >&2
        exit 2
        ;;
esac
