#!/usr/bin/env sh
# prune-ci-runs — delete every SUCCESSFUL completed run except the current one,
# so the Actions history stays a single present run (mirrors the single-release +
# single-commit-mirror discipline). A FAILED run is kept: its log is the only
# record of why it failed, and deleting it leaves a failure nobody can diagnose.
# Env: GH_TOKEN, REPO, KEEP_RUN_ID.
set -eu
: "${REPO:?REPO required}"
: "${KEEP_RUN_ID:?KEEP_RUN_ID required}"
gh run list --repo "${REPO}" --limit 400 --json databaseId,status,conclusion \
  -q '.[] | select(.status=="completed" and .conclusion=="success") | .databaseId' \
  < /dev/null > /tmp/ci_runs.txt
while read -r id; do
  if [ "${id}" = "${KEEP_RUN_ID}" ]; then
    continue
  fi
  gh api -X DELETE "/repos/${REPO}/actions/runs/${id}" < /dev/null > /dev/null 2>&1 || true
  sleep 1
done < /tmp/ci_runs.txt
echo ok
