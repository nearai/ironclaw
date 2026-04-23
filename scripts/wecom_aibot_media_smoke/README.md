# WeCom AI Bot Media Smoke Test

This script verifies whether the WeCom AI Bot WebSocket API can upload and send
an image without using a self-built WeCom App (`corp_id`, `agent_id`, or app
secret).

It uses the Go AI Bot SDK path:

1. Connect and authenticate with `WECOM_BOT_ID` and `WECOM_BOT_SECRET`.
2. Upload an image through `UploadMedia`.
3. Send the returned `media_id` through `SendMediaMessage`.

## Configuration

Required:

- `WECOM_BOT_ID`
- `WECOM_BOT_SECRET`
- `WECOM_CHAT_ID` or `WECOM_TO`

Optional:

- `WECOM_IMAGE_PATH`, a local image path. If omitted, the script sends a tiny
  generated PNG so the transport can be tested without preparing a file.
- `WECOM_SMOKE_TEXT`, a markdown text probe to send before the image.
- `WECOM_WS_URL`, defaults to the SDK default endpoint.
- `WECOM_AUTH_TIMEOUT_SECS`, defaults to `15`.

Use `WECOM_CHAT_ID` for the `ws_chat_id` captured from an incoming WeCom Bot
message. Use `WECOM_TO` only when testing a known user ID or chat ID directly.

## Run

```bash
cd scripts/wecom_aibot_media_smoke
WECOM_BOT_ID=... \
WECOM_BOT_SECRET=... \
WECOM_CHAT_ID=... \
go run .
```

If another IronClaw gateway instance is connected with the same Bot ID, stop it
before running this script. Two simultaneous WebSocket clients for the same bot
can disconnect each other or make the result ambiguous.
