"""libSQL helpers for the workflow-canary lane.

The routine engine fires on its own internal tick — by default routines
are picked up when `next_fire_at <= now()`. Rather than wait for
real wall-clock cron schedules to elapse (would push lane wall-time
to many minutes per scenario), the canary backdates `next_fire_at`
directly in the libSQL DB so the engine fires on its very next tick
(usually within seconds).

This is the same pattern auth-live-seeded uses for `expire_secret_in_db`
in `scripts/auth_live_canary/run_live_canary.py`. Direct DB writes
are appropriate here because:

- The routine engine, telegram tool, and mock telegram are the system
  under test; the agent's NL → routine_create flow is exercised by
  separate tests at the conversation tier.
- Determinism: cron timing tests are otherwise inherently flaky on
  CI under runner load.
- Speed: backdating + a 5 s poll loop replaces a 60 s cron interval.
"""

from __future__ import annotations

import json
import sqlite3
import time
import uuid
from pathlib import Path
from typing import Any


def _now_iso() -> str:
    """ISO-8601 with millisecond precision, the format libSQL uses."""
    # Match `fmt_ts(dt)` in src/db/libsql/mod.rs (RFC 3339, ms precision).
    millis = int(time.time() * 1000)
    secs, ms = divmod(millis, 1000)
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(secs)) + f".{ms:03d}Z"


def _past_iso(seconds_ago: int = 60) -> str:
    millis = int(time.time() * 1000) - seconds_ago * 1000
    secs, ms = divmod(millis, 1000)
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(secs)) + f".{ms:03d}Z"


def insert_lightweight_cron_routine(
    db_path: str | Path,
    *,
    user_id: str,
    name: str,
    prompt: str,
    schedule: str = "*/1 * * * *",
    description: str = "",
    fire_immediately: bool = True,
) -> str:
    """INSERT a new lightweight-cron routine. Returns the routine id.

    The action runs the given prompt through the LLM each fire — for the
    canary's mock LLM, that prompt should match a canned tool-call
    response so the engine deterministically issues the tool call we
    want to verify (e.g. a telegram sendMessage).

    If ``fire_immediately`` is True (the default), ``next_fire_at`` is
    backdated 60 s into the past so the engine picks it up on its very
    next tick instead of waiting for the cron schedule.
    """
    routine_id = str(uuid.uuid4())
    trigger_config = json.dumps({"schedule": schedule, "timezone": "UTC"})
    # action_config is a flat object — `RoutineAction::from_db` reads
    # `prompt`, `context_paths`, `max_tokens`, `use_tools`,
    # `max_tool_rounds` directly from the top-level JSON. The
    # `action_type` column carries the variant tag ("lightweight").
    action_config = json.dumps(
        {
            "prompt": prompt,
            "context_paths": [],
            "max_tokens": 1024,
            "use_tools": True,
            "max_tool_rounds": 3,
        }
    )
    now = _now_iso()
    next_fire_at = _past_iso(60) if fire_immediately else None

    with sqlite3.connect(str(db_path)) as conn:
        conn.execute(
            """
            INSERT INTO routines (
                id, name, description, user_id, enabled,
                trigger_type, trigger_config,
                action_type, action_config,
                cooldown_secs, max_concurrent,
                state, next_fire_at, run_count,
                consecutive_failures, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                routine_id,
                name,
                description,
                user_id,
                1,
                "cron",
                trigger_config,
                "lightweight",
                action_config,
                0,  # cooldown_secs — 0 so back-to-back fires aren't suppressed
                1,
                "{}",
                next_fire_at,
                0,
                0,
                now,
                now,
            ),
        )
        conn.commit()
    return routine_id


def backdate_routine(db_path: str | Path, routine_id: str, seconds_ago: int = 60) -> None:
    """Force a routine to fire on the next engine tick by backdating
    `next_fire_at`. Useful between successive probes in one scenario."""
    with sqlite3.connect(str(db_path)) as conn:
        conn.execute(
            "UPDATE routines SET next_fire_at = ?, updated_at = ? WHERE id = ?",
            (_past_iso(seconds_ago), _now_iso(), routine_id),
        )
        conn.commit()


def get_routine_state(db_path: str | Path, routine_id: str) -> dict[str, Any] | None:
    with sqlite3.connect(str(db_path)) as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT id, name, enabled, run_count, consecutive_failures, "
            "last_run_at, next_fire_at FROM routines WHERE id = ?",
            (routine_id,),
        ).fetchone()
    return dict(row) if row else None


def list_routine_runs(db_path: str | Path, routine_id: str) -> list[dict[str, Any]]:
    with sqlite3.connect(str(db_path)) as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            "SELECT id, status, started_at, completed_at, "
            "result_summary, tokens_used FROM routine_runs WHERE routine_id = ? "
            "ORDER BY started_at DESC",
            (routine_id,),
        ).fetchall()
    return [dict(r) for r in rows]


# RunStatus variants from src/agent/routine.rs:520-540 — running is the
# only non-terminal one. "ok" / "attention" both mean the run completed
# end-to-end (attention = produced output worth surfacing to the user).
TERMINAL_RUN_STATUSES = {"ok", "attention", "failed"}
SUCCESS_RUN_STATUSES = {"ok", "attention"}


async def wait_for_run(
    db_path: str | Path,
    routine_id: str,
    *,
    min_runs: int = 1,
    timeout_secs: float = 30.0,
    poll_interval: float = 0.5,
    require_terminal: bool = True,
) -> list[dict[str, Any]]:
    """Poll routine_runs until at least `min_runs` rows exist with a
    terminal status (or any status if require_terminal=False).

    Raises ``TimeoutError`` if the deadline elapses with fewer matching
    rows. Returns all observed runs (may be more than `min_runs` if
    multiple fired during the wait). The terminal-status check matters
    because the engine inserts the row with status=running before
    actually executing the action — checking `len(runs) >= 1` alone
    races with the executor.
    """
    import asyncio

    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        runs = list_routine_runs(db_path, routine_id)
        if require_terminal:
            terminal_runs = [
                r for r in runs if r.get("status") in TERMINAL_RUN_STATUSES
            ]
            if len(terminal_runs) >= min_runs:
                return runs
        elif len(runs) >= min_runs:
            return runs
        await asyncio.sleep(poll_interval)
    final = list_routine_runs(db_path, routine_id)
    statuses = [r.get("status") for r in final]
    raise TimeoutError(
        f"Timed out waiting for routine {routine_id} to fire "
        f"(observed {len(final)} run(s) with statuses {statuses}, "
        f"expected >= {min_runs} terminal)"
    )
