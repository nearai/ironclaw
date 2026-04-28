"""Shared scenario plumbing for workflow-canary scripts.

Each scenario inserts a Lightweight cron routine with a prompt that
includes a ``[CANARY-WORKFLOW-<key>]`` sentinel, backdates
``next_fire_at``, and polls ``routine_runs`` for terminal status.

Phase 1A coverage (default ``verify_telegram=False``):
- Routine engine cron-tick path (`spawn_cron_ticker` → `check_cron_triggers`)
- Lightweight routine action execution (`RoutineAction::Lightweight`)
- DB-backed routine state machine (`routines.next_fire_at` →
  `routine_runs.status`)

Phase 1B (opt-in ``verify_telegram=True``) attempts to additionally
verify the mock Telegram bot received an ack message via the http
tool's remapped sendMessage call. The ``[CANARY-WORKFLOW-<key>]``
sentinel triggers a deterministic http tool call from the mock LLM
(see ``TOOL_CALL_PATTERNS`` in tests/e2e/mock_llm.py).

Phase 1B is currently **gated off by default** because
``IRONCLAW_TEST_HTTP_REMAP`` does not propagate from the gateway
env into the ``ToolContext.http_interceptor`` slot used by
routine-driven http tool dispatches — the routine engine appears to
build a fresh ``ToolContext`` without the global interceptor wired
in. Surface this as a follow-up bug in the routine engine's lightweight
action context construction; once fixed, flip the default and the
existing scenarios pick up real send-side verification with no
further changes here.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import (
    SUCCESS_RUN_STATUSES,
    insert_lightweight_cron_routine,
    list_routine_runs,
    wait_for_run,
)


async def _capture_telegram_messages(
    mock_telegram_url: str,
) -> list[dict[str, Any]]:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.get(f"{mock_telegram_url}/__mock/sent_messages")
        response.raise_for_status()
        return response.json().get("messages", [])


def _scenario_key(mode: str) -> str:
    """The scenario key embedded in the prompt sentinel and asserted on
    the mock-telegram side. Lowercased so the regex matcher and the
    scenario assertion don't disagree on case."""
    return mode.lower().replace(" ", "_")


async def run_routine_probe(
    *,
    stack: Any,
    mock_telegram_url: str | None = None,
    provider: str,
    mode: str,
    routine_name: str,
    prompt_intro: str,
    description: str = "",
    schedule: str = "*/1 * * * *",
    timeout_secs: float = 60.0,
    verify_telegram: bool = False,
    extra_details: dict[str, Any] | None = None,
) -> ProbeResult:
    """Insert a lightweight cron routine, fire it, verify the engine
    + mock-LLM round-trip + (optionally) the mock-telegram capture.

    Caller supplies:
      - ``provider`` / ``mode`` — labels surfaced in results.json
      - ``routine_name`` — DB unique-constraint key (must be unique
        per probe within one stack)
      - ``prompt_intro`` — leading natural-language description of
        the script's intent. The ``[CANARY-WORKFLOW-<key>]`` sentinel
        is appended automatically so every scenario hits the shared
        TOOL_CALL_PATTERNS matcher in mock_llm.py.
      - ``verify_telegram`` — when True (the default), require the
        mock telegram bot to have received the expected ack message
        before declaring success. When False, only the routine
        terminal status is checked.
    """
    db_path = stack.db_path
    owner_user_id = "workflow-canary-owner"
    started = time.perf_counter()
    extra = extra_details or {}

    key = _scenario_key(mode)
    expected_text = f"[canary-workflow:{key}] ack"
    prompt = f"{prompt_intro}\n\n[CANARY-WORKFLOW-{key}]"

    try:
        routine_id = insert_lightweight_cron_routine(
            db_path,
            user_id=owner_user_id,
            name=routine_name,
            prompt=prompt,
            schedule=schedule,
            description=description,
            fire_immediately=True,
        )

        runs = await wait_for_run(
            db_path, routine_id, min_runs=1, timeout_secs=timeout_secs
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        # Verify the mock Telegram bot received our ack message. The
        # routine fires the lightweight action, mock LLM emits an http
        # tool call to api.telegram.org/.../sendMessage with the
        # expected text, IRONCLAW_TEST_HTTP_REMAP routes it to
        # mock_telegram, which records it.
        telegram_match: dict[str, Any] | None = None
        if verify_telegram and run_terminal and mock_telegram_url:
            # The tool dispatch happens inside the routine's lightweight
            # action loop, which completes before routine_runs reaches
            # terminal status — so the message should already be there
            # by the time we get here. A short retry handles any tail
            # latency from request → mock recording.
            import asyncio

            for _ in range(10):
                messages = await _capture_telegram_messages(mock_telegram_url)
                for m in messages:
                    if (
                        m.get("method") == "sendMessage"
                        and expected_text in (m.get("text") or "")
                    ):
                        telegram_match = m
                        break
                if telegram_match is not None:
                    break
                await asyncio.sleep(0.5)

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = run_terminal and (
            (not verify_telegram) or telegram_match is not None
        )

        details = {
            "routine_id": routine_id,
            "run_status": last_run["status"],
            "run_count": len(runs),
            "result_summary": last_run.get("result_summary"),
            "expected_text": expected_text,
            "telegram_match": (
                {
                    "chat_id": telegram_match.get("chat_id"),
                    "text": telegram_match.get("text"),
                }
                if telegram_match
                else None
            ),
            **extra,
        }
        return ProbeResult(
            provider=provider,
            mode=mode,
            success=success,
            latency_ms=latency_ms,
            details=details,
        )
    except TimeoutError as exc:
        latency_ms = int((time.perf_counter() - started) * 1000)
        observed = (
            list_routine_runs(db_path, locals().get("routine_id", ""))
            if "routine_id" in locals()
            else []
        )
        return ProbeResult(
            provider=provider,
            mode=mode,
            success=False,
            latency_ms=latency_ms,
            details={
                "error": f"timeout: {exc}",
                "observed_runs": len(observed),
                "observed_statuses": [r["status"] for r in observed],
                **extra,
            },
        )
    except Exception as exc:  # noqa: BLE001
        latency_ms = int((time.perf_counter() - started) * 1000)
        return ProbeResult(
            provider=provider,
            mode=mode,
            success=False,
            latency_ms=latency_ms,
            details={
                "error": f"{type(exc).__name__}: {exc}",
                **extra,
            },
        )
