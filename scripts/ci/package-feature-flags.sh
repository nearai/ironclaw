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
  ironclaw_product_adapters)
    printf '%s\n' "--features test-support,host-auth-mint"
    ;;
  ironclaw_product_workflow)
    printf '%s\n' "--features test-support"
    ;;
  ironclaw_reborn_composition)
    printf '%s\n' "--features test-support,libsql"
    ;;
  ironclaw_runner)
    printf '%s\n' "--features libsql-secrets,libsql-restart-tests,webui-user-store"
    ;;
  ironclaw_reborn_event_store)
    ;;
  ironclaw_hooks)
    # The durable libSQL/Postgres backends + parity matrix folded into this
    # crate are exercised by the dedicated hooks-parity job in
    # platform-and-compat.yml (postgres,libsql,integration,test-support).
    # Keep this reborn-closure job light — the framework's own unit tests only —
    # so it does not pull the heavy libSQL/Postgres driver deps that the default
    # fallback would otherwise add now that the crate declares a `libsql` feature.
    printf '%s\n' "--features test-support"
    ;;
  ironclaw_webui)
    printf '%s\n' "--features test-support"
    ;;
  ironclaw_host_runtime)
    # Integration tests (tests/) link the lib as a normal dependency, so
    # cfg(test) is false there; the deterministic test-mode behavior they assert
    # is gated behind `feature = "test-support"`. libsql exercises the embedded
    # DB paths without a Postgres server (which the crate-tests job has none of).
    printf '%s\n' "--features test-support,libsql"
    ;;
  ironclaw_reborn_openai_compat)
    ;;
  ironclaw_architecture | \
  ironclaw_channel_delivery | \
  ironclaw_product_adapter_registry | \
  ironclaw_product_context | \
  ironclaw_reborn_config | \
  ironclaw_reborn_identity | \
  ironclaw_reborn_traces | \
  ironclaw_slack_v2_adapter | \
  ironclaw_telegram_extension | \
  ironclaw_telegram_v2_adapter | \
  ironclaw_wasm_product_adapters)
    # Already on the allowlist with no feature flags; keep them flag-free now
    # that the default branch derives fallback features for closure crates.
    ;;
  *)
    fallback_feature_flags
    ;;
esac
