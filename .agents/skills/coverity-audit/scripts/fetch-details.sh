#!/usr/bin/env bash
# Fetch per-defect details from Coverity Scan for each CID in a table.json dump.
#
# Usage:
#   fetch-details.sh <input-defects.json> <output-dir>
#
# Reads cookies/XSRF/UA from .env via _lib.sh.
# For each row, calls /sourcebrowser/defectdetails.json?defectInstanceId=<lastDefectInstanceId>.
# Skips rows whose output file already exists (idempotent; safe to re-run).

set -euo pipefail

# shellcheck source=./_lib.sh
source "$(dirname "$0")/_lib.sh"
cov_load_env

INPUT="${1:?usage: $0 <input-defects.json> <output-dir>}"
OUT_DIR="${2:?usage: $0 <input-defects.json> <output-dir>}"

mkdir -p "${OUT_DIR}"

TOTAL="$(jq 'length' "${INPUT}")"
echo -e "${COV_GRAY}Fetching details for ${TOTAL} defects into ${OUT_DIR}${COV_NC}" >&2

i=0
fetched=0
skipped=0
failed=0

while IFS=$'\t' read -r cid defect_instance_id; do
    i=$((i + 1))
    out_file="${OUT_DIR}/cid-${cid}.json"
    if [[ -s "${out_file}" ]]; then
        skipped=$((skipped + 1))
        continue
    fi
    sleep 0.3

    http_code="$(curl -sS --max-time 30 \
        -w '%{http_code}' \
        -o "${out_file}" \
        -H "accept: application/json, text/plain, */*" \
        -H "referer: ${COVERITY_HOST}/" \
        -H "user-agent: ${COVERITY_USER_AGENT}" \
        -H "x-xsrf-token: ${COVERITY_XSRF}" \
        -b "${COVERITY_COOKIE}" \
        "${COVERITY_HOST}/sourcebrowser/defectdetails.json?projectId=${COVERITY_PROJECT_ID}&defectInstanceId=${defect_instance_id}" || echo "000")"

    if [[ "${http_code}" != "200" ]]; then
        failed=$((failed + 1))
        echo -e "${COV_RED}[${i}/${TOTAL}] cid=${cid} diid=${defect_instance_id} HTTP=${http_code}${COV_NC}" >&2
        if [[ "${http_code}" == "401" || "${http_code}" == "403" ]]; then
            echo -e "${COV_RED}Session likely expired. Recapture cookie from browser, update .env, retry.${COV_NC}" >&2
            rm -f "${out_file}"
            exit 2
        fi
        rm -f "${out_file}"
        continue
    fi

    fetched=$((fetched + 1))
    if (( i % 20 == 0 )); then
        echo -e "${COV_GRAY}[${i}/${TOTAL}] fetched=${fetched} skipped=${skipped} failed=${failed}${COV_NC}" >&2
    fi
done < <(jq -r '.[] | "\(.cid)\t\(.lastDefectInstanceId)"' "${INPUT}")

echo -e "${COV_GREEN}Done. fetched=${fetched} skipped=${skipped} failed=${failed} total=${TOTAL}${COV_NC}" >&2
