#!/usr/bin/env bash
# GCP infrastructure provisioning for T3Claw.
#
# Run once from your workstation with gcloud authenticated:
#   gcloud auth login
#   bash deploy-gcp/gcp-provision.sh                  # staging (default)
#   T3ENV=testnet bash deploy-gcp/gcp-provision.sh    # testnet
#
# Idempotent — safe to re-run.
#
# Flags:
#   T3ENV=staging|testnet   environment to provision (default: staging)
#   SKIP_BUILD=1            skip image build/push entirely
#   NETWORK=name            override VPC name (default: openclaw-vpc)

set -euo pipefail

# ── Environment selection ─────────────────────────────────────────────────────
T3ENV="${T3ENV:-staging}"
if [ "${T3ENV}" != "staging" ] && [ "${T3ENV}" != "testnet" ]; then
  echo "ERROR: T3ENV must be 'staging' or 'testnet' (got '${T3ENV}')"
  exit 1
fi

# ── Shared config ─────────────────────────────────────────────────────────────
PROJECT="${PROJECT:-gen-lang-client-0263867259}"
REGION="${REGION:-us-central1}"
ZONE="${ZONE:-asia-southeast1-a}"
NETWORK="${NETWORK:-openclaw-vpc}"
REPO="t3claw"
SA_NAME="t3claw-vm"
SA_EMAIL="${SA_NAME}@${PROJECT}.iam.gserviceaccount.com"
IMAGE_PREFIX="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ── Per-environment resource names ────────────────────────────────────────────
# staging keeps the legacy LB resource names already provisioned (no -staging suffix).
# testnet uses a consistent t3claw-testnet-* prefix throughout.
if [ "${T3ENV}" = "staging" ]; then
  VM_NAME="t3claw-staging"
  STATIC_IP_NAME="t3claw-staging-ip"
  HC_NAME="t3claw-health"
  IG_NAME="t3claw-staging-ig"
  BACKEND_NAME="t3claw-backend"
  URL_MAP_NAME="t3claw-urlmap"
  CERT_NAME="t3claw-cert"
  HTTPS_PROXY_NAME="t3claw-https-proxy"
  HTTPS_RULE_NAME="t3claw-https-rule"
  SECRET_NAME="t3claw-staging-env"
  DNS_ZONE="claw-dns-staging"
  DNS_NAME="t3claw.agent.staging.gc.terminal3.io"
  COMPOSE_FILE="docker-compose.staging.yml"
  IMAGE_TAG="latest"
else
  VM_NAME="t3claw-${T3ENV}"
  STATIC_IP_NAME="t3claw-${T3ENV}-ip"
  HC_NAME="t3claw-${T3ENV}-health"
  IG_NAME="t3claw-${T3ENV}-ig"
  BACKEND_NAME="t3claw-${T3ENV}-backend"
  URL_MAP_NAME="t3claw-${T3ENV}-urlmap"
  CERT_NAME="t3claw-${T3ENV}-cert-prod"
  HTTPS_PROXY_NAME="t3claw-${T3ENV}-https-proxy"
  HTTPS_RULE_NAME="t3claw-${T3ENV}-https-rule"
  SECRET_NAME="t3claw-${T3ENV}-env"
  DNS_ZONE="claw-dns-prod"
  DNS_NAME="t3claw-${T3ENV}.agent.prod.gc.terminal3.io"
  COMPOSE_FILE="docker-compose.${T3ENV}.yml"
  IMAGE_TAG="${T3ENV}"
fi

echo "==> Environment : ${T3ENV}"
echo "==> Project     : ${PROJECT}"
echo "==> Region/Zone : ${REGION} / ${ZONE}"
echo "==> VM          : ${VM_NAME}"
echo "==> Endpoint    : https://${DNS_NAME}"
echo ""

# ── Phase 1: Artifact Registry + images ───────────────────────────────────────
echo "==> [1/5] Artifact Registry"

if gcloud artifacts repositories describe "${REPO}" \
    --location="${REGION}" --project="${PROJECT}" &>/dev/null; then
  echo "     repo '${REPO}' already exists"
else
  gcloud artifacts repositories create "${REPO}" \
    --repository-format=docker \
    --location="${REGION}" \
    --project="${PROJECT}"
fi

gcloud auth configure-docker "${REGION}-docker.pkg.dev" --quiet

if [ "${SKIP_BUILD:-0}" = "1" ]; then
  echo "     SKIP_BUILD=1 — skipping image build/push"
elif [ "${T3ENV}" = "testnet" ]; then
  # Testnet promotes the current staging image rather than building from source.
  # Override with SKIP_BUILD=0 to force a fresh build instead.
  # Re-tag directly in Artifact Registry — no local pull/push needed.
  echo "     Promoting staging images → testnet tag..."
  gcloud artifacts docker tags add \
    "${IMAGE_PREFIX}/agent:latest" \
    "${IMAGE_PREFIX}/agent:testnet" \
    --project="${PROJECT}"
  gcloud artifacts docker tags add \
    "${IMAGE_PREFIX}/t3n-mcp-sidecar:latest" \
    "${IMAGE_PREFIX}/t3n-mcp-sidecar:testnet" \
    --project="${PROJECT}"
else
  echo "     Building agent image (target: runtime-staging)..."
  docker build --platform linux/amd64 --target runtime-staging \
    -t "${IMAGE_PREFIX}/agent:latest" \
    "${REPO_ROOT}"

  echo "     Building t3n-mcp-sidecar image..."
  docker build --platform linux/amd64 \
    -f "${REPO_ROOT}/docker/t3n-mcp-sidecar.Dockerfile" \
    --build-context trinity_client="${REPO_ROOT}/../trinity/client" \
    -t "${IMAGE_PREFIX}/t3n-mcp-sidecar:latest" \
    "${REPO_ROOT}"

  docker push "${IMAGE_PREFIX}/agent:latest"
  docker push "${IMAGE_PREFIX}/t3n-mcp-sidecar:latest"
fi

# ── Phase 2: Service Account + VM ─────────────────────────────────────────────
echo "==> [2/5] Service account + VM"

# VM service account is shared across environments.
if gcloud iam service-accounts describe "${SA_EMAIL}" \
    --project="${PROJECT}" &>/dev/null; then
  echo "     service account '${SA_NAME}' already exists"
else
  gcloud iam service-accounts create "${SA_NAME}" \
    --display-name="T3Claw VM" \
    --project="${PROJECT}"
fi

gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/artifactregistry.reader" \
  --condition=None --quiet

gcloud services enable secretmanager.googleapis.com --project="${PROJECT}" --quiet

if gcloud secrets describe "${SECRET_NAME}" --project="${PROJECT}" &>/dev/null; then
  echo "     secret '${SECRET_NAME}' already exists"
else
  gcloud secrets create "${SECRET_NAME}" \
    --replication-policy=automatic \
    --project="${PROJECT}"
  echo "     secret created (no versions yet) — upload your .env before starting:"
  echo "     gcloud secrets versions add ${SECRET_NAME} --data-file=your.env --project=${PROJECT}"
fi

gcloud secrets add-iam-policy-binding "${SECRET_NAME}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/secretmanager.secretAccessor" \
  --project="${PROJECT}" --quiet

# Firewall: LB health check ranges → port 3000 on t3claw-tagged VMs (shared).
if gcloud compute firewall-rules describe allow-t3claw-lb \
    --project="${PROJECT}" &>/dev/null; then
  echo "     firewall 'allow-t3claw-lb' already exists"
else
  gcloud compute firewall-rules create allow-t3claw-lb \
    --network="${NETWORK}" \
    --allow=tcp:3000 \
    --source-ranges=130.211.0.0/22,35.191.0.0/16 \
    --target-tags=t3claw \
    --project="${PROJECT}"
fi

# Firewall: IAP SSH (shared across all VMs on openclaw-vpc).
if gcloud compute firewall-rules describe allow-ssh-iap \
    --project="${PROJECT}" &>/dev/null; then
  echo "     firewall 'allow-ssh-iap' already exists"
else
  gcloud compute firewall-rules create allow-ssh-iap \
    --network="${NETWORK}" \
    --allow=tcp:22 \
    --source-ranges=35.235.240.0/20 \
    --project="${PROJECT}"
fi

if gcloud compute instances describe "${VM_NAME}" \
    --zone="${ZONE}" --project="${PROJECT}" &>/dev/null; then
  echo "     VM '${VM_NAME}' already exists"
else
  gcloud compute instances create "${VM_NAME}" \
    --project="${PROJECT}" \
    --zone="${ZONE}" \
    --machine-type=e2-standard-2 \
    --image-family=debian-12 \
    --image-project=debian-cloud \
    --boot-disk-size=30GB \
    --service-account="${SA_EMAIL}" \
    --scopes=cloud-platform \
    --network="${NETWORK}" \
    --subnet="${NETWORK}" \
    --no-address \
    --tags=t3claw \
    --metadata="t3env=${T3ENV}"
fi

# ── Phase 3: Bootstrap VM ─────────────────────────────────────────────────────
echo "==> [3/5] VM bootstrap"

if [ "${SKIP_BOOTSTRAP:-0}" = "1" ]; then
  echo "     SKIP_BOOTSTRAP=1 — skipping VM bootstrap prompt"
else
  echo ""
  echo "     The VM has no public IP — all SSH/SCP must go via IAP tunnel."
  echo "     Run these in a second terminal, then come back and press Enter:"
  echo ""
  echo "       gcloud compute ssh ${VM_NAME} --zone=${ZONE} --project=${PROJECT} --tunnel-through-iap \\"
  echo "         -- 'sudo rm -rf /var/tmp/t3claw-deploy'"
  echo "       gcloud compute scp --recurse deploy-gcp/ ${VM_NAME}:/var/tmp/t3claw-deploy \\"
  echo "         --zone=${ZONE} --project=${PROJECT} --tunnel-through-iap"
  echo ""
  echo "       gcloud compute ssh ${VM_NAME} --zone=${ZONE} --project=${PROJECT} --tunnel-through-iap \\"
  echo "         -- sudo env T3ENV=${T3ENV} bash /var/tmp/t3claw-deploy/vm-setup.sh"
  echo ""
  echo "     Then upload the .env as the first Secret Manager version:"
  echo "       gcloud secrets versions add ${SECRET_NAME} --data-file=your.env --project=${PROJECT}"
  echo ""
  read -rp "     Press Enter once the VM is bootstrapped and the secret has an enabled version..."
fi

# ── Phase 4: HTTPS Load Balancer ──────────────────────────────────────────────
echo "==> [4/5] HTTPS load balancer"

if gcloud compute addresses describe "${STATIC_IP_NAME}" \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     static IP '${STATIC_IP_NAME}' already reserved"
else
  gcloud compute addresses create "${STATIC_IP_NAME}" \
    --global --project="${PROJECT}"
fi

LB_IP=$(gcloud compute addresses describe "${STATIC_IP_NAME}" \
  --global --project="${PROJECT}" --format="value(address)")
echo "     LB static IP: ${LB_IP}"

if gcloud compute health-checks describe "${HC_NAME}" \
    --project="${PROJECT}" &>/dev/null; then
  echo "     health check '${HC_NAME}' already exists"
else
  gcloud compute health-checks create http "${HC_NAME}" \
    --port=3000 \
    --request-path=/api/health \
    --check-interval=10s \
    --timeout=5s \
    --healthy-threshold=2 \
    --unhealthy-threshold=3 \
    --project="${PROJECT}"
fi

if gcloud compute instance-groups unmanaged describe "${IG_NAME}" \
    --zone="${ZONE}" --project="${PROJECT}" &>/dev/null; then
  echo "     instance group '${IG_NAME}' already exists"
else
  gcloud compute instance-groups unmanaged create "${IG_NAME}" \
    --zone="${ZONE}" --project="${PROJECT}"
  gcloud compute instance-groups unmanaged add-instances "${IG_NAME}" \
    --instances="${VM_NAME}" \
    --zone="${ZONE}" --project="${PROJECT}"
  gcloud compute instance-groups set-named-ports "${IG_NAME}" \
    --named-ports=http:3000 \
    --zone="${ZONE}" --project="${PROJECT}"
fi

if gcloud compute backend-services describe "${BACKEND_NAME}" \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     backend service '${BACKEND_NAME}' already exists"
else
  gcloud compute backend-services create "${BACKEND_NAME}" \
    --global \
    --protocol=HTTP \
    --port-name=http \
    --health-checks="${HC_NAME}" \
    --project="${PROJECT}"
  gcloud compute backend-services add-backend "${BACKEND_NAME}" \
    --global \
    --instance-group="${IG_NAME}" \
    --instance-group-zone="${ZONE}" \
    --project="${PROJECT}"
fi

if gcloud compute url-maps describe "${URL_MAP_NAME}" \
    --project="${PROJECT}" &>/dev/null; then
  echo "     URL map '${URL_MAP_NAME}' already exists"
else
  gcloud compute url-maps create "${URL_MAP_NAME}" \
    --default-service="${BACKEND_NAME}" \
    --project="${PROJECT}"
fi

if gcloud compute ssl-certificates describe "${CERT_NAME}" \
    --project="${PROJECT}" &>/dev/null; then
  echo "     SSL cert '${CERT_NAME}' already exists"
else
  gcloud compute ssl-certificates create "${CERT_NAME}" \
    --domains="${DNS_NAME}" --project="${PROJECT}"
fi

if gcloud compute target-https-proxies describe "${HTTPS_PROXY_NAME}" \
    --project="${PROJECT}" &>/dev/null; then
  echo "     HTTPS proxy '${HTTPS_PROXY_NAME}' already exists"
else
  gcloud compute target-https-proxies create "${HTTPS_PROXY_NAME}" \
    --url-map="${URL_MAP_NAME}" \
    --ssl-certificates="${CERT_NAME}" \
    --project="${PROJECT}"
fi

if gcloud compute forwarding-rules describe "${HTTPS_RULE_NAME}" \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     forwarding rule '${HTTPS_RULE_NAME}' already exists"
else
  gcloud compute forwarding-rules create "${HTTPS_RULE_NAME}" \
    --global \
    --target-https-proxy="${HTTPS_PROXY_NAME}" \
    --address="${STATIC_IP_NAME}" \
    --ports=443 \
    --project="${PROJECT}"
fi

# ── Phase 5: Cloud DNS ─────────────────────────────────────────────────────────
echo "==> [5/5] Cloud DNS"

if gcloud dns record-sets describe "${DNS_NAME}." \
    --zone="${DNS_ZONE}" --type=A --project="${PROJECT}" &>/dev/null; then
  echo "     A record exists, updating..."
  gcloud dns record-sets update "${DNS_NAME}." \
    --zone="${DNS_ZONE}" --type=A --ttl=300 --rrdatas="${LB_IP}" \
    --project="${PROJECT}"
else
  gcloud dns record-sets create "${DNS_NAME}." \
    --zone="${DNS_ZONE}" --type=A --ttl=300 --rrdatas="${LB_IP}" \
    --project="${PROJECT}"
fi

echo ""
echo "==> Done!"
echo ""
echo "    LB IP  : ${LB_IP}"
echo "    DNS    : ${DNS_NAME} → ${LB_IP}"
echo "    HTTPS  : https://${DNS_NAME}"
echo ""
echo "    Google-managed SSL cert provisions once DNS propagates (~15 min)."
echo "    Poll: gcloud compute ssl-certificates describe ${CERT_NAME} --project=${PROJECT}"
