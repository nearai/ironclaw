# Reborn CLI Docker Deployment

`Dockerfile.reborn` builds the standalone `ironclaw-reborn` binary with the
WebUI v2 and Slack host-beta features enabled. The image defaults to:

```text
ironclaw-reborn serve --host ${IRONCLAW_REBORN_SERVE_HOST:-127.0.0.1} --port ${PORT:-3000}
```

Railway supplies `PORT`; set `IRONCLAW_REBORN_SERVE_HOST=0.0.0.0` for
Railway/public deployments. Local Docker runs can keep the loopback default and
set `IRONCLAW_REBORN_SERVE_PORT=3000`.

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
IRONCLAW_REBORN_SERVE_HOST=127.0.0.1
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

Set the service Dockerfile path to `Dockerfile.reborn`. Railway sets `PORT`;
keep `IRONCLAW_REBORN_SERVE_HOST=0.0.0.0`. The Reborn WebUI service serves
`/api/health` for Railway's healthcheck.

Leave Railway's Start Command empty for the Docker image. The image entrypoint
builds the `ironclaw-reborn serve` arguments from `PORT` and
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

For managed Postgres providers with a small session-pool cap, set
`IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE=1` or `2` rather than relying on the
provider to queue excess sessions.
For `hosted-single-tenant`, `ironclaw-reborn serve` binds the WebUI listener
and serves `/api/health` before PostgreSQL-backed runtime assembly finishes.
Non-health routes return `503` until the runtime router is ready. This lets
Railway drain the old deployment and release PgBouncer session-mode
connections before the new deployment needs one for startup migrations.
`IRONCLAW_FILESYSTEM_POSTGRES_MIGRATION_CONNECT_MAX_WAIT_SECS` still controls
how long runtime assembly waits for PostgreSQL once the healthcheck listener is
up; the default is 5 minutes.

`ironclaw-reborn serve` exits before binding the HTTP listener if the WebUI
token/user variables are missing. The bundled config selects NearAI as the
default LLM provider, so set `NEARAI_API_KEY` unless a custom mounted config
selects a different provider.

Do not use `IRONCLAW_REBORN_PROFILE=local-dev-yolo` for a public Railway
listener. That profile grants trusted host access and `serve` refuses to bind it
to a non-loopback host. Use `hosted-single-tenant` for the preview deployment
path that keeps the local-dev product surface while storing runtime state in
Postgres.

Set `IRONCLAW_REBORN_HOME` to a mounted volume path if local files should
survive redeploys. The hosted single-tenant profile stores runtime/control-plane
state, including extension installation/activation state, in Postgres; project
files, materialized system extension packages, and current skill file storage
still live under the local filesystem root. The image default is
`/data/ironclaw-reborn`; without a Railway volume, that path is ephemeral. The
container workdir is `/workspace` so the workspace root stays separate from
Reborn's state and skill roots.

To seed a custom config instead of the bundled default, mount it under
`/opt/ironclaw/` and set `IRONCLAW_REBORN_DEFAULT_CONFIG` to that path. On first
start, the entrypoint copies that file into `$IRONCLAW_REBORN_HOME/config.toml`;
later starts preserve the existing home config.

For public WebUI Google login, use the Reborn WebUI SSO variables and an HTTPS
base URL that matches the deployed Railway domain:

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

Slack routes are compiled into the image, but they are disabled by the default
config. To enable them, edit `$IRONCLAW_REBORN_HOME/config.toml` or mount a
config file with:

```toml
[slack]
enabled = true
installation_id = "<installation-id>"
team_id = "<slack-team-id>"
api_app_id = "<slack-api-app-id>"
signing_secret_env = "IRONCLAW_REBORN_SLACK_SIGNING_SECRET"
bot_token_env = "IRONCLAW_REBORN_SLACK_BOT_TOKEN"
```

Then set:

```bash
IRONCLAW_REBORN_SLACK_SIGNING_SECRET=<slack-signing-secret>
IRONCLAW_REBORN_SLACK_BOT_TOKEN=<slack-bot-token>
```

Do not store OAuth, Slack, or LLM secrets in `config.toml`; the parser treats
secrets as env-only deployment material.
