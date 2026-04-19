#!/usr/bin/env bash
set -euo pipefail

if ! command -v curl >/dev/null 2>&1; then
  echo "error: curl is required" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required" >&2
  exit 1
fi

BASE="${BASE:-http://127.0.0.1:8787}"
XMPP_BRIDGE_TOKEN="${XMPP_BRIDGE_TOKEN:-}"

usage() {
  cat <<'EOF'
Usage:
  scripts/xmpp-rate-limit.sh status
  scripts/xmpp-rate-limit.sh off [--reset]
  scripts/xmpp-rate-limit.sh set <messages_per_hour> [--reset]
  scripts/xmpp-rate-limit.sh reset

Environment:
  BASE               default: http://127.0.0.1:8787
  XMPP_BRIDGE_TOKEN  required

Commands:
  status             show configured/live outbound XMPP cap and current usage
  off                disable the live outbound XMPP cap (set to 0)
  set <n>            set the live outbound XMPP cap to n messages/hour
  reset              clear the current rolling-hour usage counter, keep live cap

Options:
  --reset            when used with off/set, clear the current rolling-hour usage too

Notes:
  This changes the live bridge override only. Restarting xmpp-bridge resets it.
EOF
}

require_token() {
  if [[ -z "$XMPP_BRIDGE_TOKEN" ]]; then
    echo "error: XMPP_BRIDGE_TOKEN is not set" >&2
    exit 1
  fi
}

print_json_or_fail() {
  local response="$1"
  local jq_filter="${2:-.}"

  if jq -e . >/dev/null 2>&1 <<<"$response"; then
    jq "$jq_filter" <<<"$response"
  else
    echo "error: bridge returned a non-JSON response:" >&2
    printf '%s\n' "$response" >&2
    return 1
  fi
}

post_rate_limit() {
  local payload="$1"
  local response
  response="$(curl -sS -X POST "$BASE/v1/outbound-rate-limit" \
    -H "Authorization: Bearer $XMPP_BRIDGE_TOKEN" \
    -H 'Content-Type: application/json' \
    -d "$payload")"
  print_json_or_fail "$response"
}

show_status() {
  local response
  response="$(curl -sS "$BASE/v1/status" \
    -H "Authorization: Bearer $XMPP_BRIDGE_TOKEN")"
  print_json_or_fail "$response" '{
      configured,
      running,
      configured_max_messages_per_hour,
      active_max_messages_per_hour,
      outbound_messages_last_hour,
      outbound_rate_limit_overridden
    }'
}

require_token

if [[ "$#" -lt 1 ]]; then
  usage >&2
  exit 1
fi

command_name="$1"
shift

reset_counter=false

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --reset)
      reset_counter=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

case "$command_name" in
  status)
    if [[ "$#" -ne 0 ]]; then
      echo "error: status takes no extra arguments" >&2
      exit 1
    fi
    show_status
    ;;
  off)
    if [[ "$#" -ne 0 ]]; then
      echo "error: off takes no extra arguments" >&2
      exit 1
    fi
    post_rate_limit "{\"max_messages_per_hour\":0,\"reset_counter\":$reset_counter}"
    ;;
  set)
    if [[ "$#" -ne 1 ]]; then
      echo "error: set requires exactly one integer argument" >&2
      exit 1
    fi
    if ! [[ "$1" =~ ^[0-9]+$ ]]; then
      echo "error: messages_per_hour must be a non-negative integer" >&2
      exit 1
    fi
    post_rate_limit "{\"max_messages_per_hour\":$1,\"reset_counter\":$reset_counter}"
    ;;
  reset)
    if [[ "$#" -ne 0 ]]; then
      echo "error: reset takes no extra arguments" >&2
      exit 1
    fi
    post_rate_limit '{"reset_counter":true}'
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    echo "error: unknown command: $command_name" >&2
    usage >&2
    exit 1
    ;;
esac
