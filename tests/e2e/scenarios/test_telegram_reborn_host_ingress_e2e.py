"""Live-sidecar E2E for the Reborn Telegram host-ingress path.

Unlike `test_telegram_e2e.py` (which exercises the *legacy* `ironclaw` web
channel via `/api/extensions/telegram/setup`), this scenario boots the
`ironclaw-reborn serve` binary built with the `telegram-v2-host-beta` feature
and proves the new host-owned ingress surface end-to-end:

- `GET /api/webchat/v2/channels/connectable` advertises Telegram.
- `POST /webhook/telegram` with a missing/wrong `X-Telegram-Bot-Api-Secret-Token`
  is rejected 401 by the host before the adapter parses anything.
- A valid signed private message returns an immediate 200 ACK, runs the turn
  against the mock LLM, and the final reply is delivered host-mediated to the
  fake Telegram API `sendMessage` (the bot token rides only in the egress URL,
  never the adapter).
- A duplicate `update_id` is idempotent.

Boot env mirrors the Reborn v2 smoke fixture; the egress origin is redirected to
the fake Telegram API via `IRONCLAW_TEST_TELEGRAM_API_BASE_URL`.
"""

from __future__ import annotations

import asyncio
import os
import signal
import socket
import subprocess
import sys
from pathlib import Path

import httpx
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from conftest import ROOT, _cargo_target_dir  # noqa: E402
from helpers import wait_for_ready  # noqa: E402

WEBUI_TOKEN = "telegram-reborn-e2e-webui-token-0123456789abcdef"
USER_ID = "local-user"
BOT_TOKEN = "123456789:AA-telegram-reborn-e2e-bot-token"
WEBHOOK_SECRET = "telegram-reborn-e2e-webhook-secret"
SECRET_HEADER = "X-Telegram-Bot-Api-Secret-Token"
WEBHOOK_PATH = "/webhooks/telegram/updates"
CHAT_ID = 4242


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _write_config(path: Path, mock_llm_server: str) -> None:
    path.write_text(
        f"""api_version = "ironclaw.runtime/v1"

[boot]
profile = "local-dev"

[identity]
default_owner = "{USER_ID}"
tenant = "telegram-reborn-e2e"
default_agent = "telegram-reborn-e2e-agent"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "mock-model"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "{mock_llm_server}/v1"

[telegram]
enabled = true
host_ingress_mode = "generic"
installation_id = "telegram-default"
bot_username = "my_bot"
bot_user_id = 123456789
user_id = "{USER_ID}"
bot_token_env = "IRONCLAW_REBORN_TELEGRAM_BOT_TOKEN"
secret_token_env = "IRONCLAW_REBORN_TELEGRAM_SECRET_TOKEN"
""",
        encoding="utf-8",
    )


def _telegram_reborn_binary() -> str:
    """Build `ironclaw-reborn` with the Telegram host feature."""
    binary = _cargo_target_dir() / "debug" / "ironclaw-reborn"
    subprocess.run(
        [
            "cargo", "build",
            "-p", "ironclaw_reborn_cli",
            "--bin", "ironclaw-reborn",
            "--features", "webui-v2-beta,telegram-v2-host-beta",
        ],
        cwd=ROOT,
        check=True,
        timeout=900,
    )
    assert binary.exists(), f"binary not found at {binary}"
    return str(binary)


@pytest.fixture(scope="module")
async def telegram_reborn_server(mock_llm_server, fake_telegram_server, tmp_path_factory):
    binary = _telegram_reborn_binary()
    home = tmp_path_factory.mktemp("telegram-reborn-home")
    _write_config(home / "config.toml", mock_llm_server)
    port = _find_free_port()
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "IRONCLAW_REBORN_HOME": str(home),
        "IRONCLAW_REBORN_PROFILE": "local-dev",
        "IRONCLAW_REBORN_WEBUI_TOKEN": WEBUI_TOKEN,
        "IRONCLAW_REBORN_WEBUI_USER_ID": USER_ID,
        "IRONCLAW_REBORN_TELEGRAM_BOT_TOKEN": BOT_TOKEN,
        "IRONCLAW_REBORN_TELEGRAM_SECRET_TOKEN": WEBHOOK_SECRET,
        "MOCK_LLM_API_KEY": "dummy",
        # Redirect host-mediated Bot API egress to the fake Telegram API.
        "IRONCLAW_TEST_TELEGRAM_API_BASE_URL": fake_telegram_server,
    }
    log_path = home / "serve.log"
    with log_path.open("wb") as log:
        proc = await asyncio.create_subprocess_exec(
            binary, "serve", "--host", "127.0.0.1", "--port", str(port),
            stdin=asyncio.subprocess.DEVNULL,
            stdout=log,
            stderr=log,
            env=env,
        )
    base_url = f"http://127.0.0.1:{port}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=90)
        yield {"base_url": base_url, "fake_tg_url": fake_telegram_server}
    finally:
        proc.send_signal(signal.SIGINT)
        try:
            await asyncio.wait_for(proc.wait(), timeout=10)
        except asyncio.TimeoutError:
            proc.kill()


def _private_message_update(update_id: int, text: str) -> dict:
    return {
        "update_id": update_id,
        "message": {
            "message_id": 5,
            "from": {"id": 99, "is_bot": False, "first_name": "Alice"},
            "chat": {"id": CHAT_ID, "type": "private"},
            "date": 1_700_000_000,
            "text": text,
        },
    }


@pytest.mark.asyncio
async def test_connectable_advertises_telegram(telegram_reborn_server):
    base_url = telegram_reborn_server["base_url"]
    async with httpx.AsyncClient() as client:
        resp = await client.get(
            f"{base_url}/api/webchat/v2/channels/connectable",
            headers={"Authorization": f"Bearer {WEBUI_TOKEN}"},
            timeout=10,
        )
    assert resp.status_code == 200, resp.text
    channels = resp.json().get("channels", [])
    telegram = [c for c in channels if c.get("channel") == "telegram"]
    assert len(telegram) == 1, channels
    assert telegram[0]["strategy"] == "inbound_proof_code"


@pytest.mark.asyncio
async def test_webhook_rejects_missing_and_wrong_secret(telegram_reborn_server):
    base_url = telegram_reborn_server["base_url"]
    update = _private_message_update(700001, "hello")
    async with httpx.AsyncClient() as client:
        missing = await client.post(f"{base_url}{WEBHOOK_PATH}", json=update, timeout=10)
        wrong = await client.post(
            f"{base_url}{WEBHOOK_PATH}",
            json=update,
            headers={SECRET_HEADER: "not-the-secret"},
            timeout=10,
        )
    assert missing.status_code == 401, missing.text
    assert wrong.status_code == 401, wrong.text


@pytest.mark.asyncio
async def test_valid_webhook_acks_and_delivers_final_reply(telegram_reborn_server):
    base_url = telegram_reborn_server["base_url"]
    fake_tg_url = telegram_reborn_server["fake_tg_url"]
    update = _private_message_update(700100, "ping from the reborn telegram e2e")

    async with httpx.AsyncClient() as client:
        ack = await client.post(
            f"{base_url}{WEBHOOK_PATH}",
            json=update,
            headers={SECRET_HEADER: WEBHOOK_SECRET},
            timeout=10,
        )
        assert ack.status_code == 200, ack.text

        # The turn runs against the mock LLM; the final-reply observer delivers
        # host-mediated to the fake Telegram API. Poll the fake API's outbox.
        delivered = None
        for _ in range(60):
            sent = await client.get(f"{fake_tg_url}/__mock/sent_messages", timeout=10)
            messages = sent.json().get("messages", [])
            match = [m for m in messages if str(m.get("chat_id")) == str(CHAT_ID)]
            if match:
                delivered = match[-1]
                break
            await asyncio.sleep(0.5)

    assert delivered is not None, "final reply was never delivered to the fake Telegram API"
    assert delivered.get("text"), "delivered Telegram message had no text body"


@pytest.mark.asyncio
async def test_duplicate_update_id_is_idempotent(telegram_reborn_server):
    base_url = telegram_reborn_server["base_url"]
    update = _private_message_update(700200, "idempotency check")
    async with httpx.AsyncClient() as client:
        first = await client.post(
            f"{base_url}{WEBHOOK_PATH}",
            json=update,
            headers={SECRET_HEADER: WEBHOOK_SECRET},
            timeout=10,
        )
        second = await client.post(
            f"{base_url}{WEBHOOK_PATH}",
            json=update,
            headers={SECRET_HEADER: WEBHOOK_SECRET},
            timeout=10,
        )
    assert first.status_code == 200, first.text
    assert second.status_code == 200, second.text
