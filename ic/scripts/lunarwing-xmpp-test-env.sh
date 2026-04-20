#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_ROOT="${LUNARWING_TEST_ROOT:-${TMPDIR:-/tmp}/lunarwing-xmpp-test}"
ENV_DIR="$TEST_ROOT/env"
LOG_DIR="$TEST_ROOT/logs"
RUN_DIR="$TEST_ROOT/run"
STATE_DIR="$TEST_ROOT/state"
SYSTEMD_DIR="$TEST_ROOT/systemd"
PROFILE="${LUNARWING_TEST_PROFILE:-debug}"

usage() {
  cat <<'EOF'
Usage:
  scripts/lunarwing-xmpp-test-env.sh <command> [args...]

Commands:
  init                     create isolated env, state, run, and log dirs
  build                    build LunarWing and xmpp-bridge binaries
  doctor                   show local dependency, binary, env, and bridge checks
  start-bridge             start xmpp-bridge with the test env
  stop-bridge              stop the bridge started by this script
  bridge-status            call authenticated GET /v1/status
  bridge-auth-check        verify missing-token rejection and valid-token status
  smoke                    run bridge start/auth/status smoke test
  configure-bridge [args]  run scripts/xmpp-configure.sh with the test env
  rate-limit [args]        run scripts/xmpp-rate-limit.sh with the test env
  start-lunarwing [-- args]
                           start target/<profile>/ironclaw with isolated state
  stop-lunarwing           stop LunarWing started by this script
  lunarwing-status         show pid/log hints for the local LunarWing process
  render-systemd           write user-service unit files under the test root
  logs [lines]             tail test logs

Environment:
  LUNARWING_TEST_ROOT      default: ${TMPDIR:-/tmp}/lunarwing-xmpp-test
  LUNARWING_TEST_PROFILE   debug or release; default: debug
  LUNARWING_TEST_SERVICE_NAME
                           default: lunarwing-test.service
  LUNARWING_TEST_BRIDGE_SERVICE_NAME
                           default: xmpp-bridge-test.service
  LUNARWING_TEST_SYSTEMCTL_SCOPE
                           user or system for configure-bridge --restart; default: user
  LUNARWING_TEST_KEEP_BRIDGE=1 keeps smoke-test bridge running

Notes:
  The generated env files are chmod 600 and may contain live XMPP secrets.
  This script never prints XMPP_BRIDGE_TOKEN or XMPP_PASSWORD.
EOF
}

say() {
  printf '%s\n' "$*"
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    die "$1 is required"
  fi
}

profile_dir() {
  case "$PROFILE" in
    debug|"")
      printf 'debug'
      ;;
    release)
      printf 'release'
      ;;
    *)
      die "LUNARWING_TEST_PROFILE must be 'debug' or 'release'"
      ;;
  esac
}

lunarwing_bin() {
  printf '%s/target/%s/ironclaw' "$REPO_ROOT" "$(profile_dir)"
}

bridge_bin() {
  printf '%s/bridges/xmpp-bridge/target/%s/xmpp-bridge' "$REPO_ROOT" "$(profile_dir)"
}

generate_token() {
  if command -v od >/dev/null 2>&1; then
    dd if=/dev/urandom bs=32 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'
  else
    printf 'replace-with-random-token-%s' "$(date +%s)"
  fi
}

init_dirs() {
  mkdir -p "$ENV_DIR" "$LOG_DIR" "$RUN_DIR" "$STATE_DIR/xmpp" "$SYSTEMD_DIR"
  chmod 700 "$ENV_DIR" "$RUN_DIR" "$STATE_DIR" 2>/dev/null || true
}

write_lunarwing_env_if_missing() {
  local path="$ENV_DIR/lunarwing.env"
  if [[ -f "$path" ]]; then
    return 0
  fi

  (
    umask 077
    {
      printf 'IRONCLAW_BASE_DIR=%s\n' "$STATE_DIR"
      printf 'DATABASE_BACKEND=libsql\n'
      printf 'LIBSQL_PATH=%s/ironclaw.db\n' "$STATE_DIR"
      printf 'RUST_LOG=ironclaw=info,lunarwing=info\n'
      printf '\n'
      printf '# Add live agent credentials here only for integration tests.\n'
      printf '# Run onboarding against this isolated IRONCLAW_BASE_DIR when possible.\n'
    } >"$path"
  )
}

write_bridge_env_if_missing() {
  local path="$ENV_DIR/xmpp-bridge.env"
  if [[ -f "$path" ]]; then
    return 0
  fi

  local token
  token="$(generate_token)"

  (
    umask 077
    {
      printf 'IRONCLAW_BASE_DIR=%s\n' "$STATE_DIR"
      printf 'XMPP_BRIDGE_BIND=127.0.0.1:8787\n'
      printf 'XMPP_BRIDGE_TOKEN=%s\n' "$token"
      printf 'XMPP_BRIDGE_MAX_MESSAGES=256\n'
      printf 'RUST_LOG=xmpp_bridge=info,info\n'
      printf '\n'
      printf '# Live XMPP credentials. Leave empty for local bridge API smoke tests.\n'
      printf 'XMPP_JID=\n'
      printf 'XMPP_PASSWORD=\n'
      printf 'XMPP_DM_POLICY=allowlist\n'
      printf 'XMPP_ALLOW_FROM_JSON=[]\n'
      printf 'XMPP_ALLOW_ROOMS_JSON=[]\n'
      printf 'XMPP_ENCRYPTED_ROOMS_JSON=[]\n'
      printf 'XMPP_DEVICE_ID=0\n'
      printf 'XMPP_OMEMO_STORE_DIR=%s/xmpp\n' "$STATE_DIR"
      printf 'XMPP_ALLOW_PLAINTEXT_FALLBACK=true\n'
      printf 'XMPP_RESOURCE=lunarwing-test\n'
      printf 'XMPP_BRIDGE_WAIT_SECONDS=15\n'
    } >"$path"
  )
}

init_env() {
  init_dirs
  write_lunarwing_env_if_missing
  write_bridge_env_if_missing
  say "test root: $TEST_ROOT"
  say "lunarwing env: $ENV_DIR/lunarwing.env"
  say "xmpp bridge env: $ENV_DIR/xmpp-bridge.env"
  say "edit the env files for live XMPP or agent credentials; secrets are not printed"
}

load_env_file() {
  local path="$1"
  [[ -f "$path" ]] || die "missing env file: $path; run init first"
  set -a
  # shellcheck disable=SC1090
  . "$path"
  set +a
}

load_lunarwing_env() {
  load_env_file "$ENV_DIR/lunarwing.env"
}

load_bridge_env() {
  load_env_file "$ENV_DIR/xmpp-bridge.env"
}

load_all_envs() {
  load_lunarwing_env
  load_bridge_env
}

ensure_env() {
  init_dirs
  write_lunarwing_env_if_missing
  write_bridge_env_if_missing
}

bridge_base() {
  local bind="${XMPP_BRIDGE_BIND:-127.0.0.1:8787}"
  local port="${bind##*:}"
  if [[ "$port" == "$bind" || -z "$port" ]]; then
    port="8787"
  fi
  printf 'http://127.0.0.1:%s' "$port"
}

normalize_service_name() {
  local name="$1"
  if [[ -z "$name" ]]; then
    die "service name cannot be empty"
  fi
  if [[ "$name" == *"/"* || "$name" =~ [[:space:]] ]]; then
    die "service name must not contain slashes or whitespace: $name"
  fi
  case "$name" in
    *.service)
      printf '%s' "$name"
      ;;
    *)
      printf '%s.service' "$name"
      ;;
  esac
}

lunarwing_service_name() {
  normalize_service_name "${LUNARWING_TEST_SERVICE_NAME:-lunarwing-test.service}"
}

bridge_service_name() {
  normalize_service_name "${LUNARWING_TEST_BRIDGE_SERVICE_NAME:-xmpp-bridge-test.service}"
}

require_binary() {
  local path="$1"
  local build_hint="$2"
  [[ -x "$path" ]] || die "missing executable: $path; run $build_hint"
}

pid_alive() {
  local pid_file="$1"
  local pid
  [[ -f "$pid_file" ]] || return 1
  pid="$(<"$pid_file")"
  [[ -n "$pid" ]] || return 1
  kill -0 "$pid" >/dev/null 2>&1
}

bridge_ready() {
  local base
  base="$(bridge_base)"
  local code
  code="$(curl -sS -o /dev/null -w '%{http_code}' \
    "$base/v1/status" \
    -H "Authorization: Bearer ${XMPP_BRIDGE_TOKEN:-}" 2>/dev/null || true)"
  [[ "$code" == "200" ]]
}

wait_for_bridge() {
  local wait_seconds="${XMPP_BRIDGE_WAIT_SECONDS:-15}"
  local deadline=$((SECONDS + wait_seconds))
  while (( SECONDS < deadline )); do
    if bridge_ready; then
      return 0
    fi
    sleep 1
  done

  say "xmpp-bridge did not become ready within ${wait_seconds}s" >&2
  if [[ -f "$LOG_DIR/xmpp-bridge.log" ]]; then
    tail -n 40 "$LOG_DIR/xmpp-bridge.log" >&2 || true
  fi
  return 1
}

build_bins() {
  ensure_env
  require_cmd cargo

  local cargo_args=(build)
  if [[ "$(profile_dir)" == "release" ]]; then
    cargo_args+=(--release)
  fi

  say "building LunarWing binary: $(lunarwing_bin)"
  (cd "$REPO_ROOT" && cargo "${cargo_args[@]}" --bin ironclaw)

  say "building xmpp-bridge binary: $(bridge_bin)"
  (cd "$REPO_ROOT/bridges/xmpp-bridge" && cargo "${cargo_args[@]}")
}

start_bridge() {
  ensure_env
  load_bridge_env
  require_cmd curl
  local bin
  bin="$(bridge_bin)"
  require_binary "$bin" "build"

  if pid_alive "$RUN_DIR/xmpp-bridge.pid"; then
    say "xmpp-bridge already running with pid $(<"$RUN_DIR/xmpp-bridge.pid")"
    return 0
  fi

  if bridge_ready; then
    say "xmpp-bridge already responds at $(bridge_base)"
    return 0
  fi

  say "starting xmpp-bridge at $(bridge_base)"
  (
    cd "$REPO_ROOT/bridges/xmpp-bridge"
    "$bin"
  ) >>"$LOG_DIR/xmpp-bridge.log" 2>&1 &
  printf '%s\n' "$!" >"$RUN_DIR/xmpp-bridge.pid"

  wait_for_bridge
  say "xmpp-bridge ready; pid $(<"$RUN_DIR/xmpp-bridge.pid")"
}

stop_by_pid_file() {
  local name="$1"
  local pid_file="$2"
  if ! pid_alive "$pid_file"; then
    rm -f "$pid_file"
    say "$name is not running from this test harness"
    return 0
  fi

  local pid
  pid="$(<"$pid_file")"
  say "stopping $name pid $pid"
  kill "$pid" 2>/dev/null || true

  local deadline=$((SECONDS + 10))
  while (( SECONDS < deadline )); do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      rm -f "$pid_file"
      say "$name stopped"
      return 0
    fi
    sleep 1
  done

  say "$name did not exit after SIGTERM; leaving pid file for inspection" >&2
  return 1
}

stop_bridge() {
  stop_by_pid_file "xmpp-bridge" "$RUN_DIR/xmpp-bridge.pid"
}

maybe_jq() {
  if command -v jq >/dev/null 2>&1; then
    jq .
  else
    cat
  fi
}

bridge_status() {
  ensure_env
  load_bridge_env
  require_cmd curl
  curl -sS "$(bridge_base)/v1/status" \
    -H "Authorization: Bearer $XMPP_BRIDGE_TOKEN" | maybe_jq
}

bridge_auth_check() {
  ensure_env
  load_bridge_env
  require_cmd curl

  local base unauth_code auth_code
  base="$(bridge_base)"
  unauth_code="$(curl -sS -o /dev/null -w '%{http_code}' "$base/v1/status" 2>/dev/null || true)"
  auth_code="$(curl -sS -o /dev/null -w '%{http_code}' "$base/v1/status" \
    -H "Authorization: Bearer $XMPP_BRIDGE_TOKEN" 2>/dev/null || true)"

  if [[ "$unauth_code" == "200" ]]; then
    die "bridge accepted a request without the bearer token"
  fi
  if [[ "$auth_code" != "200" ]]; then
    die "authenticated bridge status failed with HTTP $auth_code"
  fi

  say "missing-token status: HTTP $unauth_code"
  say "valid-token status: HTTP $auth_code"
}

smoke() {
  ensure_env
  load_bridge_env
  require_cmd curl

  local started=false
  if ! bridge_ready; then
    start_bridge
    started=true
  fi

  bridge_auth_check
  bridge_status

  if [[ "$started" == "true" && "${LUNARWING_TEST_KEEP_BRIDGE:-0}" != "1" ]]; then
    stop_bridge
  else
    say "bridge left running; stop it with: scripts/lunarwing-xmpp-test-env.sh stop-bridge"
  fi
}

configure_bridge() {
  ensure_env
  load_all_envs
  BASE="$(bridge_base)" \
    XMPP_BRIDGE_SERVICE="$(bridge_service_name)" \
    XMPP_BRIDGE_SYSTEMCTL_SCOPE="${LUNARWING_TEST_SYSTEMCTL_SCOPE:-user}" \
    "$SCRIPT_DIR/xmpp-configure.sh" "$@"
}

rate_limit() {
  ensure_env
  load_bridge_env
  BASE="$(bridge_base)" "$SCRIPT_DIR/xmpp-rate-limit.sh" "$@"
}

start_lunarwing() {
  ensure_env
  load_lunarwing_env
  local bin
  bin="$(lunarwing_bin)"
  require_binary "$bin" "build"

  if pid_alive "$RUN_DIR/lunarwing.pid"; then
    say "LunarWing already running with pid $(<"$RUN_DIR/lunarwing.pid")"
    return 0
  fi

  local args=(--no-onboard run)
  if [[ "${1:-}" == "--" ]]; then
    shift
    args=("$@")
  elif [[ "$#" -gt 0 ]]; then
    args=("$@")
  fi

  say "starting LunarWing with isolated IRONCLAW_BASE_DIR=$IRONCLAW_BASE_DIR"
  (
    cd "$REPO_ROOT"
    "$bin" "${args[@]}"
  ) >>"$LOG_DIR/lunarwing.log" 2>&1 &
  printf '%s\n' "$!" >"$RUN_DIR/lunarwing.pid"

  sleep 2
  if ! pid_alive "$RUN_DIR/lunarwing.pid"; then
    say "LunarWing exited during startup; recent log follows" >&2
    tail -n 60 "$LOG_DIR/lunarwing.log" >&2 || true
    rm -f "$RUN_DIR/lunarwing.pid"
    return 1
  fi

  say "LunarWing started; pid $(<"$RUN_DIR/lunarwing.pid")"
}

stop_lunarwing() {
  stop_by_pid_file "LunarWing" "$RUN_DIR/lunarwing.pid"
}

lunarwing_status() {
  if pid_alive "$RUN_DIR/lunarwing.pid"; then
    say "LunarWing pid: $(<"$RUN_DIR/lunarwing.pid")"
  else
    say "LunarWing is not running from this test harness"
  fi
  say "log: $LOG_DIR/lunarwing.log"
  say "state: $STATE_DIR"
}

render_systemd() {
  ensure_env
  local main_service bridge_service lunarwing_unit bridge_unit
  main_service="$(lunarwing_service_name)"
  bridge_service="$(bridge_service_name)"
  lunarwing_unit="$SYSTEMD_DIR/$main_service"
  bridge_unit="$SYSTEMD_DIR/$bridge_service"

  cat >"$bridge_unit" <<EOF
[Unit]
Description=LunarWing XMPP bridge test sidecar
After=network.target
Wants=network.target
PartOf=$main_service

[Service]
Type=simple
WorkingDirectory=$REPO_ROOT/bridges/xmpp-bridge
EnvironmentFile=$ENV_DIR/xmpp-bridge.env
ExecStart=$(bridge_bin)
Restart=on-failure
RestartSec=5
NoNewPrivileges=true

[Install]
WantedBy=default.target
EOF

  cat >"$lunarwing_unit" <<EOF
[Unit]
Description=LunarWing test daemon
After=network.target $bridge_service
Wants=$bridge_service

[Service]
Type=simple
WorkingDirectory=$REPO_ROOT
EnvironmentFile=$ENV_DIR/lunarwing.env
ExecStart=$(lunarwing_bin) --no-onboard run
Restart=on-failure
RestartSec=5
NoNewPrivileges=true

[Install]
WantedBy=default.target
EOF

  say "wrote: $bridge_unit"
  say "wrote: $lunarwing_unit"
  say "install for user-mode testing with:"
  say "  mkdir -p ~/.config/systemd/user"
  say "  cp $SYSTEMD_DIR/*.service ~/.config/systemd/user/"
  say "  systemctl --user daemon-reload"
  say "  systemctl --user start $bridge_service"
}

doctor() {
  say "test root: $TEST_ROOT"
  say "repo root: $REPO_ROOT"
  say "profile: $(profile_dir)"

  for cmd_name in cargo curl jq systemctl; do
    if command -v "$cmd_name" >/dev/null 2>&1; then
      say "$cmd_name: found"
    else
      say "$cmd_name: missing"
    fi
  done

  if [[ -f "$ENV_DIR/lunarwing.env" ]]; then
    say "lunarwing env: present at $ENV_DIR/lunarwing.env"
  else
    say "lunarwing env: missing; run init"
  fi

  if [[ -f "$ENV_DIR/xmpp-bridge.env" ]]; then
    say "xmpp bridge env: present at $ENV_DIR/xmpp-bridge.env"
    load_bridge_env
    if [[ -n "${XMPP_BRIDGE_TOKEN:-}" ]]; then
      say "xmpp bridge token: present"
    else
      say "xmpp bridge token: missing"
    fi
    if command -v curl >/dev/null 2>&1; then
      local code
      code="$(curl -sS -o /dev/null -w '%{http_code}' "$(bridge_base)/v1/status" \
        -H "Authorization: Bearer ${XMPP_BRIDGE_TOKEN:-}" 2>/dev/null || true)"
      say "xmpp bridge status endpoint: HTTP $code"
    fi
  else
    say "xmpp bridge env: missing; run init"
  fi

  if [[ -x "$(lunarwing_bin)" ]]; then
    say "LunarWing binary: $(lunarwing_bin)"
  else
    say "LunarWing binary: missing; run build"
  fi

  if [[ -x "$(bridge_bin)" ]]; then
    say "xmpp-bridge binary: $(bridge_bin)"
  else
    say "xmpp-bridge binary: missing; run build"
  fi

  if command -v systemctl >/dev/null 2>&1; then
    local bridge_service
    bridge_service="$(bridge_service_name)"
    if systemctl --user --no-pager --plain status "$bridge_service" >/dev/null 2>&1; then
      say "$bridge_service: active in user systemd"
    else
      say "$bridge_service: not active or not installed in user systemd"
    fi
  fi
}

logs() {
  local lines="${1:-80}"
  [[ "$lines" =~ ^[0-9]+$ ]] || die "lines must be an integer"
  if [[ -f "$LOG_DIR/xmpp-bridge.log" ]]; then
    say "== xmpp-bridge.log =="
    tail -n "$lines" "$LOG_DIR/xmpp-bridge.log"
  else
    say "no xmpp-bridge log at $LOG_DIR/xmpp-bridge.log"
  fi
  if [[ -f "$LOG_DIR/lunarwing.log" ]]; then
    say "== lunarwing.log =="
    tail -n "$lines" "$LOG_DIR/lunarwing.log"
  else
    say "no LunarWing log at $LOG_DIR/lunarwing.log"
  fi
}

main() {
  local command_name="${1:-}"
  if [[ -z "$command_name" ]]; then
    usage
    exit 1
  fi
  shift || true

  case "$command_name" in
    -h|--help|help)
      usage
      ;;
    init)
      init_env
      ;;
    build)
      build_bins
      ;;
    doctor)
      doctor
      ;;
    start-bridge)
      start_bridge
      ;;
    stop-bridge)
      stop_bridge
      ;;
    bridge-status)
      bridge_status
      ;;
    bridge-auth-check)
      bridge_auth_check
      ;;
    smoke)
      smoke
      ;;
    configure-bridge)
      configure_bridge "$@"
      ;;
    rate-limit)
      rate_limit "$@"
      ;;
    start-lunarwing)
      start_lunarwing "$@"
      ;;
    stop-lunarwing)
      stop_lunarwing
      ;;
    lunarwing-status)
      lunarwing_status
      ;;
    render-systemd)
      render_systemd
      ;;
    logs)
      logs "$@"
      ;;
    *)
      die "unknown command: $command_name"
      ;;
  esac
}

main "$@"
