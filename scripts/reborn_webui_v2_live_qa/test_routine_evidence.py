import json
import asyncio
import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

from scripts.reborn_webui_v2_live_qa.routine_evidence import (
    TriggerCreateInvocation,
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
                db.execute(
                    "CREATE TABLE trigger_records (tenant_id TEXT, trigger_id TEXT, "
                    "name TEXT, prompt TEXT, schedule_kind TEXT, next_run_at TEXT, "
                    "delivery_target TEXT)"
                )
                db.executemany(
                    "INSERT INTO trigger_records VALUES (?, 'same-id', ?, 'do work', "
                    "'once', '2026-07-16T12:00:00Z', NULL)",
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
                db.execute(
                    "CREATE TABLE trigger_records (tenant_id TEXT, trigger_id TEXT, "
                    "name TEXT, prompt TEXT, schedule_kind TEXT, next_run_at TEXT, "
                    "delivery_target TEXT)"
                )
            before = read_trigger_snapshot(home)
            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "INSERT INTO trigger_records VALUES "
                    "('tenant-a', 'created-id', 'routine', 'task only', 'once', "
                    "'2026-07-16T12:00:00Z', 'target-1')"
                )
            after = read_trigger_snapshot(home)
            invocation = TriggerCreateInvocation(
                trigger_id="created-id",
                name="routine",
                prompt="task only",
                schedule_kind="once",
                delivery_target_id="target-1",
            )

            validation = validate_trigger_delta(before, after, invocation)

            self.assertTrue(validation.valid)
            self.assertEqual(validation.safe_summary["new_record_count"], 1)
            self.assertNotIn("target-1", json.dumps(validation.safe_summary))

    def test_delta_rejects_missing_wrong_and_duplicate_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "CREATE TABLE trigger_records (tenant_id TEXT, trigger_id TEXT, "
                    "name TEXT, prompt TEXT, schedule_kind TEXT, next_run_at TEXT, "
                    "delivery_target TEXT)"
                )
            before = read_trigger_snapshot(home)
            expected = TriggerCreateInvocation(
                trigger_id="expected-id",
                name="routine",
                prompt="task only",
                schedule_kind="once",
                delivery_target_id=None,
            )
            self.assertFalse(validate_trigger_delta(before, before, expected).valid)

            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "INSERT INTO trigger_records VALUES "
                    "('tenant-a', 'wrong-id', 'routine', 'wrong task', 'cron', "
                    "'2026-07-16T12:00:00Z', NULL)"
                )
            wrong = read_trigger_snapshot(home)
            self.assertFalse(validate_trigger_delta(before, wrong, expected).valid)

            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "INSERT INTO trigger_records VALUES "
                    "('tenant-b', 'expected-id', 'routine', 'task only', 'once', "
                    "'2026-07-16T12:00:00Z', NULL)"
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

    def test_waiter_observes_delayed_duplicate_after_two_terminal_invocations(self):
        from scripts.reborn_webui_v2_live_qa.routine_evidence import (
            TriggerKey,
            TriggerRecordEvidence,
            TriggerSnapshot,
        )

        before = TriggerSnapshot(checked=True, delivery_target_column_present=True)
        one = TriggerSnapshot(
            checked=True,
            delivery_target_column_present=True,
            records=(
                TriggerRecordEvidence(
                    TriggerKey("tenant-a", "id-a"),
                    "routine",
                    "task",
                    "once",
                    "soon",
                    None,
                ),
            ),
        )
        duplicate = TriggerSnapshot(
            checked=True,
            delivery_target_column_present=True,
            records=(
                *one.records,
                TriggerRecordEvidence(
                    TriggerKey("tenant-a", "id-b"),
                    "routine",
                    "task",
                    "once",
                    "soon",
                    None,
                ),
            ),
        )
        invocations = [
            TriggerCreateInvocation("id-a", "routine", "task", "once", None),
            TriggerCreateInvocation("id-b", "routine", "task", "once", None),
        ]
        snapshots = iter((one, duplicate))
        clock = iter((0.0, 0.0, 0.1, 0.1))

        async def no_sleep(_seconds):
            return None

        validation = asyncio.run(
            wait_for_trigger_validation(
                before,
                snapshot_reader=lambda: next(snapshots),
                invocation_reader=lambda: invocations,
                timeout=1.0,
                poll_interval=0.1,
                monotonic=lambda: next(clock),
                sleep=no_sleep,
            )
        )

        self.assertFalse(validation.valid)
        self.assertIn("two terminal", validation.error)
        self.assertEqual(validation.safe_summary["terminal_invocation_count"], 2)
        self.assertEqual(validation.safe_summary["new_record_count"], 2)

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
