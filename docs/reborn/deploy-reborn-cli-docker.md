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
`hosted-single-tenant-volume` fail closed unless
`IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY=true` is explicitly set for a
disposable test deployment.

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
to a non-loopback host. Use `hosted-single-tenant-volume` for the mounted-volume
single-tenant preview path that keeps the local-dev product surface with durable
libSQL-backed state, or `hosted-single-tenant` for Postgres-backed hosted state.

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

## Unified Slack one-time cutover and rollback

The #6061 upgrade migrates persisted split-Slack installation records and the
retired Slack credential provider into the unified `slack` extension/provider.
The migration is bounded, idempotent, and runs before product-auth and
extension services are published. It cannot fence a pre-upgrade process that
is still writing. This upgrade is therefore a quiesced cutover, not a rolling
deployment.

Use this runbook once per durable environment. The non-negotiable order is:
block ingress, stop every pre-#6061 replica/worker, take a restorable backup,
start exactly one new replica, verify the runtime (not only `/api/health`), and
only then restore normal replica count and ingress.

### 1. Quiesce every old writer

Put the WebUI and `/webhooks/slack/events` behind maintenance/drain at the
edge, stop scheduled jobs that can write Reborn state, and scale the Reborn
service to zero. On a directly managed host, this check must print no process:

```bash
pgrep -af '[i]ronclaw-reborn' || true
```

For Postgres, inspect remaining sessions after the service is stopped. Do not
continue while an old IronClaw process still owns a session:

```bash
psql "$IRONCLAW_REBORN_POSTGRES_URL" -X -v ON_ERROR_STOP=1 -c \
  "SELECT pid, application_name, client_addr, state, query_start
     FROM pg_stat_activity
    WHERE datname = current_database()
      AND pid <> pg_backend_pid()
    ORDER BY pid"
```

### 2. Capture the pre-cutover restore point

Set a private backup directory on durable storage. Every profile also has
local home state, so archive the Reborn home even for Postgres deployments:

```bash
export CUTOVER_ID="$(date -u +%Y%m%dT%H%M%SZ)"
export CUTOVER_BACKUP_DIR="${CUTOVER_BACKUP_DIR:-$PWD/reborn-cutover-$CUTOVER_ID}"
export IRONCLAW_REBORN_HOME="${IRONCLAW_REBORN_HOME:-/data/ironclaw-reborn}"
install -d -m 0700 "$CUTOVER_BACKUP_DIR"
test -d "$IRONCLAW_REBORN_HOME"
tar -C "$(dirname "$IRONCLAW_REBORN_HOME")" -czf \
  "$CUTOVER_BACKUP_DIR/reborn-home.tgz" \
  "$(basename "$IRONCLAW_REBORN_HOME")"
tar -tzf "$CUTOVER_BACKUP_DIR/reborn-home.tgz" >/dev/null
```

For `hosted-single-tenant`, also create and verify a PostgreSQL archive:

```bash
pg_dump "$IRONCLAW_REBORN_POSTGRES_URL" \
  --format=custom \
  --file="$CUTOVER_BACKUP_DIR/reborn-postgres.dump"
pg_restore --list "$CUTOVER_BACKUP_DIR/reborn-postgres.dump" >/dev/null
```

Copy the backup off the service volume before continuing. Record the old image
digest and the backup location in the deployment change record.

### 3. Start one new replica and prove migration completion

Deploy the #6061 image with exactly one replica. A `200` from `/api/health`
only proves that the listener is alive; hosted Postgres runtime assembly may
still be migrating. Wait for an authenticated runtime route as well:

```bash
export REBORN_BASE_URL="${REBORN_BASE_URL:-https://<public-host>}"
ready=0
for attempt in $(seq 1 60); do
  if curl -fsS \
    -H "Authorization: Bearer $IRONCLAW_REBORN_WEBUI_TOKEN" \
    "$REBORN_BASE_URL/api/webchat/v2/extensions" >/dev/null; then
    ready=1
    break
  fi
  sleep 5
done
test "$ready" -eq 1
```

Check startup logs for migration failure, verify Extensions shows exactly one
Slack entry with tool/auth/channel surfaces, then run one Slack OAuth connect
and one signed Slack DM smoke test. Restore the normal replica count and reopen
ingress only after those checks pass.

### 4. Roll back only by restore, or roll forward

Do not start a pre-#6061 binary against post-migration state. If the new release
cannot remain deployed, keep ingress blocked, stop every new replica, and
choose one of these two paths:

1. Roll forward to a corrected #6061-compatible image. The migrations are
   idempotent and may run again.
2. Restore the complete pre-cutover home and database backups, then restart the
   exact recorded old image.

For a filesystem-backed profile, restore the archived home while the service
is stopped:

```bash
test -f "$CUTOVER_BACKUP_DIR/reborn-home.tgz"
mv "$IRONCLAW_REBORN_HOME" "$IRONCLAW_REBORN_HOME.failed-$CUTOVER_ID"
tar -C "$(dirname "$IRONCLAW_REBORN_HOME")" -xzf \
  "$CUTOVER_BACKUP_DIR/reborn-home.tgz"
```

For `hosted-single-tenant`, restore the database using the provider-approved
maintenance procedure; for a dedicated database with no remaining sessions,
the standard archive restore is:

```bash
pg_restore --clean --if-exists --no-owner \
  --dbname="$IRONCLAW_REBORN_POSTGRES_URL" \
  "$CUTOVER_BACKUP_DIR/reborn-postgres.dump"
```

Restore both database and home from the same cutover backup. Verify the old
runtime before reopening ingress. Never combine an old database with a new
home archive (or the reverse), and never attempt an in-place downgrade of only
the migrated Slack rows.

## Slack

Slack routes are compiled into the image, but they are disabled by the default
config. On Railway, prefer the env toggle so the seeded config can stay
unchanged:

```bash
IRONCLAW_REBORN_SLACK_ENABLED=true
```

The env var overrides only the Slack route enablement gate. `true`/`1` enables
Slack, while `false`/`0` forces Slack off for the deployment.

You can also enable Slack by editing `$IRONCLAW_REBORN_HOME/config.toml` or
mounting a config file with:

```toml
[slack]
enabled = true
```

Then configure Slack app ids, the bot token, signing secret, and channel
mappings from WebUI channel setup after the container starts.

For app scopes, callback URLs, and runtime verification, follow
[`setup-slack-for-reborn-binary.md`](setup-slack-for-reborn-binary.md). For an
upgrade from the split Slack model, complete the one-time cutover runbook above
before exposing the new service.

Set the WebUI identity environment variables as usual.

Do not store OAuth, Slack, or LLM secrets in `config.toml`. Slack bot tokens
and signing secrets are stored from WebUI channel setup.
