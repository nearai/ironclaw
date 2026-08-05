#!/usr/bin/env bash
# Print the repo-relative directory of the crate named $1, resolved through the
# shared inventory (scripts/ci/lib/crate_tree.py) rather than assuming the flat
# `crates/<name>/` shape the family move (PROPOSAL §5) removes. Exits non-zero
# and explains when the name does not resolve — a caller must never silently
# `cd` to a path that is not there.
#
# Usage:
#   scripts/ci/crate-dir.sh <crate-name> [repo-root]
#
# repo-root defaults to `git rev-parse --show-toplevel`; pass it explicitly
# from a fixture or a checkout that is not the caller's own working directory
# (e.g. a second checkout in CI, or a self-test fixture repo).

set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: crate-dir.sh <crate-name> [repo-root]" >&2
  exit 2
fi

crate_name="$1"
repo_root="${2:-$(git rev-parse --show-toplevel)}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec python3 "${script_dir}/lib/crate_tree.py" --directory "${crate_name}" "${repo_root}"
