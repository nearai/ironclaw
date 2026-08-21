# Reborn CLI Docker Deployment

`Dockerfile` builds the standalone `ironclaw` binary with the
WebUI v2 and Slack host-beta features enabled. The image defaults to:

```text
ironclaw serve --host ${IRONCLAW_REBORN_SERVE_HOST:-0.0.0.0} --port ${PORT:-3000}
```

The entrypoint binds `0.0.0.0` inside the container so Docker-published ports and
Railway ingress can reach it; bind the published host port to `127.0.0.1` for
local-only access. Railway's `PORT` is used automatically. An explicit
`IRONCLAW_REBORN_SERVE_HOST` overrides the container default.

## Build

```bash
docker build -f Dockerfile -t ironclaw-reborn:local .
```

## Local Run

Create an env file outside git, then run:

```bash
docker run --rm \
  --env-file .env.reborn \
  -p 127.0.0.1:3000:3000 \
  ironclaw-reborn:local
```

Minimum local env shape:

```bash
IRONCLAW_REBORN_SERVE_HOST=0.0.0.0
IRONCLAW_REBORN_SERVE_PORT=3000
IRONCLAW_REBORN_PROFILE=local-dev
IRONCLAW_REBORN_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli
NEARAI_BASE_URL=https://cloud-api.near.ai
NEARAI_API_KEY=<nearai-api-key>
```

The bundled Docker config selects NearAI in `[llm.default]`; set
`NEARAI_API_KEY` for that provider. To change provider or model, mount a custom
config and point `IRONCLAW_REBORN_DEFAULT_CONFIG` at it for the first start.

## Durable storage boundary

Set `IRONCLAW_REBORN_HOME` to one mounted installation directory. Its direct
namespaces are `state/` (authoritative application state and the local cached
master key), `system/` (host-managed extensions, prompts, and skills),
`workspaces/` (tenant-plus-user leaves), and `runtime/` (provider/runtime
bookkeeping), plus the operational `logs/`, `cache/`, and `tmp/` namespaces.
`logs/` is retained only according to the operator's logging policy; `cache/`
and `tmp/` are disposable. None of these three is authoritative application
state. Do not add a deployment ID or the selected profile to this path.

When upgrading a populated legacy home, stop every old container before the
new release starts (volume-backed services get recreate deploys, which already
guarantees this). The first new startup migrates the legacy layout
automatically: it takes an advisory migration lock, probes the embedded
database for a live writer, renames the legacy files into the canonical
namespaces, and binds only after `layout.toml` is committed. Nothing is copied
or deleted, and a losing candidate stays untouched and is named in
`runtime/layout-migration.toml`. Set `IRONCLAW_REBORN_STORAGE_MIGRATION=manual`
to defer migration to an operator-scheduled restart.

Profiles choose policy and a process backend, not a physical storage root. A
profile change is an operator-controlled restart: startup checks the persisted
security envelope and rejects an incompatible backend, tenancy, or workspace
isolation transition before runtime construction. Docker sandboxes receive only
`workspaces/users/<tenant-user-digest>` as `/workspace`; never mount the Reborn
home, `state/`, `system/`, `runtime/`, a workspace parent, sibling leaf, Docker
socket, provider credentials, or the cached master key. See the
[storage-layout adoption runbook](storage-layout-adoption.md) before changing
an existing installation.

Google product-auth setup:

```bash
IRONCLAW_REBORN_GOOGLE_CLIENT_ID=<google-client-id>
IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET=<google-client-secret>
IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI=http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback
```

WebUI Google login setup:

For normal Docker bridge networking, put HTTPS in front of the container and
set the public base URL. Plain `http://127.0.0.1` SSO is only valid when the
Reborn listener itself is bound to loopback, such as a non-Docker local run or a
host-network run.

```bash
IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID=<google-client-id>
IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET=<google-client-secret>
IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS=near.ai
IRONCLAW_REBORN_WEBUI_BASE_URL=https://<public-host>
```

Register this WebUI login callback in the Google OAuth client:

```text
https://<public-host>/auth/callback/google
```

## Railway

Set the service Dockerfile path to `Dockerfile`. The entrypoint binds `0.0.0.0`
inside the container and uses Railway's `PORT`. The Reborn WebUI service serves
`/api/health` for Railway's healthcheck. An explicit
`IRONCLAW_REBORN_SERVE_HOST` still overrides this derived default.

Leave Railway's Start Command empty for the Docker image. The image entrypoint
builds the `ironclaw serve` arguments from `PORT` and
`IRONCLAW_REBORN_SERVE_HOST`; Railway does not shell-expand `$VAR` placeholders
in Docker command arguments before they reach the entrypoint.

Minimum Railway variables for the hosted single-tenant Postgres profile:

```bash
IRONCLAW_REBORN_PROFILE=hosted-single-tenant
IRONCLAW_REBORN_POSTGRES_URL=<postgres-url>
IRONCLAW_REBORN_SECRET_MASTER_KEY=<random-secret-master-key>
IRONCLAW_REBORN_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli
NEARAI_API_KEY=<nearai-api-key>
```

The volume-backed sandbox profiles have distinct operator intent:

- `hosted-single-tenant-volume-sandboxed` is the local-Docker profile. It is
  for exercising the Docker worker boundary. The deployment factory enables
  the worker's default Docker network; `--network none` is only the ad-hoc
  transport's fail-closed construction default. Build its Python worker once
  with `docker build -f Dockerfile.sandbox-worker -t ironclaw-worker:latest .`.
  On Docker Desktop or Colima, keep `IRONCLAW_REBORN_HOME` under a host path
  shared with the Docker VM (for example `/Users/...` on macOS); otherwise the
  daemon cannot bind the per-user workspace.
- `hosted-single-tenant-volume-sandboxed-railway` is the Railway preview
  profile. Each command runs in a fresh inner Docker worker inside a Railway
  Sandbox. The deployment factory enables the worker's default Docker network,
  providing direct egress through the outer sandbox's Railway NAT. The
  `--network none` setting remains only the ad-hoc transport's fail-closed
  construction default. Its lifecycle requirements are documented in
  [the Railway sandbox operator runbook](railway-sandbox-operator.md).

The persisted seed config records the canonical volume-backed application
settings. The selected profile remains an explicit Rust composition profile,
so startup also validates that its matching sandbox process provider is wired.

Minimum Railway variables for the hosted single-tenant volume profile:

```bash
IRONCLAW_REBORN_PROFILE=hosted-single-tenant-volume
IRONCLAW_REBORN_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli
NEARAI_API_KEY=<nearai-api-key>
```

Attach a Railway volume and mount it at `/data`, or set
`IRONCLAW_REBORN_HOME` under `RAILWAY_VOLUME_MOUNT_PATH`. The image entrypoint
will use `$RAILWAY_VOLUME_MOUNT_PATH/ironclaw-reborn` by default when Railway
exposes a volume mount. Without a volume, Railway deployments using
`local-dev`, `local-dev-yolo`, `hosted-single-tenant`, or
`hosted-single-tenant-volume`, `hosted-single-tenant-volume-sandboxed`, or
`hosted-single-tenant-volume-sandboxed-railway` fail closed unless
`IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY=true` is explicitly set for a
disposable test deployment.

For managed Postgres providers with a small session-pool cap, set
`IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE=1` or `2` rather than relying on the
provider to queue excess sessions.
For `hosted-single-tenant`, `ironclaw serve` binds the WebUI listener
and serves `/api/health` before PostgreSQL-backed runtime assembly finishes.
Non-health routes return `503` until the runtime router is ready. This lets
Railway drain the old deployment and release PgBouncer session-mode
connections before the new deployment needs one for startup migrations.
`IRONCLAW_FILESYSTEM_POSTGRES_MIGRATION_CONNECT_MAX_WAIT_SECS` still controls
how long runtime assembly waits for PostgreSQL once the healthcheck listener is
up; the default is 5 minutes.

`ironclaw serve` exits before binding the HTTP listener if the WebUI
token/user variables are missing. The bundled config selects NearAI as the
default LLM provider, so set `NEARAI_API_KEY` unless a custom mounted config
selects a different provider.

Do not use `IRONCLAW_REBORN_PROFILE=local-dev-yolo` for a public Railway
listener. That profile grants trusted host access and `serve` refuses to bind it
to a non-loopback host. Use `hosted-single-tenant-volume` for the mounted-volume
single-tenant preview path that keeps the local-dev product surface with durable
libSQL-backed state, or `hosted-single-tenant` for Postgres-backed hosted state.
Use the sandboxed aliases only with their documented local-Docker or Railway
preview operator model; neither is a production multi-replica profile.

Set `IRONCLAW_REBORN_HOME` to a mounted volume path if local files should
survive redeploys. The hosted single-tenant profile stores runtime/control-plane
state, including extension installation/activation state, in Postgres; project
files, materialized system extension packages, and current skill file storage
still live under the local filesystem root. The image default is
`/data/ironclaw-reborn`; without a Railway volume, that path is ephemeral. The
hosted single-tenant volume profile stores runtime/control-plane state under
that Reborn home on the mounted volume and does not require
`IRONCLAW_REBORN_POSTGRES_URL`. The container workdir is `/workspace` so the
workspace root stays separate from Reborn's state and skill roots.

Railway sandbox workspace checkpoints are provider-specific and are not a
portable substitute for the canonical local `workspaces/` namespace. Do not
claim that switching a profile, backend, Railway environment, or provider moves
workspace contents; perform and verify a separate operator migration instead.
Railway and provider credentials remain in the control service and never enter
the inner worker.

The image includes `sqlite3` and `psql` for terminal inspection from Railway
shells. Use `sqlite3` for mounted-volume libSQL/SQLite state and `psql` for
`IRONCLAW_REBORN_POSTGRES_URL` deployments.

To seed a custom config instead of the bundled default, mount it under
`/opt/ironclaw/` and set `IRONCLAW_REBORN_DEFAULT_CONFIG` to that path. On first
start, the entrypoint copies that file into `$IRONCLAW_REBORN_HOME/config.toml`;
later starts preserve the existing home config.

For public WebUI Google login, use the Reborn WebUI SSO variables and an HTTPS
base URL that matches the deployed Railway domain users will open. If Railway
exposes more than one domain for the same service, choose one canonical domain
for `IRONCLAW_REBORN_WEBUI_BASE_URL` and register that same domain in Google:

```bash
IRONCLAW_REBORN_WEBUI_BASE_URL=https://<railway-domain>
IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID=<google-client-id>
IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET=<google-client-secret>
IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS=near.ai
IRONCLAW_REBORN_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli
```

Register this WebUI login callback in the Google OAuth client:

```text
https://<railway-domain>/auth/callback/google
```

Notion MCP and other product-auth OAuth setup flows use the same hosted WebUI
base URL for provider callbacks. Set `IRONCLAW_REBORN_WEBUI_BASE_URL` to the
same public host so product-auth providers see the public callback origin rather
than the local listener address. Google product-auth is separate and still uses
`IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI` explicitly.

Product-auth Google credentials are a separate flow. Configure
`IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI` only when the deployment should let
the agent connect a Google credential:

```bash
IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI=https://<railway-domain>/api/reborn/product-auth/oauth/google/callback
```

## Slack

Slack routes are compiled into the image and mounted unconditionally. No
environment variable and no `config.toml` key enables or disables Slack for a
deployment, so there is nothing Slack-specific to add to the Railway service
variables or to a mounted config file. The Slack webhook answers
`503 temporarily_unavailable` until the Slack extension's ingress signing
secret is registered.

Once the container is running, open the WebUI at `/extensions`, install the
Slack extension, and complete its setup. Slack app ids, the bot token, the
signing secret, and OAuth client credentials are all configured there after
the container starts. There is no shared-channel configuration: inviting the
bot into a channel is what enables it, the bot answers each participant as
themselves, and every user pairs by completing the Slack OAuth connect from
Extensions — there is no shared subject user, per-channel subject route, or
channel allowlist to configure.

There is no `IRONCLAW_REBORN_SLACK_ENABLED` toggle — the enablement gate it fed
was removed in #6116, and nothing has read the variable since. Do not add a
`[slack]` section either: the retired setup keys (`signing_secret_env`,
`bot_token_env`, `installation_id`, `team_id`, `api_app_id`, `channel_routes`,
…) make `ironclaw serve` **refuse to start**.

A volume seeded before #6116 may still carry a `[slack]` section. What happens
on boot depends on what the section holds. `enabled` on its own is inert and
keeps booting (with a deprecation notice in the serve log). The one shape the
entrypoint migrates for you is the old shipped default — an explicit
`enabled = false` next to `signing_secret_env`/`bot_token_env`: those two
fields are stripped on start and the container boots. Every other combination
that includes a retired setup key — `enabled = true` beside them, the legacy
fields without an explicit `enabled = false` line, or any of the other setup
keys listed above — is left alone deliberately and fails startup with a
migration pointer, rather than a live-looking channel config being rewritten
underneath you.

Set the WebUI identity environment variables as usual.

Do not store OAuth, Slack, or LLM secrets in `config.toml`. Slack bot tokens
and signing secrets are stored from the WebUI extension setup.

Migrating an existing config file: a mounted or previously seeded
`config.toml` that still carries a `[slack]` or `[telegram]` section keeps
parsing. Outside the one entrypoint-migrated shape above (`enabled = false`
beside `signing_secret_env`/`bot_token_env`), a leftover Slack *setup* field
(`installation_id`, `team_id`, `api_app_id`, `slack_user_id`, `user_id`,
`shared_subject_user_id`, `channel_routes`, `signing_secret_env`,
`bot_token_env`) fails container startup with a migration pointer rather than
being silently ignored; a section left with only inert keys still starts, and
logs a deprecation notice. Delete the section from the mounted file — nothing
reads it.
