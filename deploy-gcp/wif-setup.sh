#!/usr/bin/env bash
# One-time Workload Identity Federation setup for GitHub Actions CD.
#
# Run once from a machine authenticated as a project owner:
#   gcloud auth login
#   bash deploy-gcp/wif-setup.sh
#
# What this creates:
#   - Service account  t3claw-ci-deploy  (push images to AR, IAP SSH into VM)
#   - WIF pool         github-actions
#   - WIF OIDC provider  github-provider  (trusts tokens.actions.githubusercontent.com)
#   - IAM binding: Terminal-3/t3-claw tokens can impersonate t3claw-ci-deploy
#
# At the end the script prints the two values to add as GitHub secrets:
#   WIF_PROVIDER
#   WIF_SERVICE_ACCOUNT

set -euo pipefail

PROJECT="${PROJECT:-gen-lang-client-0263867259}"
GITHUB_REPO="Terminal-3/t3-claw"
SA_NAME="t3claw-ci-deploy"
SA_EMAIL="${SA_NAME}@${PROJECT}.iam.gserviceaccount.com"
POOL_NAME="github-actions"
PROVIDER_NAME="github-provider"

echo "==> Project : ${PROJECT}"
echo "==> Repo    : ${GITHUB_REPO}"
echo ""

# ── Service account ───────────────────────────────────────────────────────────
echo "==> [1/4] Service account"

if gcloud iam service-accounts describe "${SA_EMAIL}" \
    --project="${PROJECT}" &>/dev/null; then
  echo "     '${SA_EMAIL}' already exists"
else
  gcloud iam service-accounts create "${SA_NAME}" \
    --display-name="T3Claw CI Deploy" \
    --project="${PROJECT}"
  echo "     created — waiting for propagation..."
  sleep 15
fi

echo "     granting roles..."

# Push images to Artifact Registry
gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/artifactregistry.writer" \
  --quiet 2>/dev/null

# Create IAP tunnel to the VM
gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/iap.tunnelResourceAccessor" \
  --quiet 2>/dev/null

# Describe instances (required by gcloud compute ssh)
gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/compute.viewer" \
  --quiet 2>/dev/null

# SSH key injection: covers both OS Login (if enabled) and metadata-based fallback.
# compute.osLogin alone fails silently when OS Login isn't enabled at the project level.
gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/compute.osLogin" \
  --quiet 2>/dev/null

gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA_EMAIL}" \
  --role="roles/compute.instanceAdmin.v1" \
  --quiet 2>/dev/null

echo "     roles granted"

# ── Workload Identity pool ────────────────────────────────────────────────────
echo "==> [2/4] Workload Identity pool"

if gcloud iam workload-identity-pools describe "${POOL_NAME}" \
    --location=global --project="${PROJECT}" &>/dev/null; then
  echo "     pool '${POOL_NAME}' already exists"
else
  gcloud iam workload-identity-pools create "${POOL_NAME}" \
    --location=global \
    --display-name="GitHub Actions" \
    --project="${PROJECT}"
  echo "     created"
fi

# ── OIDC provider ─────────────────────────────────────────────────────────────
echo "==> [3/4] OIDC provider"

if gcloud iam workload-identity-pools providers describe "${PROVIDER_NAME}" \
    --workload-identity-pool="${POOL_NAME}" \
    --location=global --project="${PROJECT}" &>/dev/null; then
  echo "     provider '${PROVIDER_NAME}' already exists"
else
  gcloud iam workload-identity-pools providers create-oidc "${PROVIDER_NAME}" \
    --workload-identity-pool="${POOL_NAME}" \
    --location=global \
    --issuer-uri="https://token.actions.githubusercontent.com" \
    --attribute-mapping="google.subject=assertion.sub,attribute.repository=assertion.repository,attribute.actor=assertion.actor,attribute.ref=assertion.ref" \
    --attribute-condition="assertion.repository == '${GITHUB_REPO}'" \
    --project="${PROJECT}"
  echo "     created"
fi

# ── SA binding ────────────────────────────────────────────────────────────────
echo "==> [4/4] Binding SA to GitHub repo"

POOL_RESOURCE=$(gcloud iam workload-identity-pools describe "${POOL_NAME}" \
  --location=global --project="${PROJECT}" --format="value(name)")

MEMBER="principalSet://iam.googleapis.com/${POOL_RESOURCE}/attribute.repository/${GITHUB_REPO}"

gcloud iam service-accounts add-iam-policy-binding "${SA_EMAIL}" \
  --project="${PROJECT}" \
  --role="roles/iam.workloadIdentityUser" \
  --member="${MEMBER}" \
  --quiet 2>/dev/null

echo "     bound"

# ── Print GitHub secrets ───────────────────────────────────────────────────────
PROJECT_NUMBER=$(gcloud projects describe "${PROJECT}" --format="value(projectNumber)")
PROVIDER_RESOURCE="projects/${PROJECT_NUMBER}/locations/global/workloadIdentityPools/${POOL_NAME}/providers/${PROVIDER_NAME}"

echo ""
echo "══════════════════════════════════════════════════════════════════"
echo "  Add these two secrets to GitHub → Settings → Secrets → Actions"
echo "══════════════════════════════════════════════════════════════════"
echo ""
echo "  WIF_PROVIDER"
echo "  ${PROVIDER_RESOURCE}"
echo ""
echo "  WIF_SERVICE_ACCOUNT"
echo "  ${SA_EMAIL}"
echo ""
echo "══════════════════════════════════════════════════════════════════"
echo ""
echo "Done. The staging-gcp.yml workflow is ready to use on the next push to staging."
