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
XMPP_JID="${XMPP_JID:-example@xmpp.prosody.world}"
XMPP_PASSWORD="${XMPP_PASSWORD:-}"
XMPP_DM_POLICY="${XMPP_DM_POLICY:-allowlist}"
XMPP_ALLOW_FROM_JSON="${XMPP_ALLOW_FROM_JSON:-[]}"
XMPP_ENCRYPTED_ROOMS_JSON="${XMPP_ENCRYPTED_ROOMS_JSON:-[]}"
XMPP_DEVICE_ID="${XMPP_DEVICE_ID:-0}"
XMPP_OMEMO_STORE_DIR="${XMPP_OMEMO_STORE_DIR:-/home/user/.ironclaw/xmpp}"
XMPP_ALLOW_PLAINTEXT_FALLBACK="${XMPP_ALLOW_PLAINTEXT_FALLBACK:-true}"
XMPP_RESOURCE="${XMPP_RESOURCE:-ironclaw}"
XMPP_ALLOW_ROOMS_JSON="${XMPP_ALLOW_ROOMS_JSON:-}"
XMPP_BRIDGE_WAIT_SECONDS="${XMPP_BRIDGE_WAIT_SECONDS:-15}"

restart_bridge=false
show_status_after_configure=false

usage() {
  cat <<'EOF'
Usage:
  scripts/xmpp-configure.sh [--restart] [--show-status] room1@conference.example.com [room2@conference.example.com ...]

Environment:
  Required:
    XMPP_BRIDGE_TOKEN
    XMPP_PASSWORD

  Optional:
    BASE                          default: http://127.0.0.1:8787
    XMPP_JID                      default: ruffles@xmpp.sobe.world
    XMPP_DM_POLICY                default: allowlist
    XMPP_ALLOW_FROM_JSON          default: []
    XMPP_ALLOW_ROOMS_JSON         JSON array if you do not want to pass rooms as args
    XMPP_ENCRYPTED_ROOMS_JSON     default: []
    XMPP_DEVICE_ID                default: 0
    XMPP_OMEMO_STORE_DIR          default: /home/user/.ironclaw/xmpp
    XMPP_ALLOW_PLAINTEXT_FALLBACK default: true
    XMPP_RESOURCE                 default: ironclaw
    XMPP_BRIDGE_WAIT_SECONDS      default: 15

Notes:
  /v1/status does not list plain allow_rooms. It only shows encrypted-room counts.
EOF
}

wait_for_bridge() {
  local deadline
  deadline=$((SECONDS + XMPP_BRIDGE_WAIT_SECONDS))
  while (( SECONDS < deadline )); do
    if curl -sS "$BASE/v1/status" \
      -H "Authorization: Bearer $XMPP_BRIDGE_TOKEN" \
      >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done

  echo "error: xmpp-bridge did not become ready within ${XMPP_BRIDGE_WAIT_SECONDS}s" >&2
  return 1
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --restart)
      restart_bridge=true
      shift
      ;;
    --show-status)
      show_status_after_configure=true
      shift
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      break
      ;;
  esac
done

if [[ -z "$XMPP_BRIDGE_TOKEN" ]]; then
  echo "error: XMPP_BRIDGE_TOKEN is not set" >&2
  exit 1
fi

if [[ -z "$XMPP_PASSWORD" ]]; then
  echo "error: XMPP_PASSWORD is not set" >&2
  exit 1
fi

if [[ "$#" -gt 0 ]]; then
  allow_rooms_json="$(printf '%s\n' "$@" | jq -R . | jq -s .)"
elif [[ -n "$XMPP_ALLOW_ROOMS_JSON" ]]; then
  allow_rooms_json="$XMPP_ALLOW_ROOMS_JSON"
else
  echo "error: pass room JIDs as arguments or set XMPP_ALLOW_ROOMS_JSON" >&2
  exit 1
fi

if [[ "$restart_bridge" == "true" ]]; then
  sudo systemctl restart xmpp-bridge.service
  wait_for_bridge
fi

jq -n \
  --arg jid "$XMPP_JID" \
  --arg password "$XMPP_PASSWORD" \
  --arg dm_policy "$XMPP_DM_POLICY" \
  --arg omemo_store_dir "$XMPP_OMEMO_STORE_DIR" \
  --arg resource "$XMPP_RESOURCE" \
  --argjson allow_from "$XMPP_ALLOW_FROM_JSON" \
  --argjson allow_rooms "$allow_rooms_json" \
  --argjson encrypted_rooms "$XMPP_ENCRYPTED_ROOMS_JSON" \
  --argjson device_id "$XMPP_DEVICE_ID" \
  --argjson allow_plaintext_fallback "$XMPP_ALLOW_PLAINTEXT_FALLBACK" \
  '{
    jid: $jid,
    password: $password,
    dm_policy: $dm_policy,
    allow_from: $allow_from,
    allow_rooms: $allow_rooms,
    encrypted_rooms: $encrypted_rooms,
    device_id: $device_id,
    omemo_store_dir: $omemo_store_dir,
    allow_plaintext_fallback: $allow_plaintext_fallback,
    resource: $resource
  }' | curl -sS -X POST "$BASE/v1/configure" \
  -H "Authorization: Bearer $XMPP_BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  --data-binary @- | jq

if [[ "$show_status_after_configure" == "true" ]]; then
  curl -sS "$BASE/v1/status" \
    -H "Authorization: Bearer $XMPP_BRIDGE_TOKEN" | jq
  echo "note: /v1/status does not list plain allow_rooms" >&2
fi
