#!/usr/bin/env bash
#
# Guard the tests/integration/ (coverage-bearing, roadmap-integration) vs
# tests/ironclaw_*.rs + tests/support/ironclaw_parity_qa/ (parity/QA, NOT
# coverage-bearing) suite boundary established by the restructure that moved
# the roadmap integration suite to tests/integration/ (see
# docs/superpowers/specs/2026-06-26-ironclaw-integration-test-framework-design.md
# and the git history under tests/integration/).
#
# The direction is one-way: tests/ironclaw_*.rs parity/QA bins MAY reuse
# tests/integration/support/ (the roadmap harness), but tests/integration/
# suites must NEVER depend back on the parity/QA support tree — that would
# silently pull QA-only fixtures/harness weight into the suites this repo's
# coverage report is scoped to, and would resurrect exactly the coupling the
# restructure split apart.
#
# Checks:
#   1. Direction guard — no file under tests/integration/ mentions the
#      parity/QA support tree (by its local module alias `parity_qa_support`
#      or its mount path `support/ironclaw_parity_qa/`).
#   2. Partition guard — every tests/ironclaw_*.rs bin that references one of
#      the 6 parity/QA modules (binary_e2e, model_replay, qa_trace,
#      qa_scenarios, delivery, network — see tests/support/ironclaw_parity_qa/
#      mod.rs) via its `parity_qa_support::` alias must declare the
#      `#[path = "support/ironclaw_parity_qa/mod.rs"]` mount; and no file under
#      tests/integration/ may declare that mount (redundant with #1's path
#      check, kept as an explicit, separately-named assertion per the mount
#      itself rather than the module names).
#   3. Regression guard — tests/support/ironclaw_parity_qa/ (the pre-restructure location,
#      superseded by tests/support/ironclaw_parity_qa/) must not reappear.
#
# Exits non-zero with one message per violation (all checks run before
# exiting, so a single invocation reports every violation found, not just the
# first).

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

violations=0

fail() {
  printf 'BOUNDARY VIOLATION: %s\n' "$1" >&2
  violations=$((violations + 1))
}

# ---------------------------------------------------------------------------
# 1. Direction guard: tests/integration/ must never reference the parity/QA
#    support tree, by module alias or by mount path.
# ---------------------------------------------------------------------------

if [ -d tests/integration ]; then
  mapfile -t direction_hits < <(
    grep -rl -E 'parity_qa_support|ironclaw_parity_qa' tests/integration/ 2>/dev/null | LC_ALL=C sort
  )
  if [ "${#direction_hits[@]}" -gt 0 ]; then
    fail "tests/integration/ must not depend on the parity/QA support tree, but found references in:"
    printf '  %s\n' "${direction_hits[@]}" >&2
  fi
fi

# ---------------------------------------------------------------------------
# 2. Partition guard: every tests/ironclaw_*.rs bin that uses one of the 6
#    parity/QA modules must declare the parity_qa_support mount; no file
#    under tests/integration/ may declare that mount.
# ---------------------------------------------------------------------------

parity_qa_modules=(binary_e2e model_replay qa_trace qa_scenarios delivery network)
mount_pattern='#\[path = "support/ironclaw_parity_qa/mod\.rs"\]'

mapfile -t root_test_files < <(find tests -maxdepth 1 -type f -name 'ironclaw_*.rs' | LC_ALL=C sort)

for file in "${root_test_files[@]}"; do
  uses_parity_qa_module=false
  for module in "${parity_qa_modules[@]}"; do
    if grep -qE "parity_qa_support::${module}\b" "${file}"; then
      uses_parity_qa_module=true
      break
    fi
  done

  if [ "${uses_parity_qa_module}" = true ] && ! grep -qE "${mount_pattern}" "${file}"; then
    fail "${file} references a parity_qa_support module but does not declare" \
      "the #[path = \"support/ironclaw_parity_qa/mod.rs\"] mount"
  fi
done

if [ -d tests/integration ]; then
  mapfile -t mount_in_integration < <(
    grep -rlE "${mount_pattern}" tests/integration/ 2>/dev/null | LC_ALL=C sort
  )
  if [ "${#mount_in_integration[@]}" -gt 0 ]; then
    fail "tests/integration/ must not declare the parity/QA support mount, but found it in:"
    printf '  %s\n' "${mount_in_integration[@]}" >&2
  fi
fi

# ---------------------------------------------------------------------------
# 3. Regression guard: the pre-restructure tests/support/ironclaw_parity_qa/ dir must not
#    reappear (superseded by tests/support/ironclaw_parity_qa/).
# ---------------------------------------------------------------------------

if [ -d tests/support/reborn ]; then
  fail "tests/support/ironclaw_parity_qa/ has reappeared; the parity/QA support tree now lives at tests/support/ironclaw_parity_qa/"
fi

# ---------------------------------------------------------------------------
# 4. Stale-reference guard: no file under tests/ may cite the retired
#    tests/support/ironclaw_parity_qa/ path (comments included -- stale pointers mislead
#    readers and tools; the live homes are tests/integration/support/ and
#    tests/support/ironclaw_parity_qa/).
# ---------------------------------------------------------------------------

mapfile -t stale_refs < <(
  grep -rl 'tests/support/ironclaw_parity_qa/' tests/ 2>/dev/null | LC_ALL=C sort
)
if [ "${#stale_refs[@]}" -gt 0 ]; then
  fail "stale 'tests/support/ironclaw_parity_qa/' references (retired path) found in:"
  printf '  %s\n' "${stale_refs[@]}" >&2
fi

if [ "${violations}" -gt 0 ]; then
  printf '\n%s test-suite boundary violation(s) found\n' "${violations}" >&2
  exit 1
fi

echo "IronClaw test-suite boundaries OK: tests/integration/ <-> tests/support/ironclaw_parity_qa/ direction holds."
exit 0
