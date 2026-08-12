# Railway Sandboxed-Volume Preview Operator Runbook

This is a preview-only deployment procedure for
`hosted-single-tenant-volume-sandboxed-railway`. It deploys exactly one
IronClaw control-service replica on Railway with durable state on a Railway
volume, and uses the Railway CLI carried by `Dockerfile` for the sandbox
lifecycle. It is not a production multi-replica topology.

## Status and scope

Railway Sandboxes are a Priority Boarding feature, and their programmatic
surface is still subject to change. Treat the sandbox API and CLI commands as
experimental: before operating a preview, verify `railway sandbox --help` with
the image's pinned Railway CLI version and re-check Railway's current sandbox
documentation. Do not upgrade the CLI independently of `Dockerfile`.

This runbook applies to one designated Railway project and one designated
environment. Never rely on a linked directory, the CLI's current project, or a
default environment. Record and provide both IDs for every operator action:

```text
RAILWAY_PROJECT_ID=<designated-preview-project-id>
RAILWAY_ENVIRONMENT_ID=<designated-preview-environment-id>
```

Pass those values explicitly as the CLI project and environment selectors. Do
not copy them from, or point them at, another Railway project.

## Image and credential boundary

`Dockerfile` downloads Railway CLI `5.30.4` by architecture and verifies the
release checksum before copying only the executable into the final image. No
Railway credential is a Docker build argument, environment instruction, layer,
or file.

Provide at most one token to the control service at runtime, through the
designated Railway secret mechanism:

- `RAILWAY_TOKEN` is project-scoped. Prefer it for actions confined to this
  one preview project.
- `RAILWAY_API_TOKEN` is account/workspace-scoped. Use it only when the
  sandbox operation genuinely requires broader account/workspace authority.

The Railway CLI rejects a process that has both variables. Do not place either
token in `config.toml`, source control, Docker build arguments, log output, or
sandbox-worker environment. The control service may use its selected token to
call Railway; the untrusted worker must not receive it.

## Required deployment shape

Configure the Railway service with all of the following:

- Dockerfile path: `Dockerfile`; leave Start Command empty so the image
  entrypoint builds the `ironclaw serve` command.
- `IRONCLAW_REBORN_PROFILE=hosted-single-tenant-volume-sandboxed-railway`.
- A persistent Railway volume mounted at `/data`. The entrypoint derives
  `IRONCLAW_REBORN_HOME` from `RAILWAY_VOLUME_MOUNT_PATH` and fails closed for
  this profile family without a volume (unless the explicit disposable-test
  override is set).
- Exactly one IronClaw replica. Do not enable horizontal scaling, a second
  control service, or concurrent deploy overlap against the same volume-backed
  state. Scale back to one before a preview is considered healthy.
- The preview transport tracks at most 4,096 user lifecycle entries per
  process. At capacity it evicts the least-recently-used idle entry, never an
  entry held by an active command. The default Railway idle timeout is five
  minutes and may be changed with
  `IRONCLAW_REBORN_RAILWAY_IDLE_TIMEOUT_MINUTES`.
- IronClaw attempts to checkpoint the user workspace after every completed
  command. On graceful shutdown, it attempts to retry stale checkpoints and
  destroy the live Railway sandboxes owned by that process. Shutdown time is
  finite in a hosted deployment, and a crash cannot run cleanup at all, so the
  configured idle timeout is the hard resource-leak backstop.
- The usual WebUI and LLM secrets described in
  [the Docker deployment guide](deploy-reborn-cli-docker.md); do not invent or
  bake values into this runbook.

The volume is the durable IronClaw state boundary. It keeps the Reborn home,
libSQL-backed runtime/control-plane state, and process checkpoints across a
control-service restart. A Railway Sandbox checkpoint is only a snapshot of
that sandbox's own filesystem; it does not replace the IronClaw durable
checkpoint or the Railway volume.

## Worker network boundary

Each command runs in a fresh inner Docker worker inside its Railway Sandbox.
The Railway preview deployment factory enables that worker's default Docker
network, providing direct egress through the outer sandbox's Railway NAT.
`--network none` remains only the ad-hoc transport's fail-closed construction
default; it is not active for this profile. The Railway control service also
needs egress to invoke Railway's sandbox API.

Do not describe a Railway Sandbox's `ISOLATED` mode as equivalent to Docker
`--network none`. Railway documents `ISOLATED` as private-network isolation;
it still has outbound internet access through Railway's NAT. A Railway sandbox
therefore is not a deny-egress security boundary for untrusted code. Keep
credentials host-side and use an infrastructure-enforced egress boundary for
any worker that requires a deny-by-default guarantee.

## Operator checklist

1. Confirm the project and environment IDs are the designated preview pair and
   that the selected token has only the authority required for that pair.
2. Confirm only one Railway token variable is present in the control-service
   runtime, and that neither token is passed to a worker or written to disk.
3. Confirm the service has one replica and a `/data` volume before deploy.
4. Confirm the service boots with the Railway-preview profile and that the
   volume-backed Reborn home survives one controlled restart.
5. Before a sandbox lifecycle operation, confirm the pinned CLI's sandbox help
   and current Railway API behavior. Record the explicit project/environment
   IDs with the operation evidence, then verify the resulting sandbox state.
6. After a controlled restart, confirm the previous process-owned sandboxes
   were destroyed or expire within the configured idle timeout. After a crash,
   confirm they expire within that timeout.
7. On failure, stop creating new sandboxes first. Preserve the volume and
   checkpoint evidence, reduce the control service to one replica, and inspect
   the host-side logs without printing tokens.

For current Railway token semantics, see the
[Railway CLI authentication documentation](https://docs.railway.com/cli/login).
For the current sandbox release status, see Railway's
[Sandboxes announcement](https://railway.com/changelog/2026-06-05-sandboxes).
