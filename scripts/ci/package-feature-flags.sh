#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <package>" >&2
  exit 2
fi

package="$1"

# Default flags for closure crates without an explicit recipe above: opt into
# `default` and `libsql` when the crate declares them, so storage-backed crates
# build their libSQL paths. Crates with no matching features build bare.
fallback_feature_flags() {
  local metadata
  metadata="$(cargo metadata --no-deps --format-version 1)"

  local feature_list
  feature_list="$(
    jq -r --arg package "${package}" '
      .packages[]
      | select(.name == $package)
      | .features
      | keys[]
    ' <<< "${metadata}"
  )"

  local features=()
  if printf '%s\n' "${feature_list}" | grep -Fxq "default"; then
    features+=("default")
  fi
  if printf '%s\n' "${feature_list}" | grep -Fxq "libsql"; then
    features+=("libsql")
  fi

  if [ "${#features[@]}" -gt 0 ]; then
    local IFS=,
    printf '%s\n' "--features ${features[*]}"
  fi
}

case "${package}" in
  ironclaw_reborn_cli)
    printf '%s\n' "--features webui-v2-beta,slack-v2-host-beta"
    ;;
  ironclaw_product_adapters)
    printf '%s\n' "--features test-support,host-auth-mint"
    ;;
  ironclaw_product_workflow)
    printf '%s\n' "--features test-support"
    ;;
  ironclaw_product_workflow_storage)
    printf '%s\n' "--features libsql"
    ;;
  ironclaw_reborn_composition)
    printf '%s\n' "--features test-support,webui-v2-beta,slack-v2-host-beta,libsql"
    ;;
  ironclaw_reborn)
    printf '%s\n' "--features root-llm-provider,libsql-secrets,libsql-restart-tests,webui-user-store"
    ;;
  ironclaw_reborn_event_store)
    printf '%s\n' "--features libsql"
    ;;
  ironclaw_reborn_webui_ingress)
    printf '%s\n' "--features dev-in-memory-session"
    ;;
  ironclaw_host_runtime)
    # Integration tests (tests/) link the lib as a normal dependency, so
    # cfg(test) is false there; the deterministic test-mode behavior they assert
    # is gated behind `feature = "test-support"`. libsql exercises the embedded
    # DB paths without a Postgres server (which the crate-tests job has none of).
    printf '%s\n' "--features test-support,libsql"
    ;;
  ironclaw_webui_v2 | ironclaw_webui_v2_static)
    printf '%s\n' "--features webui-v2-beta"
    ;;
  ironclaw_architecture | \
  ironclaw_product_adapter_registry | \
  ironclaw_product_context | \
  ironclaw_reborn_config | \
  ironclaw_reborn_identity | \
  ironclaw_reborn_openai_compat | \
  ironclaw_reborn_openai_compat_storage | \
  ironclaw_reborn_traces | \
  ironclaw_slack_v2_adapter | \
  ironclaw_telegram_v2_adapter | \
  ironclaw_wasm_product_adapters)
    # Already on the allowlist with no feature flags; keep them flag-free now
    # that the default branch derives fallback features for closure crates.
    ;;
  *)
    fallback_feature_flags
    ;;
esac
