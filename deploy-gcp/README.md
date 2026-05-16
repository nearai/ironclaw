# T3Claw — GCP Staging Deployment

**Stack:** Compute Engine VM (asia-southeast1-a) · Artifact Registry (us-central1) · Global HTTPS Load Balancer · Cloud DNS
**Endpoint:** `https://t3claw.agent.staging.gc.terminal3.io`
**Project:** `gen-lang-client-0263867259` (openclaw)

The VM has no public IP. All shell access is via IAP tunnel.

---

## Prerequisites

| Requirement | Verify with |
|---|---|
| Project owner/editor on `gen-lang-client-0263867259` | `gcloud projects get-iam-policy gen-lang-client-0263867259` |
| `gcloud` installed and logged in | `gcloud auth login` |
| `docker` (Buildx — Docker Desktop on macOS works) | `docker buildx version` |
| `openssl` (for generating secrets) | `openssl version` |
| Anthropic API key (for the agent's LLM backend) | — |
| Cloud NAT on `openclaw-vpc` in `asia-southeast1` (lets the no-IP VM reach apt/Docker Hub) | see below |
| Google account 2FA configured with **Google prompts** preferred over SMS | `https://myaccount.google.com/two-step-verification` |

Verify Cloud NAT (one router with non-empty NATs is enough):

```bash
gcloud compute routers list \
  --project=gen-lang-client-0263867259 \
  --filter="network:openclaw-vpc AND region:asia-southeast1" \
  --format="table(name,region.basename(),nats[].name.list():label=NATS)"
```

### About MFA prompts

Workspace policy requires step-up MFA on **every** IAP-tunnelled SSH/SCP. Two things that make this less painful:

1. **Use Google prompts, not SMS.** Open `https://myaccount.google.com/two-step-verification`, confirm your phone number is current, and ensure Google prompts are listed as a method. SMS delivery is unreliable and was the primary blocker on the first provisioning run.
2. **Batch operations into one SSH call.** Each `gcloud compute ssh ...` is one MFA challenge. Chain commands with `&&` inside a single quoted argument so a five-step procedure is one prompt, not five.
3. **Persistent IAP tunnel** (advanced, optional): `gcloud compute start-iap-tunnel <vm> 22 --local-host-port=localhost:2222 --zone=... --project=...` opens one tunnel with one MFA, then plain `ssh -p 2222 ...` works without further prompts.

---

## First-time provisioning

The provision script orchestrates everything, but pauses at a manual VM-bootstrap step in the middle. You will need **two terminals**.

### Step 1 — Run the provision script

```bash
bash deploy-gcp/gcp-provision.sh
```

The script is idempotent — re-runs are safe. It will:

1. Create the `t3claw` Artifact Registry repo (if absent) and build + push the agent and sidecar images.
2. Create the `t3claw-vm` service account, grant `artifactregistry.reader`, create the LB-health-check and IAP-SSH firewall rules, and create the `t3claw-staging` VM (no public IP).
3. **Pause** with `Press Enter once the VM is bootstrapped and required secrets are available in Secret Manager (do not place a .env on the VM)...`. **Leave this terminal at the prompt** and switch to a second terminal for steps 2–5.
4. After you press Enter: provision the global HTTPS load balancer (static IP, health check, instance group, backend service, URL map, Google-managed SSL cert, HTTPS proxy, forwarding rule).
5. Create the Cloud DNS A record.

If you have already pushed images and only want to (re)create downstream infrastructure, skip the slow Docker build:

```bash
SKIP_BUILD=1 bash deploy-gcp/gcp-provision.sh
```

If your project uses a non-default VPC name, override:

```bash
NETWORK=my-vpc bash deploy-gcp/gcp-provision.sh
```

> **Why SKIP_BUILD exists.** `SKIP_BUILD=1` skips Phase 1 (image build + push) entirely, useful when you've already pushed images and only want to (re-)create GCP infrastructure.

### Step 2 — Copy bootstrap files to the VM (in a second terminal)

```bash
gcloud compute scp --recurse deploy-gcp/ t3claw-staging:/var/tmp/deploy \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap
```

The first SCP from a fresh laptop will:
- prompt to generate `~/.ssh/google_compute_engine` (accept with empty passphrase if you don't want to retype it on every command);
- trigger Google MFA — pick the in-app **Google prompt** and approve.

### Step 3 — Run the VM bootstrap

```bash
gcloud compute ssh t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
  -- sudo bash /var/tmp/deploy/vm-setup.sh
```

This installs Docker, configures Artifact Registry auth, pre-pulls the agent image, installs `/opt/t3claw/docker-compose.yml` (from `docker-compose.staging.yml`) and the `t3claw.service` systemd unit. It will warn that `/opt/t3claw/.env` is missing — expected, fixed in step 4.

### Step 4 — Store `.env` in Secret Manager

The agent service fetches its environment at every start from the `t3claw-staging-env` Secret Manager secret. You never place a `.env` file on the VM directly.

Generate the random values locally:

```bash
echo "POSTGRES_PASSWORD=$(openssl rand -hex 24)"
echo "GATEWAY_AUTH_TOKEN=$(openssl rand -hex 32)"
echo "SECRETS_MASTER_KEY=$(openssl rand -hex 32)"
```

> ⚠ Save `SECRETS_MASTER_KEY` somewhere durable (a team password manager). Losing it means losing access to every secret stored in the workspace.

Fill in the template and upload it as the first secret version (outside the repo so it's never committed):

```bash
ENV_LOCAL="$(mktemp -t t3claw.env.XXXXXX)"
cp deploy-gcp/env.example "$ENV_LOCAL"
"${EDITOR:-vi}" "$ENV_LOCAL"     # fill in the five CHANGE_ME values
gcloud secrets versions add t3claw-staging-env \
  --data-file="$ENV_LOCAL" \
  --project=gen-lang-client-0263867259
rm "$ENV_LOCAL"
```

Required edits:

| Line | Variable | Value |
|---|---|---|
| 8 | `DATABASE_URL` | replace `CHANGE_ME` with the same password used in line 9 |
| 9 | `POSTGRES_PASSWORD` | from `openssl rand -hex 24` above |
| 16 | `ANTHROPIC_API_KEY` | a real Anthropic API key (`sk-ant-...`) |
| 30 | `GATEWAY_AUTH_TOKEN` | from `openssl rand -hex 32` above |
| 34 | `SECRETS_MASTER_KEY` | from `openssl rand -hex 32` above |

Lines 8 and 9 must use the same password — Postgres uses it on container init, the agent uses it to connect.

To update a secret value later, add a new version:

```bash
gcloud secrets versions add t3claw-staging-env \
  --data-file=/path/to/updated.env \
  --project=gen-lang-client-0263867259
```

> **Secret vs image updates.** The CI deploy workflow runs `docker compose pull && up -d` directly — it does **not** invoke `systemctl restart t3claw`. This means secret updates are not picked up automatically by a code deploy. To apply a new secret version, SSH in and restart the service manually:
> ```bash
> gcloud compute ssh t3claw-staging \
>   --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
>   -- 'sudo systemctl restart t3claw'
> ```

### Step 5 — Start the service and verify

```bash
gcloud compute ssh t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
  -- 'sudo systemctl enable t3claw && sudo systemctl restart t3claw && sleep 30 && sudo docker ps --format "table {{.Names}}\t{{.Status}}" && curl -fsS http://localhost:3000/api/health && echo'
```

Expected output:

```
t3claw-t3claw-1     Up X seconds
t3claw-postgres-1   Up X seconds (healthy)
{"status":"healthy","channel":"gateway"}
```

Postgres takes ~20 seconds to bootstrap on first boot (it does an `initdb`, then exits and restarts itself). The 30-second sleep accounts for that. If health fails, debug with:

```bash
gcloud compute ssh t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
  -- 'sudo journalctl -u t3claw -n 200 --no-pager; sudo docker logs --tail 100 t3claw-t3claw-1'
```

The most common failure is a typo in `.env` — e.g. password mismatch between lines 8 and 9, or a malformed Anthropic key.

### Step 6 — Resume the provision script

Return to **terminal A** (parked at the Phase 3 prompt) and press **Enter**. The script provisions the load balancer (~1 min) and the DNS record. At the end it prints the LB IP and the HTTPS endpoint.

### Step 7 — Wait for the SSL certificate

Google-managed certs provision once DNS propagates. Typically 10–20 minutes. Poll until `ACTIVE`:

```bash
gcloud compute ssl-certificates describe t3claw-cert \
  --project=gen-lang-client-0263867259 \
  --format="value(managed.status,managed.domainStatus)"
```

Until then HTTPS will fail with a cert error — normal.

### Step 8 — Final smoke test

```bash
curl -fsS https://t3claw.agent.staging.gc.terminal3.io/api/health
```

Should return the same `{"status":"healthy","channel":"gateway"}` you saw in step 5.

---

## Day-to-day operations

### SSH into the VM

```bash
gcloud compute ssh t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap
```

Once inside:

```bash
sudo docker ps -a                                              # container status
sudo journalctl -u t3claw -f                                   # live service logs
sudo docker logs -f t3claw-t3claw-1                            # agent logs only
sudo bash -c 'cd /opt/t3claw && docker compose --profile app restart t3claw'
```

### Rolling image update (no VM reset, fastest)

After pushing a new image to AR:

```bash
gcloud compute ssh t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 --tunnel-through-iap \
  -- "sudo bash -c 'cd /opt/t3claw && docker compose --profile app pull && docker compose --profile app up -d'"
```

### Hard reset (re-runs the embedded startup-script.sh from scratch)

A reset reinstalls Docker, regenerates `/opt/t3claw/`, and starts the service. Useful if something is fundamentally broken.

```bash
gcloud compute instances add-metadata t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 \
  --metadata-from-file startup-script=deploy-gcp/startup-script.sh && \
gcloud compute instances reset t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259
```

Watch boot progress without SSH:

```bash
gcloud compute instances get-serial-port-output t3claw-staging \
  --zone=asia-southeast1-a --project=gen-lang-client-0263867259 | tail -80
```

> **Note.** `vm-setup.sh` and `startup-script.sh` are two different bootstrap paths: `startup-script.sh` (used by `instances reset`) embeds its own compose definition that includes the t3n-mcp sidecar and is the authoritative running configuration. `vm-setup.sh` installs `docker-compose.staging.yml` which omits the sidecar — that file is outdated relative to what actually runs. If you change the compose configuration, update `startup-script.sh`.

---

## Image rebuild and push

The provision script does this automatically as Phase 1, but you'll often want to rebuild and push without touching infrastructure.

### Both images, via the script

```bash
bash deploy-gcp/gcp-provision.sh   # Phase 2+ are skipped because everything exists
```

### Just the agent image, manually

```bash
docker buildx build --platform linux/amd64 \
  --target runtime-staging \
  -t us-central1-docker.pkg.dev/gen-lang-client-0263867259/t3claw/agent:latest \
  --push .
```

`--platform linux/amd64` is non-negotiable — the VM is x86 and Apple Silicon machines build arm64 by default, which won't run on the VM.

The first build from a cold cache takes 30–60 minutes (Rust + WASM compile). Subsequent builds use BuildKit's layer cache and finish in seconds **provided** the inputs to the slow steps haven't changed. The agent Dockerfile uses cargo-chef so dependency builds are cached; unrelated repo edits do not bust them.

### t3n-mcp sidecar

The sidecar (`t3n-mcp-sidecar:latest`) runs alongside the agent on the VM. CI builds and pushes it to AR as part of `.github/workflows/staging-gcp.yml`. To rebuild it manually:

---

## Reference

### Required `.env` values

See `deploy-gcp/env.example` for the full template. Five values must be replaced before starting the service:

- `POSTGRES_PASSWORD` and `DATABASE_URL` (must match)
- `ANTHROPIC_API_KEY`
- `GATEWAY_AUTH_TOKEN`
- `SECRETS_MASTER_KEY`

### Infrastructure summary

| Resource | Name | Notes |
|---|---|---|
| Network | `openclaw-vpc` | shared with `bastionclaw-staging`, `ironclaw-staging` |
| Cloud NAT | `openclaw-nat` on `openclaw-nat-router-sg` | required for no-IP VMs to bootstrap |
| Artifact Registry repo | `t3claw` | `us-central1` |
| VM | `t3claw-staging` | e2-standard-2, asia-southeast1-a, no public IP |
| Service account | `t3claw-vm` | `roles/artifactregistry.reader` |
| Firewall: LB health checks | `allow-t3claw-lb` | `tcp:3000` from `130.211.0.0/22, 35.191.0.0/16` to `tag:t3claw` |
| Firewall: IAP SSH | `allow-ssh-iap` | `tcp:22` from `35.235.240.0/20` |
| Static IP | `t3claw-staging-ip` | global, IPv4 |
| Health check | `t3claw-health` | HTTP `GET /api/health` on `:3000` |
| Instance group | `t3claw-staging-ig` | unmanaged, single VM, named-port `http:3000` |
| Backend service | `t3claw-backend` | global, HTTP, weighted to the instance group |
| URL map | `t3claw-urlmap` | default-routes to `t3claw-backend` |
| SSL cert | `t3claw-cert` | Google-managed, auto-renews |
| HTTPS proxy | `t3claw-https-proxy` | `t3claw-cert` + `t3claw-urlmap` |
| Forwarding rule | `t3claw-https-rule` | `:443` on the static IP |
| DNS zone | `claw-dns-staging` | `agent.staging.gc.terminal3.io` |
| DNS record | `t3claw` | A record → static IP |

### Files in this directory

| File | Role |
|---|---|
| `gcp-provision.sh` | One-shot provisioning of all GCP resources. Idempotent. |
| `wif-setup.sh` | One-shot Workload Identity Federation setup for GitHub Actions CD. Idempotent. |
| `vm-setup.sh` | Run on the VM during step 3 of first-time provisioning. |
| `startup-script.sh` | Self-contained boot-time bootstrap, used by `instances reset`. |
| `docker-compose.staging.yml` | Compose file installed at `/opt/t3claw/docker-compose.yml` on the VM. |
| `t3claw.service` | systemd unit. |
| `env.example` | Template for the `t3claw-staging-env` Secret Manager secret. |

---

## CI/CD — automated deploy on push to staging

`.github/workflows/staging-gcp.yml` runs automatically on every push to `staging`. It builds and pushes both images to Artifact Registry, then IAP-SSHes
into the VM and does a rolling update. No human action required.

### One-time GCP setup

Run the setup script once from a machine authenticated as a project owner:

```bash
gcloud auth login
bash deploy-gcp/wif-setup.sh
```

The script is idempotent — safe to re-run. It creates the `t3claw-ci-deploy` service
account, grants the four required IAM roles, creates the `github-actions` WIF pool and
`github-provider` OIDC provider scoped to this repo, and prints the two secret values.

**Add GitHub repository secrets**

Go to **Settings → Secrets and variables → Actions** and add the two values printed
by the script:

| Secret | Value (printed by `wif-setup.sh`) |
|---|---|
| `WIF_PROVIDER` | `projects/PROJECT_NUMBER/locations/global/workloadIdentityPools/github-actions/providers/github-provider` |
| `WIF_SERVICE_ACCOUNT` | `t3claw-ci-deploy@gen-lang-client-0263867259.iam.gserviceaccount.com` |

### How the workflow runs

        ↓ both succeed
  └── deploy         IAP SSH → docker compose --profile app pull && up -d
                     Smoke test: curl /api/health (retries 10×10 s)
```

The deploy job runs `docker compose --profile app pull` which pulls both the agent and sidecar images, then restarts with `up -d`.

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `gcloud compute firewall-rules create ... ERROR: The resource ... networks/default was not found` | Project has no `default` VPC | Set `NETWORK=openclaw-vpc` (or your VPC name). The script defaults to this. |
| `gcloud compute instances create ... ERROR: ... networks/default was not found` | Same as above for the VM | Same. |
| `vm-setup.sh` succeeds but `t3claw.service` fails with `yaml: did not find expected key` | Old `vm-setup.sh` did sed-mangling of the dev `docker-compose.yml` and produced broken YAML | Re-run from a checkout that has `docker-compose.staging.yml` and the updated `vm-setup.sh`. |
| `gcloud crashed (SSLError): UNEXPECTED_EOF_WHILE_READING` | Transient TLS handshake flake | Just retry. Three failures in a row → check VPN/proxy. |
| `A security code has been sent to your phone` and the SMS never arrives | Wrong/stale phone number on Google account, or SMS is being filtered | Update phone at `https://myaccount.google.com/two-step-verification` and prefer Google prompts. Use a backup code as a one-off. |
| Agent container restart-loops | Bad value in env (most often password mismatch on lines 8/9, or malformed `ANTHROPIC_API_KEY`) | `sudo docker logs t3claw-t3claw-1` and read the first 30 lines. Fix by adding a corrected secret version: `gcloud secrets versions add t3claw-staging-env --data-file=fixed.env` then `sudo systemctl restart t3claw`. |
| HTTPS endpoint returns cert error | Google-managed cert hasn't provisioned yet | Wait. `gcloud compute ssl-certificates describe t3claw-cert ...` — wait for `managed.status: ACTIVE`. |
| Docker build is slow on every script run | `gcp-provision.sh` builds the agent (cargo-chef cached) and the sidecar (Node, fast). Use `SKIP_BUILD=1` when iterating on infrastructure rather than code. |
| `gcloud compute scp ... ERROR: ... Permission denied` on first run | Stale `~/.ssh/google_compute_engine` from a previous account | `rm ~/.ssh/google_compute_engine*` and retry — gcloud will regenerate. |
