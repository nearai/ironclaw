#!/usr/bin/env bash
# Pin the explicit per-crate feature recipes in package-feature-flags.sh.
# Only explicit case arms are asserted here — the fallback branch shells out
# to `cargo metadata`, which this self-test deliberately avoids so it stays
# hermetic and instant.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
flags_script="${script_dir}/package-feature-flags.sh"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

assert_flags() {
  local package="$1"
  local expected="$2"
  local actual
  actual="$(bash "${flags_script}" "${package}")"
  if [ "${actual}" != "${expected}" ]; then
    fail "${package}: expected '${expected}', got '${actual}'"
  fi
  echo "PASS ${package} -> '${expected}'"
}

# The telegram host crate is deliberately flag-free: its whole surface is
# unconditional inside the crate.
assert_flags ironclaw_telegram_extension ""

# The runner restart regression is active without a crate feature gate.
assert_flags ironclaw_runner ""

# Guard the case-arm structure itself. Composition also carries `memory-mem0`
# (the off-by-default mem0 third-party memory provider, #5264) so this lane
# compiles it — see package-feature-flags.sh's ironclaw_reborn_composition arm.
assert_flags ironclaw_reborn_composition "--features test-support,memory-mem0"

echo "PASS package-feature-flags recipes"
