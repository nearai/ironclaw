#!/usr/bin/env python3
"""Local Slack smoke runner for pre-release validation.

Runs a small set of real Slack flows against an already-running IronClaw
instance configured with the Slack channel.

This script uses two Slack tokens:
  - User token (xoxp-): sends messages as a real Slack user to trigger webhooks
  - Bot token (xoxb-): reads conversation history to find the bot's replies
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
from slack_sdk import WebClient
from slack_sdk.errors import SlackApiError


DEFAULT_TIMEOUT_SECS = 45.0
DEFAULT_POLL_INTERVAL_SECS = 1.0
DEFAULT_CASES = ("dm", "attachment", "thread")
MENTION_CASES = ("mention",)


class SmokeError(RuntimeError):
    """A smoke case failed."""


@dataclass(frozen=True)
class SmokeConfig:
    bot_token: str
    user_token: str
    bot_user_id: str
    dm_channel: str
    public_channel: str | None
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


def load_config() -> SmokeConfig:
    bot_token = env_str("SLACK_SMOKE_BOT_TOKEN")
    user_token = env_str("SLACK_SMOKE_USER_TOKEN")
    bot_user_id = env_str("SLACK_SMOKE_BOT_USER_ID")
    dm_channel = env_str("SLACK_SMOKE_DM_CHANNEL")
    public_channel = env_str("SLACK_SMOKE_PUBLIC_CHANNEL")
    expect_substring = env_str("SLACK_SMOKE_EXPECT_SUBSTRING")
    healthcheck_url = env_str("SLACK_SMOKE_HEALTHCHECK_URL")
    timeout_secs = float(env_str("SLACK_SMOKE_TIMEOUT_SECS") or DEFAULT_TIMEOUT_SECS)
    poll_interval_secs = float(
        env_str("SLACK_SMOKE_POLL_INTERVAL_SECS") or DEFAULT_POLL_INTERVAL_SECS
    )

    if not bot_token:
        raise SmokeError("SLACK_SMOKE_BOT_TOKEN is required")
    if not user_token:
        raise SmokeError("SLACK_SMOKE_USER_TOKEN is required")
    if not bot_user_id:
        raise SmokeError("SLACK_SMOKE_BOT_USER_ID is required")
    if not dm_channel:
        raise SmokeError("SLACK_SMOKE_DM_CHANNEL is required")

    return SmokeConfig(
        bot_token=bot_token,
        user_token=user_token,
        bot_user_id=bot_user_id,
        dm_channel=dm_channel,
        public_channel=public_channel,
        expect_substring=expect_substring,
        timeout_secs=timeout_secs,
        poll_interval_secs=poll_interval_secs,
        healthcheck_url=healthcheck_url,
    )


async def check_health(url: str) -> None:
    async with httpx.AsyncClient(timeout=10.0) as client:
        response = await client.get(url)
        response.raise_for_status()


def poll_for_reply(
    bot_client: WebClient,
    *,
    channel: str,
    oldest: str,
    timeout_secs: float,
    poll_interval_secs: float,
    bot_user_id: str,
    expect_substring: str | None,
    thread_ts: str | None = None,
) -> dict:
    """Poll conversations.history or conversations.replies for a bot reply."""
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        try:
            if thread_ts:
                result = bot_client.conversations_replies(
                    channel=channel, ts=thread_ts, oldest=oldest, limit=20
                )
                messages = result.get("messages", [])
            else:
                result = bot_client.conversations_history(
                    channel=channel, oldest=oldest, limit=20
                )
                messages = result.get("messages", [])

            for msg in messages:
                if msg.get("user") != bot_user_id:
                    continue
                text = (msg.get("text") or "").strip()
                if expect_substring and expect_substring not in text:
                    continue
                return msg
        except SlackApiError as e:
            print(f"  Slack API error during poll: {e}", file=sys.stderr)

        time.sleep(poll_interval_secs)

    suffix = f" containing '{expect_substring}'" if expect_substring else ""
    raise SmokeError(f"Timed out waiting for bot reply{suffix}")


def run_dm_case(
    user_client: WebClient,
    bot_client: WebClient,
    cfg: SmokeConfig,
) -> None:
    run_id = uuid.uuid4().hex[:8]
    text = f"release smoke dm {run_id}"
    sent = user_client.chat_postMessage(channel=cfg.dm_channel, text=text)
    sent_ts = sent["ts"]

    reply = poll_for_reply(
        bot_client,
        channel=cfg.dm_channel,
        oldest=sent_ts,
        timeout_secs=cfg.timeout_secs,
        poll_interval_secs=cfg.poll_interval_secs,
        bot_user_id=cfg.bot_user_id,
        expect_substring=cfg.expect_substring,
    )
    print(f"PASS dm: sent_ts={sent_ts} reply_ts={reply['ts']}")


def run_mention_case(
    user_client: WebClient,
    bot_client: WebClient,
    cfg: SmokeConfig,
) -> None:
    if cfg.public_channel is None:
        print("SKIP mention: SLACK_SMOKE_PUBLIC_CHANNEL is not configured")
        return

    run_id = uuid.uuid4().hex[:8]
    text = f"<@{cfg.bot_user_id}> release smoke mention {run_id}"
    sent = user_client.chat_postMessage(channel=cfg.public_channel, text=text)
    sent_ts = sent["ts"]

    reply = poll_for_reply(
        bot_client,
        channel=cfg.public_channel,
        oldest=sent_ts,
        timeout_secs=cfg.timeout_secs,
        poll_interval_secs=cfg.poll_interval_secs,
        bot_user_id=cfg.bot_user_id,
        expect_substring=cfg.expect_substring,
        thread_ts=sent_ts,
    )
    print(f"PASS mention: sent_ts={sent_ts} reply_ts={reply['ts']}")


def run_attachment_case(
    user_client: WebClient,
    bot_client: WebClient,
    cfg: SmokeConfig,
) -> None:
    run_id = uuid.uuid4().hex[:8]
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as tmp:
        tmp.write(f"ironclaw slack smoke attachment {run_id}\n")
        attachment_path = Path(tmp.name)

    try:
        sent = user_client.files_upload_v2(
            channel=cfg.dm_channel,
            file=str(attachment_path),
            title=f"smoke-{run_id}.txt",
            initial_comment=f"release smoke attachment {run_id}",
        )
        # files_upload_v2 returns file info, get the message ts from shares
        file_info = sent.get("file", {})
        shares = file_info.get("shares", {})
        # Find the ts from DM channel shares
        dm_shares = shares.get("private", {}).get(cfg.dm_channel, [])
        if dm_shares:
            sent_ts = dm_shares[0]["ts"]
        else:
            # Fallback: use current time minus a second
            sent_ts = str(time.time() - 1)

        reply = poll_for_reply(
            bot_client,
            channel=cfg.dm_channel,
            oldest=sent_ts,
            timeout_secs=cfg.timeout_secs,
            poll_interval_secs=cfg.poll_interval_secs,
            bot_user_id=cfg.bot_user_id,
            expect_substring=cfg.expect_substring,
        )
        print(f"PASS attachment: reply_ts={reply['ts']}")
    finally:
        attachment_path.unlink(missing_ok=True)


def run_thread_case(
    user_client: WebClient,
    bot_client: WebClient,
    cfg: SmokeConfig,
) -> None:
    run_id = uuid.uuid4().hex[:8]
    # Send initial message and wait for reply
    text = f"release smoke thread {run_id}"
    sent = user_client.chat_postMessage(channel=cfg.dm_channel, text=text)
    sent_ts = sent["ts"]

    reply = poll_for_reply(
        bot_client,
        channel=cfg.dm_channel,
        oldest=sent_ts,
        timeout_secs=cfg.timeout_secs,
        poll_interval_secs=cfg.poll_interval_secs,
        bot_user_id=cfg.bot_user_id,
        expect_substring=cfg.expect_substring,
    )
    reply_ts = reply["ts"]

    # Now reply in thread
    thread_text = f"release smoke thread reply {run_id}"
    thread_sent = user_client.chat_postMessage(
        channel=cfg.dm_channel, text=thread_text, thread_ts=sent_ts
    )
    thread_sent_ts = thread_sent["ts"]

    thread_reply = poll_for_reply(
        bot_client,
        channel=cfg.dm_channel,
        oldest=thread_sent_ts,
        timeout_secs=cfg.timeout_secs,
        poll_interval_secs=cfg.poll_interval_secs,
        bot_user_id=cfg.bot_user_id,
        expect_substring=cfg.expect_substring,
        thread_ts=sent_ts,
    )
    print(
        f"PASS thread: sent_ts={sent_ts} reply_ts={reply_ts} "
        f"thread_reply_ts={thread_reply['ts']}"
    )


CASE_HANDLERS: dict[
    str,
    Callable[[WebClient, WebClient, SmokeConfig], None],
] = {
    "dm": run_dm_case,
    "mention": run_mention_case,
    "attachment": run_attachment_case,
    "thread": run_thread_case,
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--case",
        action="append",
        choices=sorted(CASE_HANDLERS),
        help="Smoke case to run. May be specified multiple times. Defaults to dm/attachment/thread.",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="Run dm, attachment, thread, and mention (if configured).",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="Print the available smoke cases and exit.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.list_cases:
        print("Available cases:", ", ".join(sorted(CASE_HANDLERS)))
        return 0

    cfg = load_config()

    if cfg.healthcheck_url:
        print(f"Checking IronClaw health at {cfg.healthcheck_url} ...")
        asyncio.run(check_health(cfg.healthcheck_url))

    selected_cases = tuple(args.case or ())
    if args.all:
        selected_cases = DEFAULT_CASES + MENTION_CASES
    elif not selected_cases:
        selected_cases = DEFAULT_CASES

    user_client = WebClient(token=cfg.user_token)
    bot_client = WebClient(token=cfg.bot_token)

    failures: list[str] = []
    for case in selected_cases:
        handler = CASE_HANDLERS[case]
        print(f"Running {case} ...")
        try:
            handler(user_client, bot_client, cfg)
        except (SlackApiError, SmokeError, httpx.HTTPError) as exc:
            failures.append(f"{case}: {exc}")
            print(f"FAIL {case}: {exc}")

    if failures:
        print("\nSmoke failures:")
        for failure in failures:
            print(f"  - {failure}")
        return 1

    print("\nAll requested Slack smoke cases passed.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("\nInterrupted.", file=sys.stderr)
        raise SystemExit(130)
    except SmokeError as exc:
        print(f"Configuration error: {exc}", file=sys.stderr)
        raise SystemExit(2)
