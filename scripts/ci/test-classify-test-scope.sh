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
  "crates/ironclaw_product_workflow_storage/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn v2 adapter crate" \
  "crates/ironclaw_telegram_v2_adapter/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn channel host support crate" \
  "crates/ironclaw_channel_host/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn channel delivery support crate" \
  "crates/ironclaw_channel_delivery/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn telegram extension crate" \
  "crates/ironclaw_telegram_extension/src/telegram_pairing.rs" \
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
  "legacy root runtime" \
  "src/agent/session.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=false"

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
  "mixed legacy and reborn" \
  "src/agent/session.rs
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
