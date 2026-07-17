import json
import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

from scripts.reborn_webui_v2_live_qa.capability_evidence import (
    TRIGGER_CREATE_CAPABILITY_ID,
    current_turn_capability_previews,
    current_turn_trigger_create_invocations,
    evaluate_google_docs_chain,
)


def _db(home: Path) -> Path:
    directory = home / "local-dev"
    directory.mkdir(parents=True, exist_ok=True)
    return directory / "reborn-local-dev.db"


def _preview_message(
    *,
    invocation_id: str,
    capability_id: str,
    input_summary: dict[str, object],
    output_preview: object,
) -> str:
    return json.dumps(
        {
            "kind": "capability_display_preview",
            "thread_id": "thread-1",
            "turn_run_id": "run-1",
            "content": json.dumps(
                {
                    "invocation_id": invocation_id,
                    "capability_id": capability_id,
                    "input_summary": json.dumps(input_summary),
                    "output_preview": (
                        json.dumps(output_preview)
                        if isinstance(output_preview, dict)
                        else output_preview
                    ),
                }
            ),
        }
    )


class CapabilityEvidenceTests(unittest.TestCase):
    def test_production_shaped_google_docs_previews_accept_plain_text_readback(self):
        phrase = "production-shaped exact strategy phrase"
        identity = {"thread_id": "thread-1", "run_id": "run-1"}
        expected = [
            "google-docs.create_document",
            "google-docs.insert_text",
            "google-docs.read_content",
        ]
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "CREATE TABLE root_filesystem_entries "
                    "(is_dir INTEGER, content_type TEXT, kind TEXT, path TEXT, "
                    "contents TEXT)"
                )
                rows = (
                    _preview_message(
                        invocation_id="create",
                        capability_id=expected[0],
                        input_summary={"title": "QA"},
                        output_preview={"document_id": "doc-id"},
                    ),
                    _preview_message(
                        invocation_id="insert",
                        capability_id=expected[1],
                        input_summary={"document_id": "doc-id", "text": phrase},
                        output_preview="inserted",
                    ),
                    _preview_message(
                        invocation_id="read",
                        capability_id=expected[2],
                        input_summary={"document_id": "doc-id"},
                        output_preview=f"document body: {phrase}",
                    ),
                )
                db.executemany(
                    "INSERT INTO root_filesystem_entries VALUES "
                    "(0, 'application/json', 'thread_message', "
                    "'/threads/thread-1/messages/message.json', ?)",
                    ((row,) for row in rows),
                )

            previews = current_turn_capability_previews(home, identity, expected)

        self.assertTrue(previews.checked)
        terminal = {
            "statuses": {capability: ["completed"] for capability in expected},
            "terminal_sequence": [
                {
                    "capability_id": capability,
                    "invocation_id": invocation_id,
                    "status": "completed",
                }
                for capability, invocation_id in zip(
                    expected, ("create", "insert", "read"), strict=True
                )
            ],
        }
        evidence = evaluate_google_docs_chain(terminal, list(previews.previews), phrase)
        self.assertTrue(evidence["verified"])
        self.assertTrue(evidence["authoritative_phrase_readback"])

    def test_preview_reader_distinguishes_missing_and_malformed_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            missing = current_turn_capability_previews(
                Path(tmp),
                {"thread_id": "thread", "run_id": "run"},
                ["google-docs.read_content"],
            )
            self.assertFalse(missing.checked)
            self.assertIn("missing", str(missing.error))

        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            with closing(sqlite3.connect(_db(home))) as db, db:
                db.execute(
                    "CREATE TABLE root_filesystem_entries "
                    "(is_dir INTEGER, content_type TEXT, kind TEXT, path TEXT, "
                    "contents TEXT)"
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries VALUES "
                    "(0, 'application/json', 'thread_message', "
                    "'/threads/thread/messages/bad.json', 'not-json')"
                )
            malformed = current_turn_capability_previews(
                home,
                {"thread_id": "thread", "run_id": "run"},
                ["google-docs.read_content"],
            )
            self.assertFalse(malformed.checked)
            self.assertGreater(malformed.malformed_count, 0)
            self.assertIn("malformed", str(malformed.error))

    def test_trigger_invocation_retains_terminal_tenant_and_run_accounting(self):
        terminal = {
            "run_terminal": True,
            "terminal_sequence": [
                {
                    "capability_id": TRIGGER_CREATE_CAPABILITY_ID,
                    "invocation_id": "create-1",
                    "status": "completed",
                    "tenant_id": "tenant-a",
                }
            ],
        }
        previews = [
            {
                "invocation_id": "create-1",
                "capability_id": TRIGGER_CREATE_CAPABILITY_ID,
                "input": {
                    "name": "routine",
                    "prompt": "task",
                    "schedule": {
                        "kind": "cron",
                        "expression": "0 9 * * 1",
                        "timezone": "Europe/London",
                    },
                },
                "output": {"trigger": {"trigger_id": "trigger-1"}},
            }
        ]
        evidence = current_turn_trigger_create_invocations(
            Path("/tmp/home"),
            [{"thread_id": "thread", "run_id": "run"}],
            terminal_reader=lambda *_args: terminal,
            preview_reader=lambda *_args: previews,
        )

        self.assertTrue(evidence.checked)
        self.assertTrue(evidence.run_terminal)
        invocation = evidence.invocations[0]
        self.assertEqual(invocation.tenant_id, "tenant-a")
        self.assertEqual(invocation.schedule.expression, "0 9 * * 1")
        self.assertEqual(invocation.schedule.timezone, "Europe/London")

if __name__ == "__main__":
    unittest.main()
