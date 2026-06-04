#!/usr/bin/env bash
# VM bootstrap for T3Claw on GCP Compute Engine (Debian 12).
#
# Copy deploy-gcp/ to the VM first, then run:
#   gcloud compute ssh VM --zone=... --project=... --tunnel-through-iap \
#     -- T3ENV=staging sudo -E bash /var/tmp/deploy/vm-setup.sh
#
# T3ENV defaults to "staging". Pass T3ENV=testnet for the testnet VM.
#
# Expected files under /var/tmp/deploy/:
#   docker-compose.staging.yml  or  docker-compose.testnet.yml
#   t3claw.service

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "ERROR: run as root: sudo -E bash vm-setup.sh"
  exit 1
fi

T3ENV="${T3ENV:-staging}"
REGION="${REGION:-us-central1}"
PROJECT="${PROJECT:-gen-lang-client-0263867259}"
REPO="t3claw"
IMAGE_PREFIX="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}"
SECRET_NAME="t3claw-${T3ENV}-env"
COMPOSE_SRC="/var/tmp/deploy/docker-compose.${T3ENV}.yml"
IMAGE_TAG="latest"
if [ "${T3ENV}" != "staging" ]; then IMAGE_TAG="${T3ENV}"; fi

echo "==> T3ENV   : ${T3ENV}"
echo "==> Project : ${PROJECT}"
echo "==> Secret  : ${SECRET_NAME}"

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
echo "deb [arch=${ARCH} signed-by=/etc/apt/keyrings/docker.asc] \
https://download.docker.com/linux/debian ${CODENAME} stable" \
  > /etc/apt/sources.list.d/docker.list
apt-get update -qq
apt-get install -y --no-install-recommends \
  docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
systemctl enable docker
systemctl start docker

# ── Artifact Registry auth ────────────────────────────────────────────────────
echo "==> Configuring Artifact Registry auth"
if ! command -v gcloud &>/dev/null; then
  apt-get install -y --no-install-recommends apt-transport-https ca-certificates gnupg curl
  curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg \
    | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg
  echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] \
https://packages.cloud.google.com/apt cloud-sdk main" \
    > /etc/apt/sources.list.d/google-cloud-sdk.list
  apt-get update -qq
  apt-get install -y google-cloud-cli
fi
gcloud auth configure-docker "${REGION}-docker.pkg.dev" --quiet

echo "==> Pre-pulling agent image (tag: ${IMAGE_TAG})"
docker pull "${IMAGE_PREFIX}/agent:${IMAGE_TAG}"

# ── App directory ─────────────────────────────────────────────────────────────
echo "==> Setting up /opt/t3claw"
mkdir -p /opt/t3claw
chmod 700 /opt/t3claw

if [ ! -f "${COMPOSE_SRC}" ]; then
  echo "ERROR: compose file not found: ${COMPOSE_SRC}"
  echo "       Re-run: gcloud compute scp --recurse deploy-gcp/ VM:/var/tmp/deploy ..."
  exit 1
fi
install -m 644 "${COMPOSE_SRC}" /opt/t3claw/docker-compose.yml

# ── Systemd service ───────────────────────────────────────────────────────────
echo "==> Installing t3claw.service and fetch-env.sh"
install -m 755 /var/tmp/t3claw-deploy/fetch-env.sh /opt/t3claw/fetch-env.sh
install -m 644 /var/tmp/t3claw-deploy/t3claw.service /etc/systemd/system/t3claw.service
systemctl daemon-reload

# ── Wait for secret, then start ───────────────────────────────────────────────
echo "==> Waiting for ${SECRET_NAME} to be accessible..."
_ready=0
for _i in $(seq 1 24); do
  if gcloud secrets versions access latest --secret="${SECRET_NAME}" \
       --project="${PROJECT}" &>/dev/null; then
    _ready=1
    break
  fi
  echo "    attempt ${_i}/24 — not accessible yet, retrying in 10 s..."
  sleep 10
done
if [ "${_ready}" -eq 0 ]; then
  echo "ERROR: timed out waiting for ${SECRET_NAME} to be accessible"
  echo "       Upload with: gcloud secrets versions add ${SECRET_NAME} --data-file=your.env --project=${PROJECT}"
  exit 1
fi

echo "==> Starting T3Claw"
systemctl enable t3claw
systemctl start t3claw

echo ""
echo "==> Bootstrap complete"
echo "    Verify: systemctl status t3claw"
echo "            docker logs t3claw-t3claw-1"
