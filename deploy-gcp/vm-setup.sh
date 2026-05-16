#!/usr/bin/env bash
# VM bootstrap for T3Claw on GCP Compute Engine (Debian 12).
#
# Copy deploy-gcp/ to the VM first, then run:
#   sudo bash /var/tmp/deploy/vm-setup.sh
#
# The script expects:
#   /var/tmp/deploy/                          — contents of deploy-gcp/
#   /var/tmp/deploy/docker-compose.staging.yml — installed as /opt/t3claw/docker-compose.yml

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "ERROR: run as root: sudo bash vm-setup.sh"
  exit 1
fi

REGION="${REGION:-us-central1}"
PROJECT="${PROJECT:-gen-lang-client-0263867259}"
REPO="t3claw"
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

# Pre-pull the agent image so the first start is fast and any AR auth problems
# surface here rather than inside systemd.
echo "==> Pre-pulling agent image"
docker pull "${IMAGE_PREFIX}/agent:latest"

# ── App directory ─────────────────────────────────────────────────────────────
echo "==> Setting up /opt/t3claw"
mkdir -p /opt/t3claw
chmod 700 /opt/t3claw

# Install the staging-specific compose file directly. The repo-root
# docker-compose.yml is for local development (build from source, 127.0.0.1
# binds, sidecar contexts) and cannot be safely rewritten in place; the
# staging file is the dedicated VM variant.
install -m 644 /var/tmp/deploy/docker-compose.staging.yml /opt/t3claw/docker-compose.yml

# ── Systemd service ───────────────────────────────────────────────────────────
# The service fetches /opt/t3claw/.env from Secret Manager (t3claw-staging-env)
# on every start via ExecStartPre — no manual .env file needed on the VM.
echo "==> Installing t3claw.service"
cp /var/tmp/deploy/t3claw.service /etc/systemd/system/t3claw.service
systemctl daemon-reload

echo "==> Waiting for t3claw-staging-env secret to have an enabled version..."
_secret_ready=0
for _i in $(seq 1 24); do
  if gcloud secrets versions list t3claw-staging-env \
       --filter="state=ENABLED" --limit=1 --format="value(name)" 2>/dev/null \
     | grep -q .; then
    _secret_ready=1
    break
  fi
  echo "    attempt ${_i}/24 — no enabled version yet, retrying in 10 s..."
  sleep 10
done
if [ "${_secret_ready}" -eq 0 ]; then
  echo "ERROR: timed out after 240 s waiting for an enabled version of t3claw-staging-env"
  exit 1
fi

echo "==> Starting T3Claw"
systemctl enable t3claw
systemctl start t3claw

echo ""
echo "==> Bootstrap complete"
echo ""
echo "    Verify with:"
echo "      systemctl status t3claw"
echo "      docker logs t3claw-t3claw-1"
