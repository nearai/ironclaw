"""Typed durable evidence used by scheduled-routine live canaries."""

from __future__ import annotations

import hashlib
import json
import re
import sqlite3
import asyncio
import time
from contextlib import closing
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Awaitable, Callable, Mapping
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError


def evidence_hash(value: object) -> str:
    return hashlib.sha256(str(value).encode("utf-8", errors="replace")).hexdigest()


_SELF_DELIVERY_ROUTING = re.compile(
    r"(?i)\b(?:send|post|deliver)\b.{0,80}\b(?:me|requester|result|slack\s+dm)\b"
    r"|\bslack\s+dm\b.{0,80}\b(?:send|post|deliver|result)\b"
)


def trigger_prompt_has_self_delivery_routing(prompt: str) -> bool:
    return _SELF_DELIVERY_ROUTING.search(prompt) is not None


@dataclass(frozen=True)
class TriggerKey:
    tenant_id: str
    trigger_id: str


@dataclass(frozen=True)
class TriggerRecordEvidence:
    key: TriggerKey
    name: str = field(repr=False)
    prompt: str = field(repr=False)
    schedule_kind: str
    next_run_at: str | None
    delivery_target: str | None = field(repr=False)
    schedule_expression: str = field(default="", repr=False)
    schedule_timezone: str = ""
    schedule_at: str | None = field(default=None, repr=False)

    @property
    def schedule(self) -> "TriggerScheduleEvidence":
        return TriggerScheduleEvidence(
            kind=self.schedule_kind,
            expression=self.schedule_expression,
            timezone=self.schedule_timezone,
            at=self.schedule_at,
        )


@dataclass(frozen=True)
class TriggerSnapshot:
    checked: bool
    records: tuple[TriggerRecordEvidence, ...] = ()
    delivery_target_column_present: bool = False
    error: str | None = None
    malformed_count: int = 0


@dataclass(frozen=True)
class TriggerScheduleEvidence:
    kind: str
    expression: str = field(default="", repr=False)
    timezone: str = ""
    at: str | None = field(default=None, repr=False)


@dataclass(frozen=True)
class TriggerCreateInvocation:
    trigger_id: str = field(repr=False)
    name: str = field(repr=False)
    prompt: str = field(repr=False)
    schedule_kind: str
    delivery_target_id: str | None = field(repr=False)
    tenant_id: str = field(default="", repr=False)
    schedule_expression: str = field(default="", repr=False)
    schedule_timezone: str = ""
    schedule_at: str | None = field(default=None, repr=False)

    @property
    def schedule(self) -> TriggerScheduleEvidence:
        return TriggerScheduleEvidence(
            kind=self.schedule_kind,
            expression=self.schedule_expression,
            timezone=self.schedule_timezone,
            at=self.schedule_at,
        )

    @classmethod
    def from_preview(
        cls,
        preview: Mapping[str, object],
        *,
        tenant_id: str = "",
    ) -> "TriggerCreateInvocation":
        raw_input = _json_object(
            preview.get("input")
            if "input" in preview
            else preview.get("input_summary")
        )
        raw_output = _json_object(
            preview.get("output")
            if "output" in preview
            else preview.get("output_preview")
        )
        trigger = raw_output.get("trigger")
        schedule = raw_input.get("schedule")
        if not isinstance(trigger, dict) or not isinstance(schedule, dict):
            raise ValueError("trigger_create preview omitted input schedule or output trigger")
        schedule_kind = str(schedule.get("kind") or "").strip().lower()
        if not schedule_kind:
            schedule_kind = "once" if "at" in schedule else "cron" if "expression" in schedule else ""
        values = {
            "trigger_id": trigger.get("trigger_id"),
            "name": raw_input.get("name"),
            "prompt": raw_input.get("prompt"),
        }
        if any(not isinstance(value, str) or not value for value in values.values()):
            raise ValueError("trigger_create preview omitted required identity fields")
        if schedule_kind not in {"cron", "once"}:
            raise ValueError("trigger_create preview had an unsupported schedule kind")
        schedule_timezone = schedule.get("timezone")
        if not isinstance(schedule_timezone, str) or not schedule_timezone:
            raise ValueError("trigger_create preview omitted schedule timezone")
        schedule_expression = schedule.get("expression") if schedule_kind == "cron" else ""
        schedule_at = schedule.get("at") if schedule_kind == "once" else None
        if schedule_kind == "cron" and (
            not isinstance(schedule_expression, str) or not schedule_expression
        ):
            raise ValueError("trigger_create preview omitted cron expression")
        if schedule_kind == "once" and (
            not isinstance(schedule_at, str) or not schedule_at
        ):
            raise ValueError("trigger_create preview omitted once timestamp")
        target = raw_input.get("delivery_target_id")
        if target is not None and not isinstance(target, str):
            raise ValueError("trigger_create preview delivery_target_id was malformed")
        return cls(
            trigger_id=str(values["trigger_id"]),
            name=str(values["name"]),
            prompt=str(values["prompt"]),
            schedule_kind=schedule_kind,
            delivery_target_id=target,
            tenant_id=tenant_id,
            schedule_expression=str(schedule_expression),
            schedule_timezone=schedule_timezone,
            schedule_at=str(schedule_at) if schedule_at is not None else None,
        )


@dataclass(frozen=True)
class RoutineValidation:
    valid: bool
    error: str | None
    safe_summary: dict[str, object]
    inconclusive: bool = False


@dataclass(frozen=True)
class TriggerInvocationEvidence:
    checked: bool
    invocations: tuple[TriggerCreateInvocation, ...] = ()
    run_terminal: bool = False
    error: str | None = None
    malformed_count: int = 0


@dataclass(frozen=True)
class DefaultTargetSnapshot:
    checked: bool
    bindings: tuple[tuple[str, str | None], ...] = field(default=(), repr=False)
    error: str | None = None

    @property
    def safe_summary(self) -> dict[str, object]:
        return {
            "checked": self.checked,
            "target_count": len(self.bindings),
            "snapshot_sha256": evidence_hash(self.bindings) if self.checked else None,
            **({"error": self.error} if self.error else {}),
        }


def _json_object(value: object) -> dict[str, object]:
    if isinstance(value, dict):
        return value
    if isinstance(value, bytes):
        value = value.decode("utf-8", errors="replace")
    if not isinstance(value, str):
        raise ValueError("expected a JSON object")
    parsed = json.loads(value)
    if not isinstance(parsed, dict):
        raise ValueError("expected a JSON object")
    return parsed


def _database_path(reborn_home: Path) -> Path:
    return reborn_home / "local-dev" / "reborn-local-dev.db"


def read_trigger_snapshot(reborn_home: Path) -> TriggerSnapshot:
    db_path = _database_path(reborn_home)
    if not db_path.exists():
        return TriggerSnapshot(checked=False, error="reborn-local-dev.db missing")
    try:
        with closing(
            sqlite3.connect(
                f"{db_path.resolve().as_uri()}?mode=ro",
                uri=True,
                timeout=0.0,
            )
        ) as db:
            columns = {
                str(row[1]) for row in db.execute("PRAGMA table_info(trigger_records)").fetchall()
            }
            required = {
                "tenant_id",
                "trigger_id",
                "name",
                "prompt",
                "schedule_expression",
                "schedule_timezone",
                "schedule_kind",
                "schedule_at",
                "next_run_at",
            }
            missing = sorted(required - columns)
            if missing:
                return TriggerSnapshot(
                    checked=False,
                    error=f"trigger_records missing required columns: {missing!r}",
                )
            has_target = "delivery_target" in columns
            target_column = "delivery_target" if has_target else "NULL"
            rows = db.execute(
                "SELECT tenant_id, trigger_id, name, prompt, schedule_kind, "
                "schedule_expression, schedule_timezone, schedule_at, next_run_at, "
                f"{target_column} FROM trigger_records"
            ).fetchall()
    except sqlite3.Error as exc:
        return TriggerSnapshot(checked=False, error=f"{type(exc).__name__}: {exc}")
    records: list[TriggerRecordEvidence] = []
    malformed_count = 0
    for row in rows:
        required = (*row[:5], row[6], row[8])
        kind = row[4]
        expression = row[5]
        schedule_at = row[7]
        malformed = (
            any(not isinstance(value, str) or not value for value in required)
            or kind not in {"cron", "once"}
            or (kind == "cron" and (not isinstance(expression, str) or not expression))
            or (kind == "once" and (not isinstance(schedule_at, str) or not schedule_at))
            or (row[9] is not None and not isinstance(row[9], str))
        )
        if malformed:
            malformed_count += 1
            continue
        records.append(
            TriggerRecordEvidence(
                key=TriggerKey(row[0], row[1]),
                name=row[2],
                prompt=row[3],
                schedule_kind=kind,
                schedule_expression=expression,
                schedule_timezone=row[6],
                schedule_at=schedule_at,
                next_run_at=row[8],
                delivery_target=row[9],
            )
        )
    if malformed_count:
        return TriggerSnapshot(
            checked=False,
            records=tuple(records),
            delivery_target_column_present=has_target,
            error="malformed trigger_records schedule or identity evidence",
            malformed_count=malformed_count,
        )
    return TriggerSnapshot(
        checked=True,
        records=tuple(records),
        delivery_target_column_present=has_target,
    )


def validate_trigger_delta(
    before: TriggerSnapshot,
    after: TriggerSnapshot,
    invocation: TriggerCreateInvocation,
) -> RoutineValidation:
    summary: dict[str, object] = {
        "before_checked": before.checked,
        "after_checked": after.checked,
        "before_record_count": len(before.records),
        "after_record_count": len(after.records),
    }
    if not before.checked or not after.checked:
        error = before.error or after.error or "trigger evidence unreadable"
        return RoutineValidation(False, error, summary)
    prior = {record.key for record in before.records}
    created = [record for record in after.records if record.key not in prior]
    summary["new_record_count"] = len(created)
    summary["new_record_identity_sha256"] = (
        evidence_hash((created[0].key.tenant_id, created[0].key.trigger_id))
        if len(created) == 1
        else None
    )
    if len(created) != 1:
        return RoutineValidation(
            False,
            f"expected exactly one new durable trigger record, found {len(created)}",
            summary,
        )
    record = created[0]
    mismatches = []
    if record.key.tenant_id != invocation.tenant_id:
        mismatches.append("tenant_id")
    if record.key.trigger_id != invocation.trigger_id:
        mismatches.append("trigger_id")
    if record.name != invocation.name:
        mismatches.append("name")
    if record.prompt != invocation.prompt:
        mismatches.append("prompt")
    mismatches.extend(_schedule_mismatches(record.schedule, invocation.schedule))
    if record.delivery_target != invocation.delivery_target_id:
        mismatches.append("delivery_target")
    summary.update(
        {
            "record_matches_current_invocation": not mismatches,
            "record_name_sha256": evidence_hash(record.name),
            "record_prompt_sha256": evidence_hash(record.prompt),
            "record_schedule_kind": record.schedule_kind,
            "record_delivery_target_sha256": (
                evidence_hash(record.delivery_target) if record.delivery_target else None
            ),
        }
    )
    if mismatches:
        return RoutineValidation(
            False,
            f"durable trigger record mismatched current invocation fields: {mismatches!r}",
            summary,
        )
    return RoutineValidation(True, None, summary)


def _schedule_mismatches(
    record: TriggerScheduleEvidence,
    invocation: TriggerScheduleEvidence,
) -> list[str]:
    mismatches: list[str] = []
    if record.kind != invocation.kind:
        return ["schedule_kind"]
    if record.timezone != invocation.timezone:
        mismatches.append("schedule_timezone")
    if record.kind == "cron":
        if record.expression != invocation.expression:
            mismatches.append("schedule_expression")
    elif record.kind == "once":
        record_at = _schedule_instant(record.at, record.timezone)
        invocation_at = _schedule_instant(invocation.at, invocation.timezone)
        if record_at is None or invocation_at is None or record_at != invocation_at:
            mismatches.append("schedule_at")
    return mismatches


def _schedule_instant(value: str | None, timezone_name: str) -> datetime | None:
    if not value:
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=ZoneInfo(timezone_name))
    except (ValueError, ZoneInfoNotFoundError):
        return None
    return parsed.astimezone(timezone.utc)


async def wait_for_trigger_validation(
    before: TriggerSnapshot,
    *,
    snapshot_reader: Callable[[], TriggerSnapshot],
    invocation_reader: Callable[[], object],
    timeout: float,
    poll_interval: float,
    monotonic: Callable[[], float] = time.monotonic,
    sleep: Callable[[float], Awaitable[object]] = asyncio.sleep,
) -> RoutineValidation:
    """Wait for terminal create evidence and its durable row(s) to converge.

    There is no arbitrary post-success settle delay. A single terminal
    invocation can finish only when its exact row is visible. If two terminal
    invocations exist, the waiter keeps polling until both durable writes are
    visible (or the overall deadline), then reports the duplicate.
    """
    deadline = monotonic() + timeout
    last_after = before
    last_invocations: tuple[TriggerCreateInvocation, ...] = ()
    last_run_terminal = False
    while True:
        last_after = snapshot_reader()
        raw_invocations = invocation_reader()
        if isinstance(raw_invocations, TriggerInvocationEvidence):
            invocation_evidence = raw_invocations
        else:
            invocation_evidence = TriggerInvocationEvidence(
                checked=bool(getattr(raw_invocations, "checked", True)),
                invocations=tuple(raw_invocations),  # type: ignore[arg-type]
                run_terminal=bool(getattr(raw_invocations, "run_terminal", True)),
                error=getattr(raw_invocations, "error", None),
            )
        if not last_after.checked or not invocation_evidence.checked:
            error = (
                last_after.error
                or invocation_evidence.error
                or "typed trigger evidence could not be read"
            )
            return RoutineValidation(
                False,
                error,
                {
                    "before_checked": before.checked,
                    "after_checked": last_after.checked,
                    "invocations_checked": invocation_evidence.checked,
                },
                inconclusive=True,
            )
        last_invocations = invocation_evidence.invocations
        last_run_terminal = invocation_evidence.run_terminal
        prior = {record.key for record in before.records}
        new_count = sum(record.key not in prior for record in last_after.records)
        terminal_count = len(last_invocations)
        if last_run_terminal and terminal_count == 1 and new_count >= 1:
            validation = validate_trigger_delta(
                before,
                last_after,
                last_invocations[0],
            )
            validation.safe_summary["terminal_invocation_count"] = terminal_count
            return validation
        if last_run_terminal and terminal_count > 1 and new_count >= terminal_count:
            return RoutineValidation(
                False,
                f"observed {terminal_count} terminal trigger_create invocations; "
                "expected exactly one (two terminal invocations create duplicate routines)",
                {
                    "before_checked": before.checked,
                    "after_checked": last_after.checked,
                    "terminal_invocation_count": terminal_count,
                    "new_record_count": new_count,
                    "submitted_run_terminal": last_run_terminal,
                },
            )
        now = monotonic()
        if now >= deadline:
            return RoutineValidation(
                False,
                "authoritative trigger creation evidence did not converge before deadline",
                {
                    "before_checked": before.checked,
                    "after_checked": last_after.checked,
                    "terminal_invocation_count": terminal_count,
                    "new_record_count": new_count,
                    "submitted_run_terminal": last_run_terminal,
                },
            )
        await sleep(min(poll_interval, deadline - now))


def read_default_target_snapshot(reborn_home: Path) -> DefaultTargetSnapshot:
    db_path = _database_path(reborn_home)
    if not db_path.exists():
        return DefaultTargetSnapshot(checked=False, error="reborn-local-dev.db missing")
    try:
        with closing(
            sqlite3.connect(
                f"{db_path.resolve().as_uri()}?mode=ro",
                uri=True,
                timeout=0.0,
            )
        ) as db:
            rows = db.execute(
                "SELECT path, contents FROM root_filesystem_entries "
                "WHERE path LIKE '%/outbound/communication-preferences/%' AND is_dir = 0 "
                "ORDER BY path"
            ).fetchall()
    except sqlite3.Error as exc:
        return DefaultTargetSnapshot(checked=False, error=f"{type(exc).__name__}: {exc}")
    bindings: list[tuple[str, str | None]] = []
    for path, contents in rows:
        try:
            record = _json_object(contents)
        except (ValueError, json.JSONDecodeError, UnicodeDecodeError):
            return DefaultTargetSnapshot(
                checked=False,
                error="malformed outbound communication preference record",
            )
        target = record.get("final_reply_target")
        if target is not None and not isinstance(target, str):
            return DefaultTargetSnapshot(
                checked=False,
                error="malformed final_reply_target binding",
            )
        bindings.append((str(path), target))
    return DefaultTargetSnapshot(checked=True, bindings=tuple(bindings))


def outbound_delivery_target_for_channel(
    payload: object,
    channel: str,
) -> dict[str, object] | None:
    if not isinstance(payload, dict) or not isinstance(payload.get("targets"), list):
        return None
    for option in payload["targets"]:
        if not isinstance(option, dict):
            continue
        target = option.get("target")
        capabilities = option.get("capabilities")
        if (
            isinstance(target, dict)
            and isinstance(capabilities, dict)
            and str(target.get("channel") or "").lower() == channel.lower()
            and capabilities.get("final_replies") is True
            and isinstance(target.get("target_id"), str)
        ):
            return target
    return None
