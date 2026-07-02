#!/usr/bin/env bash
set -euo pipefail

has_core_code=false
docs_only=true
has_legacy_tests=false
has_reborn_tests=false

is_docs_only_path() {
  local path="$1"
  case "$path" in
    docs/*|.github/ISSUE_TEMPLATE/*|.github/pull_request_template.md)
      return 0
      ;;
    *.md)
      case "$path" in
        */*) return 1 ;;
        *) return 0 ;;
      esac
      ;;
    *)
      return 1
      ;;
  esac
}

is_shared_test_path() {
  local path="$1"
  case "$path" in
    Cargo.toml|Cargo.lock|build.rs|providers.json|Dockerfile)
      return 0
      ;;
    scripts/ci/classify-test-scope.sh|scripts/ci/test-classify-test-scope.sh|scripts/ci/package-feature-flags.sh)
      return 0
      ;;
    .github/workflows/test.yml|.github/workflows/reborn-tests.yml|.github/workflows/reborn-e2e.yml|.github/workflows/nightly-deep-ci.yml)
      return 0
      ;;
    crates/ironclaw_common/*|crates/ironclaw_host_api/*|crates/ironclaw_host_runtime/*|crates/ironclaw_loop_support/*)
      return 0
      ;;
    crates/ironclaw_filesystem/*|crates/ironclaw_memory/*|crates/ironclaw_events/*|crates/ironclaw_event_projections/*|crates/ironclaw_event_streams/*)
      return 0
      ;;
    crates/ironclaw_capabilities/*|crates/ironclaw_secrets/*|crates/ironclaw_network/*|crates/ironclaw_runtime_policy/*)
      return 0
      ;;
    crates/ironclaw_authorization/*|crates/ironclaw_run_state/*|crates/ironclaw_approvals/*|crates/ironclaw_resources/*)
      return 0
      ;;
    crates/ironclaw_auth/*|crates/ironclaw_trust/*|crates/ironclaw_turns/*|crates/ironclaw_agent_loop/*|crates/ironclaw_threads/*)
      return 0
      ;;
    crates/ironclaw_prompt_envelope/*|crates/ironclaw_hooks/*|crates/ironclaw_first_party_extensions/*|crates/ironclaw_llm/*)
      return 0
      ;;
    crates/ironclaw_embeddings/*|crates/ironclaw_safety/*|crates/ironclaw_skills/*|crates/ironclaw_oauth/*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

is_reborn_test_path() {
  local path="$1"
  case "$path" in
    docs/reborn/*|scripts/reborn-e2e-rust.sh|scripts/ci/run-reborn-root-partition.sh|scripts/ci/run-reborn-group-tests.sh|tests/reborn_*|tests/support/reborn/*|tests/fixtures/llm_traces/reborn_qa/*|tests/e2e/scenarios/test_reborn_*)
      return 0
      ;;
    crates/ironclaw_architecture/*)
      return 0
      ;;
    crates/ironclaw_reborn/*|crates/ironclaw_reborn_*/*)
      return 0
      ;;
    crates/ironclaw_product_*/*|crates/ironclaw_slack_v2_adapter/*|crates/ironclaw_telegram_v2_adapter/*)
      return 0
      ;;
    crates/ironclaw_wasm_product_adapters/*|crates/ironclaw_webui_v2/*)
      return 0
      ;;
    crates/ironclaw_conversations/*|crates/ironclaw_outbound/*|crates/ironclaw_triggers/*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

is_code_path() {
  local path="$1"
  case "$path" in
    src/*|crates/*|channels-src/*|tools-src/*|tests/*|migrations/*)
      return 0
      ;;
    Cargo.toml|Cargo.lock|Dockerfile|build.rs|providers.json)
      return 0
      ;;
    scripts/check_no_panics.py|scripts/check_gateway_boundaries.py|scripts/build-wasm-extensions.sh|scripts/check-version-bumps.sh|scripts/reborn-e2e-rust.sh|scripts/ci/*)
      return 0
      ;;
    .github/workflows/*.yml|.github/actions/install-cargo-component/*|.github/dependabot.yml|.github/labeler.yml)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

while IFS= read -r path || [ -n "$path" ]; do
  [ -n "$path" ] || continue

  if ! is_docs_only_path "$path"; then
    docs_only=false
  fi

  if is_code_path "$path"; then
    has_core_code=true
  fi

  if is_shared_test_path "$path"; then
    has_legacy_tests=true
    has_reborn_tests=true
  elif is_reborn_test_path "$path"; then
    has_reborn_tests=true
  elif is_code_path "$path"; then
    has_legacy_tests=true
  fi
done

cat <<EOF
docs_only=${docs_only}
has_core_code=${has_core_code}
has_legacy_tests=${has_legacy_tests}
has_reborn_tests=${has_reborn_tests}
EOF
