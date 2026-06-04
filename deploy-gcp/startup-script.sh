#!/usr/bin/env bash
# Self-contained GCP startup script for T3Claw VMs.
# Used for the "hard reset" path (instances reset). Injected via:
#
#   gcloud compute instances add-metadata VM_NAME \
#     --zone=asia-southeast1-a --project=gen-lang-client-0263867259 \
#     --metadata-from-file startup-script=deploy-gcp/startup-script.sh
#   gcloud compute instances reset VM_NAME \
#     --zone=asia-southeast1-a --project=gen-lang-client-0263867259
#
# Runs as root automatically on VM boot. The VM must have a `t3env` metadata
# attribute (staging or testnet) — set by gcp-provision.sh at VM creation.
# Logs to /var/log/t3claw-startup.log and to the serial port.

set -euo pipefail
exec > >(tee /var/log/t3claw-startup.log | logger -t t3claw-startup) 2>&1

PROJECT="gen-lang-client-0263867259"
REGION="us-central1"
REPO="t3claw"
IMAGE_PREFIX="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}"

# Read environment from instance metadata (set by gcp-provision.sh).
T3ENV=$(curl -sf \
  "http://metadata.google.internal/computeMetadata/v1/instance/attributes/t3env" \
  -H "Metadata-Flavor: Google" 2>/dev/null || echo "staging")
SECRET_NAME="t3claw-${T3ENV}-env"
IMAGE_TAG="${T3ENV}"
# staging uses :latest; other envs use their own tag
if [ "${T3ENV}" = "staging" ]; then
  IMAGE_TAG="latest"
fi

echo "==> T3ENV  : ${T3ENV}"
echo "==> Secret : ${SECRET_NAME}"

echo "==> [1/6] Installing Docker (official repo)"
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

echo "==> [2/6] Installing gcloud CLI"
if ! command -v gcloud &>/dev/null; then
  apt-get install -y --no-install-recommends apt-transport-https ca-certificates gnupg curl
  curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg \
    | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg
  echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" \
    > /etc/apt/sources.list.d/google-cloud-sdk.list
  apt-get update -qq
  apt-get install -y google-cloud-cli
fi

echo "==> [3/6] Configuring Artifact Registry auth"
gcloud auth configure-docker "${REGION}-docker.pkg.dev" --quiet

echo "==> [4/6] Setting up /opt/t3claw"
mkdir -p /opt/t3claw
chmod 700 /opt/t3claw

# Write docker-compose.yml — parametrized by IMAGE_TAG from metadata
cat > /opt/t3claw/docker-compose.yml << COMPOSE
services:
  postgres:
    image: pgvector/pgvector:pg16
    profiles: ["app"]
    restart: unless-stopped
    environment:
      POSTGRES_USER: t3claw
      POSTGRES_PASSWORD: \${POSTGRES_PASSWORD:?POSTGRES_PASSWORD must be set in .env}
      POSTGRES_DB: t3claw
    volumes:
      - postgres-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U t3claw"]
      interval: 5s
      timeout: 3s
      retries: 5

  t3claw:
    profiles: ["app"]
    image: ${IMAGE_PREFIX}/agent:${IMAGE_TAG}
    restart: unless-stopped
    depends_on:
      postgres:
        condition: service_healthy
    ports:
      - "0.0.0.0:3000:3000"
    env_file:
      - .env
    environment:
      DATABASE_URL: postgres://t3claw:\${POSTGRES_PASSWORD}@postgres:5432/t3claw
      GATEWAY_ENABLED: "true"
      GATEWAY_HOST: "0.0.0.0"
      GATEWAY_PORT: "3000"
      CLI_ENABLED: "false"
      ONBOARD_COMPLETED: "true"
      T3CLAW_IN_DOCKER: "true"
      SANDBOX_ENABLED: \${SANDBOX_ENABLED:-false}
      SANDBOX_POLICY: \${SANDBOX_POLICY:-readonly}
      T3N_MCP_SOCKET_PATH: /var/run/t3n-mcp/t3n-mcp.sock
    volumes:
      - t3claw-data:/home/t3claw/.t3claw
      - t3n-mcp-socket:/var/run/t3n-mcp
      - /var/run/docker.sock:/var/run/docker.sock

  t3n-mcp-sidecar:
    profiles: ["app"]
    user: "0:0"
    image: ${IMAGE_PREFIX}/t3n-mcp-sidecar:${IMAGE_TAG}
    restart: unless-stopped
    environment:
      T3N_SDK_ENV: \${T3N_SDK_ENV:-${T3ENV}}
      T3N_MCP_RPC_URL: \${T3N_MCP_RPC_URL:-}
      T3N_MCP_DASHBOARD_URL: \${T3N_MCP_DASHBOARD_URL:-}
      PRIVATE_KEY: \${T3N_MCP_PRIVATE_KEY:-}
      T3N_MCP_AGENT_SECRET_HEX: \${T3N_MCP_AGENT_SECRET_HEX:-}
      T3N_RPC_EXTRA_HEADERS: \${T3N_RPC_EXTRA_HEADERS:-}
      MCP_SOCKET_PATH: /var/run/t3n-mcp/t3n-mcp.sock
    volumes:
      - t3n-mcp-socket:/var/run/t3n-mcp

volumes:
  postgres-data:
  t3claw-data:
  t3n-mcp-socket:
COMPOSE

echo "==> [5/6] Installing fetch-env.sh and t3claw.service"
# Use a helper script for the secret fetch so systemd doesn't expand shell variables.
cat > /opt/t3claw/fetch-env.sh << 'FETCHENV'
#!/bin/bash
set -e
PROJECT="gen-lang-client-0263867259"
T3ENV=$(curl -sf \
  "http://metadata.google.internal/computeMetadata/v1/instance/attributes/t3env" \
  -H "Metadata-Flavor: Google" 2>/dev/null || echo "staging")
SECRET="t3claw-${T3ENV}-env"
tmp=$(mktemp /opt/t3claw/.env.XXXXXX)
chmod 600 "$tmp"
gcloud secrets versions access latest --secret="${SECRET}" --project="${PROJECT}" > "$tmp"
mv "$tmp" /opt/t3claw/.env
FETCHENV
chmod 755 /opt/t3claw/fetch-env.sh

cat > /etc/systemd/system/t3claw.service << 'SERVICE'
[Unit]
Description=T3Claw AI Assistant
After=docker.service network-online.target
Requires=docker.service
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/t3claw
ExecStartPre=/opt/t3claw/fetch-env.sh
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

echo "==> [6/6] Pre-pulling images and starting service"
docker pull "${IMAGE_PREFIX}/agent:${IMAGE_TAG}"
docker pull "${IMAGE_PREFIX}/t3n-mcp-sidecar:${IMAGE_TAG}"
systemctl enable t3claw

if gcloud secrets versions list "${SECRET_NAME}" \
     --filter="state=ENABLED" --limit=1 --format="value(name)" \
     --project="${PROJECT}" 2>/dev/null | grep -q .; then
  systemctl restart t3claw
  echo "==> Bootstrap complete — t3claw.service started"
else
  echo "==> Bootstrap complete — upload the secret, then start manually:"
  echo "      gcloud secrets versions add ${SECRET_NAME} --data-file=your.env --project=${PROJECT}"
  echo "      systemctl start t3claw"
fi
