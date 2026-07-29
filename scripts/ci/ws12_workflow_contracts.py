#!/usr/bin/env python3
"""Fail loud when a WS12 lane is removed or silently disconnected."""

from __future__ import annotations

import sys
from pathlib import Path

REQUIRED_MARKERS: dict[str, tuple[str, ...]] = {
    ".github/workflows/reborn-tests.yml": (
        "merge_group:",
        "push:",
        "PROPTEST_CASES: ${{ inputs.deep_generations && '2048' || '256' }}",
        "python3 scripts/ci/test_reborn_changed_coverage.py",
        "python3 scripts/ci/reborn_changed_coverage.py",
    ),
    ".github/workflows/reborn-e2e.yml": (
        "merge_group:",
        "push:",
        "tests/e2e/scenarios/test_journey_coverage.py",
        "tests/e2e/scenarios/test_reborn_qa_trace_full_path.py",
        "tests/e2e/scenarios/test_provider_fault_proxy.py",
        "tests/e2e/product_surface_coverage.py",
    ),
    ".github/workflows/nightly-deep-ci.yml": (
        "schedule:",
        "mutation-frontier:",
        "scripts/test-mutation-audit.sh",
        "scripts/mutation-audit.sh",
    ),
    ".github/workflows/ironclaw-stress.yml": (
        "schedule:",
        "libsql-user-session-soak:",
        "--preset soak-user-session",
        "postgres-api-capacity:",
        "cargo build --locked --profile dist",
        "target/dist/ironclaw serve",
    ),
    ".github/workflows/live-canary.yml": (
        '- cron: "0 */3 * * *"',
        '- cron: "30 5 * * 1"',
        "github.event.schedule == '0 */3 * * *'",
        "github.event.schedule == '30 5 * * 1'",
        "provider-matrix:",
    ),
    ".github/workflows/reborn-playwright.yml": (
        "python3 scripts/ci/ws12_suite_shards.py --github-output",
        'test "${{ matrix.retry }}" = "never"',
    ),
    ".github/workflows/ironclaw-release.yml": (
        "Smoke exact binaries before packaging upload",
        "scripts/ci/smoke-release-binary.py",
    ),
}


def validate_workflow_texts(workflows: dict[str, str]) -> list[str]:
    """Return every missing lane marker; an empty result is the only pass."""
    errors: list[str] = []
    for path, markers in REQUIRED_MARKERS.items():
        text = workflows.get(path)
        if text is None:
            errors.append(f"missing workflow: {path}")
            continue
        for marker in markers:
            if marker not in text:
                errors.append(f"{path}: missing {marker!r}")
        if "if: false" in text or "if: ${{ false }}" in text:
            errors.append(f"{path}: contains an unconditionally skipped lane")
    return errors


def load_workflows(root: Path) -> dict[str, str]:
    return {
        path: (root / path).read_text(encoding="utf-8") for path in REQUIRED_MARKERS
    }


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    try:
        errors = validate_workflow_texts(load_workflows(root))
    except OSError as error:
        print(f"WS12 workflow contract failed: {error}", file=sys.stderr)
        return 1
    if errors:
        for error in errors:
            print(f"WS12 workflow contract failed: {error}", file=sys.stderr)
        return 1
    print("WS12 workflow contracts passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
