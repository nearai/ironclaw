#!/usr/bin/env bash
set -euo pipefail

guard_library="${IRONCLAW_HERMETIC_NETWORK_GUARD_LIBRARY:?network guard library is not configured}"
hermetic_root="${IRONCLAW_HERMETIC_ROOT:?hermetic root is not configured}"
violation_log="$(mktemp "${hermetic_root}/network-violations.XXXXXX")"
trap 'rm -f "${violation_log}"' EXIT

export IRONCLAW_HERMETIC_NETWORK_VIOLATIONS="${violation_log}"
case "$(uname -s)" in
  Linux)
    export LD_PRELOAD="${guard_library}${LD_PRELOAD:+:${LD_PRELOAD}}"
    ;;
  Darwin)
    export DYLD_INSERT_LIBRARIES="${guard_library}${DYLD_INSERT_LIBRARIES:+:${DYLD_INSERT_LIBRARIES}}"
    export DYLD_FORCE_FLAT_NAMESPACE=1
    ;;
  *)
    echo "unsupported platform for hermetic network guard: $(uname -s)" >&2
    exit 2
    ;;
esac

set +e
"$@"
command_status=$?
set -e

if [[ -s "${violation_log}" ]]; then
  cat "${violation_log}" >&2
  exit 86
fi
exit "${command_status}"
