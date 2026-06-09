#!/usr/bin/env bash
# prune-old-releases — keep ONLY the highest-version release + tag on this repo.
# Single-release policy: the release list always shows exactly one entry (newest).
# Deletes strictly-older tags/releases only — never a newer concurrent tag, so a
# release racing behind a newer one prunes itself instead of clobbering it.
# Version ordering = sort -V (handles pre-release/build metadata), no hand-rolled compare.
# Env: GH_TOKEN, GITHUB_REF_NAME (the tag just published), GITHUB_REPOSITORY
set -euo pipefail
mine="${GITHUB_REF_NAME:?tag required}"
this_repo="${GITHUB_REPOSITORY:?}"

all_tags() { gh api "repos/${1}/git/refs/tags" --jq '.[].ref' 2>/dev/null | sed 's#refs/tags/##'; }

older_than() {
  local candidate="${1}" keeper="${2}" newest
  if [[ "${candidate}" == "${keeper}" ]]; then
    echo no
    return
  fi
  newest=$(printf '%s\n%s\n' "${candidate}" "${keeper}" | sort -V | tail -1)
  if [[ "${newest}" == "${keeper}" ]]; then
    echo yes
  else
    echo no
  fi
}

candidates=$(
  {
    echo "${mine}"
    all_tags "${this_repo}"
  } | grep -v '^$' | sort -V
)
keep=$(printf '%s\n' "${candidates}" | tail -1)
echo "keeper = ${keep}"

releases=$(gh release list --repo "${this_repo}" --limit 200 --json tagName --jq '.[].tagName' 2>/dev/null) || releases=""
for t in ${releases}; do
  [[ -z "${t}" ]] && continue
  verdict=$(older_than "${t}" "${keep}")
  [[ "${verdict}" == yes ]] || continue
  gh release delete "${t}" --repo "${this_repo}" --yes --cleanup-tag 2>/dev/null &&
    echo "deleted release+tag ${t}" || true
  sleep 1
done

all_tags "${this_repo}" >/tmp/dangling_tags.txt
tags=$(cat /tmp/dangling_tags.txt)
for t in ${tags}; do
  [[ -z "${t}" ]] && continue
  verdict=$(older_than "${t}" "${keep}")
  [[ "${verdict}" == yes ]] || continue
  gh api -X DELETE "repos/${this_repo}/git/refs/tags/${t}" 2>/dev/null &&
    echo "deleted dangling tag ${t}" || true
  sleep 1
done

echo "prune complete — only ${keep} remains"
