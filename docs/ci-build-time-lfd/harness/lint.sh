#!/usr/bin/env bash
set -euo pipefail

base_ref="${BASE_REF:-origin/main}"
range="${base_ref}...HEAD"

if ! git rev-parse --verify "$base_ref" >/dev/null 2>&1; then
  echo "missing base ref $base_ref; run git fetch origin main or set BASE_REF" >&2
  exit 2
fi

diff_tmp="$(mktemp)"
name_status_tmp="$(mktemp)"
trap 'rm -f "$diff_tmp" "$name_status_tmp"' EXIT

git diff -U0 "$range" -- .github/workflows .github/actions scripts/ci scripts/live-canary tests Cargo.toml Cargo.lock > "$diff_tmp"
git diff --name-status "$range" > "$name_status_tmp"

failures=0

fail_if_diff_matches() {
  name="$1"
  pattern="$2"
  if grep -En "$pattern" "$diff_tmp"; then
    echo "FAIL: $name" >&2
    failures=$((failures + 1))
  fi
}

warn_if_diff_matches() {
  name="$1"
  pattern="$2"
  if grep -En "$pattern" "$diff_tmp"; then
    echo "WARN: $name" >&2
  fi
}

fail_if_diff_matches "new continue-on-error in CI paths" '^\+.*continue-on-error:[[:space:]]*true'
fail_if_diff_matches "new explicit test skip/ignore marker" '^\+.*(--skip|--ignored|#\[ignore\]|#\[cfg\(ignore)'
fail_if_diff_matches "new cargo test exclusion flag" '^\+.*cargo (nextest run|test|llvm-cov).*--exclude[[:space:]]'
fail_if_diff_matches "new pytest exclusion expression" '^\+.*pytest .* -k .*not '
fail_if_diff_matches "new neutralized shell failure handling" '^\+.*\|\|[[:space:]]*true([[:space:]]|$)'

deleted_tests="$(awk '$1 ~ /^D/ && $2 ~ /(^tests\/|\/tests\/|_test\.rs$|\.test\.(js|mjs|ts|tsx)$)/ { print }' "$name_status_tmp")"
if [ -n "$deleted_tests" ]; then
  printf '%s\n' "$deleted_tests"
  echo "FAIL: test files were deleted" >&2
  failures=$((failures + 1))
fi

warn_if_diff_matches "workflow trigger or path filters changed; reviewer must confirm no CI is avoided" '^[-+].*(pull_request:|merge_group:|push:|paths:|paths-ignore:)'
warn_if_diff_matches "job guard changed; reviewer must confirm checks still run" '^[-+].*if:[[:space:]]'
warn_if_diff_matches "timeout changed; reviewer must confirm this is not hiding work" '^[-+].*timeout-minutes:'
warn_if_diff_matches "test command changed; compare test inventory before merging" '^[-+].*(cargo test|cargo nextest run|cargo llvm-cov|pytest |node --test)'

if [ "$failures" -gt 0 ]; then
  echo "CI build-time lint failed with $failures hard guard violation(s)." >&2
  exit 1
fi

echo "CI build-time lint passed."
