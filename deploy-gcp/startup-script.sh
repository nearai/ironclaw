#!/usr/bin/env bash
# Self-contained GCP startup script for BastionClaw staging VM.
# Injected via: gcloud compute instances add-metadata ... --metadata-from-file startup-script=...
# Triggered by: gcloud compute instances reset ...
#
# Runs as root automatically on VM boot. Logs to /var/log/bastionclaw-startup.log
# and to the serial port (visible via: gcloud compute instances get-serial-port-output).

set -euo pipefail
exec > >(tee /var/log/bastionclaw-startup.log | logger -t bastionclaw-startup) 2>&1

PROJECT="gen-lang-client-0263867259"
REGION="us-central1"
REPO="bastionclaw"
IMAGE_PREFIX="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}"

echo "==> [1/5] Installing Docker (official repo)"
apt-get update -qq
apt-get install -y --no-install-recommends ca-certificates curl gnupg
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
ARCH=$(dpkg --print-architecture)
CODENAME=$(. /etc/os-release && echo "$VERSION_CODENAME")
echo "deb [arch=${ARCH} signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian ${CODENAME} stable" \
  > /etc/apt/sources.list.d/docker.list
apt-get update -qq
apt-get install -y --no-install-recommends \
  docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
systemctl enable docker
systemctl start docker

echo "==> [2/5] Configuring Artifact Registry auth"
gcloud auth configure-docker "${REGION}-docker.pkg.dev" --quiet

echo "==> [3/5] Setting up /opt/bastionclaw"
mkdir -p /opt/bastionclaw
chmod 700 /opt/bastionclaw

# Write docker-compose.yml — uses AR images, no local build needed
cat > /opt/bastionclaw/docker-compose.yml << 'COMPOSE'
services:
  postgres:
    image: pgvector/pgvector:pg16
    ports:
      - "127.0.0.1:5432:5432"
    environment:
      POSTGRES_DB: bastionclaw
      POSTGRES_USER: bastionclaw
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:-bastionclaw}
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U bastionclaw"]
      interval: 5s
      timeout: 3s
      retries: 5

  bastionclaw:
    profiles: ["app"]
    image: us-central1-docker.pkg.dev/gen-lang-client-0263867259/bastionclaw/agent:latest
    restart: unless-stopped
    depends_on:
      postgres:
        condition: service_healthy
    ports:
      - "0.0.0.0:3000:3000"
    env_file:
      - .env
    environment:
      DATABASE_URL: postgres://bastionclaw:${POSTGRES_PASSWORD:-bastionclaw}@postgres:5432/bastionclaw
      GATEWAY_ENABLED: "true"
      GATEWAY_HOST: "0.0.0.0"
      GATEWAY_PORT: "3000"
      CLI_ENABLED: "false"
      ONBOARD_COMPLETED: "true"
      BASTIONCLAW_IN_DOCKER: "true"
      SANDBOX_ENABLED: "false"
    volumes:
      - bastionclaw_data:/home/bastionclaw/.bastionclaw
      - t3n_mcp_socket:/var/run/t3n-mcp

  t3n-mcp-sidecar:
    profiles: ["app"]
    user: "0:0"
    image: us-central1-docker.pkg.dev/gen-lang-client-0263867259/bastionclaw/t3n-mcp-sidecar:latest
    restart: unless-stopped
    environment:
      T3N_SDK_ENV: ${T3N_MCP_ENV:-staging}
      T3N_MCP_RPC_URL: ${T3N_MCP_RPC_URL:-}
      T3N_MCP_DASHBOARD_URL: ${T3N_MCP_DASHBOARD_URL:-}
      PRIVATE_KEY: ${T3N_MCP_PRIVATE_KEY:-}
      MCP_SOCKET_PATH: /var/run/t3n-mcp/t3n-mcp.sock
      T3N_PROJECT_DIR: /app/node_modules/@terminal-3/t3n-mcp
    volumes:
      - t3n_mcp_socket:/var/run/t3n-mcp

volumes:
  pgdata:
  bastionclaw_data:
  t3n_mcp_socket:
COMPOSE

echo "==> [4/5] Installing bastionclaw.service"
cat > /etc/systemd/system/bastionclaw.service << 'SERVICE'
[Unit]
Description=BastionClaw AI Assistant
After=docker.service network-online.target
Requires=docker.service
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/bastionclaw
ExecStartPre=/usr/bin/docker compose --profile app pull
ExecStart=/usr/bin/docker compose --profile app up --remove-orphans
ExecStop=/usr/bin/docker compose --profile app down
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=bastionclaw

[Install]
WantedBy=multi-user.target
SERVICE
systemctl daemon-reload

echo "==> [5/6] Seeding t3n-mcp server config"
mkdir -p /home/bastionclaw/.bastionclaw 2>/dev/null || true
# Pre-register t3n-mcp as a Unix socket MCP server so it appears in the UI on
# first boot without a manual `bastionclaw mcp add` step.
# The file lives inside the bastionclaw_data volume; write it there now so it's
# present before the agent container starts.
VOLUME_PATH=$(docker volume inspect bastionclaw_bastionclaw_data --format '{{.Mountpoint}}' 2>/dev/null || true)
if [ -n "$VOLUME_PATH" ]; then
  cat > "${VOLUME_PATH}/mcp-servers.json" << 'MCP'
{
  "schema_version": 1,
  "servers": [
    {
      "name": "t3n-mcp",
      "url": "",
      "transport": { "transport": "unix", "socket_path": "/var/run/t3n-mcp/t3n-mcp.sock" },
      "enabled": true,
      "description": "Trinity MCP — on-chain actions via the t3n sidecar"
    }
  ]
}
MCP
  echo "     wrote mcp-servers.json to volume"
else
  echo "     WARNING: bastionclaw_data volume not found yet, skipping mcp seed (run after first compose up)"
fi

echo "==> [6/6] Pre-pulling images"
docker pull "${IMAGE_PREFIX}/agent:latest"
docker pull "${IMAGE_PREFIX}/t3n-mcp-sidecar:latest"

echo ""
echo "==> Bootstrap complete."
echo "    Create /opt/bastionclaw/.env then run:"
echo "      systemctl enable bastionclaw && systemctl start bastionclaw"
