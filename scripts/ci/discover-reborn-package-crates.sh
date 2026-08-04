#!/usr/bin/env bash
set -euo pipefail

# Canonical package set for the deterministic Reborn crate lanes. Keep both the
# local runner and CI matrix on this helper so package-family changes cannot
# silently diverge.
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

# Reborn/product crate families that are always tested, including crates the
# shipped binary does not link (channel adapters and WebUI packages). The
# integration-test host is excluded because its [[test]] targets run in the
# dedicated root and integration lanes; adding --all-targets here would execute
# those environment-sensitive suites twice.
allowlist="$(
  cargo metadata --no-deps --format-version 1 \
    | jq -c '
        [
          .packages[]
          | select(
              (
                (.name == "ironclaw")
                or (.name == "ironclaw_runner")
                or (.name | startswith("ironclaw_reborn"))
                or (.name | startswith("ironclaw_product"))
                or (.name == "ironclaw_architecture")
                or (.name == "ironclaw_slack_extension")
                or (.name == "ironclaw_telegram_extension")
                or (.name | startswith("ironclaw_webui"))
              )
              and (.name != "ironclaw_reborn_integration_tests")
            )
          | .name
        ]
        | unique
      '
)"

# Every workspace crate linked by the shipped binary through a normal or build
# dependency. The union below keeps non-closure allowlist crates covered too.
closure="$(
  LC_ALL=C comm -12 \
    <(cargo tree -p ironclaw -e normal,build --prefix none \
      | grep -oE 'ironclaw_[a-z0-9_]+' \
      | LC_ALL=C sort -u) \
    <(cargo metadata --no-deps --format-version 1 \
      | jq -r '.packages[].name' \
      | LC_ALL=C sort -u) \
    | jq -R -s -c 'split("\n") | map(select(length > 0))'
)"

if [[ -z "${closure}" || "${closure}" == "[]" ]]; then
  echo "No Reborn CLI workspace dependency closure crates discovered" >&2
  exit 1
fi

packages="$(
  jq -n -c \
    --argjson allowlist "${allowlist}" \
    --argjson closure "${closure}" \
    '$allowlist + $closure | unique'
)"
if [[ -z "${packages}" || "${packages}" == "[]" ]]; then
  echo "No Reborn workspace crates discovered" >&2
  exit 1
fi

printf '%s\n' "${packages}"
