# Reborn CLI Docker Deployment

`Dockerfile.reborn` builds the standalone `ironclaw` binary with the
WebUI v2 and Slack host-beta features enabled. The entrypoint defaults to:

```text
ironclaw serve --host ${IRONCLAW_SERVE_HOST:-127.0.0.1} --port ${PORT:-3000}
```

Railway supplies `PORT`; when a `RAILWAY_*` runtime marker is present and no
serve host is explicitly configured, the entrypoint binds `0.0.0.0`
automatically. Local Docker runs keep the loopback default and can set
`IRONCLAW_SERVE_PORT=3000`.

## Build

```bash
docker build -f Dockerfile.reborn -t ironclaw-reborn:local .
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
IRONCLAW_SERVE_HOST=127.0.0.1
IRONCLAW_SERVE_PORT=3000
IRONCLAW_PROFILE=local-dev
IRONCLAW_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_WEBUI_USER_ID=reborn-cli
NEARAI_BASE_URL=https://cloud-api.near.ai
NEARAI_API_KEY=<nearai-api-key>
```

The bundled Docker config selects NearAI in `[llm.default]`; set
`NEARAI_API_KEY` for that provider. To change provider or model, mount a custom
config and point `IRONCLAW_DEFAULT_CONFIG` at it for the first start.

Google product-auth setup:

```bash
IRONCLAW_GOOGLE_CLIENT_ID=<google-client-id>
IRONCLAW_GOOGLE_CLIENT_SECRET=<google-client-secret>
IRONCLAW_GOOGLE_OAUTH_REDIRECT_URI=http://127.0.0.1:3000/api/product-auth/oauth/google/callback
```

WebUI Google login setup:

For normal Docker bridge networking, put HTTPS in front of the container and
set the public base URL. Plain `http://127.0.0.1` SSO is only valid when the
Reborn listener itself is bound to loopback, such as a non-Docker local run or a
host-network run.

```bash
IRONCLAW_WEBUI_GOOGLE_CLIENT_ID=<google-client-id>
IRONCLAW_WEBUI_GOOGLE_CLIENT_SECRET=<google-client-secret>
IRONCLAW_WEBUI_ALLOWED_EMAIL_DOMAINS=near.ai
IRONCLAW_WEBUI_BASE_URL=https://<public-host>
```

Register this WebUI login callback in the Google OAuth client:

```text
https://<public-host>/auth/callback/google
```

## Railway

Set the service Dockerfile path to `Dockerfile.reborn`. Railway sets `PORT`;
leave `IRONCLAW_SERVE_HOST` unset to let the entrypoint select `0.0.0.0`
automatically, or set it explicitly to `0.0.0.0`. The Reborn WebUI service
serves `/api/health` for Railway's healthcheck.

Leave Railway's Start Command empty for the Docker image. The image entrypoint
builds the `ironclaw serve` arguments from `PORT` and
`IRONCLAW_SERVE_HOST`; Railway does not shell-expand `$VAR` placeholders
in Docker command arguments before they reach the entrypoint.

Minimum Railway variables for the hosted single-tenant Postgres profile:

```bash
IRONCLAW_PROFILE=hosted-single-tenant
IRONCLAW_POSTGRES_URL=<postgres-url>
IRONCLAW_SECRET_MASTER_KEY=<random-secret-master-key>
IRONCLAW_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_WEBUI_USER_ID=reborn-cli
NEARAI_API_KEY=<nearai-api-key>
```

Minimum Railway variables for the hosted single-tenant volume profile:

```bash
IRONCLAW_PROFILE=hosted-single-tenant-volume
IRONCLAW_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_WEBUI_USER_ID=reborn-cli
NEARAI_API_KEY=<nearai-api-key>
```

Attach a Railway volume and mount it at `/data`, or set
`IRONCLAW_HOME` under `RAILWAY_VOLUME_MOUNT_PATH`. The image entrypoint
will use `$RAILWAY_VOLUME_MOUNT_PATH/ironclaw-reborn` by default when Railway
exposes a volume mount. Without a volume, Railway deployments using
`local-dev`, `local-dev-yolo`, `hosted-single-tenant`, or
`hosted-single-tenant-volume` fail closed unless
`IRONCLAW_ALLOW_EPHEMERAL_RAILWAY=true` is explicitly set for a
disposable test deployment.

For managed Postgres providers with a small session-pool cap, set
`IRONCLAW_POSTGRES_POOL_MAX_SIZE=1` or `2` rather than relying on the
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

Do not use `IRONCLAW_PROFILE=local-dev-yolo` for a public Railway
listener. That profile grants trusted host access and `serve` refuses to bind it
to a non-loopback host. Use `hosted-single-tenant-volume` for the mounted-volume
single-tenant preview path that keeps the local-dev product surface with durable
libSQL-backed state, or `hosted-single-tenant` for Postgres-backed hosted state.

Set `IRONCLAW_HOME` to a mounted volume path if local files should
survive redeploys. The hosted single-tenant profile stores runtime/control-plane
state, including extension installation/activation state, in Postgres; project
files, materialized system extension packages, and current skill file storage
still live under the local filesystem root. The image default is
`/data/ironclaw-reborn`; without a Railway volume, that path is ephemeral. The
hosted single-tenant volume profile stores runtime/control-plane state under
that Reborn home on the mounted volume and does not require
`IRONCLAW_POSTGRES_URL`. The container workdir is `/workspace` so the
workspace root stays separate from Reborn's state and skill roots.

The image includes `sqlite3` and `psql` for terminal inspection from Railway
shells. Use `sqlite3` for mounted-volume libSQL/SQLite state and `psql` for
`IRONCLAW_POSTGRES_URL` deployments.

To seed a custom config instead of the bundled default, mount it under
`/opt/ironclaw/` and set `IRONCLAW_DEFAULT_CONFIG` to that path. On first
start, the entrypoint copies that file into `$IRONCLAW_HOME/config.toml`;
later starts preserve the existing home config.

For public WebUI Google login, use the Reborn WebUI SSO variables and an HTTPS
base URL that matches the deployed Railway domain users will open. If Railway
exposes more than one domain for the same service, choose one canonical domain
for `IRONCLAW_WEBUI_BASE_URL` and register that same domain in Google:

```bash
IRONCLAW_WEBUI_BASE_URL=https://<railway-domain>
IRONCLAW_WEBUI_GOOGLE_CLIENT_ID=<google-client-id>
IRONCLAW_WEBUI_GOOGLE_CLIENT_SECRET=<google-client-secret>
IRONCLAW_WEBUI_ALLOWED_EMAIL_DOMAINS=near.ai
IRONCLAW_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_WEBUI_USER_ID=reborn-cli
```

Register this WebUI login callback in the Google OAuth client:

```text
https://<railway-domain>/auth/callback/google
```

Notion MCP and other product-auth OAuth setup flows use the same hosted WebUI
base URL for provider callbacks. Set `IRONCLAW_WEBUI_BASE_URL` to the
same public host so product-auth providers see the public callback origin rather
than the local listener address. Google product-auth is separate and still uses
`IRONCLAW_GOOGLE_OAUTH_REDIRECT_URI` explicitly.

Product-auth Google credentials are a separate flow. Configure
`IRONCLAW_GOOGLE_OAUTH_REDIRECT_URI` only when the deployment should let
the agent connect a Google credential:

```bash
IRONCLAW_GOOGLE_OAUTH_REDIRECT_URI=https://<railway-domain>/api/product-auth/oauth/google/callback
```

## Slack

Slack routes are compiled into the image, but they are disabled by the default
config. On Railway, prefer the env toggle so the seeded config can stay
unchanged:

```bash
IRONCLAW_SLACK_ENABLED=true
```

The env var overrides only the Slack route enablement gate. `true`/`1` enables
Slack, while `false`/`0` forces Slack off for the deployment.

You can also enable Slack by editing `$IRONCLAW_HOME/config.toml` or
mounting a config file with:

```toml
[slack]
enabled = true
```

Then configure Slack app ids, the bot token, signing secret, and channel
mappings from WebUI channel setup after the container starts.

Set the WebUI identity environment variables as usual.

Do not store OAuth, Slack, or LLM secrets in `config.toml`. Slack bot tokens
and signing secrets are stored from WebUI channel setup.
