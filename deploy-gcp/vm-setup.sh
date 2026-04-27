#!/usr/bin/env bash
# VM bootstrap for BastionClaw on GCP Compute Engine (Debian 12).
#
# Copy this directory and docker-compose.yml to the VM first, then run:
#   sudo bash /tmp/deploy/vm-setup.sh
#
# The script expects:
#   /tmp/deploy/              — contents of deploy-gcp/
#   /tmp/docker-compose.yml   — repo-root docker-compose.yml

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "ERROR: run as root: sudo bash vm-setup.sh"
  exit 1
fi

REGION="${REGION:-us-central1}"
PROJECT="${PROJECT:-gen-lang-client-0263867259}"
REPO="bastionclaw"
IMAGE_PREFIX="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}"

# ── Docker (official repo — Debian 12 default repos lack docker-compose-plugin)
echo "==> Installing Docker"
apt-get update -qq
apt-get install -y --no-install-recommends ca-certificates curl gnupg
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/debian/gpg \
  -o /etc/apt/keyrings/docker.asc
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

# ── Artifact Registry auth (uses the attached VM service account) ─────────────
echo "==> Configuring Artifact Registry auth"
# Install gcloud CLI if not present (Debian 12 base images may not include it)
if ! command -v gcloud &>/dev/null; then
  apt-get install -y --no-install-recommends apt-transport-https ca-certificates gnupg curl
  curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg \
    | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg
  echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" \
    > /etc/apt/sources.list.d/google-cloud-sdk.list
  apt-get update -qq
  apt-get install -y google-cloud-cli
fi

gcloud auth configure-docker "${REGION}-docker.pkg.dev" --quiet

# Pre-pull images so the first start is fast (the service does this too, but
# doing it here surfaces auth problems before systemd gets involved).
echo "==> Pre-pulling images"
docker pull "${IMAGE_PREFIX}/agent:latest"
docker pull "${IMAGE_PREFIX}/t3n-mcp-sidecar:latest"

# ── App directory ─────────────────────────────────────────────────────────────
echo "==> Setting up /opt/bastionclaw"
mkdir -p /opt/bastionclaw
chmod 700 /opt/bastionclaw

cp /tmp/docker-compose.yml /opt/bastionclaw/docker-compose.yml

# Rewrite image references so compose uses the Artifact Registry images instead
# of building from source (the VM has no source tree).
sed -i \
  "s|build:.*||g;
   /context:/d;
   /dockerfile:/d;
   /target:/d;
   s|image: bastionclaw.*|image: ${IMAGE_PREFIX}/agent:latest|g" \
  /opt/bastionclaw/docker-compose.yml

# ── Environment file ──────────────────────────────────────────────────────────
if [ ! -f /opt/bastionclaw/.env ]; then
  echo ""
  echo "WARNING: /opt/bastionclaw/.env does not exist."
  echo "Create it with your secrets before starting the service."
  echo "See deploy-gcp/env.example for the required variables."
  echo ""
  echo "Once .env is in place, run:"
  echo "  systemctl enable bastionclaw && systemctl start bastionclaw"
else
  chmod 600 /opt/bastionclaw/.env
fi

# ── Systemd service ───────────────────────────────────────────────────────────
echo "==> Installing bastionclaw.service"
cp /tmp/deploy/bastionclaw.service /etc/systemd/system/bastionclaw.service
systemctl daemon-reload

if [ -f /opt/bastionclaw/.env ]; then
  echo "==> Starting BastionClaw"
  systemctl enable bastionclaw
  systemctl start bastionclaw
fi

echo ""
echo "==> Bootstrap complete"
echo ""
echo "    Verify with:"
echo "      systemctl status bastionclaw"
echo "      docker logs bastion-claw-bastionclaw-1"
