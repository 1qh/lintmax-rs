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

all_tags() { gh api "repos/$1/git/refs/tags" --jq '.[].ref' 2>/dev/null | sed 's#refs/tags/##'; }

is_older() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -1)" = "$2" ]
}

keep=$(
  { echo "$mine"; all_tags "$this_repo"; } | grep -v '^$' | sort -V | tail -1
)
echo "keeper = $keep"

gh release list --repo "$this_repo" --limit 200 --json tagName --jq '.[].tagName' 2>/dev/null \
  | while read -r t; do
      [ -z "$t" ] && continue
      is_older "$t" "$keep" || continue
      gh release delete "$t" --repo "$this_repo" --yes --cleanup-tag 2>/dev/null \
        && echo "deleted release+tag $t" || true
      sleep 1
    done

all_tags "$this_repo" \
  | while read -r t; do
      [ -z "$t" ] && continue
      is_older "$t" "$keep" || continue
      gh api -X DELETE "repos/$this_repo/git/refs/tags/$t" 2>/dev/null \
        && echo "deleted dangling tag $t" || true
      sleep 1
    done

echo "prune complete — only $keep remains"
