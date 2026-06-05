# Reborn CLI Docker Deployment

`Dockerfile.reborn` builds the standalone `ironclaw-reborn` binary with the
WebUI v2 and Slack host-beta features enabled. The image defaults to:

```text
ironclaw-reborn serve --host ${IRONCLAW_REBORN_SERVE_HOST:-0.0.0.0} --port ${PORT:-3000}
```

Railway supplies `PORT`; local Docker runs can set
`IRONCLAW_REBORN_SERVE_HOST=127.0.0.1` and
`IRONCLAW_REBORN_SERVE_PORT=3000`.

## Build

```bash
docker build -f Dockerfile.reborn -t ironclaw-reborn:local .
```

## Local Run

Create an env file outside git, then run:

```bash
docker run --rm \
  --env-file .env.reborn \
  -p 3000:3000 \
  ironclaw-reborn:local
```

Minimum local env shape:

```bash
IRONCLAW_REBORN_SERVE_HOST=127.0.0.1
IRONCLAW_REBORN_SERVE_PORT=3000
IRONCLAW_REBORN_PROFILE=local-dev-yolo
IRONCLAW_REBORN_CONFIRM_HOST_ACCESS=1
IRONCLAW_REBORN_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli
LLM_BACKEND=nearai
NEARAI_BASE_URL=https://cloud-api.near.ai
NEARAI_API_KEY=<nearai-api-key>
NEARAI_MODEL=anthropic/claude-sonnet-4-5
```

Google product-auth setup:

```bash
IRONCLAW_REBORN_GOOGLE_CLIENT_ID=<google-client-id>
IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET=<google-client-secret>
IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI=http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback
```

WebUI Google login setup:

```bash
IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID=<google-client-id>
IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET=<google-client-secret>
IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS=near.ai
```

## Railway

Set the service Dockerfile path to `Dockerfile.reborn`. Railway sets `PORT`;
keep `IRONCLAW_REBORN_SERVE_HOST=0.0.0.0`.

Do not use `IRONCLAW_REBORN_PROFILE=local-dev-yolo` for a public Railway
listener. That profile grants trusted host access and `serve` refuses to bind it
to a non-loopback host. Use the default `local-dev` container config until the
production Reborn deployment profile is fully wired for this service.

Set `IRONCLAW_REBORN_HOME` to a mounted volume path if state should survive
redeploys. The image default is `/data/ironclaw-reborn`; without a Railway
volume, that path is ephemeral.

For public WebUI Google login, use HTTPS callback URLs that match the deployed
Railway domain:

```bash
IRONCLAW_REBORN_WEBUI_BASE_URL=https://<railway-domain>
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
