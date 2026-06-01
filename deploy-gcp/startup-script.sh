#!/usr/bin/env bash
# Self-contained GCP startup script for T3Claw staging VM.
# Injected via (then reset to run on boot):
#   gcloud compute instances add-metadata t3claw-staging \
#     --zone=asia-southeast1-a --project=gen-lang-client-0263867259 \
#     --metadata-from-file startup-script=deploy-gcp/startup-script.sh
#   gcloud compute instances reset t3claw-staging \
#     --zone=asia-southeast1-a --project=gen-lang-client-0263867259
#
# Runs as root automatically on VM boot. Logs to /var/log/t3claw-startup.log
# and to the serial port (visible via: gcloud compute instances get-serial-port-output).

set -euo pipefail
exec > >(tee /var/log/t3claw-startup.log | logger -t t3claw-startup) 2>&1

PROJECT="gen-lang-client-0263867259"
REGION="us-central1"
REPO="t3claw"
IMAGE_PREFIX="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}"

echo "==> [1/7] Installing Docker (official repo)"
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

echo "==> [2/7] Installing gcloud CLI (required for AR auth and Secret Manager)"
if ! command -v gcloud &>/dev/null; then
  apt-get install -y --no-install-recommends apt-transport-https ca-certificates gnupg curl
  curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg \
    | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg
  echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" \
    > /etc/apt/sources.list.d/google-cloud-sdk.list
  apt-get update -qq
  apt-get install -y google-cloud-cli
fi

echo "==> [3/7] Configuring Artifact Registry auth"
gcloud auth configure-docker "${REGION}-docker.pkg.dev" --quiet

echo "==> [4/7] Setting up /opt/t3claw"
mkdir -p /opt/t3claw
chmod 700 /opt/t3claw

# Write docker-compose.yml — uses AR images, no local build needed
cat > /opt/t3claw/docker-compose.yml << 'COMPOSE'
services:
  postgres:
    image: pgvector/pgvector:pg16
    ports:
      - "127.0.0.1:5432:5432"
    environment:
      POSTGRES_DB: t3claw
      POSTGRES_USER: t3claw
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:-t3claw}
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U t3claw"]
      interval: 5s
      timeout: 3s
      retries: 5

  t3claw:
    profiles: ["app"]
    image: us-central1-docker.pkg.dev/gen-lang-client-0263867259/t3claw/agent:latest
    restart: unless-stopped
    depends_on:
      postgres:
        condition: service_healthy
    ports:
      - "0.0.0.0:3000:3000"
    env_file:
      - .env
    environment:
      DATABASE_URL: postgres://t3claw:${POSTGRES_PASSWORD:-t3claw}@postgres:5432/t3claw
      GATEWAY_ENABLED: "true"
      GATEWAY_HOST: "0.0.0.0"
      GATEWAY_PORT: "3000"
      CLI_ENABLED: "false"
      ONBOARD_COMPLETED: "true"
      T3CLAW_IN_DOCKER: "true"
      SANDBOX_ENABLED: "false"
      # Triggers bootstrap_t3n_mcp_server() at agent startup so the t3n-mcp
      # server is auto-registered in the DB under owner_id. Without this the
      # CLI still works (it falls back to the seeded mcp-servers.json on
      # disk) but the gateway settings API returns 404 and the web UI shows
      # nothing under MCP servers.
      T3N_MCP_SOCKET_PATH: /var/run/t3n-mcp/t3n-mcp.sock
    volumes:
      - t3claw_data:/home/t3claw/.t3claw
      - t3n_mcp_socket:/var/run/t3n-mcp

  t3n-mcp-sidecar:
    profiles: ["app"]
    user: "0:0"
    image: us-central1-docker.pkg.dev/gen-lang-client-0263867259/t3claw/t3n-mcp-sidecar:latest
    restart: unless-stopped
    environment:
      T3N_SDK_ENV: ${T3N_MCP_ENV:-staging}
      T3N_MCP_RPC_URL: ${T3N_MCP_RPC_URL:-}
      T3N_MCP_DASHBOARD_URL: ${T3N_MCP_DASHBOARD_URL:-}
      PRIVATE_KEY: ${T3N_MCP_PRIVATE_KEY:-}
      T3N_MCP_AGENT_SECRET_HEX: ${T3N_MCP_AGENT_SECRET_HEX:-}
      # Optional static headers (JSON string->string) the SDK attaches to every
      # /api/rpc request. Staging gates /api/rpc behind a Cloud Armor bypass
      # header, so set T3N_RPC_EXTRA_HEADERS={"x-staging-bypass":"…"} in .env.
      T3N_RPC_EXTRA_HEADERS: ${T3N_RPC_EXTRA_HEADERS:-}
      MCP_SOCKET_PATH: /var/run/t3n-mcp/t3n-mcp.sock
    volumes:
      - t3n_mcp_socket:/var/run/t3n-mcp

volumes:
  pgdata:
  t3claw_data:
  t3n_mcp_socket:
COMPOSE

echo "==> [5/7] Installing t3claw.service"
cat > /etc/systemd/system/t3claw.service << 'SERVICE'
[Unit]
Description=T3Claw AI Assistant
After=docker.service network-online.target
Requires=docker.service
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/t3claw
ExecStartPre=/bin/bash -c 'set -e; tmp=$(mktemp /opt/t3claw/.env.XXXXXX); chmod 600 "$tmp"; gcloud secrets versions access latest --secret=t3claw-staging-env --project=gen-lang-client-0263867259 > "$tmp"; mv "$tmp" /opt/t3claw/.env'
ExecStartPre=/usr/bin/docker compose --profile app pull
ExecStart=/usr/bin/docker compose --profile app up --remove-orphans
ExecStop=/usr/bin/docker compose --profile app down
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=t3claw

[Install]
WantedBy=multi-user.target
SERVICE
systemctl daemon-reload

echo "==> [6/7] Seeding t3n-mcp server config"
mkdir -p /home/t3claw/.t3claw 2>/dev/null || true
# Pre-register t3n-mcp as a Unix socket MCP server so it appears in the UI on
# first boot without a manual `t3claw mcp add` step.
# The file lives inside the t3claw_data volume; write it there now so it's
# present before the agent container starts.
VOLUME_PATH=$(docker volume inspect t3claw_t3claw_data --format '{{.Mountpoint}}' 2>/dev/null || true)
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
  echo "     WARNING: t3claw_data volume not found yet, skipping mcp seed (run after first compose up)"
fi

echo "==> [7/7] Pre-pulling images and starting service"
docker pull "${IMAGE_PREFIX}/agent:latest"
docker pull "${IMAGE_PREFIX}/t3n-mcp-sidecar:latest"

systemctl enable t3claw
if gcloud secrets versions list t3claw-staging-env \
     --filter="state=ENABLED" --limit=1 --format="value(name)" \
     --project="${PROJECT}" 2>/dev/null | grep -q .; then
  systemctl restart t3claw
  echo "==> Bootstrap complete — t3claw.service started"
else
  echo "==> Bootstrap complete — start manually after uploading t3claw-staging-env:"
  echo "      systemctl start t3claw"
fi
