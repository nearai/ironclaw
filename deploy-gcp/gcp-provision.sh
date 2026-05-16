#!/usr/bin/env bash
# GCP infrastructure provisioning for T3Claw staging.
#
# Run once from your workstation with gcloud authenticated:
#   gcloud auth login
#   bash deploy-gcp/gcp-provision.sh
#
# What this script creates:
#   - Artifact Registry repo "t3claw" (us-central1)
#   - Builds and pushes agent + t3n-mcp-sidecar Docker images
#   - IAM service account for the VM with AR read access
#   - Firewall rule for LB health checks on port 3000
#   - Compute Engine VM (e2-standard-2, Debian 12, us-central1-a)
#   - Global static IP
#   - HTTP health check, unmanaged instance group, backend service
#   - URL map, Google-managed SSL cert, HTTPS proxy, forwarding rule
#   - Cloud DNS A record in zone claw-dns-staging

set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────────────
PROJECT="${PROJECT:-gen-lang-client-0263867259}"
REGION="${REGION:-us-central1}"
ZONE="${ZONE:-asia-southeast1-a}"
# This project has no `default` VPC; all VMs run on the shared openclaw-vpc.
NETWORK="${NETWORK:-openclaw-vpc}"
REPO="t3claw"
IMAGE_PREFIX="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}"
VM_NAME="t3claw-staging"
SA_NAME="t3claw-vm"
SA_EMAIL="${SA_NAME}@${PROJECT}.iam.gserviceaccount.com"
DNS_ZONE="claw-dns-staging"
DNS_NAME="t3claw.agent.staging.gc.terminal3.io"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "==> Project : ${PROJECT}"
echo "==> Region  : ${REGION} / ${ZONE}"
echo "==> Images  : ${IMAGE_PREFIX}/{agent,t3n-mcp-sidecar}:latest"
echo ""

# ── Phase 1: Artifact Registry ────────────────────────────────────────────────
echo "==> [1/5] Artifact Registry"

if gcloud artifacts repositories describe "${REPO}" \
    --location="${REGION}" --project="${PROJECT}" &>/dev/null; then
  echo "     repo '${REPO}' already exists, skipping create"
else
  gcloud artifacts repositories create "${REPO}" \
    --repository-format=docker \
    --location="${REGION}" \
    --project="${PROJECT}"
fi

gcloud auth configure-docker "${REGION}-docker.pkg.dev" --quiet

# Skip the (slow) image build+push entirely. Useful when you've already pushed
# images and only want to (re-)create downstream GCP infrastructure.
#   SKIP_BUILD=1 bash deploy-gcp/gcp-provision.sh
if [ "${SKIP_BUILD:-0}" = "1" ]; then
  echo "     SKIP_BUILD=1 set — skipping image build/push"
else
  echo "     Building agent image (target: runtime-staging) ..."
  docker build --platform linux/amd64 --target runtime-staging \
    -t "${IMAGE_PREFIX}/agent:latest" \
    "${REPO_ROOT}"

  echo "     Building t3n-mcp-sidecar image ..."
  docker build --platform linux/amd64 \
    -f "${REPO_ROOT}/docker/t3n-mcp-sidecar.Dockerfile" \
    --build-context trinity_client="${REPO_ROOT}/../trinity/client" \
    -t "${IMAGE_PREFIX}/t3n-mcp-sidecar:latest" \
    "${REPO_ROOT}"

  echo "     Pushing images ..."
  docker push "${IMAGE_PREFIX}/agent:latest"
  docker push "${IMAGE_PREFIX}/t3n-mcp-sidecar:latest"
fi

# ── Phase 2: Service Account + VM ─────────────────────────────────────────────
echo "==> [2/5] Service account + VM"

if gcloud iam service-accounts describe "${SA_EMAIL}" \
    --project="${PROJECT}" &>/dev/null; then
  echo "     service account already exists, skipping create"
else
  gcloud iam service-accounts create "${SA_NAME}" \
    --display-name="T3Claw VM" \
    --project="${PROJECT}"
fi

gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/artifactregistry.reader" \
  --condition=None \
  --quiet

# Secret Manager: enable API, grant VM SA read access, create secret placeholder
gcloud services enable secretmanager.googleapis.com --project="${PROJECT}" --quiet

if gcloud secrets describe t3claw-staging-env \
    --project="${PROJECT}" &>/dev/null; then
  echo "     secret 't3claw-staging-env' already exists"
else
  gcloud secrets create t3claw-staging-env \
    --replication-policy=automatic \
    --project="${PROJECT}"
  echo "     secret created (no versions yet) — add your .env as the first version:"
  echo "     gcloud secrets versions add t3claw-staging-env --data-file=.env --project=${PROJECT}"
fi

gcloud secrets add-iam-policy-binding t3claw-staging-env \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/secretmanager.secretAccessor" \
  --project="${PROJECT}" \
  --quiet

# Firewall: allow LB health check IP ranges to reach port 3000 on tagged VMs.
# Google health check source ranges: 130.211.0.0/22, 35.191.0.0/16
if gcloud compute firewall-rules describe allow-t3claw-lb \
    --project="${PROJECT}" &>/dev/null; then
  echo "     firewall rule already exists, skipping create"
else
  gcloud compute firewall-rules create allow-t3claw-lb \
    --network="${NETWORK}" \
    --allow=tcp:3000 \
    --source-ranges=130.211.0.0/22,35.191.0.0/16 \
    --target-tags=t3claw \
    --project="${PROJECT}"
fi

# Firewall: allow IAP SSH tunnelling (35.235.240.0/20 is the IAP range).
# Required because the VM is created with no public IP.
if gcloud compute firewall-rules describe allow-ssh-iap \
    --project="${PROJECT}" &>/dev/null; then
  echo "     IAP SSH firewall rule already exists, skipping create"
else
  gcloud compute firewall-rules create allow-ssh-iap \
    --network="${NETWORK}" \
    --allow=tcp:22 \
    --source-ranges=35.235.240.0/20 \
    --project="${PROJECT}"
fi

if gcloud compute instances describe "${VM_NAME}" \
    --zone="${ZONE}" --project="${PROJECT}" &>/dev/null; then
  echo "     VM '${VM_NAME}' already exists, skipping create"
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
    --tags=t3claw
fi

# ── Phase 3: Copy files + bootstrap VM ────────────────────────────────────────
echo "==> [3/5] VM bootstrap"
echo ""
echo "     The VM has no public IP — all SSH/SCP must go via IAP tunnel."
echo "     Copy deploy-gcp/ to the VM, then run vm-setup.sh:"
echo ""
echo "       gcloud compute scp --recurse deploy-gcp/ ${VM_NAME}:/var/tmp/deploy --zone=${ZONE} --project=${PROJECT} --tunnel-through-iap"
echo "       gcloud compute ssh ${VM_NAME} --zone=${ZONE} --project=${PROJECT} --tunnel-through-iap -- sudo bash /var/tmp/deploy/vm-setup.sh"
echo ""
echo "     Then create /opt/t3claw/.env on the VM (see deploy-gcp/env.example),"
echo "     and start the service:"
echo ""
echo "       gcloud compute ssh ${VM_NAME} --zone=${ZONE} --project=${PROJECT} --tunnel-through-iap -- \\"
echo "         'sudo systemctl enable t3claw && sudo systemctl start t3claw'"
echo ""
read -rp "     Press Enter once the VM is bootstrapped and .env is in place, or Ctrl-C to stop here..."

# ── Phase 4: Load Balancer ─────────────────────────────────────────────────────
echo "==> [4/5] HTTPS load balancer"

if gcloud compute addresses describe t3claw-staging-ip \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     static IP already reserved"
else
  gcloud compute addresses create t3claw-staging-ip \
    --global --project="${PROJECT}"
fi

LB_IP=$(gcloud compute addresses describe t3claw-staging-ip \
  --global --project="${PROJECT}" --format="value(address)")
echo "     LB static IP: ${LB_IP}"

if gcloud compute health-checks describe t3claw-health \
    --project="${PROJECT}" &>/dev/null; then
  echo "     health check already exists"
else
  gcloud compute health-checks create http t3claw-health \
    --port=3000 \
    --request-path=/api/health \
    --check-interval=10s \
    --timeout=5s \
    --healthy-threshold=2 \
    --unhealthy-threshold=3 \
    --project="${PROJECT}"
fi

if gcloud compute instance-groups unmanaged describe t3claw-staging-ig \
    --zone="${ZONE}" --project="${PROJECT}" &>/dev/null; then
  echo "     instance group already exists"
else
  gcloud compute instance-groups unmanaged create t3claw-staging-ig \
    --zone="${ZONE}" --project="${PROJECT}"
  gcloud compute instance-groups unmanaged add-instances t3claw-staging-ig \
    --instances="${VM_NAME}" \
    --zone="${ZONE}" --project="${PROJECT}"
  gcloud compute instance-groups set-named-ports t3claw-staging-ig \
    --named-ports=http:3000 \
    --zone="${ZONE}" --project="${PROJECT}"
fi

if gcloud compute backend-services describe t3claw-backend \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     backend service already exists"
else
  gcloud compute backend-services create t3claw-backend \
    --global \
    --protocol=HTTP \
    --port-name=http \
    --health-checks=t3claw-health \
    --project="${PROJECT}"
  gcloud compute backend-services add-backend t3claw-backend \
    --global \
    --instance-group=t3claw-staging-ig \
    --instance-group-zone="${ZONE}" \
    --project="${PROJECT}"
fi

if gcloud compute url-maps describe t3claw-urlmap \
    --project="${PROJECT}" &>/dev/null; then
  echo "     URL map already exists"
else
  gcloud compute url-maps create t3claw-urlmap \
    --default-service=t3claw-backend \
    --project="${PROJECT}"
fi

if gcloud compute ssl-certificates describe t3claw-cert \
    --project="${PROJECT}" &>/dev/null; then
  echo "     SSL cert already exists"
else
  gcloud compute ssl-certificates create t3claw-cert \
    --domains="${DNS_NAME}" --project="${PROJECT}"
fi

if gcloud compute target-https-proxies describe t3claw-https-proxy \
    --project="${PROJECT}" &>/dev/null; then
  echo "     HTTPS proxy already exists"
else
  gcloud compute target-https-proxies create t3claw-https-proxy \
    --url-map=t3claw-urlmap \
    --ssl-certificates=t3claw-cert \
    --project="${PROJECT}"
fi

if gcloud compute forwarding-rules describe t3claw-https-rule \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     forwarding rule already exists"
else
  gcloud compute forwarding-rules create t3claw-https-rule \
    --global \
    --target-https-proxy=t3claw-https-proxy \
    --address=t3claw-staging-ip \
    --ports=443 \
    --project="${PROJECT}"
fi

# ── Phase 5: Cloud DNS ─────────────────────────────────────────────────────────
echo "==> [5/5] Cloud DNS"

if gcloud dns record-sets describe "${DNS_NAME}." \
    --zone="${DNS_ZONE}" --type=A --project="${PROJECT}" &>/dev/null; then
  echo "     A record already exists, updating..."
  gcloud dns record-sets update "${DNS_NAME}." \
    --zone="${DNS_ZONE}" \
    --type=A \
    --ttl=300 \
    --rrdatas="${LB_IP}" \
    --project="${PROJECT}"
else
  gcloud dns record-sets create "${DNS_NAME}." \
    --zone="${DNS_ZONE}" \
    --type=A \
    --ttl=300 \
    --rrdatas="${LB_IP}" \
    --project="${PROJECT}"
fi

echo ""
echo "==> Done!"
echo ""
echo "    LB IP  : ${LB_IP}"
echo "    DNS    : ${DNS_NAME} -> ${LB_IP}"
echo "    HTTPS  : https://${DNS_NAME}"
echo ""
echo "    Google-managed SSL cert will provision once DNS propagates (~15 min)."
echo "    Check cert status:"
echo "      gcloud compute ssl-certificates describe t3claw-cert --project=${PROJECT}"
