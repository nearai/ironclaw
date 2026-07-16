"""In-memory capability preview evidence for live canaries."""

from __future__ import annotations

import hashlib
import json
import sqlite3
import time
from contextlib import closing
from pathlib import Path
from typing import Callable

from scripts.reborn_webui_v2_live_qa.routine_evidence import (
    RoutineValidation,
    TriggerCreateInvocation,
    TriggerSnapshot,
    outbound_delivery_target_for_channel,
    read_trigger_snapshot,
    wait_for_trigger_validation,
)


TRIGGER_CREATE_CAPABILITY_ID = "builtin.trigger_create"


def _json_object(value: object) -> dict[str, object] | None:
    if isinstance(value, dict):
        return value
    if isinstance(value, bytes):
        value = value.decode("utf-8", errors="replace")
    if not isinstance(value, str):
        return None
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError:
        return None
    return parsed if isinstance(parsed, dict) else None


def current_turn_capability_previews(
    reborn_home: Path,
    submission_identity: dict[str, object],
    capability_ids: list[str],
) -> list[dict[str, object]]:
    """Read raw preview payloads for in-memory verification only."""
    thread_id = str(submission_identity.get("thread_id") or "")
    run_id = str(submission_identity.get("run_id") or "")
    if not thread_id or not run_id:
        return []
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return []
    try:
        with closing(
            sqlite3.connect(f"{db_path.resolve().as_uri()}?mode=ro", uri=True)
        ) as db:
            rows = db.execute(
                "SELECT contents FROM root_filesystem_entries "
                "WHERE is_dir = 0 AND content_type = 'application/json' "
                "AND kind = 'thread_message' AND path LIKE ?",
                (f"%/threads/{thread_id}/messages/%",),
            ).fetchall()
    except sqlite3.Error:
        return []
    wanted = set(capability_ids)
    previews: list[dict[str, object]] = []
    for (raw_contents,) in rows:
        message = _json_object(raw_contents)
        if (
            message is None
            or message.get("kind") != "capability_display_preview"
            or message.get("thread_id") != thread_id
            or message.get("turn_run_id") != run_id
        ):
            continue
        preview = _json_object(message.get("content"))
        if preview is None or preview.get("capability_id") not in wanted:
            continue
        input_payload = _json_object(preview.get("input_summary"))
        output_payload = _json_object(preview.get("output_preview"))
        if input_payload is not None and output_payload is not None:
            previews.append(
                {
                    "invocation_id": str(preview.get("invocation_id") or ""),
                    "capability_id": str(preview.get("capability_id") or ""),
                    "input": input_payload,
                    "output": output_payload,
                }
            )
    return previews


def evaluate_google_docs_chain(
    terminal_evidence: dict[str, object],
    previews: list[dict[str, object]],
    phrase: str,
) -> dict[str, object]:
    expected = [
        "google-docs.create_document",
        "google-docs.insert_text",
        "google-docs.read_content",
    ]
    statuses = terminal_evidence.get("statuses")
    sequence = terminal_evidence.get("terminal_sequence")
    terminal_ok = isinstance(statuses, dict) and all(
        statuses.get(capability_id) == ["completed"] for capability_id in expected
    )
    ordered = (
        [
            (str(item.get("capability_id") or ""), str(item.get("invocation_id") or ""))
            for item in sequence
            if isinstance(item, dict) and item.get("capability_id") in expected
        ]
        if isinstance(sequence, list)
        else []
    )
    sequence_ok = [capability for capability, _ in ordered] == expected
    by_invocation = {
        str(preview.get("invocation_id") or ""): preview for preview in previews
    }
    chain = [by_invocation.get(invocation_id) for _, invocation_id in ordered]
    complete = len(chain) == 3 and all(chain)
    created_id = inserted_id = read_input_id = read_output_id = ""
    inserted_phrase = readback = False
    if complete:
        create, insert, read = chain
        values = (
            create.get("output"),
            insert.get("input"),
            read.get("input"),
            read.get("output"),
        )
        if all(isinstance(value, dict) for value in values):
            create_output, insert_input, read_input, read_output = values
            created_id = str(create_output.get("document_id") or "")
            inserted_id = str(insert_input.get("document_id") or "")
            read_input_id = str(read_input.get("document_id") or "")
            read_output_id = str(read_output.get("document_id") or "")
            inserted_phrase = phrase in str(insert_input.get("text") or "")
            readback = phrase in str(read_output.get("content") or "")
    same_document = bool(
        created_id and created_id == inserted_id == read_input_id == read_output_id
    )
    verified = bool(
        terminal_ok and sequence_ok and complete and same_document and inserted_phrase and readback
    )
    return {
        "verified": verified,
        "terminal_chain_complete": terminal_ok,
        "terminal_chain_ordered": sequence_ok,
        "preview_chain_complete": complete,
        "same_document": same_document,
        "inserted_expected_phrase": inserted_phrase,
        "authoritative_phrase_readback": readback,
        "document_id_sha256": (
            hashlib.sha256(created_id.encode("utf-8")).hexdigest() if created_id else None
        ),
    }


def current_turn_outbound_final_reply_target_ids(
    reborn_home: Path,
    submission_identities: list[dict[str, object]],
    *,
    capability_id: str,
    terminal_reader: Callable[..., dict[str, object]],
    preview_reader: Callable[..., list[dict[str, object]]] = current_turn_capability_previews,
) -> list[str]:
    target_ids: list[str] = []
    for identity in submission_identities:
        evidence = terminal_reader(
            reborn_home, identity, [capability_id], {"completed"}
        )
        sequence = evidence.get("terminal_sequence")
        terminal_ids = {
            str(item.get("invocation_id") or "")
            for item in sequence
            if isinstance(item, dict)
            and item.get("capability_id") == capability_id
            and item.get("status") == "completed"
        } if isinstance(sequence, list) else set()
        for preview in preview_reader(
            reborn_home, identity, [capability_id]
        ):
            if str(preview.get("invocation_id") or "") not in terminal_ids:
                continue
            target = outbound_delivery_target_for_channel(
                preview.get("output"), "slack"
            )
            if target is not None:
                target_id = str(target.get("target_id") or "")
                if target_id and target_id not in target_ids:
                    target_ids.append(target_id)
    return target_ids


def current_turn_trigger_create_invocations(
    reborn_home: Path,
    submission_identities: list[dict[str, object]],
    *,
    terminal_reader: Callable[..., dict[str, object]],
    preview_reader: Callable[..., list[dict[str, object]]] = current_turn_capability_previews,
) -> list[TriggerCreateInvocation]:
    invocations: dict[str, TriggerCreateInvocation] = {}
    for identity in submission_identities:
        evidence = terminal_reader(
            reborn_home,
            identity,
            [TRIGGER_CREATE_CAPABILITY_ID],
            {"completed"},
        )
        sequence = evidence.get("terminal_sequence")
        terminal_ids = {
            str(item.get("invocation_id") or "")
            for item in sequence
            if isinstance(item, dict)
            and item.get("capability_id") == TRIGGER_CREATE_CAPABILITY_ID
            and item.get("status") == "completed"
        } if isinstance(sequence, list) else set()
        for preview in preview_reader(
            reborn_home, identity, [TRIGGER_CREATE_CAPABILITY_ID]
        ):
            invocation_id = str(preview.get("invocation_id") or "")
            if not invocation_id or invocation_id not in terminal_ids:
                continue
            try:
                invocations[invocation_id] = TriggerCreateInvocation.from_preview(preview)
            except (ValueError, json.JSONDecodeError, TypeError):
                continue
    return list(invocations.values())


async def wait_for_current_turn_trigger_validation(
    reborn_home: Path,
    before: TriggerSnapshot,
    submission_identities: list[dict[str, object]],
    *,
    terminal_reader: Callable[..., dict[str, object]],
    preview_reader: Callable[..., list[dict[str, object]]] = current_turn_capability_previews,
    timeout: float,
    poll_interval: float = 0.25,
) -> RoutineValidation:
    started = time.monotonic()
    validation = await wait_for_trigger_validation(
        before,
        snapshot_reader=lambda: read_trigger_snapshot(reborn_home),
        invocation_reader=lambda: current_turn_trigger_create_invocations(
            reborn_home,
            submission_identities,
            terminal_reader=terminal_reader,
            preview_reader=preview_reader,
        ),
        timeout=timeout,
        poll_interval=poll_interval,
    )
    return RoutineValidation(
        validation.valid,
        validation.error,
        {
            **validation.safe_summary,
            "wait_ms": int((time.monotonic() - started) * 1000),
        },
    )
