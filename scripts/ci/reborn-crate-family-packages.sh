#!/usr/bin/env bash
#
# Print, one per line, the workspace package names that belong to the Reborn
# crate family (per reborn-crate-family-regex.sh), sorted and deduped.
#
# This is the single discovery mechanism for "which Reborn-family crates have
# their own test suite to count" — consumed by:
#   - .github/workflows/reborn-tests.yml (package-matrix job's allowlist,
#     unioned there with the separate `cargo tree` dependency-closure list)
#   - .github/workflows/reborn-coverage.yml (Tier-B crate-tests coverage pass)
# Deliberately narrower than reborn-tests.yml's closure union: crates outside
# the Reborn family are filtered out of the coverage %/hole-list by
# reborn-coverage-summary.sh anyway, so running their test suites here would
# spend CI time with no visible effect on the report.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${script_dir}/../.."

family_regex="$("${script_dir}/reborn-crate-family-regex.sh")"

cargo metadata --no-deps --format-version 1 \
  | jq -r --arg re "${family_regex}" '
      [ .packages[] | select(.name | test("^(" + $re + ")$")) | .name ]
      | unique
      | .[]
    '
