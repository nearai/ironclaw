"""Contract tests for bounded Reborn Playwright diagnostic artifacts."""

import asyncio
import os

import pytest

import reborn_webui_harness as harness
from reborn_webui_harness import (
    _ArtifactContext,
    _artifact_max_bytes,
    _directory_size,
    _drain_stream_to_bounded_file,
    _enforce_artifact_budget,
    _finalize_registered_artifact_bundles,
    _mark_artifact_bundle_outcome,
    _mark_registered_artifact_bundles_failed,
    _register_artifact_bundle,
)


def _write_bundle(root, name: str, sizes: list[int], mtime: int):
    bundle = root / name
    bundle.mkdir()
    for index, size in enumerate(sizes):
        (bundle / f"artifact-{index}.bin").write_bytes(b"x" * size)
    os.utime(bundle, ns=(mtime, mtime))
    return bundle


def test_artifact_budget_removes_oldest_context_bundle(tmp_path):
    artifact_root = tmp_path / "artifacts"
    browser_root = artifact_root / "browser"
    browser_root.mkdir(parents=True)
    oldest = _write_bundle(browser_root, "oldest", [8], 1)
    current = _write_bundle(browser_root, "current", [8], 2)

    _enforce_artifact_budget(artifact_root, 10, current)

    assert not oldest.exists()
    assert current.exists()
    assert _directory_size(artifact_root) <= 10


def test_artifact_budget_prunes_largest_file_from_oversized_current_bundle(tmp_path):
    artifact_root = tmp_path / "artifacts"
    browser_root = artifact_root / "browser"
    browser_root.mkdir(parents=True)
    current = _write_bundle(browser_root, "current", [3, 9], 1)

    _enforce_artifact_budget(artifact_root, 7, current)

    assert (current / "artifact-0.bin").exists()
    assert not (current / "artifact-1.bin").exists()
    assert _directory_size(artifact_root) <= 7


def test_artifact_budget_preserves_failed_bundle_before_successful_bundle(tmp_path):
    artifact_root = tmp_path / "artifacts"
    browser_root = artifact_root / "browser"
    browser_root.mkdir(parents=True)
    failed = _write_bundle(browser_root, "failed", [8], 1)
    current = _write_bundle(browser_root, "current", [8], 2)
    _mark_artifact_bundle_outcome(failed, "failed")

    _enforce_artifact_budget(artifact_root, 10, current)

    assert (failed / "artifact-0.bin").exists()
    assert not (current / "artifact-0.bin").exists()
    assert _directory_size(artifact_root) <= 10


def test_artifact_budget_prunes_failed_bundle_to_enforce_hard_limit(tmp_path):
    artifact_root = tmp_path / "artifacts"
    browser_root = artifact_root / "browser"
    browser_root.mkdir(parents=True)
    failed = _write_bundle(browser_root, "failed", [12], 1)
    _mark_artifact_bundle_outcome(failed, "failed")

    _enforce_artifact_budget(artifact_root, 7, failed)

    assert not (failed / "artifact-0.bin").exists()
    assert (failed / ".pytest-outcome-failed").exists()
    assert _directory_size(artifact_root) <= 7


def test_artifact_budget_includes_server_logs_in_uploaded_tree(tmp_path):
    artifact_root = tmp_path / "artifacts"
    browser_root = artifact_root / "browser"
    server_log_root = artifact_root / "server-logs" / "server"
    browser_root.mkdir(parents=True)
    server_log_root.mkdir(parents=True)
    current = _write_bundle(browser_root, "current", [6], 2)
    (server_log_root / "stderr.log").write_bytes(b"x" * 8)

    _enforce_artifact_budget(artifact_root, 10, current)

    assert current.exists()
    assert _directory_size(artifact_root) <= 10


async def test_server_log_drain_retains_a_bounded_tail(tmp_path):
    stream = asyncio.StreamReader()
    stream.feed_data(b"0123456789abcdef")
    stream.feed_eof()
    log_path = tmp_path / "server.log"

    await _drain_stream_to_bounded_file(stream, log_path, 8)

    assert log_path.stat().st_size <= 8
    assert log_path.read_bytes() == b"cdef"


class _FakeTracing:
    async def stop(self, *, path):
        del path


class _FakeContext:
    def __init__(self):
        self.pages = []
        self.tracing = _FakeTracing()
        self.closed = False

    async def close(self):
        self.closed = True


async def test_artifact_cleanup_error_does_not_mask_scenario_result(
    monkeypatch,
    tmp_path,
):
    context = _FakeContext()
    artifact_dir = tmp_path / "artifacts" / "browser" / "current"
    artifact_dir.mkdir(parents=True)

    def fail_cleanup(*args):
        del args
        raise PermissionError("cleanup denied")

    monkeypatch.setattr(harness, "_enforce_artifact_budget", fail_cleanup)
    wrapped = _ArtifactContext(context, artifact_dir, tmp_path / "artifacts", 10)

    await wrapped.close()

    assert context.closed


def test_artifact_outcome_cleanup_error_does_not_mask_scenario_result(
    monkeypatch,
    tmp_path,
):
    artifact_root = tmp_path / "artifacts"
    browser_root = artifact_root / "browser"
    browser_root.mkdir(parents=True)
    artifact_dir = _write_bundle(browser_root, "current", [8], 1)
    node_id = "scenario.py::test_cleanup_error"
    _register_artifact_bundle(node_id, artifact_root, artifact_dir, 10)

    def fail_cleanup(*args):
        del args
        raise PermissionError("cleanup denied")

    monkeypatch.setattr(harness, "_enforce_artifact_budget", fail_cleanup)

    _finalize_registered_artifact_bundles(node_id)

    assert not (artifact_dir / ".pytest-outcome-pending").exists()


@pytest.mark.parametrize("failed", [False, True], ids=["passed", "failed"])
def test_artifact_outcome_is_finalized_after_pytest_teardown(
    tmp_path,
    failed,
):
    artifact_root = tmp_path / "artifacts"
    browser_root = artifact_root / "browser"
    browser_root.mkdir(parents=True)
    artifact_dir = _write_bundle(browser_root, "current", [8], 1)
    node_id = f"scenario.py::test_outcome[{failed}]"
    _register_artifact_bundle(node_id, artifact_root, artifact_dir, 10)
    assert (artifact_dir / ".pytest-outcome-pending").exists()

    if failed:
        _mark_registered_artifact_bundles_failed(node_id)
    _finalize_registered_artifact_bundles(node_id)

    assert not (artifact_dir / ".pytest-outcome-pending").exists()
    assert (artifact_dir / ".pytest-outcome-failed").exists() is failed


@pytest.mark.parametrize("raw_value", ["0", "not-a-number"])
def test_artifact_budget_rejects_invalid_environment_values(
    monkeypatch,
    raw_value,
):
    monkeypatch.setenv("IRONCLAW_E2E_ARTIFACT_MAX_BYTES", raw_value)

    with pytest.raises(ValueError, match="must be a positive integer"):
        _artifact_max_bytes()
