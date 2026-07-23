#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
classifier="${script_dir}/classify-test-scope.sh"

assert_scope() {
  local name="$1"
  local files="$2"
  local expected="$3"
  local actual

  expected="$(printf '%s\n' "$expected" | sort)"
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

  expected="$(printf '%s\n' "$expected" | sort)"
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

  expected="$(printf '%s\n' "$expected" | sort)"
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
  "IronClaw binary crate" \
  "crates/ironclaw_cli/src/main.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw product storage crate" \
  "crates/ironclaw_product_workflow_storage/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw v2 adapter crate" \
  "crates/ironclaw_telegram_extension/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw telegram extension crate" \
  "crates/ironclaw_telegram_extension/src/channel.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw telegram v2 protocol adapter crate" \
  "crates/ironclaw_telegram_v2_adapter/src/render.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw support crate" \
  "crates/ironclaw_outbound/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw root test runner script" \
  "scripts/ci/run-ironclaw-root-partition.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw group test runner script" \
  "scripts/ci/run-ironclaw-group-tests.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw root tests and support" \
  "tests/ironclaw_qa_smoke_scenarios_e2e.rs
tests/integration/support/harness/mod.rs
tests/e2e/scenarios/test_ironclaw_gateway_smoke.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw QA trace fixture" \
  "tests/fixtures/llm_traces/ironclaw_qa/routine_health_ping.json" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw E2E scenario" \
  "tests/e2e/scenarios/test_ironclaw_scope_isolation.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "legacy e2e scenario" \
  "tests/e2e/scenarios/test_live_flow.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=false"

assert_scope \
  "mixed legacy and IronClaw root tests" \
  "tests/e2e_live.rs
tests/ironclaw_trace_first_party_tool_coverage.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=true"

assert_scope \
  "non-IronClaw channel source" \
  "channels-src/telegram/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=false"

assert_scope \
  "shared manifest" \
  "Cargo.toml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=true"

assert_scope \
  "shared substrate crate" \
  "crates/ironclaw_host_runtime/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=true"

assert_scope \
  "shared classifier script" \
  "scripts/ci/classify-test-scope.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=true"

assert_scope \
  "shared package feature flags script" \
  "scripts/ci/package-feature-flags.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=true"

assert_scope \
  "IronClaw crate bucket script" \
  "scripts/ci/ironclaw-crate-test-buckets.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw crate bucket regression suite" \
  "scripts/ci/test-ironclaw-crate-test-buckets.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw Responses E2E manifest checker" \
  "scripts/ci/check-ironclaw-responses-e2e-manifest.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw Responses E2E manifest" \
  "tests/e2e/ironclaw_responses_e2e_tests.txt" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw coverage manifest" \
  "tests/e2e/ironclaw_coverage_tests.txt" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "shared IronClaw tests workflow" \
  ".github/workflows/ironclaw-tests.yml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=true"

assert_scope \
  "legacy code style workflow" \
  ".github/workflows/code_style.yml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=false"

assert_scope \
  "docs only" \
  "README.md" \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_ironclaw_tests=false"

assert_empty_scope \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_ironclaw_tests=false"

assert_scope \
  "nested markdown is not docs only" \
  "crates/ironclaw_runner/CLAUDE.md" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw docs only" \
  "docs/ironclaw/harness/e2e.md" \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "mixed non-IronClaw and IronClaw" \
  "channels-src/telegram/src/lib.rs
crates/ironclaw_composition/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=true"

assert_scope_no_trailing_newline \
  "final path without trailing newline" \
  "crates/ironclaw_cli/src/main.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw coverage lane-run script" \
  "scripts/ci/ironclaw-coverage-lane-run.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw coverage merge-lcov script" \
  "scripts/ci/ironclaw-coverage-merge-lcov.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw coverage summary script" \
  "scripts/ci/ironclaw-coverage-summary.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw coverage regression suite" \
  "scripts/ci/test-ironclaw-coverage.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "IronClaw coverage regression suite, sourced sibling (R-section split)" \
  "scripts/ci/test-ironclaw-coverage-ratchet-cases.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "test suite boundaries checker script" \
  "scripts/ci/check-test-suite-boundaries.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"

assert_scope \
  "test-classify-test-scope script is itself IronClaw-scoped" \
  "scripts/ci/test-classify-test-scope.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_ironclaw_tests=true"

assert_scope \
  "shared coverage lcov lib is IronClaw-scoped (gemini: PR #5718 comment)" \
  "scripts/ci/lib/ironclaw_coverage_lcov.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_ironclaw_tests=true"
