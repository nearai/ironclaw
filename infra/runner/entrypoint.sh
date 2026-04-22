#!/usr/bin/env bash
# Entrypoint for the private-oauth GitHub Actions runner.
#
# First boot: downloads the runner binary and registers against GH_RUNNER_URL
# using GH_RUNNER_TOKEN. State is written to $RUNNER_DATA/runner, which lives
# on a persistent Railway volume — so every boot after the first finds a
# configured runner and skips straight to `./run.sh`.
#
# The registration token is one-shot (expires ~1h after generation). Once the
# runner is registered, you can and should remove GH_RUNNER_TOKEN from the
# service env. See README.md for the full bring-up sequence.

set -euo pipefail

: "${GH_RUNNER_URL:?GH_RUNNER_URL is required (e.g. https://github.com/ORG/REPO)}"
: "${RUNNER_DATA:=/runner-data}"
: "${RUNNER_NAME:=railway-private-oauth}"
: "${RUNNER_LABELS:=self-hosted,ironclaw-live}"

RUNNER_DIR="${RUNNER_DATA}/runner"
WORK_DIR="${RUNNER_DATA}/_work"

mkdir -p "${RUNNER_DIR}" "${WORK_DIR}" "${HOME}" "${RUNNER_TOOL_CACHE}" "${RUNNER_TEMP}"

# One-shot ironclaw DB bootstrap. The `private-oauth` canary lane expects
# an existing libsql DB at `$HOME/.ironclaw/ironclaw.db` with pre-seeded
# Google OAuth secrets (`google_oauth_token`, `..._refresh_token`,
# `..._scopes`). Minting those requires a human clicking "Allow" in a
# browser; the pragmatic flow is to do the consent on a laptop and
# transfer the resulting DB onto the runner volume.
#
# `IRONCLAW_DB_B64` is a base64-encoded copy of that DB. When set AND
# the target file doesn't already exist, we decode once into place.
# Running daily canary jobs rotate the refresh token on the runner's
# DB; the `-f` guard ensures we never overwrite those rotations with
# the stale laptop snapshot. To force a re-seed (e.g., after a volume
# wipe), the file won't be there so the decode fires automatically.
#
# After a successful decode operators should remove `IRONCLAW_DB_B64`
# from the Railway service env — the value is large (~1 MB base64'd
# for a typical DB) and doesn't need to persist.
DB_TARGET="${HOME}/.ironclaw/ironclaw.db"
if [[ -n "${IRONCLAW_DB_B64:-}" && ! -f "${DB_TARGET}" ]]; then
    echo "[entrypoint] Bootstrapping ${DB_TARGET} from IRONCLAW_DB_B64"
    mkdir -p "$(dirname "${DB_TARGET}")"
    # Strip any whitespace the Railway UI may have introduced on paste
    # (wrapped lines, trailing newlines) before decode.
    if ! printf '%s' "${IRONCLAW_DB_B64}" | tr -d '[:space:]' \
            | base64 -d > "${DB_TARGET}"; then
        echo "[entrypoint] ERROR: base64 decode of IRONCLAW_DB_B64 failed" >&2
        rm -f "${DB_TARGET}"
        exit 1
    fi
    chmod 600 "${DB_TARGET}"
    # stat flag differs across GNU/BSD; fall back silently if neither matches.
    db_size="$(stat -c %s "${DB_TARGET}" 2>/dev/null \
        || stat -f %z "${DB_TARGET}" 2>/dev/null || echo unknown)"
    echo "[entrypoint] Wrote ${db_size} bytes to ${DB_TARGET}"
fi

# Fallback bootstrap route for when the base64-in-env path blows past
# the service plan's env-var size limit (Railway varies by plan, some
# cap at 64 KB). Set IRONCLAW_DB_URL to a short-lived pre-signed URL
# the runner can GET once; the file is written to the same target as
# IRONCLAW_DB_B64 and the same `-f` guard applies — once the DB exists
# on the volume, subsequent boots skip the fetch so in-flight refresh
# token rotations aren't clobbered.
#
# Operator hygiene: use a URL that expires in an hour, from a service
# you control (S3/R2 presigned URL, private gist asset, etc.). The
# libsql file has encrypted secret values but plaintext schema — don't
# park it on a public pastebin.
if [[ -n "${IRONCLAW_DB_URL:-}" && ! -f "${DB_TARGET}" ]]; then
    echo "[entrypoint] Bootstrapping ${DB_TARGET} from IRONCLAW_DB_URL"
    mkdir -p "$(dirname "${DB_TARGET}")"
    if ! curl --fail --silent --show-error --location \
            --max-time 120 \
            --output "${DB_TARGET}" \
            "${IRONCLAW_DB_URL}"; then
        echo "[entrypoint] ERROR: fetch of IRONCLAW_DB_URL failed" >&2
        rm -f "${DB_TARGET}"
        exit 1
    fi
    chmod 600 "${DB_TARGET}"
    db_size="$(stat -c %s "${DB_TARGET}" 2>/dev/null \
        || stat -f %z "${DB_TARGET}" 2>/dev/null || echo unknown)"
    echo "[entrypoint] Fetched ${db_size} bytes to ${DB_TARGET}"
fi

# Recovery path. If the volume holds a stale `.runner` sentinel for a
# registration that GitHub has since deleted (because the UI "Remove"
# button was clicked, or GitHub auto-GC'd a runner that went offline
# for long enough), `./run.sh` fails with
#   "Failed to create a session. The runner registration has been deleted
#    from the server, please re-configure."
# and the `[[ ! -f .runner ]]` gate below would keep short-circuiting
# re-registration forever. Set RUNNER_FORCE_REREGISTER=1 + a fresh
# GH_RUNNER_TOKEN on the service to wipe the sentinel on next boot,
# re-register, and then unset the var once Idle.
if [[ "${RUNNER_FORCE_REREGISTER:-0}" == "1" && -f "${RUNNER_DIR}/.runner" ]]; then
    echo "[entrypoint] RUNNER_FORCE_REREGISTER=1 — wiping stale registration state"
    rm -f \
        "${RUNNER_DIR}/.runner" \
        "${RUNNER_DIR}/.credentials" \
        "${RUNNER_DIR}/.credentials_rsaparams" \
        "${RUNNER_DIR}/.path"
fi

# Sentinel written by ./config.sh on successful registration. Absent → first
# boot (or a wiped volume); present → rebooting an already-registered runner.
if [[ ! -f "${RUNNER_DIR}/.runner" ]]; then
    : "${GH_RUNNER_TOKEN:?GH_RUNNER_TOKEN is required on first boot. Generate it at Settings → Actions → Runners → New self-hosted runner, then unset after registration.}"

    echo "[entrypoint] Downloading actions-runner v${RUNNER_VERSION}"
    cd "${RUNNER_DIR}"
    curl -fsSL \
        "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" \
        | tar xz

    echo "[entrypoint] Registering runner ${RUNNER_NAME} at ${GH_RUNNER_URL}"
    ./config.sh \
        --unattended \
        --replace \
        --url "${GH_RUNNER_URL}" \
        --token "${GH_RUNNER_TOKEN}" \
        --name "${RUNNER_NAME}" \
        --labels "${RUNNER_LABELS}" \
        --work "${WORK_DIR}"
    echo "[entrypoint] Registration complete. Unset GH_RUNNER_TOKEN in Railway env now."
fi

cd "${RUNNER_DIR}"

# ./run.sh exits on SIGTERM; Railway sends SIGTERM before kill on deploy, so a
# clean shutdown happens without us intervening. We deliberately do NOT call
# `./config.sh remove` on shutdown — the runner stays registered so the next
# container boot picks up exactly where this one left off.
exec ./run.sh
