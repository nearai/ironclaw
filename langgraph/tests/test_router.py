"""Tests for the submission parser — mirrors Rust router/submission tests."""

import pytest
from ironclaw.nodes.router import SubmissionKind, parse_submission


def test_undo():
    s = parse_submission("/undo")
    assert s.kind == SubmissionKind.Undo


def test_interrupt():
    for cmd in ("/interrupt", "/stop"):
        s = parse_submission(cmd)
        assert s.kind == SubmissionKind.Interrupt


def test_job_create():
    s = parse_submission("/job build a website")
    assert s.kind == SubmissionKind.CreateJob
    assert s.args["description"] == "build a website"


def test_job_status_all():
    s = parse_submission("/list")
    assert s.kind == SubmissionKind.JobStatus
    assert s.args["job_id"] is None


def test_job_status_with_id():
    s = parse_submission("/status abc-123")
    assert s.kind == SubmissionKind.JobStatus
    assert s.args["job_id"] == "abc-123"


def test_job_cancel():
    s = parse_submission("/cancel my-job-id")
    assert s.kind == SubmissionKind.JobCancel
    assert s.args["job_id"] == "my-job-id"


def test_approval_yes():
    for word in ("yes", "y", "approve", "ok", "Yes", "YES"):
        s = parse_submission(word)
        assert s.kind == SubmissionKind.ApprovalResponse
        assert s.args["approved"] is True


def test_approval_always():
    for word in ("always", "a"):
        s = parse_submission(word)
        assert s.kind == SubmissionKind.ApprovalResponse
        assert s.args["approved"] is True
        assert s.args["always"] is True


def test_approval_no():
    for word in ("no", "n", "deny", "reject", "cancel"):
        s = parse_submission(word)
        assert s.kind == SubmissionKind.ApprovalResponse
        assert s.args["approved"] is False


def test_system_command_help():
    s = parse_submission("/help")
    assert s.kind == SubmissionKind.SystemCommand
    assert s.args["command"] == "help"


def test_quit():
    s = parse_submission("/quit")
    assert s.kind == SubmissionKind.SystemCommand
    assert s.args["command"] == "quit"


def test_natural_language_is_user_input():
    s = parse_submission("What's the weather today?")
    assert s.kind == SubmissionKind.UserInput
    assert s.args["content"] == "What's the weather today?"


def test_thread_switch():
    uuid = "550e8400-e29b-41d4-a716-446655440000"
    s = parse_submission(f"/thread {uuid}")
    assert s.kind == SubmissionKind.SwitchThread
    assert s.args["thread_id"] == uuid
