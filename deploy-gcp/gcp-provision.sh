#!/usr/bin/env bash
# GCP infrastructure provisioning for BastionClaw staging.
#
# Run once from your workstation with gcloud authenticated:
#   gcloud auth login
#   bash deploy-gcp/gcp-provision.sh
#
# What this script creates:
#   - Artifact Registry repo "bastionclaw" (us-central1)
#   - Builds and pushes agent + worker Docker images
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
REPO="bastionclaw"
IMAGE_PREFIX="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}"
VM_NAME="bastionclaw-staging"
SA_NAME="bastionclaw-vm"
SA_EMAIL="${SA_NAME}@${PROJECT}.iam.gserviceaccount.com"
DNS_ZONE="claw-dns-staging"
DNS_NAME="t3claw.agent.staging.gc.terminal3.io"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "==> Project : ${PROJECT}"
echo "==> Region  : ${REGION} / ${ZONE}"
echo "==> Images  : ${IMAGE_PREFIX}/{agent,worker}:latest"
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

echo "     Building agent image (target: runtime-staging) ..."
docker build --platform linux/amd64 --target runtime-staging \
  -t "${IMAGE_PREFIX}/agent:latest" \
  "${REPO_ROOT}"

echo "     Building worker image ..."
docker build --platform linux/amd64 \
  -f "${REPO_ROOT}/Dockerfile.worker" \
  -t "${IMAGE_PREFIX}/worker:latest" \
  "${REPO_ROOT}"

echo "     Pushing images ..."
docker push "${IMAGE_PREFIX}/agent:latest"
docker push "${IMAGE_PREFIX}/worker:latest"

# ── Phase 2: Service Account + VM ─────────────────────────────────────────────
echo "==> [2/5] Service account + VM"

if gcloud iam service-accounts describe "${SA_EMAIL}" \
    --project="${PROJECT}" &>/dev/null; then
  echo "     service account already exists, skipping create"
else
  gcloud iam service-accounts create "${SA_NAME}" \
    --display-name="BastionClaw VM" \
    --project="${PROJECT}"
fi

gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/artifactregistry.reader" \
  --condition=None \
  --quiet

# Firewall: allow LB health check IP ranges to reach port 3000 on tagged VMs.
# Google health check source ranges: 130.211.0.0/22, 35.191.0.0/16
if gcloud compute firewall-rules describe allow-bastionclaw-lb \
    --project="${PROJECT}" &>/dev/null; then
  echo "     firewall rule already exists, skipping create"
else
  gcloud compute firewall-rules create allow-bastionclaw-lb \
    --network=default \
    --allow=tcp:3000 \
    --source-ranges=130.211.0.0/22,35.191.0.0/16 \
    --target-tags=bastionclaw \
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
    --tags=bastionclaw
fi

# ── Phase 3: Copy files + bootstrap VM ────────────────────────────────────────
echo "==> [3/5] VM bootstrap"
echo ""
echo "     Copy deploy-gcp/ and docker-compose.yml to the VM, then run vm-setup.sh:"
echo ""
echo "       gcloud compute scp --recurse deploy-gcp/ ${VM_NAME}:/tmp/deploy --zone=${ZONE} --project=${PROJECT}"
echo "       gcloud compute scp docker-compose.yml ${VM_NAME}:/tmp/docker-compose.yml --zone=${ZONE} --project=${PROJECT}"
echo "       gcloud compute ssh ${VM_NAME} --zone=${ZONE} --project=${PROJECT} -- sudo bash /tmp/deploy/vm-setup.sh"
echo ""
echo "     Then create /opt/bastionclaw/.env on the VM (see deploy-gcp/env.example),"
echo "     and start the service:"
echo ""
echo "       gcloud compute ssh ${VM_NAME} --zone=${ZONE} --project=${PROJECT} -- \\"
echo "         'sudo systemctl enable bastionclaw && sudo systemctl start bastionclaw'"
echo ""
read -rp "     Press Enter once the VM is bootstrapped and .env is in place, or Ctrl-C to stop here..."

# ── Phase 4: Load Balancer ─────────────────────────────────────────────────────
echo "==> [4/5] HTTPS load balancer"

if gcloud compute addresses describe bastionclaw-staging-ip \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     static IP already reserved"
else
  gcloud compute addresses create bastionclaw-staging-ip \
    --global --project="${PROJECT}"
fi

LB_IP=$(gcloud compute addresses describe bastionclaw-staging-ip \
  --global --project="${PROJECT}" --format="value(address)")
echo "     LB static IP: ${LB_IP}"

if gcloud compute health-checks describe bastionclaw-health \
    --project="${PROJECT}" &>/dev/null; then
  echo "     health check already exists"
else
  gcloud compute health-checks create http bastionclaw-health \
    --port=3000 \
    --request-path=/api/health \
    --check-interval=10s \
    --timeout=5s \
    --healthy-threshold=2 \
    --unhealthy-threshold=3 \
    --project="${PROJECT}"
fi

if gcloud compute instance-groups unmanaged describe bastionclaw-staging-ig \
    --zone="${ZONE}" --project="${PROJECT}" &>/dev/null; then
  echo "     instance group already exists"
else
  gcloud compute instance-groups unmanaged create bastionclaw-staging-ig \
    --zone="${ZONE}" --project="${PROJECT}"
  gcloud compute instance-groups unmanaged add-instances bastionclaw-staging-ig \
    --instances="${VM_NAME}" \
    --zone="${ZONE}" --project="${PROJECT}"
  gcloud compute instance-groups set-named-ports bastionclaw-staging-ig \
    --named-ports=http:3000 \
    --zone="${ZONE}" --project="${PROJECT}"
fi

if gcloud compute backend-services describe bastionclaw-backend \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     backend service already exists"
else
  gcloud compute backend-services create bastionclaw-backend \
    --global \
    --protocol=HTTP \
    --port-name=http \
    --health-checks=bastionclaw-health \
    --project="${PROJECT}"
  gcloud compute backend-services add-backend bastionclaw-backend \
    --global \
    --instance-group=bastionclaw-staging-ig \
    --instance-group-zone="${ZONE}" \
    --project="${PROJECT}"
fi

if gcloud compute url-maps describe bastionclaw-urlmap \
    --project="${PROJECT}" &>/dev/null; then
  echo "     URL map already exists"
else
  gcloud compute url-maps create bastionclaw-urlmap \
    --default-service=bastionclaw-backend \
    --project="${PROJECT}"
fi

if gcloud compute ssl-certificates describe bastionclaw-cert \
    --project="${PROJECT}" &>/dev/null; then
  echo "     SSL cert already exists"
else
  gcloud compute ssl-certificates create bastionclaw-cert \
    --domains="${DNS_NAME}" --project="${PROJECT}"
fi

if gcloud compute target-https-proxies describe bastionclaw-https-proxy \
    --project="${PROJECT}" &>/dev/null; then
  echo "     HTTPS proxy already exists"
else
  gcloud compute target-https-proxies create bastionclaw-https-proxy \
    --url-map=bastionclaw-urlmap \
    --ssl-certificates=bastionclaw-cert \
    --project="${PROJECT}"
fi

if gcloud compute forwarding-rules describe bastionclaw-https-rule \
    --global --project="${PROJECT}" &>/dev/null; then
  echo "     forwarding rule already exists"
else
  gcloud compute forwarding-rules create bastionclaw-https-rule \
    --global \
    --target-https-proxy=bastionclaw-https-proxy \
    --address=bastionclaw-staging-ip \
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
echo "      gcloud compute ssl-certificates describe bastionclaw-cert --project=${PROJECT}"
