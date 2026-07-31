#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
classifier="${script_dir}/classify-test-scope.sh"

assert_scope() {
  local name="$1"
  local files="$2"
  local expected="$3"
  local actual

  actual="$(printf '%s\n' "$files" | "$classifier" | sort)"

  if [ "$actual" != "$expected" ]; then
    printf 'FAIL %s\n' "$name" >&2
    printf 'Expected:\n%s\n' "$expected" >&2
    printf 'Actual:\n%s\n' "$actual" >&2
    exit 1
  fi

  printf 'PASS %s\n' "$name"
}

assert_scope_no_trailing_newline() {
  local name="$1"
  local files="$2"
  local expected="$3"
  local actual

  actual="$(printf '%s' "$files" | "$classifier" | sort)"

  if [ "$actual" != "$expected" ]; then
    printf 'FAIL %s\n' "$name" >&2
    printf 'Expected:\n%s\n' "$expected" >&2
    printf 'Actual:\n%s\n' "$actual" >&2
    exit 1
  fi

  printf 'PASS %s\n' "$name"
}

assert_empty_scope() {
  local expected="$1"
  local actual

  actual="$(printf '' | "$classifier" | sort)"

  if [ "$actual" != "$expected" ]; then
    printf 'FAIL empty input\n' >&2
    printf 'Expected:\n%s\n' "$expected" >&2
    printf 'Actual:\n%s\n' "$actual" >&2
    exit 1
  fi

  printf 'PASS empty input\n'
}

assert_scope \
  "reborn binary crate" \
  "crates/ironclaw_reborn_cli/src/main.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn product storage crate" \
  "crates/ironclaw_product_storage/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn v2 adapter crate" \
  "crates/ironclaw_telegram_extension/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn telegram extension crate" \
  "crates/ironclaw_telegram_extension/src/channel.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn telegram v2 protocol adapter crate" \
  "crates/ironclaw_telegram_v2_adapter/src/render.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn support crate" \
  "crates/ironclaw_outbound/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn root test runner script" \
  "scripts/ci/run-reborn-root-partition.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn group test runner script" \
  "scripts/ci/run-reborn-group-tests.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn root tests and support" \
  "tests/reborn_qa_smoke_scenarios_e2e.rs
tests/integration/support/harness/mod.rs
tests/e2e/scenarios/test_reborn_gateway_smoke.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn qa trace fixture" \
  "tests/fixtures/llm_traces/reborn_qa/routine_health_ping.json" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn e2e scenario" \
  "tests/e2e/scenarios/test_reborn_scope_isolation.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "legacy e2e scenario" \
  "tests/e2e/scenarios/test_live_flow.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=false"

assert_scope \
  "mixed legacy and reborn root tests" \
  "tests/e2e_live.rs
tests/reborn_trace_first_party_tool_coverage.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared manifest" \
  "Cargo.toml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared substrate crate" \
  "crates/ironclaw_host_runtime/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "process lifecycle authority crate" \
  "crates/ironclaw_processes/src/journal_store.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared classifier script" \
  "scripts/ci/classify-test-scope.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared package feature flags script" \
  "scripts/ci/package-feature-flags.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "Reborn crate bucket script" \
  "scripts/ci/reborn-crate-test-buckets.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn crate bucket regression suite" \
  "scripts/ci/test-reborn-crate-test-buckets.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn Responses E2E manifest checker" \
  "scripts/ci/check-reborn-responses-e2e-manifest.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn Responses E2E manifest" \
  "tests/e2e/reborn_responses_e2e_tests.txt" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn coverage manifest" \
  "tests/e2e/reborn_coverage_tests.txt" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "shared reborn tests workflow" \
  ".github/workflows/reborn-tests.yml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "legacy code style workflow" \
  ".github/workflows/code_style.yml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=false"

assert_scope \
  "docs only" \
  "README.md" \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_reborn_tests=false"

assert_empty_scope \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_reborn_tests=false"

assert_scope \
  "nested markdown is not docs only" \
  "crates/ironclaw_runner/CLAUDE.md" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn docs only" \
  "docs/reborn/harness/e2e.md" \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "mixed tests and reborn" \
  "tests/e2e/scenarios/test_live_flow.py
crates/ironclaw_reborn_composition/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope_no_trailing_newline \
  "final path without trailing newline" \
  "crates/ironclaw_reborn_cli/src/main.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage lane-run script" \
  "scripts/ci/reborn-coverage-lane-run.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage merge-lcov script" \
  "scripts/ci/reborn-coverage-merge-lcov.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage summary script" \
  "scripts/ci/reborn-coverage-summary.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage regression suite" \
  "scripts/ci/test-reborn-coverage.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage regression suite, sourced sibling (R-section split)" \
  "scripts/ci/test-reborn-coverage-ratchet-cases.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "WS12 generated gates and sabotage tests are reborn-scoped" \
  "scripts/ci/reborn_changed_coverage.py
scripts/ci/test_reborn_changed_coverage.py
scripts/ci/ws12-suite-shards.toml
scripts/ci/ws12_suite_shards.py
scripts/ci/test_ws12_suite_shards.py
scripts/ci/ws12_workflow_contracts.py
scripts/ci/test_ws12_workflow_contracts.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "test suite boundaries checker script" \
  "scripts/ci/check-test-suite-boundaries.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "test-classify-test-scope script is itself reborn-scoped" \
  "scripts/ci/test-classify-test-scope.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared coverage lcov lib is reborn-scoped (gemini: PR #5718 comment)" \
  "scripts/ci/lib/reborn_coverage_lcov.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

# ---------------------------------------------------------------------------
# Tree-shape independence (CHECKLIST WS10)
#
# The arms above are keyed to crate identity, not to how deep a crate sits
# under crates/. These cases pin that: the same crate classifies the same way
# from a family directory, and a crate tree the classifier cannot read is a
# refusal rather than a silent "nothing in scope".
# ---------------------------------------------------------------------------

# Fixture repo root with the target-architecture layout: crates live one level
# down, inside a family directory.
nested_root="$(mktemp -d)"
trap 'rm -rf "${nested_root}"' EXIT
for crate in ironclaw_host_runtime ironclaw_webui ironclaw_extension_host; do
  mkdir -p "${nested_root}/crates/substrates/${crate}/src"
  printf '[package]\nname = "%s"\n' "${crate}" \
    > "${nested_root}/crates/substrates/${crate}/Cargo.toml"
done
# 20 more so the discovery floor is met by a realistic tree, not one fixture.
for i in $(seq 1 20); do
  mkdir -p "${nested_root}/crates/domains/ironclaw_filler_${i}"
  printf '[package]\nname = "ironclaw_filler_%s"\n' "${i}" \
    > "${nested_root}/crates/domains/ironclaw_filler_${i}/Cargo.toml"
done

assert_scope_with_root() {
  local name="$1" root="$2" files="$3" expected="$4" actual
  actual="$(printf '%s\n' "$files" | IRONCLAW_REPO_ROOT="$root" "$classifier" | sort)"
  if [ "$actual" != "$expected" ]; then
    printf 'FAIL %s\n' "$name" >&2
    printf 'Expected:\n%s\nActual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  fi
  printf 'PASS %s\n' "$name"
}

assert_refusal_with_root() {
  local name="$1" root="$2" files="$3" needle="$4" err rc
  err="$(printf '%s\n' "$files" | IRONCLAW_REPO_ROOT="$root" "$classifier" 2>&1 >/dev/null)" && rc=0 || rc=$?
  if [ "$rc" -eq 0 ] || [ "${err#*"$needle"}" = "$err" ]; then
    printf 'FAIL %s\n' "$name" >&2
    printf 'Expected exit!=0 and stderr containing: %s\nActual rc=%s stderr:\n%s\n' \
      "$needle" "$rc" "$err" >&2
    exit 1
  fi
  printf 'PASS %s\n' "$name"
}

assert_scope_with_root \
  "nested shared crate still classifies as shared" \
  "${nested_root}" \
  "crates/substrates/ironclaw_host_runtime/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope_with_root \
  "nested reborn crate still classifies as reborn-only" \
  "${nested_root}" \
  "crates/substrates/ironclaw_webui/src/handlers.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope_with_root \
  "nested unlisted crate still classifies as legacy-only (unchanged bucketing)" \
  "${nested_root}" \
  "crates/substrates/ironclaw_extension_host/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=false"

assert_refusal_with_root \
  "a crates/ path attributable to no crate is refused, not bucketed" \
  "${nested_root}" \
  "crates/extensions/packages/slack/manifest.toml" \
  "cannot attribute"

assert_refusal_with_root \
  "an unreadable crate tree is refused, not classified as out-of-scope" \
  "${nested_root}/does-not-exist" \
  "crates/ironclaw_webui/src/lib.rs" \
  "no crates/ directory"

empty_tree_root="$(mktemp -d "${nested_root}/empty.XXXXXX")"
mkdir -p "${empty_tree_root}/crates"
assert_refusal_with_root \
  "a crate tree below the discovery floor is refused" \
  "${empty_tree_root}" \
  "crates/ironclaw_webui/src/lib.rs" \
  "crate director"

# The bash inventory in classify-test-scope.sh and the Python one in
# scripts/ci/lib/crate_tree.py are two implementations of one rule. Pin them
# equal so a fix to either cannot leave the other keyed to a stale tree shape.
repo_root_for_inventory="$(cd "${script_dir}/../.." && pwd)"
bash_inventory="$(
  find "${repo_root_for_inventory}/crates" -type f -name Cargo.toml \
    -not -path '*/target/*' -not -path '*/.*' 2>/dev/null \
    | sed -e "s|^${repo_root_for_inventory}/||" -e 's|/Cargo.toml$||' \
    | sort
)"
python_inventory="$(
  python3 "${script_dir}/lib/crate_tree.py" "${repo_root_for_inventory}" | sort
)"
# The bash side keeps nested manifests in the list (resolution walks outward-in
# and stops at the outermost hit); the Python side prunes them. Compare the
# outermost sets.
bash_outermost="$(
  printf '%s\n' "${bash_inventory}" | awk '
    { keep = 1
      for (i = 1; i <= n; i++) if (index($0, kept[i] "/") == 1) { keep = 0; break }
      if (keep) { kept[++n] = $0; print } }'
)"
if [ "${bash_outermost}" != "${python_inventory}" ]; then
  printf 'FAIL bash and python crate inventories agree\n' >&2
  diff <(printf '%s\n' "${bash_outermost}") <(printf '%s\n' "${python_inventory}") >&2 || true
  exit 1
fi
printf 'PASS bash and python crate inventories agree (%s crate directories)\n' \
  "$(printf '%s\n' "${python_inventory}" | grep -c .)"
