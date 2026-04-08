#!/usr/bin/env python3
"""Local Telegram smoke runner for pre-release validation.

Runs a small set of real Telegram flows against an already-running IronClaw
instance configured with the Telegram channel.

This script logs in as a human Telegram user through Telethon and sends
messages/files to a dedicated test bot and optional test group.
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
import tempfile
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Awaitable, Callable

import httpx
from telethon import TelegramClient
from telethon.errors import RPCError


DEFAULT_TIMEOUT_SECS = 45.0
DEFAULT_POLL_INTERVAL_SECS = 1.0
DEFAULT_CASES = ("dm", "edit", "attachment")
GROUP_CASES = ("group",)


class SmokeError(RuntimeError):
    """A smoke case failed."""


@dataclass(frozen=True)
class SmokeConfig:
    api_id: int
    api_hash: str
    session: str
    bot_target: str | int
    dm_target: str | int
    group_target: str | int | None
    expect_substring: str | None
    timeout_secs: float
    poll_interval_secs: float
    healthcheck_url: str | None


def env_str(name: str, default: str | None = None) -> str | None:
    value = os.environ.get(name, default)
    if value is None:
        return None
    value = value.strip()
    return value or None


def parse_target(raw: str | None) -> str | int | None:
    if raw is None:
        return None
    if raw.lstrip("-").isdigit():
        return int(raw)
    return raw


def load_config() -> SmokeConfig:
    api_id_raw = env_str("TG_SMOKE_API_ID")
    api_hash = env_str("TG_SMOKE_API_HASH")
    session = env_str("TG_SMOKE_SESSION") or "scripts/telegram_smoke/.sessions/release-smoke"
    bot_username = env_str("TG_SMOKE_BOT_USERNAME")
    bot_user_id = parse_target(env_str("TG_SMOKE_BOT_USER_ID"))
    dm_target = parse_target(env_str("TG_SMOKE_DM_TARGET"))
    group_target = parse_target(env_str("TG_SMOKE_GROUP_TARGET"))
    expect_substring = env_str("TG_SMOKE_EXPECT_SUBSTRING")
    healthcheck_url = env_str("TG_SMOKE_HEALTHCHECK_URL")
    timeout_secs = float(env_str("TG_SMOKE_TIMEOUT_SECS") or DEFAULT_TIMEOUT_SECS)
    poll_interval_secs = float(
        env_str("TG_SMOKE_POLL_INTERVAL_SECS") or DEFAULT_POLL_INTERVAL_SECS
    )

    if not api_id_raw:
        raise SmokeError("TG_SMOKE_API_ID is required")
    if not api_hash:
        raise SmokeError("TG_SMOKE_API_HASH is required")
    if bot_username is None and bot_user_id is None:
        raise SmokeError("Set TG_SMOKE_BOT_USERNAME or TG_SMOKE_BOT_USER_ID")

    try:
        api_id = int(api_id_raw)
    except ValueError as exc:
        raise SmokeError("TG_SMOKE_API_ID must be an integer") from exc

    bot_target = parse_target(bot_username) if bot_username is not None else bot_user_id
    assert bot_target is not None
    if dm_target is None:
        dm_target = bot_target

    return SmokeConfig(
        api_id=api_id,
        api_hash=api_hash,
        session=session,
        bot_target=bot_target,
        dm_target=dm_target,
        group_target=group_target,
        expect_substring=expect_substring,
        timeout_secs=timeout_secs,
        poll_interval_secs=poll_interval_secs,
        healthcheck_url=healthcheck_url,
    )


async def check_health(url: str) -> None:
    async with httpx.AsyncClient(timeout=10.0) as client:
        response = await client.get(url)
        response.raise_for_status()


async def poll_for_reply(
    client: TelegramClient,
    *,
    entity: str | int,
    min_id: int,
    timeout_secs: float,
    poll_interval_secs: float,
    bot_id: int,
    expect_substring: str | None,
    require_reply_to: int | None = None,
) -> object:
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        async for message in client.iter_messages(entity, limit=10):
            if message.id <= min_id:
                break
            if message.sender_id != bot_id:
                continue
            if require_reply_to is not None:
                reply_to = getattr(message, "reply_to", None)
                reply_to_msg_id = getattr(reply_to, "reply_to_msg_id", None)
                if reply_to_msg_id != require_reply_to:
                    continue

            text = (message.raw_text or "").strip()
            if expect_substring and expect_substring not in text:
                continue
            return message
        await asyncio.sleep(poll_interval_secs)

    suffix = f" containing substring {expect_substring!r}" if expect_substring else ""
    raise SmokeError(f"Timed out waiting for bot reply{suffix}")


async def run_dm_case(client: TelegramClient, cfg: SmokeConfig, bot_id: int) -> None:
    run_id = uuid.uuid4().hex[:8]
    text = f"release smoke dm {run_id}"
    sent = await client.send_message(cfg.dm_target, text)
    reply = await poll_for_reply(
        client,
        entity=cfg.dm_target,
        min_id=sent.id,
        timeout_secs=cfg.timeout_secs,
        poll_interval_secs=cfg.poll_interval_secs,
        bot_id=bot_id,
        expect_substring=cfg.expect_substring,
    )
    print(f"PASS dm: sent={sent.id} reply={reply.id}")


async def run_edit_case(client: TelegramClient, cfg: SmokeConfig, bot_id: int) -> None:
    run_id = uuid.uuid4().hex[:8]
    sent = await client.send_message(cfg.dm_target, f"release smoke edit {run_id} draft")
    await asyncio.sleep(1.0)
    edited = await client.edit_message(
        cfg.dm_target,
        sent,
        f"release smoke edit {run_id} final",
    )
    reply = await poll_for_reply(
        client,
        entity=cfg.dm_target,
        min_id=edited.id,
        timeout_secs=cfg.timeout_secs,
        poll_interval_secs=cfg.poll_interval_secs,
        bot_id=bot_id,
        expect_substring=cfg.expect_substring,
    )
    print(f"PASS edit: sent={edited.id} reply={reply.id}")


async def run_attachment_case(client: TelegramClient, cfg: SmokeConfig, bot_id: int) -> None:
    run_id = uuid.uuid4().hex[:8]
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as tmp:
        tmp.write(f"ironclaw telegram smoke attachment {run_id}\n")
        attachment_path = Path(tmp.name)

    try:
        sent = await client.send_file(
            cfg.dm_target,
            file=str(attachment_path),
            caption=f"release smoke attachment {run_id}",
        )
        reply = await poll_for_reply(
            client,
            entity=cfg.dm_target,
            min_id=sent.id,
            timeout_secs=cfg.timeout_secs,
            poll_interval_secs=cfg.poll_interval_secs,
            bot_id=bot_id,
            expect_substring=cfg.expect_substring,
        )
        print(f"PASS attachment: sent={sent.id} reply={reply.id}")
    finally:
        attachment_path.unlink(missing_ok=True)


async def run_group_case(client: TelegramClient, cfg: SmokeConfig, bot_id: int) -> None:
    if cfg.group_target is None:
        print("SKIP group: TG_SMOKE_GROUP_TARGET is not configured")
        return

    run_id = uuid.uuid4().hex[:8]
    bot_username = str(cfg.bot_target)
    if not bot_username.startswith("@"):
        raise SmokeError(
            "Group smoke requires TG_SMOKE_BOT_USERNAME so the bot can be mentioned"
        )

    sent = await client.send_message(
        cfg.group_target,
        f"{bot_username} release smoke group {run_id}",
    )
    reply = await poll_for_reply(
        client,
        entity=cfg.group_target,
        min_id=sent.id,
        timeout_secs=cfg.timeout_secs,
        poll_interval_secs=cfg.poll_interval_secs,
        bot_id=bot_id,
        expect_substring=cfg.expect_substring,
        require_reply_to=sent.id,
    )
    print(f"PASS group: sent={sent.id} reply={reply.id}")


CASE_HANDLERS: dict[str, Callable[[TelegramClient, SmokeConfig, int], Awaitable[None]]] = {
    "dm": run_dm_case,
    "edit": run_edit_case,
    "attachment": run_attachment_case,
    "group": run_group_case,
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--case",
        action="append",
        choices=sorted(CASE_HANDLERS),
        help="Smoke case to run. May be specified multiple times. Defaults to dm/edit/attachment.",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="Run dm, edit, attachment, and group (if configured).",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="Print the available smoke cases and exit.",
    )
    return parser.parse_args()


async def async_main() -> int:
    args = parse_args()
    if args.list_cases:
        print("Available cases:", ", ".join(sorted(CASE_HANDLERS)))
        return 0

    cfg = load_config()
    session_path = Path(cfg.session)
    session_path.parent.mkdir(parents=True, exist_ok=True)

    if cfg.healthcheck_url:
        print(f"Checking IronClaw health at {cfg.healthcheck_url} ...")
        await check_health(cfg.healthcheck_url)

    selected_cases = tuple(args.case or ())
    if args.all:
        selected_cases = DEFAULT_CASES + GROUP_CASES
    elif not selected_cases:
        selected_cases = DEFAULT_CASES

    print(f"Using Telethon session: {session_path}")
    async with TelegramClient(str(session_path), cfg.api_id, cfg.api_hash) as client:
        await client.start()

        bot_entity = await client.get_entity(cfg.bot_target)
        bot_id = bot_entity.id

        failures: list[str] = []
        for case in selected_cases:
            handler = CASE_HANDLERS[case]
            print(f"Running {case} ...")
            try:
                await handler(client, cfg, bot_id)
            except (RPCError, SmokeError, httpx.HTTPError) as exc:
                failures.append(f"{case}: {exc}")
                print(f"FAIL {case}: {exc}")

        if failures:
            print("\nSmoke failures:")
            for failure in failures:
                print(f"  - {failure}")
            return 1

    print("\nAll requested Telegram smoke cases passed.")
    return 0


def main() -> int:
    try:
        return asyncio.run(async_main())
    except KeyboardInterrupt:
        print("\nInterrupted.", file=sys.stderr)
        return 130
    except SmokeError as exc:
        print(f"Configuration error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
