#!/usr/bin/env bash
# Apply a verdict to one Coverity defect (high-level wrapper around update-triage.sh).
#
# Usage:
#   finalize-defect.sh <cid> <verdict> <phase> <comment-file> [commit-sha]
#
# Verdicts:
#   TRUE_BUG_MEMORY_CORRUPTION, TRUE_BUG_CRASH, TRUE_BUG_RESOURCE_LEAK,
#   TRUE_BUG_LOGIC, TRUE_BUG_UB              -> Bug + Fix Submitted
#   FALSE_POSITIVE_GUARD_EXISTS, FALSE_POSITIVE_UNREACHABLE,
#   FALSE_POSITIVE_TRUSTED_INPUT,
#   FALSE_POSITIVE_TOOL_MODEL,
#   IMPOSSIBLE_CONDITIONS                    -> False Positive + Ignore
#   COSMETIC                                 -> Intentional + Ignore
#   NEEDS_HUMAN, CODE_GONE                   -> NO-OP (skipped, exit 0)
#
# Phases:
#   1     outstanding queue (default, applied unconditionally)
#   2..5  old/dismissed/fixed/unclassified — caller asserts the verdict
#         disagrees with the existing classification
#
# Severity is mapped from Coverity's displayImpact field. The script reads it
# from .local/audits/coverity/raw/outstanding-all.json (phase 1) or
# .local/audits/coverity/raw/all-in-project-all.json (fallback).
# If neither file exists, severity defaults to Unspecified.
#
# If a commit SHA is given, the script appends "Fix commit: <sha>" to the
# comment before posting.

set -euo pipefail

# shellcheck source=./_lib.sh
source "$(dirname "$0")/_lib.sh"

CID="${1:?usage: $0 <cid> <verdict> <phase> <comment-file> [commit-sha]}"
VERDICT="${2:?usage}"
PHASE="${3:?usage}"
COMMENT_FILE="${4:?usage}"
COMMIT_SHA="${5:-}"

# Verdicts that never touch the UI.
case "${VERDICT}" in
    NEEDS_HUMAN|CODE_GONE)
        echo -e "${COV_YELLOW}Skipping Coverity update for CID ${CID} -- verdict=${VERDICT}.${COV_NC}" >&2
        exit 0
        ;;
esac

# Verdict -> (classification, action).
case "${VERDICT}" in
    TRUE_BUG_MEMORY_CORRUPTION|TRUE_BUG_CRASH|TRUE_BUG_RESOURCE_LEAK|TRUE_BUG_LOGIC|TRUE_BUG_UB)
        CLASS_ID=24; ACT_ID=3 ;;
    FALSE_POSITIVE_GUARD_EXISTS|FALSE_POSITIVE_UNREACHABLE|FALSE_POSITIVE_TRUSTED_INPUT|FALSE_POSITIVE_TOOL_MODEL|IMPOSSIBLE_CONDITIONS)
        CLASS_ID=22; ACT_ID=5 ;;
    COSMETIC)
        CLASS_ID=23; ACT_ID=5 ;;
    *)
        echo -e "${COV_RED}Unknown verdict: ${VERDICT}${COV_NC}" >&2; exit 1 ;;
esac

# Severity from displayImpact.
audit="$(cov_audit_dir)"
impact=""
for f in "${audit}/raw/outstanding-all.json" "${audit}/raw/all-in-project-all.json"; do
    if [[ -r "${f}" ]]; then
        impact="$(jq -r ".[] | select(.cid==${CID}) | .displayImpact" "${f}" 2>/dev/null || true)"
        [[ -n "${impact}" && "${impact}" != "null" ]] && break
    fi
done

case "${impact}" in
    High)        SEV_ID=11 ;;
    Medium)      SEV_ID=12 ;;
    Low)         SEV_ID=13 ;;
    *)           SEV_ID=10 ;;
esac

echo -e "${COV_GRAY}CID ${CID}: verdict=${VERDICT} -> class=${CLASS_ID} sev=${SEV_ID} (impact=${impact:-unknown}) act=${ACT_ID}${COV_NC}" >&2

# If a commit SHA is given, append it to the comment before posting.
if [[ -n "${COMMIT_SHA}" ]]; then
    tmp_comment="$(mktemp --tmpdir cov-comment-XXXXXX.txt)"
    trap 'rm -f "${tmp_comment}"' EXIT
    {
        cat "${COMMENT_FILE}"
        printf '\nFix commit: %s\n' "${COMMIT_SHA}"
    } > "${tmp_comment}"
    effective="${tmp_comment}"
else
    effective="${COMMENT_FILE}"
fi

if [[ "${PHASE}" != "1" ]]; then
    echo -e "${COV_YELLOW}Phase ${PHASE}: applying ONLY because caller asserts verdict disagrees with the existing classification.${COV_NC}" >&2
fi

"$(dirname "$0")/update-triage.sh" "${CID}" "${CLASS_ID}" "${SEV_ID}" "${ACT_ID}" "${effective}"
