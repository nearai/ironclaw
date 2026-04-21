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
