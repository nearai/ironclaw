#!/usr/bin/env bash
set -euo pipefail

# Security: Never log, echo, or print the values of these variables:
# - NEARAI_API_KEY / NEARAI_SESSION_TOKEN
# - OPENCLAW_GATEWAY_TOKEN
# - SSH_PUBKEY
#
# WARNING: Do not enable debug mode (set -x) as it will expose all variable values.

# Ensure volume mount points are writable by agent
mkdir -p /home/agent/.ironclaw /home/agent/workspace
chown -R agent:agent /home/agent/.ironclaw /home/agent/workspace

# ============================================
# SSH Server Configuration
# ============================================
if [ -n "${SSH_PUBKEY:-}" ]; then
    echo "Configuring SSH authorized_keys..."
    mkdir -p /home/agent/.ssh
    echo "${SSH_PUBKEY}" > /home/agent/.ssh/authorized_keys
    chmod 755 /home/agent
    chmod 700 /home/agent/.ssh
    chmod 600 /home/agent/.ssh/authorized_keys
    chown -R agent:agent /home/agent/.ssh

    mkdir -p /run/sshd
    chmod 0755 /run/sshd

    passwd -d agent 2>/dev/null || usermod -U agent 2>/dev/null || true

    echo "Starting SSH daemon on port 2222..."
    SSHD_OUTPUT=$(/usr/sbin/sshd -f /dev/null \
        -o Port=2222 \
        -o ListenAddress=0.0.0.0 \
        -o HostKey=/home/agent/ssh/ssh_host_ed25519_key \
        -o AuthorizedKeysFile=/home/agent/.ssh/authorized_keys \
        -o PasswordAuthentication=no \
        -o PermitRootLogin=no \
        -o PidFile=/home/agent/ssh/sshd.pid \
        -o StrictModes=yes 2>&1) && SSHD_RC=0 || SSHD_RC=$?
    if [ "$SSHD_RC" -eq 0 ]; then
        echo "SSH daemon started on port 2222"
    else
        echo "Warning: Failed to start SSH daemon (exit code: $SSHD_RC)" >&2
        echo "SSHD output: $SSHD_OUTPUT" >&2
    fi
else
    echo "Warning: SSH_PUBKEY not set - SSH access will not be available" >&2
fi
chown -R agent:agent /home/agent/.ssh 2>/dev/null || true

# ============================================
# IronClaw Configuration (env var mapping)
# ============================================

# Database: libSQL embedded (no external DB dependency)
export DATABASE_BACKEND=libsql
export LIBSQL_PATH=/home/agent/.ironclaw/ironclaw.db

# Gateway: bind to all interfaces on port 18789 (matches compose port mapping)
export GATEWAY_ENABLED=true
export GATEWAY_HOST=0.0.0.0
export GATEWAY_PORT=18789
export GATEWAY_AUTH_TOKEN="${OPENCLAW_GATEWAY_TOKEN:-changeme}"

# Disable interactive REPL (no TTY in container; would cause immediate shutdown)
export CLI_ENABLED=false

# NEAR AI: map compose-api env vars to IronClaw config
NEARAI_API_URL="${NEARAI_API_URL:-https://cloud-api.near.ai/v1}"
# Strip trailing /v1 — IronClaw's NEARAI_BASE_URL is the root (it adds /v1 internally)
export NEARAI_BASE_URL="${NEARAI_API_URL%/v1}"

# Map NEARAI_API_KEY to session token (IronClaw picks up NEARAI_SESSION_TOKEN
# on startup and saves it to ~/.ironclaw/session.json)
if [ -n "${NEARAI_API_KEY:-}" ]; then
    export NEARAI_SESSION_TOKEN="${NEARAI_API_KEY}"
else
    echo "Warning: NEARAI_API_KEY not set. IronClaw may not function correctly." >&2
fi

export RUST_LOG="${RUST_LOG:-ironclaw=info}"

# ============================================
# Workspace Bootstrap
# ============================================
if [ -d /app/workspace ]; then
    for f in /app/workspace/*.md; do
        [ -f "$f" ] || continue
        fname=$(basename "$f")
        if [ ! -f "/home/agent/workspace/$fname" ]; then
            cp "$f" "/home/agent/workspace/$fname"
            chown agent:agent "/home/agent/workspace/$fname"
            echo "Bootstrap file $fname installed to workspace"
        fi
    done
fi

# ============================================
# Final Ownership Fix
# ============================================
chown -R agent:agent /home/agent/.ironclaw /home/agent/workspace

# ============================================
# Start IronClaw with auto-restart
# ============================================
RESTART_DELAY="${IRONCLAW_RESTART_DELAY:-5}"

while true; do
    echo "Starting IronClaw..."
    chown -R agent:agent /home/agent/.ironclaw /home/agent/workspace 2>/dev/null || true
    runuser -p -u agent -- ironclaw run --no-onboard || true
    echo "IronClaw exited. Restarting in ${RESTART_DELAY}s..."
    sleep "$RESTART_DELAY"
done
