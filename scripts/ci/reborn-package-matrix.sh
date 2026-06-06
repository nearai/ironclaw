#!/usr/bin/env bash
set -euo pipefail

cargo metadata --no-deps --format-version 1 \
  | jq -c '
      [
        .packages[]
        | select(
            (.name | startswith("ironclaw_reborn"))
            or (.name | startswith("ironclaw_product"))
            or (.name == "ironclaw_architecture")
            or (.name == "ironclaw_slack_v2_adapter")
            or (.name == "ironclaw_telegram_v2_adapter")
            or (.name == "ironclaw_wasm_product_adapters")
            or (.name | startswith("ironclaw_webui_v2"))
          )
        | .name
      ]
      | unique
    '
