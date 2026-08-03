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

# Discarding the error made an unreachable API indistinguishable from a repo with no tags: the
# list came back empty, every loop below had nothing to walk, and the run still announced the
# prune complete. A repo with no tags answers Not Found; anything else means the answer is unknown.
all_tags() {
  local out status
  set +e
  out=$(gh api "repos/$1/git/refs/tags" --jq '.[].ref' 2>&1)
  status=$?
  set -e
  if [[ ${status} -ne 0 ]]; then
    if printf '%s' "${out}" | grep -q 'Not Found'; then
      return 0
    fi
    echo "prune-old-releases: cannot list tags for $1: ${out}" >&2
    exit 1
  fi
  printf '%s\n' "${out}" | sed 's#refs/tags/##'
}

tags_file=$(mktemp)
releases_file=$(mktemp)
trap 'rm -f "${tags_file}" "${releases_file}"' EXIT

all_tags "${this_repo}" > "${tags_file}"
gh release list --repo "${this_repo}" --limit 200 --json tagName --jq '.[].tagName' \
  < /dev/null > "${releases_file}"

keep=$(
  {
    echo "${mine}"
    cat "${tags_file}"
  } | grep -v '^$' | sort -V | tail -1
)
echo "keeper = ${keep}"

# Read from a file, never a pipe: a piped `while` runs in a subshell, so a failure counted inside
# it is lost the moment the loop ends and the prune reports clean regardless.
failed=0
while read -r t; do
  if [[ -z ${t} ]]; then
    continue
  fi
  newest=$(printf '%s\n%s\n' "${t}" "${keep}" | sort -V | tail -1)
  if [[ ${t} == "${keep}" || ${newest} != "${keep}" ]]; then
    continue
  fi
  if gh release delete "${t}" --repo "${this_repo}" --yes --cleanup-tag 2> /dev/null; then
    echo "deleted release+tag ${t}"
  else
    echo "prune-old-releases: could not delete release ${t}" >&2
    failed=$((failed + 1))
  fi
  sleep 1
done < "${releases_file}"

while read -r t; do
  if [[ -z ${t} ]]; then
    continue
  fi
  newest=$(printf '%s\n%s\n' "${t}" "${keep}" | sort -V | tail -1)
  if [[ ${t} == "${keep}" || ${newest} != "${keep}" ]]; then
    continue
  fi
  # The release loop above deletes its tag via --cleanup-tag, so a tag listed before that runs is
  # already gone by the time this loop reaches it. An absent reference is the work already done,
  # never a failure — counting it as one is what failed a release whose prune fully succeeded.
  set +e
  out=$(gh api -X DELETE "repos/${this_repo}/git/refs/tags/${t}" < /dev/null 2>&1)
  status=$?
  set -e
  if [[ ${status} -eq 0 ]]; then
    echo "deleted dangling tag ${t}"
  elif printf '%s' "${out}" | grep -q 'Reference does not exist'; then
    echo "tag ${t} already gone"
  else
    echo "prune-old-releases: could not delete tag ${t}: ${out}" >&2
    failed=$((failed + 1))
  fi
  sleep 1
done < "${tags_file}"

# Announcing the prune regardless is what let a systematic failure — a token without the scope —
# look exactly like a working prune.
if [[ ${failed} -gt 0 ]]; then
  echo "prune-old-releases: ${failed} deletion(s) failed; older releases remain" >&2
  exit 1
fi
echo "prune complete — only ${keep} remains"
