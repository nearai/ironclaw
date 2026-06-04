# T3Claw — GCP Deployment

**Stack:** Compute Engine VM · Artifact Registry (us-central1) · Global HTTPS LB · Cloud DNS
**Project:** `gen-lang-client-0263867259` (openclaw)
**VPC:** `openclaw-vpc` (shared with bastionclaw, ironclaw)
**Zone:** `asia-southeast1-a`
**DNS zones:** `claw-dns-staging` (staging) · `claw-dns-prod` (testnet)

All VMs have no public IP. Shell access is via IAP tunnel only.

## Environments

| Environment | VM | Endpoint | Secret | Image tag |
|---|---|---|---|---|
| **staging** | `t3claw-staging` | `https://t3claw.agent.staging.gc.terminal3.io` | `t3claw-staging-env` | `:latest` |
| **testnet** | `t3claw-testnet` | `https://t3claw-testnet.agent.prod.gc.terminal3.io` | `t3claw-testnet-env` | `:testnet` |

- **staging** — auto-deploys on every push to the `staging` branch.
- **testnet** — manually promoted from staging via the `testnet-gcp.yml` workflow. Stable snapshot of a known-good staging build.

---

## Prerequisites

| Requirement | Verify |
|---|---|
| Project owner/editor on `gen-lang-client-0263867259` | `gcloud projects get-iam-policy gen-lang-client-0263867259` |
| `gcloud` installed and authenticated | `gcloud auth login` |
| Docker with Buildx | `docker buildx version` |
| `openssl` | `openssl version` |
| Cloud NAT on `openclaw-vpc` in `asia-southeast1` | `gcloud compute routers list --project=gen-lang-client-0263867259 --filter="network:openclaw-vpc AND region:asia-southeast1"` |
| Google account 2FA with **Google prompts** preferred | `https://myaccount.google.com/two-step-verification` |

> **MFA tip.** Every IAP-tunnelled SSH/SCP triggers MFA. Use Google prompts (not SMS). Batch related commands into a single SSH call to minimize prompts.

---

## First-time provisioning

The provision script orchestrates everything but pauses for a manual VM bootstrap step. You need **two terminals**.

### Terminal A — run the provision script

```bash
bash deploy-gcp/gcp-provision.sh                  # staging
T3ENV=testnet bash deploy-gcp/gcp-provision.sh    # testnet
```

The script is idempotent. It creates all GCP resources, then **pauses** at Phase 3. Leave terminal A at the prompt and switch to terminal B.

Flags:

```bash
SKIP_BUILD=1 bash deploy-gcp/gcp-provision.sh         # skip image build/push
NETWORK=my-vpc bash deploy-gcp/gcp-provision.sh       # override VPC name
```

For testnet, Phase 1 promotes `agent:latest` → `agent:testnet` instead of building from source.

### Terminal B — bootstrap the VM (steps 1–3)

**Step 1 — copy deploy-gcp/ to the VM**

```bash
VM=t3claw-staging   # or t3claw-testnet
# Clear first — scp nests the dir if the destination already exists
gcloud compute ssh ${VM} \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
  -- 'sudo rm -rf /var/tmp/t3claw-deploy'
gcloud compute scp --recurse deploy-gcp/ ${VM}:/var/tmp/t3claw-deploy \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap
```

**Step 2 — run vm-setup.sh on the VM**

```bash
gcloud compute ssh ${VM} \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
  -- sudo env T3ENV=staging bash /var/tmp/t3claw-deploy/vm-setup.sh
  # or T3ENV=testnet for the testnet VM
```

This installs Docker, configures AR auth, installs the compose file and systemd service.

**Step 3 — create the .env secret**

Generate the random values:

```bash
echo "POSTGRES_PASSWORD=$(openssl rand -hex 24)"
echo "GATEWAY_AUTH_TOKEN=$(openssl rand -hex 32)"
echo "SECRETS_MASTER_KEY=$(openssl rand -hex 32)"
```

> Save `SECRETS_MASTER_KEY` in a team password manager — losing it means losing all workspace secrets.

Fill in the template and upload as the first secret version:

```bash
SECRET=t3claw-staging-env   # or t3claw-testnet-env
ENV_FILE=$(mktemp)
cp deploy-gcp/env.example "$ENV_FILE"
${EDITOR:-vi} "$ENV_FILE"   # fill in the five CHANGE_ME values (see table below)
gcloud secrets versions add "${SECRET}" \
  --data-file="$ENV_FILE" --project=gen-lang-client-0263867259
rm "$ENV_FILE"
```

Required edits:

| Variable | Value |
|---|---|
| `DATABASE_URL` | replace `CHANGE_ME` with the same password as `POSTGRES_PASSWORD` |
| `POSTGRES_PASSWORD` | from `openssl rand -hex 24` |
| `ANTHROPIC_API_KEY` | a real key (`sk-ant-...`) |
| `GATEWAY_AUTH_TOKEN` | from `openssl rand -hex 32` |
| `SECRETS_MASTER_KEY` | from `openssl rand -hex 32` |

`DATABASE_URL` and `POSTGRES_PASSWORD` must use the same password.

### Terminal A — resume (step 4)

Press **Enter** in terminal A. The script provisions the HTTPS load balancer and DNS record (~1 min), then prints the LB IP.

### Step 5 — wait for SSL cert

```bash
gcloud compute ssl-certificates describe t3claw-cert \
  --project=gen-lang-client-0263867259 \
  --format="value(managed.status,managed.domainStatus)"
# For testnet: t3claw-testnet-cert
```

Google-managed certs provision once DNS propagates. Typically 10–20 minutes. HTTPS will fail with a cert error until `managed.status: ACTIVE`.

### Step 6 — smoke test

```bash
curl -fsS https://t3claw.agent.staging.gc.terminal3.io/api/health
# or: https://t3claw-testnet.agent.staging.gc.terminal3.io/api/health
```

Expected: `{"status":"healthy","channel":"gateway"}`

---

## CI/CD

### staging — auto-deploy on push

`.github/workflows/staging-gcp.yml` triggers on every push to `staging`. Builds and pushes both images to AR, then IAP-SSHes into the staging VM for a rolling update.

### testnet — manual promote

`.github/workflows/testnet-gcp.yml` is triggered manually via **Actions → Deploy to GCP Testnet → Run workflow**.

- **image_sha** (optional): git SHA whose AR image to promote. Leave blank to promote current `agent:latest`.
- Promotes `agent:<sha>` → `agent:testnet` (and same for `t3n-mcp-sidecar`), then deploys to `t3claw-testnet`.

### One-time WIF setup

Run once from a machine authenticated as project owner:

```bash
gcloud auth login
bash deploy-gcp/wif-setup.sh
```

Add the printed values as GitHub repository secrets (**Settings → Secrets → Actions**):

| Secret | Value |
|---|---|
| `WIF_PROVIDER` | Printed by `wif-setup.sh` |
| `WIF_SERVICE_ACCOUNT` | Printed by `wif-setup.sh` |
| `TRINITY_CHECKOUT_TOKEN` | PAT with `contents:read` on `Terminal-3/trinity` |

---

## Day-to-day operations

### SSH into a VM

```bash
gcloud compute ssh t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap
```

Once inside:

```bash
sudo docker ps -a
sudo journalctl -u t3claw -f
sudo docker logs -f t3claw-t3claw-1
```

### Update the .env secret

```bash
gcloud secrets versions add t3claw-staging-env \
  --data-file=/path/to/updated.env \
  --project=gen-lang-client-0263867259
# Then restart to pick it up:
gcloud compute ssh t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
  -- 'sudo systemctl restart t3claw'
```

### Hard reset (re-runs startup-script.sh from scratch)

Reinstalls Docker, regenerates `/opt/t3claw/`, starts the service. Use when something is fundamentally broken.

```bash
VM=t3claw-staging   # or t3claw-testnet
gcloud compute instances add-metadata ${VM} \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 \
  --metadata-from-file startup-script=deploy-gcp/startup-script.sh
gcloud compute instances reset ${VM} \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259
```

Watch boot progress:

```bash
gcloud compute instances get-serial-port-output ${VM} \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 | tail -80
```

---

## Reference

### Infrastructure resources

Shared resources (both environments use these):

| Resource | Name |
|---|---|
| VPC | `openclaw-vpc` |
| Cloud NAT | `openclaw-nat` on `openclaw-nat-router-sg` (asia-southeast1) |
| Artifact Registry | `t3claw` (us-central1) |
| VM service account | `t3claw-vm` |
| Firewall: LB health checks | `allow-t3claw-lb` — `tcp:3000` from `130.211.0.0/22, 35.191.0.0/16` → tag `t3claw` |
| Firewall: IAP SSH | `allow-ssh-iap` — `tcp:22` from `35.235.240.0/20` |
| DNS zone (staging) | `claw-dns-staging` — `agent.staging.gc.terminal3.io` |
| DNS zone (testnet) | `claw-dns-prod` — `agent.prod.gc.terminal3.io` |

Per-environment resources:

| Resource | staging | testnet |
|---|---|---|
| VM | `t3claw-staging` | `t3claw-testnet` |
| Static IP | `t3claw-staging-ip` | `t3claw-testnet-ip` |
| Health check | `t3claw-health` | `t3claw-testnet-health` |
| Instance group | `t3claw-staging-ig` | `t3claw-testnet-ig` |
| Backend service | `t3claw-backend` | `t3claw-testnet-backend` |
| URL map | `t3claw-urlmap` | `t3claw-testnet-urlmap` |
| SSL cert | `t3claw-cert` | `t3claw-testnet-cert` |
| HTTPS proxy | `t3claw-https-proxy` | `t3claw-testnet-https-proxy` |
| Forwarding rule | `t3claw-https-rule` | `t3claw-testnet-https-rule` |
| DNS A record | `t3claw.agent.staging...` | `t3claw-testnet.agent.staging...` |
| Secret Manager | `t3claw-staging-env` | `t3claw-testnet-env` |

> Staging LB resource names lack the `-staging` suffix for historical reasons — they were created before the testnet environment existed.

### Files in this directory

| File | Role |
|---|---|
| `gcp-provision.sh` | Provisions all GCP resources. Parametrized via `T3ENV`. |
| `wif-setup.sh` | One-time Workload Identity Federation setup for GitHub Actions. |
| `vm-setup.sh` | Bootstraps the VM (Docker, AR auth, compose, service). Parametrized via `T3ENV`. |
| `startup-script.sh` | Self-contained boot-time bootstrap used by `instances reset`. Reads `T3ENV` from VM metadata. |
| `docker-compose.staging.yml` | Compose file for the staging VM (`:latest` images). |
| `docker-compose.testnet.yml` | Compose file for the testnet VM (`:testnet` images). |
| `t3claw.service` | systemd unit. Reads secret name from VM `t3env` metadata attribute. |
| `env.example` | Template for the Secret Manager `.env` secret. |

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `networks/default was not found` | Project has no `default` VPC | Set `NETWORK=openclaw-vpc` (the script defaults to this). |
| `vm-setup.sh: compose file not found` | deploy-gcp/ not copied to VM yet | Re-run the `gcloud compute scp` step. |
| Agent container restart-loops | Bad value in .env (most often password mismatch, malformed API key) | `sudo docker logs t3claw-t3claw-1` — check first 30 lines. Fix: add corrected secret version, then `sudo systemctl restart t3claw`. |
| HTTPS cert error | Google-managed cert not yet provisioned | Wait 10–20 min. Poll `gcloud compute ssl-certificates describe <cert> ...` for `ACTIVE`. |
| `gcloud crashed (SSLError)` | Transient TLS flake | Retry. Three in a row → check VPN/proxy. |
| `Permission denied` on first SCP | Stale `~/.ssh/google_compute_engine` | `rm ~/.ssh/google_compute_engine*` and retry. |
| SMS MFA never arrives | Wrong/stale phone or SMS filtered | Update at `https://myaccount.google.com/two-step-verification`; use Google prompts instead. |
| CI: `iam.serviceAccounts.actAs` error | `t3claw-ci-deploy` missing `roles/iam.serviceAccountUser` on VM SA | Re-run `bash deploy-gcp/wif-setup.sh`. |
| Build slow on every provision run | Phase 1 builds from source | Use `SKIP_BUILD=1` when iterating on infrastructure only. |
