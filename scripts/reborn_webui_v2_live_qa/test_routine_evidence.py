import json
import asyncio
import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path
from unittest.mock import patch

from scripts.reborn_webui_v2_live_qa.routine_evidence import (
    TriggerCreateInvocation,
    TriggerKey,
    TriggerRecordEvidence,
    TriggerSnapshot,
    outbound_delivery_target_for_channel,
    read_default_target_snapshot,
    read_trigger_snapshot,
    validate_trigger_delta,
    wait_for_trigger_validation,
    trigger_prompt_has_self_delivery_routing,
)


def _db(home: Path) -> Path:
    db_dir = home / "local-dev"
    db_dir.mkdir(parents=True, exist_ok=True)
    return db_dir / "reborn-local-dev.db"


def _create_trigger_table(db: sqlite3.Connection) -> None:
    db.execute(
        "CREATE TABLE trigger_records (tenant_id TEXT, trigger_id TEXT, "
        "name TEXT, prompt TEXT, schedule_kind TEXT, next_run_at TEXT, "
        "delivery_target TEXT, schedule_expression TEXT NOT NULL DEFAULT '', "
        "schedule_timezone TEXT NOT NULL DEFAULT 'UTC', schedule_at TEXT)"
    )


def _record(
    trigger_id: str,
    *,
    tenant_id: str = "tenant-a",
    kind: str = "once",
    expression: str = "",
    timezone: str = "UTC",
    at: str | None = "2026-07-16T12:00:00Z",
) -> TriggerRecordEvidence:
    return TriggerRecordEvidence(
        TriggerKey(tenant_id, trigger_id),
        "routine",
        "task",
        kind,
        at,
        None,
        schedule_expression=expression,
        schedule_timezone=timezone,
        schedule_at=at if kind == "once" else None,
    )


def _invocation(
    trigger_id: str,
    *,
    tenant_id: str = "tenant-a",
    kind: str = "once",
    expression: str = "",
    timezone: str = "UTC",
    at: str | None = "2026-07-16T12:00:00Z",
) -> TriggerCreateInvocation:
    return TriggerCreateInvocation(
        trigger_id,
        "routine",
        "task",
        kind,
        None,
        tenant_id=tenant_id,
        schedule_expression=expression,
        schedule_timezone=timezone,
        schedule_at=at if kind == "once" else None,
    )


class RoutineEvidenceTests(unittest.TestCase):
    def test_trigger_prompt_detects_requester_delivery_routing(self):
        self.assertTrue(
            trigger_prompt_has_self_delivery_routing(
                "Check the time and send me the result in a Slack DM"
            )
        )
        self.assertFalse(
            trigger_prompt_has_self_delivery_routing(
                "Report the current UTC time using the time capability"
            )
        )

    def test_trigger_snapshot_uses_composite_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                _create_trigger_table(db)
                db.executemany(
                    "INSERT INTO trigger_records "
                    "(tenant_id, trigger_id, name, prompt, schedule_kind, next_run_at, "
                    "delivery_target, schedule_at) VALUES (?, 'same-id', ?, 'do work', "
                    "'once', '2026-07-16T12:00:00Z', NULL, "
                    "'2026-07-16T12:00:00Z')",
                    (("tenant-a", "alpha"), ("tenant-b", "beta")),
                )

            snapshot = read_trigger_snapshot(home)

            self.assertTrue(snapshot.checked)
            self.assertEqual(
                {(record.key.tenant_id, record.key.trigger_id) for record in snapshot.records},
                {("tenant-a", "same-id"), ("tenant-b", "same-id")},
            )

    def test_delta_requires_one_exact_current_invocation_record(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                _create_trigger_table(db)
            before = read_trigger_snapshot(home)
            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "INSERT INTO trigger_records "
                    "(tenant_id, trigger_id, name, prompt, schedule_kind, next_run_at, "
                    "delivery_target, schedule_at) VALUES "
                    "('tenant-a', 'created-id', 'routine', 'task only', 'once', "
                    "'2026-07-16T12:00:00Z', 'target-1', "
                    "'2026-07-16T12:00:00Z')"
                )
            after = read_trigger_snapshot(home)
            invocation = TriggerCreateInvocation(
                trigger_id="created-id",
                name="routine",
                prompt="task only",
                schedule_kind="once",
                delivery_target_id="target-1",
                tenant_id="tenant-a",
                schedule_timezone="UTC",
                schedule_at="2026-07-16T12:00:00Z",
            )

            validation = validate_trigger_delta(before, after, invocation)

            self.assertTrue(validation.valid)
            self.assertEqual(validation.safe_summary["new_record_count"], 1)
            self.assertNotIn("target-1", json.dumps(validation.safe_summary))

    def test_delta_rejects_missing_wrong_and_duplicate_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                _create_trigger_table(db)
            before = read_trigger_snapshot(home)
            expected = TriggerCreateInvocation(
                trigger_id="expected-id",
                name="routine",
                prompt="task only",
                schedule_kind="once",
                delivery_target_id=None,
                tenant_id="tenant-a",
                schedule_timezone="UTC",
                schedule_at="2026-07-16T12:00:00Z",
            )
            self.assertFalse(validate_trigger_delta(before, before, expected).valid)

            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "INSERT INTO trigger_records "
                    "(tenant_id, trigger_id, name, prompt, schedule_kind, next_run_at, "
                    "delivery_target, schedule_expression) VALUES "
                    "('tenant-a', 'wrong-id', 'routine', 'wrong task', 'cron', "
                    "'2026-07-16T12:00:00Z', NULL, '0 * * * *')"
                )
            wrong = read_trigger_snapshot(home)
            self.assertFalse(validate_trigger_delta(before, wrong, expected).valid)

            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "INSERT INTO trigger_records "
                    "(tenant_id, trigger_id, name, prompt, schedule_kind, next_run_at, "
                    "delivery_target, schedule_at) VALUES "
                    "('tenant-b', 'expected-id', 'routine', 'task only', 'once', "
                    "'2026-07-16T12:00:00Z', NULL, '2026-07-16T12:00:00Z')"
                )
            duplicate = read_trigger_snapshot(home)
            validation = validate_trigger_delta(before, duplicate, expected)
            self.assertFalse(validation.valid)
            self.assertIn("exactly one", validation.error)

    def test_default_target_snapshot_fails_closed_and_only_summarizes_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            missing = read_default_target_snapshot(Path(tmp))
            self.assertFalse(missing.checked)
            self.assertTrue(missing.error)

        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "CREATE TABLE root_filesystem_entries "
                    "(path TEXT, contents TEXT, is_dir INTEGER)"
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries VALUES (?, ?, 0)",
                    (
                        "/outbound/communication-preferences/a.json",
                        json.dumps({"final_reply_target": "reply:conversation:11:D0123456789;"}),
                    ),
                )
            snapshot = read_default_target_snapshot(home)
            self.assertTrue(snapshot.checked)
            encoded = json.dumps(snapshot.safe_summary)
            self.assertNotIn("D0123456789", encoded)
            self.assertNotIn("conversation", encoded)
            self.assertEqual(snapshot.safe_summary["target_count"], 1)

        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "CREATE TABLE root_filesystem_entries "
                    "(path TEXT, contents TEXT, is_dir INTEGER)"
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries VALUES "
                    "('/outbound/communication-preferences/a.json', 'not json', 0)"
                )
            malformed = read_default_target_snapshot(home)
            self.assertFalse(malformed.checked)
            self.assertIn("malformed", malformed.error)

    def test_target_resolver_requires_channel_and_final_reply_capability(self):
        payload = {
            "targets": [
                {
                    "target": {"target_id": "status-only", "channel": "slack"},
                    "capabilities": {"final_replies": False},
                },
                {
                    "target": {"target_id": "email-target", "channel": "email"},
                    "capabilities": {"final_replies": True},
                },
                {
                    "target": {"target_id": "slack-final", "channel": "slack"},
                    "capabilities": {"final_replies": True},
                },
            ]
        }

        target = outbound_delivery_target_for_channel(payload, "slack")

        self.assertEqual(target["target_id"], "slack-final")

    def test_waiter_does_not_accept_first_row_before_run_terminal_accounting(self):
        class InvocationBatch(list):
            checked = True
            error = None

            def __init__(self, values, *, run_terminal):
                super().__init__(values)
                self.run_terminal = run_terminal

        before = TriggerSnapshot(checked=True)
        first_record, second_record = _record("id-a"), _record("id-b")
        snapshots = iter(
            (
                TriggerSnapshot(checked=True, records=(first_record,)),
                TriggerSnapshot(
                    checked=True,
                    records=(first_record, second_record),
                ),
            )
        )
        invocations = (_invocation("id-a"), _invocation("id-b"))
        batches = iter(
            (
                InvocationBatch(invocations[:1], run_terminal=False),
                InvocationBatch(invocations, run_terminal=True),
            )
        )
        clock = iter((0.0, 0.0, 0.1))

        async def no_sleep(_seconds):
            return None

        validation = asyncio.run(
            wait_for_trigger_validation(
                before,
                snapshot_reader=lambda: next(snapshots),
                invocation_reader=lambda: next(batches),
                timeout=1.0,
                poll_interval=0.1,
                monotonic=lambda: next(clock),
                sleep=no_sleep,
            )
        )

        self.assertFalse(validation.valid)
        self.assertIn("two terminal", validation.error)
        self.assertEqual(validation.safe_summary["terminal_invocation_count"], 2)

    def test_delta_rejects_same_kind_but_different_schedule(self):
        after = TriggerSnapshot(
            checked=True,
            records=(
                _record(
                    "created-id",
                    kind="cron",
                    expression="5 9 * * 1",
                    timezone="Europe/London",
                ),
            ),
        )
        invocation = _invocation(
            "created-id",
            kind="cron",
            expression="0 9 * * 1",
            timezone="Europe/London",
        )

        validation = validate_trigger_delta(TriggerSnapshot(checked=True), after, invocation)

        self.assertFalse(validation.valid)
        self.assertIn("schedule_expression", validation.error)

    def test_delta_rejects_same_kind_but_different_once_time_and_timezone(self):
        before = TriggerSnapshot(checked=True)
        after = TriggerSnapshot(checked=True, records=(_record("created-id"),))
        cases = (
            _invocation("created-id", at="2026-07-16T12:05:00Z"),
            _invocation(
                "created-id",
                timezone="Europe/London",
                at="2026-07-16T13:00:00+01:00",
            ),
        )

        for invocation in cases:
            with self.subTest(schedule=invocation.schedule):
                validation = validate_trigger_delta(before, after, invocation)
                self.assertFalse(validation.valid)
                self.assertIn("schedule", validation.error)

    def test_delta_rejects_matching_trigger_id_in_wrong_terminal_tenant(self):
        after = TriggerSnapshot(
            checked=True,
            records=(_record("created-id", tenant_id="tenant-b"),),
        )
        validation = validate_trigger_delta(
            TriggerSnapshot(checked=True),
            after,
            _invocation("created-id", tenant_id="tenant-a"),
        )

        self.assertFalse(validation.valid)
        self.assertIn("tenant_id", validation.error)

    def test_trigger_snapshot_read_uses_nonblocking_sqlite_timeout(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "CREATE TABLE trigger_records (tenant_id TEXT, trigger_id TEXT, "
                    "name TEXT, prompt TEXT, schedule_kind TEXT, next_run_at TEXT)"
                )
            real_connect = sqlite3.connect
            calls: list[dict[str, object]] = []

            def capture_connect(*args, **kwargs):
                calls.append(dict(kwargs))
                return real_connect(*args, **kwargs)

            with patch(
                "scripts.reborn_webui_v2_live_qa.routine_evidence.sqlite3.connect",
                side_effect=capture_connect,
            ):
                read_trigger_snapshot(home)

        self.assertTrue(calls)
        self.assertLessEqual(float(calls[0].get("timeout", 5.0)), 0.05)

    def test_trigger_snapshot_distinguishes_malformed_schedule_rows(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                _create_trigger_table(db)
                db.execute(
                    "INSERT INTO trigger_records "
                    "(tenant_id, trigger_id, name, prompt, schedule_kind, next_run_at, "
                    "delivery_target) VALUES "
                    "('tenant', 'trigger', 'routine', 'task', 'once', "
                    "'2026-07-16T12:00:00Z', NULL)"
                )

            snapshot = read_trigger_snapshot(home)

        self.assertFalse(snapshot.checked)
        self.assertGreater(getattr(snapshot, "malformed_count", 0), 0)
        self.assertIn("malformed", snapshot.error)

    def test_waiter_marks_typed_snapshot_read_failure_inconclusive(self):
        from scripts.reborn_webui_v2_live_qa.routine_evidence import TriggerSnapshot

        before = TriggerSnapshot(checked=True)
        unreadable = TriggerSnapshot(checked=False, error="database is locked")
        times = iter((0.0, 1.0))

        async def no_sleep(_seconds):
            return None

        validation = asyncio.run(
            wait_for_trigger_validation(
                before,
                snapshot_reader=lambda: unreadable,
                invocation_reader=lambda: [],
                timeout=1.0,
                poll_interval=1.0,
                monotonic=lambda: next(times),
                sleep=no_sleep,
            )
        )

        self.assertFalse(validation.valid)
        self.assertTrue(getattr(validation, "inconclusive", False))
        self.assertIn("database is locked", validation.error)

    def test_waiter_never_sleeps_past_overall_deadline(self):
        from scripts.reborn_webui_v2_live_qa.routine_evidence import TriggerSnapshot

        before = TriggerSnapshot(checked=True, delivery_target_column_present=True)
        sleeps: list[float] = []
        times = iter((10.0, 10.8, 11.0))

        async def record_sleep(seconds):
            sleeps.append(seconds)

        validation = asyncio.run(
            wait_for_trigger_validation(
                before,
                snapshot_reader=lambda: before,
                invocation_reader=lambda: [],
                timeout=1.0,
                poll_interval=5.0,
                monotonic=lambda: next(times),
                sleep=record_sleep,
            )
        )

        self.assertFalse(validation.valid)
        self.assertEqual(sleeps, [0.1999999999999993])
        self.assertIn("deadline", validation.error)


if __name__ == "__main__":
    unittest.main()
