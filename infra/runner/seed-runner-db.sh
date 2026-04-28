#!/usr/bin/env bash
# One-shot bootstrap helper: serves the ironclaw libsql DB over a
# short-lived Cloudflare Quick Tunnel so the Railway runner can fetch
# it via `IRONCLAW_DB_URL`. See infra/runner/README.md.
#
# Usage:
#   ./infra/runner/seed-runner-db.sh                 # uses $HOME/.ironclaw/ironclaw.db
#   ./infra/runner/seed-runner-db.sh /some/other.db  # override DB path
#
# The script:
#   1. Copies the DB into an isolated tempdir so no other local files
#      are exposed through the tunnel.
#   2. Starts a loopback-only Python http.server on a random port.
#   3. Starts `cloudflared tunnel` pointed at it.
#   4. Prints the public URL to paste into Railway as IRONCLAW_DB_URL.
#   5. Blocks until you Ctrl-C — access logs stream to stderr so you
#      can confirm the runner actually pulled the file.
#   6. On exit (Ctrl-C or failure), kills both processes and wipes
#      the tempdir.
#
# Dependencies: python3, cloudflared (`brew install cloudflared`).

set -euo pipefail

DB_PATH="${1:-$HOME/.ironclaw/ironclaw.db}"

if [[ ! -f "${DB_PATH}" ]]; then
    echo "ERROR: ${DB_PATH} does not exist" >&2
    exit 1
fi

if ! command -v cloudflared >/dev/null 2>&1; then
    echo "ERROR: cloudflared not installed. Install with:" >&2
    echo "    brew install cloudflared" >&2
    exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 not found in PATH" >&2
    exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
    echo "ERROR: sqlite3 not found in PATH (needed to checkpoint WAL before copy)" >&2
    exit 1
fi

SERVE_DIR="$(mktemp -d -t ironclaw-seed-XXXXXX)"
TUNNEL_LOG="$(mktemp -t ironclaw-seed-tunnel-XXXXXX)"

cleanup() {
    local exit_code=$?
    if [[ -n "${HTTP_PID:-}" ]]; then
        kill "${HTTP_PID}" 2>/dev/null || true
    fi
    if [[ -n "${TUNNEL_PID:-}" ]]; then
        kill "${TUNNEL_PID}" 2>/dev/null || true
    fi
    rm -rf "${SERVE_DIR}" "${TUNNEL_LOG}"
    exit "${exit_code}"
}
trap cleanup EXIT INT TERM

# libSQL runs in WAL mode (see `src/db/libsql/mod.rs` —
# `PRAGMA journal_mode=WAL`), so recent committed writes may live in
# `ironclaw.db-wal` rather than in the main `ironclaw.db` file. A
# naive `cp` of just the main file would silently drop those writes —
# meaning the runner could boot with a stale OAuth access / refresh
# token even though the local DB looks current.
#
# Run `PRAGMA wal_checkpoint(TRUNCATE)` first so every committed page
# is flushed into the main file and the WAL is emptied. Safe whether
# or not a writer is currently open: SQLite's checkpoint API is
# multi-writer aware. On an idle DB this is ~10 ms; on a busy DB it
# blocks briefly until a quiet window.
echo "[seed] Checkpointing WAL into ${DB_PATH}"
sqlite3 "${DB_PATH}" "PRAGMA wal_checkpoint(TRUNCATE);" >/dev/null

cp "${DB_PATH}" "${SERVE_DIR}/ironclaw.db"
chmod 600 "${SERVE_DIR}/ironclaw.db"

# Random high port to avoid collision with a local gateway that might
# already be bound to 8000 / 3000.
PORT=$((20000 + RANDOM % 10000))

echo "[seed] Serving $(du -h "${SERVE_DIR}/ironclaw.db" | cut -f1) from 127.0.0.1:${PORT}"
(cd "${SERVE_DIR}" && python3 -m http.server "${PORT}" --bind 127.0.0.1) \
    >/dev/null &
HTTP_PID=$!

# Give Python a moment to bind.
sleep 1

# Make sure the local server is actually up before spawning the tunnel;
# otherwise cloudflared can publish a URL before the backend is ready
# and the runner's first GET races with it.
if ! curl --silent --fail --max-time 3 \
        --head "http://127.0.0.1:${PORT}/ironclaw.db" >/dev/null; then
    echo "ERROR: local http server didn't come up on port ${PORT}" >&2
    exit 1
fi

echo "[seed] Starting cloudflared Quick Tunnel"
cloudflared tunnel --url "http://127.0.0.1:${PORT}" \
    >"${TUNNEL_LOG}" 2>&1 &
TUNNEL_PID=$!

# Wait up to 30s for the tunnel to print its URL. Cloudflared's log
# format is stable enough that grep on `*.trycloudflare.com` works.
URL=""
for _ in $(seq 1 30); do
    URL="$(grep -oE 'https://[a-z0-9-]+\.trycloudflare\.com' "${TUNNEL_LOG}" \
            | head -1 || true)"
    if [[ -n "${URL}" ]]; then
        break
    fi
    sleep 1
done

if [[ -z "${URL}" ]]; then
    echo "ERROR: cloudflared didn't produce a tunnel URL within 30s" >&2
    echo "--- tunnel log ---" >&2
    cat "${TUNNEL_LOG}" >&2
    exit 1
fi

FETCH_URL="${URL}/ironclaw.db"
printf '\n'
printf '============================================================\n'
printf 'Paste this into Railway as IRONCLAW_DB_URL:\n\n'
printf '    %s\n\n' "${FETCH_URL}"
printf 'Then redeploy the service. Watch the Railway log for:\n'
printf '    [entrypoint] Fetched N bytes to /runner-data/home/.ironclaw/ironclaw.db\n\n'
printf 'Each GET below this line is the runner pulling the file.\n'
printf 'Press Ctrl-C here once you see the fetch succeed, then\n'
printf 'remove IRONCLAW_DB_URL from Railway env.\n'
printf '============================================================\n\n'

# Tail the tunnel log in the background so we also see cloudflared-side
# request logs, but strip its noisy `INF` prefix for readability.
tail -F "${TUNNEL_LOG}" 2>/dev/null \
    | grep --line-buffered -E 'GET|POST|HEAD|ERROR|error' \
    | sed -u 's/^/[tunnel] /' &

# Block on the tunnel process. User kills with Ctrl-C.
wait "${TUNNEL_PID}"
