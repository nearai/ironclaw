"""In-memory capability preview evidence for live canaries."""

from __future__ import annotations

import hashlib
import json
import sqlite3
import time
from contextlib import closing
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from scripts.reborn_webui_v2_live_qa.routine_evidence import (
    RoutineValidation,
    TriggerCreateInvocation,
    TriggerInvocationEvidence,
    TriggerSnapshot,
    outbound_delivery_target_for_channel,
    read_trigger_snapshot,
    wait_for_trigger_validation,
)


TRIGGER_CREATE_CAPABILITY_ID = "builtin.trigger_create"


@dataclass(frozen=True)
class CapabilityPreviewSnapshot:
    checked: bool
    previews: tuple[dict[str, object], ...] = ()
    error: str | None = None
    malformed_count: int = 0


@dataclass(frozen=True)
class OutboundTargetEvidence:
    checked: bool
    target_ids: tuple[str, ...] = ()
    error: str | None = None
    malformed_count: int = 0


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


def _json_value(value: object) -> object:
    if isinstance(value, bytes):
        value = value.decode("utf-8", errors="replace")
    if not isinstance(value, str):
        return value
    try:
        return json.loads(value)
    except json.JSONDecodeError:
        return value


def _preview_snapshot(value: object) -> CapabilityPreviewSnapshot:
    if isinstance(value, CapabilityPreviewSnapshot):
        return value
    return CapabilityPreviewSnapshot(
        checked=bool(getattr(value, "checked", True)),
        previews=tuple(value),  # type: ignore[arg-type]
        error=getattr(value, "error", None),
        malformed_count=int(getattr(value, "malformed_count", 0)),
    )


def current_turn_capability_previews(
    reborn_home: Path,
    submission_identity: dict[str, object],
    capability_ids: list[str],
) -> CapabilityPreviewSnapshot:
    """Read raw preview payloads for in-memory verification only."""
    thread_id = str(submission_identity.get("thread_id") or "")
    run_id = str(submission_identity.get("run_id") or "")
    if not thread_id or not run_id:
        return CapabilityPreviewSnapshot(
            checked=False,
            error="submission identity omitted thread_id or run_id",
        )
    db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
    if not db_path.exists():
        return CapabilityPreviewSnapshot(
            checked=False,
            error="reborn-local-dev.db missing",
        )
    try:
        with closing(
            sqlite3.connect(
                f"{db_path.resolve().as_uri()}?mode=ro",
                uri=True,
                timeout=0.0,
            )
        ) as db:
            rows = db.execute(
                "SELECT contents FROM root_filesystem_entries "
                "WHERE is_dir = 0 AND content_type = 'application/json' "
                "AND kind = 'thread_message' AND path LIKE ?",
                (f"%/threads/{thread_id}/messages/%",),
            ).fetchall()
    except sqlite3.Error as exc:
        return CapabilityPreviewSnapshot(
            checked=False,
            error=f"{type(exc).__name__}: {exc}",
        )
    wanted = set(capability_ids)
    previews: list[dict[str, object]] = []
    malformed_count = 0
    for (raw_contents,) in rows:
        message = _json_object(raw_contents)
        if message is None:
            malformed_count += 1
            continue
        if (
            message.get("kind") != "capability_display_preview"
            or message.get("thread_id") != thread_id
            or message.get("turn_run_id") != run_id
        ):
            continue
        preview = _json_object(message.get("content"))
        if preview is None:
            malformed_count += 1
            continue
        if preview.get("capability_id") not in wanted:
            continue
        input_payload = _json_object(preview.get("input_summary"))
        if input_payload is None or "output_preview" not in preview:
            malformed_count += 1
            continue
        previews.append(
            {
                "invocation_id": str(preview.get("invocation_id") or ""),
                "capability_id": str(preview.get("capability_id") or ""),
                "input": input_payload,
                "output": _json_value(preview.get("output_preview")),
            }
        )
    if malformed_count:
        return CapabilityPreviewSnapshot(
            checked=False,
            previews=tuple(previews),
            error="malformed current-turn capability display preview evidence",
            malformed_count=malformed_count,
        )
    return CapabilityPreviewSnapshot(checked=True, previews=tuple(previews))


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
        create_output = create.get("output")
        insert_input = insert.get("input")
        read_input = read.get("input")
        read_output = read.get("output")
        if (
            isinstance(create_output, dict)
            and isinstance(insert_input, dict)
            and isinstance(read_input, dict)
            and isinstance(read_output, (dict, str))
        ):
            created_id = str(create_output.get("document_id") or "")
            inserted_id = str(insert_input.get("document_id") or "")
            read_input_id = str(read_input.get("document_id") or "")
            inserted_phrase = phrase in str(insert_input.get("text") or "")
            if isinstance(read_output, dict):
                read_output_id = str(read_output.get("document_id") or "")
                authoritative_text = str(read_output.get("content") or "")
            else:
                authoritative_text = read_output
            readback = phrase in authoritative_text
    same_document = bool(
        created_id
        and created_id == inserted_id == read_input_id
        and (not read_output_id or read_output_id == created_id)
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
    preview_reader: Callable[..., object] = current_turn_capability_previews,
) -> OutboundTargetEvidence:
    target_ids: list[str] = []
    malformed_count = 0
    if not submission_identities:
        return OutboundTargetEvidence(
            checked=False,
            error="no submitted turn identity available for outbound target evidence",
        )
    for identity in submission_identities:
        evidence = terminal_reader(
            reborn_home, identity, [capability_id], {"completed"}
        )
        if evidence.get("read_error"):
            return OutboundTargetEvidence(
                checked=False,
                error=str(evidence["read_error"]),
            )
        sequence = evidence.get("terminal_sequence")
        terminal_ids = {
            str(item.get("invocation_id") or "")
            for item in sequence
            if isinstance(item, dict)
            and item.get("capability_id") == capability_id
            and item.get("status") == "completed"
        } if isinstance(sequence, list) else set()
        preview_snapshot = _preview_snapshot(
            preview_reader(reborn_home, identity, [capability_id])
        )
        if not preview_snapshot.checked:
            return OutboundTargetEvidence(
                checked=False,
                error=preview_snapshot.error,
                malformed_count=preview_snapshot.malformed_count,
            )
        malformed_count += preview_snapshot.malformed_count
        for preview in preview_snapshot.previews:
            if str(preview.get("invocation_id") or "") not in terminal_ids:
                continue
            target = outbound_delivery_target_for_channel(
                preview.get("output"), "slack"
            )
            if target is not None:
                target_id = str(target.get("target_id") or "")
                if target_id and target_id not in target_ids:
                    target_ids.append(target_id)
    return OutboundTargetEvidence(
        checked=True,
        target_ids=tuple(target_ids),
        malformed_count=malformed_count,
    )


def current_turn_trigger_create_invocations(
    reborn_home: Path,
    submission_identities: list[dict[str, object]],
    *,
    terminal_reader: Callable[..., dict[str, object]],
    preview_reader: Callable[..., object] = current_turn_capability_previews,
) -> TriggerInvocationEvidence:
    invocations: dict[str, TriggerCreateInvocation] = {}
    all_runs_terminal = bool(submission_identities)
    malformed_count = 0
    if not submission_identities:
        return TriggerInvocationEvidence(
            checked=False,
            error="no submitted turn identity available for trigger evidence",
        )
    for identity in submission_identities:
        evidence = terminal_reader(
            reborn_home,
            identity,
            [TRIGGER_CREATE_CAPABILITY_ID],
            {"completed"},
        )
        if evidence.get("read_error"):
            return TriggerInvocationEvidence(
                checked=False,
                error=str(evidence["read_error"]),
            )
        all_runs_terminal = all_runs_terminal and evidence.get("run_terminal") is True
        sequence = evidence.get("terminal_sequence")
        terminal_scopes = {
            str(item.get("invocation_id") or ""): str(item.get("tenant_id") or "")
            for item in sequence
            if isinstance(item, dict)
            and item.get("capability_id") == TRIGGER_CREATE_CAPABILITY_ID
            and item.get("status") == "completed"
        } if isinstance(sequence, list) else {}
        if any(not tenant_id for tenant_id in terminal_scopes.values()):
            return TriggerInvocationEvidence(
                checked=False,
                error="terminal trigger_create evidence omitted tenant_id",
                malformed_count=1,
            )
        preview_snapshot = _preview_snapshot(
            preview_reader(reborn_home, identity, [TRIGGER_CREATE_CAPABILITY_ID])
        )
        if not preview_snapshot.checked:
            return TriggerInvocationEvidence(
                checked=False,
                error=preview_snapshot.error,
                malformed_count=preview_snapshot.malformed_count,
            )
        malformed_count += preview_snapshot.malformed_count
        matched_preview_ids: set[str] = set()
        for preview in preview_snapshot.previews:
            invocation_id = str(preview.get("invocation_id") or "")
            if not invocation_id or invocation_id not in terminal_scopes:
                continue
            try:
                invocations[invocation_id] = TriggerCreateInvocation.from_preview(
                    preview,
                    tenant_id=terminal_scopes[invocation_id],
                )
                matched_preview_ids.add(invocation_id)
            except (ValueError, json.JSONDecodeError, TypeError):
                malformed_count += 1
        missing_previews = set(terminal_scopes) - matched_preview_ids
        if all_runs_terminal and (missing_previews or malformed_count):
            return TriggerInvocationEvidence(
                checked=False,
                error="malformed or missing terminal trigger_create preview evidence",
                malformed_count=malformed_count + len(missing_previews),
            )
    return TriggerInvocationEvidence(
        checked=True,
        invocations=tuple(invocations.values()),
        run_terminal=all_runs_terminal,
        malformed_count=malformed_count,
    )


async def wait_for_current_turn_trigger_validation(
    reborn_home: Path,
    before: TriggerSnapshot,
    submission_identities: list[dict[str, object]],
    *,
    terminal_reader: Callable[..., dict[str, object]],
    preview_reader: Callable[..., object] = current_turn_capability_previews,
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
        validation.inconclusive,
    )
