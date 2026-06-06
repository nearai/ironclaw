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
  "reborn support crate" \
  "crates/ironclaw_outbound/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
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
  "docs only" \
  "README.md" \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_reborn_tests=false"

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
