#!/usr/bin/env python3
"""Fail loud when a WS12 lane is removed or silently disconnected."""

from __future__ import annotations

import re
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
        "Validate product-surface evidence contracts",
        "tests/e2e/scenarios/test_product_surface_coverage.py",
        "tests/e2e/scenarios/test_journey_coverage.py",
        "tests/e2e/scenarios/test_reborn_qa_trace_full_path.py",
        "tests/e2e/scenarios/test_provider_fault_proxy.py",
        "tests/e2e/product_surface_coverage.py",
        "uses: ./.github/actions/setup-sccache-dist",
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

UNCONDITIONAL_SKIP = re.compile(
    r"""(?mx)
    ^[ \t]*if[ \t]*:[ \t]*
    (?:
        ["']?[ \t]*false[ \t]*["']?[ \t]*$
        |
        [|>][-+]?[ \t]*\n[ \t]+["']?[ \t]*false[ \t]*["']?[ \t]*$
        |
        \$\{\{[ \t]*false[ \t]*\}\}[ \t]*$
    )
    """
)

# The Reborn E2E workflow decides "is this change in scope?" twice: a `paths:`
# glob list for push runs, and a mirrored grep -E in the `changes` job for
# pull_request/merge_group. Both are path filters, so neither can assert
# anything about itself — a filter that matches nothing skips every job and the
# roll-up reports success. That is the WS10 failure mode
# (docs/reborn/target-architecture/CHECKLIST.md), and it arrives silently the
# day crates move into family directories.
#
# So the pin lives here: extract the `changes`-job regex from the workflow text
# and replay real paths through it, including a crate nested one level down.
E2E_WORKFLOW = ".github/workflows/reborn-e2e.yml"
E2E_SCOPE_REGEX = re.compile(r"grep -Eq '(\^\([^']+\))'")
E2E_PATHS_GLOB = '- "crates/**"'

# (path, must_be_in_scope)
E2E_SCOPE_PROBES: tuple[tuple[str, bool], ...] = (
    ("crates/ironclaw_webui/src/lib.rs", True),
    # The target-architecture layout. A `crates/ironclaw_[^/]+/` filter misses
    # every one of these.
    ("crates/substrates/ironclaw_events/src/lib.rs", True),
    ("crates/extensions/packages/slack/manifest.toml", True),
    ("docs/reborn/target-architecture/CHECKLIST.md", True),
    ("tests/e2e/scenarios/test_reborn_blackbox_smoke.py", True),
    ("Cargo.toml", True),
    # Still out of scope: the filter must stay a filter.
    ("README.md", False),
    ("docs/plans/whatever.md", False),
    (".github/workflows/code_style.yml", False),
    ("src/main.rs", False),
)


def validate_e2e_scope_filters(text: str) -> list[str]:
    """Return every way the Reborn E2E scope filters could scan nothing."""
    errors: list[str] = []

    if E2E_PATHS_GLOB not in text:
        errors.append(
            f"{E2E_WORKFLOW}: the push `paths:` filter must contain {E2E_PATHS_GLOB} "
            "so it keeps matching when crates move into family directories"
        )

    match = E2E_SCOPE_REGEX.search(text)
    if match is None:
        errors.append(
            f"{E2E_WORKFLOW}: could not find the `changes` job scope regex "
            "(grep -Eq '^(...)') — it is the only scope gate for pull_request and "
            "merge_group runs and must stay assertable"
        )
        return errors

    scope = re.compile(match.group(1))
    for path, expected in E2E_SCOPE_PROBES:
        if bool(scope.search(path)) != expected:
            verdict = "must be in scope" if expected else "must NOT be in scope"
            errors.append(
                f"{E2E_WORKFLOW}: scope regex {match.group(1)!r} — {path!r} {verdict}"
            )
    return errors


def validate_workflow_texts(workflows: dict[str, str]) -> list[str]:
    """Return every missing lane marker; an empty result is the only pass."""
    errors: list[str] = []
    for path, markers in REQUIRED_MARKERS.items():
        text = workflows.get(path)
        if text is None:
            errors.append(f"missing workflow: {path}")
            continue
        errors.extend(
            f"{path}: missing {marker!r}" for marker in markers if marker not in text
        )
        if UNCONDITIONAL_SKIP.search(text):
            errors.append(f"{path}: contains an unconditionally skipped lane")
    e2e = workflows.get(E2E_WORKFLOW)
    if e2e is not None:
        errors.extend(validate_e2e_scope_filters(e2e))
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
