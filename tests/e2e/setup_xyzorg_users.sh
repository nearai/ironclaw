#!/usr/bin/env bash
#
# setup_xyzorg_users.sh — provision the xyzorg org on a RUNNING local
# ironclaw-reborn instance via the admin REST surface, and print each user's
# minted login bearer so you can sign into the WebUI as them.
#
# The OWNER is `director`, established via env vars ONLY:
#     IRONCLAW_REBORN_WEBUI_USER_ID=director
#     IRONCLAW_REBORN_WEBUI_TOKEN=matoken
# `director` is NOT created via REST and is the single owner — REST mints no
# owners. `matoken` is director's login bearer.
#
# What it does:
#   * create officer (member -> promoted admin), alice/bob/carl (member)
#     (director, the env owner, is the bootstrap admin that creates them)
#   * GRANT per-user capabilities (steps 6-8 of the xyzorg test) — this script
#     only ADDS capabilities (sets availability=available); it never hides/removes:
#       6. alice: grant builtin.shell + nearai.web_search
#       7. bob:   grant the live google-drive.* (+ github.*) cap-ids
#       8. carl:  grant nothing extra (keeps just the essential baseline)
#     Every member already comes with the essential baseline (extension_search,
#     extension_activate, echo, time, json, memory_read, memory_search,
#     memory_write, memory_tree) + capability_info;
#     mechanism: PUT {"availability":"available"} on each cap to grant.
#
# The login bearer is shown ONCE at creation (stored only as a hash), so this
# script's output is your only copy.
#
# IMPORTANT: the admin routes are mounted ONLY when serve is built with the
# `capability-policy` feature, and enforcement only bites with
# IRONCLAW_REBORN_CAPABILITY_POLICY=1. Run the instance like:
#   IRONCLAW_REBORN_CAPABILITY_POLICY=1 \
#     cargo run -p ironclaw_reborn_cli --features webui-v2-beta,capability-policy -- serve
#
# Usage:
#   tests/e2e/setup_xyzorg_users.sh
#   BASE_URL=http://127.0.0.1:3000 tests/e2e/setup_xyzorg_users.sh
#   OPERATOR_TOKEN=matoken tests/e2e/setup_xyzorg_users.sh
#
# Compatible with macOS stock bash 3.2 (no associative arrays / `declare -A`).
set -o pipefail

# --- config ----------------------------------------------------------------
BASE_URL="${BASE_URL:-http://127.0.0.1:3000}"
API="${BASE_URL}/api/webchat/v2"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENV_FILE="${ENV_FILE:-${REPO_ROOT}/.env}"

# Owner (director) bearer: env override, else read from .env. matoken authenticates
# as `director`, the env-configured owner.
OPERATOR_TOKEN="${OPERATOR_TOKEN:-}"
if [ -z "${OPERATOR_TOKEN}" ] && [ -f "${ENV_FILE}" ]; then
  OPERATOR_TOKEN="$(grep -E '^[[:space:]]*IRONCLAW_REBORN_WEBUI_TOKEN[[:space:]]*=' "${ENV_FILE}" \
    | tail -n1 | sed -E 's/^[^=]*=[[:space:]]*//; s/^"(.*)"$/\1/; s/[[:space:]]+$//')"
fi
if [ -z "${OPERATOR_TOKEN}" ]; then
  echo "ERROR: no owner token. Set OPERATOR_TOKEN=... or IRONCLAW_REBORN_WEBUI_TOKEN in ${ENV_FILE}" >&2
  exit 1
fi

# Per-user capability GRANTS (steps 6-8). Space-separated cap-ids to GRANT to
# each member (set availability=available). This script only ever SETS/ADDS
# capabilities — it never hides or removes any. Members already come with the
# essential baseline (extension_search, extension_activate, echo, time, json,
# memory_read, memory_search, memory_write, memory_tree), so we grant only the
# EXTRA caps each one needs.
# Extension caps (nearai.*, google-drive.*) are reached by the agent via
# extension_search, so granting them just makes them available to discover.
ALICE_ALLOW="builtin.shell nearai.web_search"
BOB_ALLOW=""   # filled with the live google-drive.* cap-ids in step [3/3]
CARL_ALLOW=""  # nothing extra granted (just the essential baseline)

# REST-created users (director is the env owner, NOT created here).
ORDER="officer alice bob carl"
final_role_for() {  # role to DISPLAY (officer ends up admin)
  case "$1" in officer) echo admin ;; *) echo member ;; esac
}
allow_for() {
  case "$1" in alice) echo "$ALICE_ALLOW" ;; bob) echo "$BOB_ALLOW" ;; carl) echo "$CARL_ALLOW" ;; *) echo "" ;; esac
}

# --- helpers ---------------------------------------------------------------
need() { command -v "$1" >/dev/null 2>&1 || { echo "ERROR: '$1' is required" >&2; exit 1; }; }
need curl
need python3

contains() { case " $1 " in *" $2 "*) return 0 ;; *) return 1 ;; esac; }

json_get() {
  python3 -c 'import sys,json
try:
    d=json.load(sys.stdin); print(d.get(sys.argv[1],""))
except Exception:
    print("")' "$2" <<EOF
$1
EOF
}

# GET /settings/tools entries -> capability ids (strip "tool." prefix), one per line.
parse_caps() {
  python3 -c 'import sys,json
try:
    d=json.load(sys.stdin)
except Exception:
    sys.exit(0)
for e in d.get("entries",[]):
    k=e.get("key","")
    if k.startswith("tool."):
        print(k[len("tool."):])' <<EOF
$1
EOF
}

create_user() {  # <admin> <uid> <role> -> echoes token (200); logs to stderr
  admin="$1"; uid="$2"; role="$3"
  body="$(curl -sS -m 15 -w $'\n%{http_code}' -X POST "${API}/admin/users" \
    -H "Authorization: Bearer ${admin}" -H "Content-Type: application/json" \
    -d "{\"user_id\":\"${uid}\",\"role\":\"${role}\"}")"
  code="$(printf '%s' "$body" | tail -n1)"; json="$(printf '%s' "$body" | sed '$d')"
  if [ "$code" = "200" ]; then
    printf '  ✓ created %-10s role=%-7s\n' "$uid" "$role" >&2
    json_get "$json" token; return 0
  fi
  printf '  ✗ create %-10s -> HTTP %s: %s\n' "$uid" "$code" "$json" >&2
  return 1
}

set_role() {
  admin="$1"; uid="$2"; role="$3"
  body="$(curl -sS -m 15 -w $'\n%{http_code}' -X PUT "${API}/admin/users/${uid}/role" \
    -H "Authorization: Bearer ${admin}" -H "Content-Type: application/json" \
    -d "{\"role\":\"${role}\"}")"
  code="$(printf '%s' "$body" | tail -n1)"
  if [ "$code" = "200" ]; then printf '  ✓ promoted %-10s -> %s\n' "$uid" "$role" >&2
  else printf '  ✗ promote %-10s -> HTTP %s: %s\n' "$uid" "$code" "$(printf '%s' "$body" | sed '$d')" >&2; fi
}

# grant_capability <admin> <user> <cap> ; SETS a capability available for the
# user (an `available` per-user delta) — it adds, never hides/removes. Retries on
# 429 (the per-user-caps route is rate-limited 60/60s); only a few writes per
# member, so the backoff (14 attempts, capped 12s, > one 60s window) is just a
# safety net.
grant_capability() {
  admin="$1"; uid="$2"; cap="$3"; attempt=0
  while [ "$attempt" -lt 14 ]; do
    body="$(curl -sS -m 15 -w $'\n%{http_code}' \
      -X PUT "${API}/admin/users/${uid}/capabilities/${cap}" \
      -H "Authorization: Bearer ${admin}" -H "Content-Type: application/json" \
      -d '{"availability":"available"}')"
    code="$(printf '%s' "$body" | tail -n1)"
    if [ "$code" != "429" ]; then
      [ "$code" = "200" ] && return 0
      printf '    ! grant %s/%s -> HTTP %s\n' "$uid" "$cap" "$code" >&2
      return 1
    fi
    attempt=$((attempt + 1))
    wait=$((2 + attempt * 2)); [ "$wait" -gt 12 ] && wait=12
    sleep "$wait"
  done
  printf '    ! grant %s/%s -> 429 (gave up)\n' "$uid" "$cap" >&2
  return 1
}

# create_sso_user <admin> <email> <role> — provision an SSO user BY EMAIL
# (token-less: the user logs in via Google SSO, so no bearer is minted). The
# email-based admin endpoint is NOT built yet, so this call is expected to fail
# (400/404/422) until the admin email surface lands; we log the code and keep
# going so the rest of setup still runs.
create_sso_user() {
  admin="$1"; email="$2"; role="$3"
  body="$(curl -sS -m 15 -w $'\n%{http_code}' -X POST "${API}/admin/users" \
    -H "Authorization: Bearer ${admin}" -H "Content-Type: application/json" \
    -d "{\"email\":\"${email}\",\"role\":\"${role}\",\"sso\":true}")"
  code="$(printf '%s' "$body" | tail -n1)"
  if [ "$code" = "200" ] || [ "$code" = "201" ]; then
    printf '  ✓ SSO %-24s role=%-7s (token-less)\n' "$email" "$role" >&2
  else
    printf '  … SSO %-24s role=%-7s -> HTTP %s (email endpoint not wired yet)\n' "$email" "$role" "$code" >&2
  fi
}

# --- create users + roles --------------------------------------------------
echo "Target:   ${BASE_URL}"
echo "Owner:    director (env bearer from ${ENV_FILE##*/}; not created via REST)"
echo
echo "[1/3] Creating users (officer/alice/bob/carl)..."

# director (the env owner) is the bootstrap admin that creates everyone.
admin_bearer="${OPERATOR_TOKEN}"
for uid in $ORDER; do
  tok="$(create_user "${admin_bearer}" "$uid" member)"
  eval "TOK_${uid}=\"\${tok}\""
done

echo
echo "[2/3] Promoting officer -> admin..."
set_role "${admin_bearer}" officer admin

# --- per-user grants (steps 6-8) -------------------------------------------
# GRANT each member's extra capabilities (an `available` delta) on top of the
# essential baseline they already have. Adds only — nothing is hidden or removed.
# Only a few writes per member.
echo
echo "[3/3] Granting per-user capabilities (set available; never removed)..."
# Resolve bob's Google Drive grants to the REAL google-drive.* cap-ids from the
# live catalog (the per-user-caps route rejects unknown ids with 400, so package
# labels like "gdrive" would fail). If Drive isn't installed, bob simply gets the
# essential baseline only.
tools_json="$(curl -sS -m 15 "${API}/settings/tools" -H "Authorization: Bearer ${OPERATOR_TOKEN}")"
BOB_ALLOW="$(parse_caps "$tools_json" | grep -E '^(google-drive|github)\.' | tr '\n' ' ')"
for member in alice bob carl; do
  allow="$(allow_for "$member")"
  granted=0
  for cap in $allow; do
    if grant_capability "${admin_bearer}" "$member" "$cap"; then granted=$((granted + 1)); fi
  done
  printf '  %-6s grant=[%s] granted=%d\n' "$member" "$allow" "$granted"
done

# --- SSO users (email-keyed, token-less) -----------------------------------
# Two accounts that authenticate via Google SSO (mocked in the e2e validator;
# real Google for manual local verification): a NEW admin, and carl — who has
# moved from a directory token to SSO. Created BY EMAIL through the admin REST;
# no login token is minted (they log in via SSO). The email-based endpoint is
# NOT built yet, so these calls log + continue (see create_sso_user).
echo
# Emails are env-driven so no real address is committed; defaults use the
# fictional test org. Set SSO_ADMIN_EMAIL / SSO_MEMBER_EMAIL to real accounts.
SSO_ADMIN_EMAIL="${SSO_ADMIN_EMAIL:-product@xyzorg.com}"
SSO_MEMBER_EMAIL="${SSO_MEMBER_EMAIL:-carl@xyzorg.com}"
echo "[+] Provisioning SSO users by email (admin + carl; override via SSO_ADMIN_EMAIL/SSO_MEMBER_EMAIL)..."
create_sso_user "${admin_bearer}" "${SSO_ADMIN_EMAIL}" admin
create_sso_user "${admin_bearer}" "${SSO_MEMBER_EMAIL}" member

# --- login tokens ----------------------------------------------------------
echo
echo "============================================================"
echo "  LOGIN TOKENS  (Authorization: Bearer <token>)"
echo "============================================================"
printf '  %-10s (%-6s)  %s   <- env owner (IRONCLAW_REBORN_WEBUI_TOKEN)\n' "director" "owner" "${OPERATOR_TOKEN}"
for uid in $ORDER; do
  eval "tok=\"\${TOK_${uid}:-}\""
  role="$(final_role_for "$uid")"
  if [ -n "$tok" ]; then printf '  %-10s (%-6s)  %s\n' "$uid" "$role" "$tok"
  else printf '  %-10s (%-6s)  <not minted — see errors above>\n' "$uid" "$role"; fi
done
echo "============================================================"
echo
echo "Log in:  open ${BASE_URL}/?token=<token>   (or send Authorization: Bearer <token>)"
