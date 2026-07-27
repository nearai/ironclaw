#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <package>" >&2
  exit 2
fi

package="$1"

# Default flags for closure crates without an explicit recipe above: opt into
# `default` when the crate declares it. Crates with no matching features build
# bare; database backends compile unconditionally.
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
  if [ "${#features[@]}" -gt 0 ]; then
    local IFS=,
    printf '%s\n' "--features ${features[*]}"
  fi
}

case "${package}" in
  ironclaw_product)
    printf '%s\n' "--features test-support,host-auth-mint"
    ;;
  ironclaw_reborn_composition)
    # memory-mem0 turns on the (off-by-default) mem0 third-party memory provider
    # so its factory + swap tests run here; the feature-off build stays covered
    # by every other CI lane (memory-mem0 is in no default set). Database
    # backends compile unconditionally now, so no libsql feature is selected.
    printf '%s\n' "--features test-support,memory-mem0"
    ;;
  ironclaw_runner)
    ;;
  ironclaw_reborn_event_store)
    ;;
  ironclaw_hooks)
    # The durable libSQL/Postgres backends + parity matrix folded into this
    # crate are exercised by the dedicated hooks-parity job in
    # platform-and-compat.yml (integration,test-support).
    # Keep this reborn-closure job light — the framework's own unit tests only —
    # so it does not pull more integration-tier work into the crate bucket.
    printf '%s\n' "--features test-support"
    ;;
  ironclaw_webui)
    printf '%s\n' "--features test-support"
    ;;
  ironclaw_host_runtime)
    # Integration tests (tests/) link the lib as a normal dependency, so
    # cfg(test) is false there; the deterministic test-mode behavior they assert
    # is gated behind `feature = "test-support"`.
    printf '%s\n' "--features test-support"
    ;;
  ironclaw_reborn_openai_compat)
    ;;
  ironclaw_architecture | \
  ironclaw_reborn_config | \
  ironclaw_reborn_identity | \
  ironclaw_reborn_traces | \
  ironclaw_telegram_extension | \
  ironclaw_telegram_v2_adapter)
    # Already on the allowlist with no feature flags; keep them flag-free now
    # that the default branch derives fallback features for closure crates.
    ;;
  *)
    fallback_feature_flags
    ;;
esac
