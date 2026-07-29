"""Contract tests for bounded Reborn Playwright diagnostic artifacts."""

import os

import pytest

from reborn_webui_harness import (
    _artifact_max_bytes,
    _directory_size,
    _enforce_artifact_budget,
)


def _write_bundle(root, name: str, sizes: list[int], mtime: int):
    bundle = root / name
    bundle.mkdir()
    for index, size in enumerate(sizes):
        (bundle / f"artifact-{index}.bin").write_bytes(b"x" * size)
    os.utime(bundle, ns=(mtime, mtime))
    return bundle


def test_artifact_budget_removes_oldest_context_bundle(tmp_path):
    browser_root = tmp_path / "browser"
    browser_root.mkdir()
    oldest = _write_bundle(browser_root, "oldest", [8], 1)
    current = _write_bundle(browser_root, "current", [8], 2)

    _enforce_artifact_budget(browser_root, 10, current)

    assert not oldest.exists()
    assert current.exists()
    assert _directory_size(browser_root) <= 10


def test_artifact_budget_prunes_largest_file_from_oversized_current_bundle(tmp_path):
    browser_root = tmp_path / "browser"
    browser_root.mkdir()
    current = _write_bundle(browser_root, "current", [3, 9], 1)

    _enforce_artifact_budget(browser_root, 7, current)

    assert (current / "artifact-0.bin").exists()
    assert not (current / "artifact-1.bin").exists()
    assert _directory_size(browser_root) <= 7


@pytest.mark.parametrize("raw_value", ["0", "not-a-number"])
def test_artifact_budget_rejects_invalid_environment_values(
    monkeypatch,
    raw_value,
):
    monkeypatch.setenv("IRONCLAW_E2E_ARTIFACT_MAX_BYTES", raw_value)

    with pytest.raises(ValueError, match="must be a positive integer"):
        _artifact_max_bytes()
