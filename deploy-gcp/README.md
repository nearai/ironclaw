# BastionClaw — GCP Staging Deployment

**Stack:** Compute Engine VM (asia-southeast1-a) · Artifact Registry · Global HTTPS Load Balancer · Cloud DNS  
**Endpoint:** `https://t3claw.agent.staging.gc.terminal3.io`  
**Project:** `gen-lang-client-0263867259` (openclaw)

---

## Prerequisites

- `gcloud` authenticated as an owner of the project
- `docker` with buildx support (Docker Desktop on Mac is fine)
- Artifact Registry auth configured:

```bash
gcloud auth configure-docker us-central1-docker.pkg.dev
```

---

## 1 — Build images

Both images must be built for `linux/amd64` (the VM is x86). Your Mac is ARM so a cross-platform build is required.

```bash
# Source GITHUB_TOKEN from your local .env (required for the t3n-mcp-sidecar)
export $(grep GITHUB_TOKEN .env | xargs)

# Agent
docker buildx build --platform linux/amd64 \
  -t us-central1-docker.pkg.dev/gen-lang-client-0263867259/bastionclaw/agent:latest \
  --push .

# t3n-mcp sidecar
docker buildx build --platform linux/amd64 \
  --build-arg GITHUB_TOKEN=$GITHUB_TOKEN \
  -f docker/t3n-mcp-sidecar.Dockerfile \
  -t us-central1-docker.pkg.dev/gen-lang-client-0263867259/bastionclaw/t3n-mcp-sidecar:latest \
  --push .
```

> The agent build takes 30–60 min from scratch (Rust + WASM compilation). Subsequent builds use the BuildKit layer cache and are much faster.

---

## 2 — Push to Artifact Registry

`--push` in the build commands above pushes directly. If you need to push a pre-built local image (e.g. one built by `make up`):

```bash
IMAGE_PREFIX="us-central1-docker.pkg.dev/gen-lang-client-0263867259/bastionclaw"

docker tag bastion-claw-bastionclaw:latest ${IMAGE_PREFIX}/agent:latest
docker tag bastion-claw-t3n-mcp-sidecar:latest ${IMAGE_PREFIX}/t3n-mcp-sidecar:latest

docker push ${IMAGE_PREFIX}/agent:latest
docker push ${IMAGE_PREFIX}/t3n-mcp-sidecar:latest
```

> Local images built by `make up` on an Apple Silicon Mac are `arm64`. They will not run on the VM. Always build with `--platform linux/amd64` for GCP.

---

## 3 — Update the startup script

`deploy-gcp/startup-script.sh` is the single source of truth for what runs on the VM. It embeds the `docker-compose.yml`, the `bastionclaw.service` systemd unit, and the `mcp-servers.json` seed. Edit it here, then upload to VM metadata:

```bash
gcloud compute instances add-metadata bastionclaw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 \
  --metadata-from-file startup-script=deploy-gcp/startup-script.sh
```

The script runs automatically on every VM boot. To apply it immediately, reset the VM (see step 4).

---

## 4 — Reset / redeploy the VM

A reset is a hard reboot. It re-runs the startup script from scratch — reinstalls Docker, pulls the latest images, and starts the service.

```bash
# Upload latest startup script then reset
gcloud compute instances add-metadata bastionclaw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 \
  --metadata-from-file startup-script=deploy-gcp/startup-script.sh && \
gcloud compute instances reset bastionclaw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259
```

Monitor boot progress (no SSH needed):

```bash
gcloud compute instances get-serial-port-output bastionclaw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 | tail -50
```

---

## 5 — Rolling image update (no VM reset)

To deploy a new image without resetting the VM, SSH in and pull + restart:

```bash
gcloud compute ssh bastionclaw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
  -- "sudo bash -c 'cd /opt/bastionclaw && docker compose --profile app pull && docker compose --profile app up -d'"
```

---

## 6 — SSH into the VM

The VM has no public IP. Use IAP tunneling:

```bash
gcloud compute ssh bastionclaw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap
```

Useful commands once inside:

```bash
sudo docker ps -a                                        # container status
sudo journalctl -u bastionclaw -f                        # live service logs
sudo docker logs -f bastionclaw-bastionclaw-1            # agent logs only
sudo bash -c 'cd /opt/bastionclaw && docker compose --profile app restart bastionclaw'
```

---

## 7 — First-time provisioning

To provision the full GCP stack from scratch (AR repo, service account, VM, LB, DNS):

```bash
bash deploy-gcp/gcp-provision.sh
```

See `gcp-provision.sh` for details. After the VM is created, populate `/opt/bastionclaw/.env` using `deploy-gcp/env.example` as a template, then start the service.

---

## Infrastructure summary

| Resource | Name | Details |
|----------|------|---------|
| Artifact Registry | `bastionclaw` | `us-central1` |
| VM | `bastionclaw-staging` | e2-standard-2, asia-southeast1-a, no public IP |
| Service account | `bastionclaw-vm` | `roles/artifactregistry.reader` |
| Static IP | `bastionclaw-staging-ip` | `34.102.220.159` |
| SSL cert | `bastionclaw-cert-v2` | Google-managed, auto-renews |
| Load balancer | `bastionclaw-backend` | HTTPS → VM:3000 |
| DNS zone | `claw-dns-staging` | `agent.staging.gc.terminal3.io` |
| DNS record | `t3claw` | A → `34.102.220.159` |
