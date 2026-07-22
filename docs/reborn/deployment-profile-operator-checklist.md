# Deployment Profile Operator Checklist

**Issue:** #6454

**Parent Epic:** #6369

Use this checklist to verify deployment variables that live outside the
repository. It is an operator procedure, not authorization to modify a live
Railway or GCP environment. Record profile names and verification results, but
never copy database URLs, tokens, master keys, OAuth secrets, or other secret
values into issues, pull requests, logs, or screenshots.

## Checked-in profile contract

| Target | Profile source | Required profile |
| --- | --- | --- |
| Local Docker | `docker/reborn/config.toml` | `local-dev` |
| Railway, Postgres-backed single tenant | Railway service variable | `hosted-single-tenant` |
| Railway, volume-backed single tenant | Railway service variable | `hosted-single-tenant-volume` |
| Railway, production multi-tenant | Railway service variable | `production` |
| GCP Compute Engine systemd unit | `/opt/ironclaw/.env` created from `deploy/env.example` | `production` |
| Production-shaped validation without traffic | Explicit operator invocation | `migration-dry-run` |

Public Railway and GCP deployments must set `IRONCLAW_REBORN_PROFILE`
explicitly. Do not use `local-dev-yolo` on a public listener. Do not use
`migration-dry-run` as a serving profile.

The image ships an explicit local `local-dev` seed so a loopback-only local
Docker run remains usable without PostgreSQL. That seed is not a public
deployment default.

## Railway checklist

Before deploying:

- [ ] Confirm `railway.toml` uses the repository root `Dockerfile` and has no
  custom Start Command that bypasses the image entrypoint.
- [ ] Confirm `IRONCLAW_REBORN_PROFILE` is present in Railway service variables
  and matches the intended row in the table above.
- [ ] Confirm `IRONCLAW_REBORN_SERVE_HOST=0.0.0.0` and let Railway supply
  `PORT`.
- [ ] For `hosted-single-tenant`, confirm the PostgreSQL URL and independent
  secret master key variables are present without displaying their values.
- [ ] For a volume-backed profile, confirm a persistent Railway volume is
  attached and `IRONCLAW_REBORN_HOME` resolves beneath its mount.
- [ ] Confirm WebUI authentication variables and the selected LLM provider's
  credential variables are present without displaying their values.
- [ ] Confirm the service does not set
  `IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY=true` unless it is explicitly a
  disposable test deployment.

After a safe deployment or restart:

- [ ] Confirm Railway reports `/api/health` healthy.
- [ ] Confirm non-health routes leave the temporary startup `503` state after
  runtime assembly completes.
- [ ] Inspect redacted startup logs for the intended profile and absence of
  blocking readiness diagnostics; do not paste raw environment output.
- [ ] Record only the target, profile name, image tag/digest, timestamp, and
  pass/fail result in the deployment record.

## GCP Compute Engine checklist

Before enabling or restarting `ironclaw.service`:

- [ ] Start from `deploy/env.example`; replace every `CHANGE_ME` value in the
  root-readable `/opt/ironclaw/.env` without committing that file.
- [ ] Confirm `IRONCLAW_VERSION` is an explicit immutable release tag and is
  neither empty nor `latest`; an unset value must prevent the unit from starting.
- [ ] Confirm the file contains exactly one active
  `IRONCLAW_REBORN_PROFILE=production` assignment.
- [ ] Confirm the PostgreSQL URL uses the intended percent-encoded `/cloudsql/`
  Unix socket path and the independent secret master key is present, without
  printing either value. Do not expose a Cloud SQL Proxy TCP port.
- [ ] Confirm `IRONCLAW_REBORN_SERVE_HOST=0.0.0.0`, the WebUI authentication
  variables are present, and the image version is pinned.
- [ ] Confirm `/opt/ironclaw/.env` remains root-owned and mode `0600`.
- [ ] Confirm `/opt/ironclaw/data` is owned by UID/GID `1000:1000`, has mode
  `0700`, and is included in a tested backup/restore procedure.
- [ ] Confirm `cloud-sql-proxy.service` is active before starting IronClaw.
- [ ] Confirm the GCP firewall or trusted reverse proxy exposes only the intended
  WebUI port; `docker inspect ironclaw` must show no published PostgreSQL port.

After a safe deployment or restart:

- [ ] Confirm `systemctl is-active cloud-sql-proxy ironclaw` succeeds.
- [ ] Confirm the proxy socket exists below
  `/run/cloud-sql-proxy/ironclaw-prod:us-central1:ironclaw-db/` and the IronClaw
  container has both `/cloudsql` and `/data/ironclaw-reborn` bind mounts.
- [ ] Confirm `curl --fail http://127.0.0.1:3000/api/health` succeeds from the
  VM.
- [ ] Inspect the bounded, redacted service logs for the intended profile and
  absence of blocking readiness diagnostics.
- [ ] Record only the target, profile name, image tag/digest, timestamp, and
  pass/fail result.

## Local Docker checklist

- [ ] Keep the listener on `127.0.0.1` unless a trusted external proxy requires
  a different bind.
- [ ] Use the shipped `local-dev` seed or set `IRONCLAW_REBORN_PROFILE`
  explicitly when testing another profile.
- [ ] Supply the storage and secret variables required by the selected profile.
- [ ] Do not treat a successful local `local-dev` start as production-profile
  readiness evidence.

## Rollback and evidence

This audit does not change runtime defaults. Rollback for documentation or
static-test corrections is a normal source revert. A live deployment rollback
must use the platform's previously verified image tag/digest and its matching
profile/config contract; never fall back to an uncontrolled `latest` tag.

For issue evidence, record:

```text
Target: Railway | GCP | local Docker
Profile: <non-secret profile name>
Image: <version tag or digest>
Checked at: <UTC timestamp>
Health: pass | fail
Readiness: pass | blocking diagnostic id only
Operator: <GitHub handle or team>
```

Do not attach complete environment dumps or unredacted logs.
