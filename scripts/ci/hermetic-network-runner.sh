#!/usr/bin/env bash
set -euo pipefail

guard_library="${IRONCLAW_HERMETIC_NETWORK_GUARD_LIBRARY:?network guard library is not configured}"
hermetic_root="${IRONCLAW_HERMETIC_ROOT:?hermetic root is not configured}"
violation_log="$(mktemp "${hermetic_root}/network-violations.XXXXXX")"
trap 'rm -f "${violation_log}"' EXIT
guarded_command=("$@")

export IRONCLAW_HERMETIC_NETWORK_VIOLATIONS="${violation_log}"
case "$(uname -s)" in
  Linux)
    case ":${LD_PRELOAD:-}:" in
      *":${guard_library}:"*) ;;
      *) export LD_PRELOAD="${guard_library}${LD_PRELOAD:+:${LD_PRELOAD}}" ;;
    esac
    ;;
  Darwin)
    case ":${DYLD_INSERT_LIBRARIES:-}:" in
      *":${guard_library}:"*) ;;
      *)
        export DYLD_INSERT_LIBRARIES="${guard_library}${DYLD_INSERT_LIBRARIES:+:${DYLD_INSERT_LIBRARIES}}"
        ;;
    esac
    export DYLD_FORCE_FLAT_NAMESPACE=1
    # SIP-protected Apple executables strip DYLD_* before their descendants
    # inherit it. The process sandbox keeps the whole tree fail-closed; the
    # interposer still records actionable diagnostics for ordinary binaries.
    if [[ "${IRONCLAW_HERMETIC_SANDBOX_ACTIVE:-0}" != "1" ]]; then
      if ! command -v sandbox-exec >/dev/null 2>&1; then
        echo "sandbox-exec is required for fail-closed macOS hermetic networking" >&2
        exit 2
      fi
      if ! sandbox-exec -p '(version 1) (allow default)' /usr/bin/true; then
        echo "sandbox-exec is present but cannot enforce a process sandbox" >&2
        exit 2
      fi
      export IRONCLAW_HERMETIC_SANDBOX_ACTIVE=1
      guarded_command=(
        sandbox-exec
        -p
        '(version 1) (allow default) (deny network-outbound) (allow network-outbound (remote ip "localhost:*"))'
        "$@"
      )
    fi
    ;;
  *)
    echo "unsupported platform for hermetic network guard: $(uname -s)" >&2
    exit 2
    ;;
esac

set +e
"${guarded_command[@]}"
command_status=$?
set -e

if [[ -s "${violation_log}" ]]; then
  cat "${violation_log}" >&2
  exit 86
fi
exit "${command_status}"
