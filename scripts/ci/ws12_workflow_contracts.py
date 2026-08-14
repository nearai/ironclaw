#!/usr/bin/env python3
"""Fail loud when a WS12 lane is removed or silently disconnected."""

from __future__ import annotations

import dataclasses
import glob
import os
import pathlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))

from crate_tree import (  # noqa: E402
    CrateTreeError,
    crate_directories,
    crate_directory,
)

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
        "libsql-scripted-memory:",
        'profile = "hosted-single-tenant-volume"',
        "memory_roundtrip",
        "memory_grow",
        "memory_mixed",
        "--api-scripted-doc-sizes 4096,32768,131072,1048576",
        "--api-poll-interval-ms 10000",
        "--api-terminal-timeout-ms 120000",
        "--max-p95-ms 120000",
        "--max-failure-rate 0 \\",
        "ironclaw-stress-libsql-scripted-${script}",
        "libsql-user-session-soak:",
        "--preset soak-user-session",
        "postgres-api-capacity:",
        "cargo build --locked --profile dist",
        "target/dist/ironclaw serve",
        "ironclaw-stress-postgres-scripted-memory-roundtrip",
    ),
    ".github/workflows/live-canary.yml": (
        '- cron: "0 */3 * * *"',
        "github.event.schedule == '0 */3 * * *'",
    ),
    ".github/workflows/code_style.yml": (
        # The docs publication-boundary gate: the job, its self-test step, its
        # check step, and the roll-up guard that fails closed BEFORE the
        # has_code early exit (docs-only PRs have has_code=false, so a guard
        # placed after it could never block).
        "docs-publication-boundary:",
        "python3 scripts/ci/test_docs_publication_boundary.py",
        "python3 scripts/ci/docs_publication_boundary.py",
        '"${{ needs.docs-publication-boundary.result }}" != "success"',
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
# (docs/internal/reborn/target-architecture/CHECKLIST.md), and it arrives silently the
# day crates move into family directories.
#
# So the pin lives here: extract the `changes`-job regex from the workflow text
# and replay real paths through it, including a crate nested one level down.
E2E_WORKFLOW = ".github/workflows/reborn-e2e.yml"
# `grep -Eq` and its pattern may be separated by an escaped-newline
# continuation — a normal way to keep a long guard readable. A guardrail that
# only understands the one-line form reports a perfectly good guard as missing
# and fails the build for a formatting choice (.claude/rules/review-discipline.md,
# "Guardrails are code": checks must handle multiline syntax).
E2E_SCOPE_REGEX = re.compile(r"grep -Eq(?:[ \t]+|[ \t]*\\\n[ \t]*)'(\^\([^']+\))'")
E2E_PATHS_GLOB = '- "crates/**"'

# (path, must_be_in_scope)
E2E_SCOPE_PROBES: tuple[tuple[str, bool], ...] = (
    ("crates/ironclaw_webui/src/lib.rs", True),
    # The target-architecture layout. A `crates/ironclaw_[^/]+/` filter misses
    # every one of these.
    ("crates/substrates/ironclaw_event_log/src/lib.rs", True),
    ("crates/extensions/packages/slack/manifest.toml", True),
    ("docs/internal/reborn/target-architecture/CHECKLIST.md", True),
    ("tests/e2e/scenarios/test_reborn_blackbox_smoke.py", True),
    ("Cargo.toml", True),
    # Still out of scope: the filter must stay a filter.
    ("README.md", False),
    ("docs/internal/plans/whatever.md", False),
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


# ---------------------------------------------------------------------------
# Crate-keyed scope filters (#6963)
#
# The E2E block above pins one workflow's scope filter. Three more filters key
# on the flat `crates/ironclaw_*` shape and go dark the same way when crates
# move into family directories: `code_style.yml`'s `has_reborn_cli` (the
# dist-build lane skips), `platform-and-compat.yml`'s
# `has_direct_wasm_abi_risk` (every WASM ABI check skips), and
# `ironclaw-stress.yml`'s `paths:` filter (the workflow stops triggering at
# all). None of them can assert anything about itself — a path filter that
# matches nothing looks exactly like "nothing in scope".
#
# So the pin lives here, and it is inventory-driven rather than probe-only:
# every crate NAME a filter enumerates is resolved against the real crate
# inventory (scripts/ci/lib/crate_tree.py), and the filter must match that
# crate's ACTUAL directory plus a nested equivalent. That gives three
# fail-closed properties a probe list alone cannot:
#
#   1. a filter naming a crate that no longer exists is an error — the class
#      that let `crates/ironclaw_wasm_product_adapters/` sit in the WASM filter
#      long after the crate was deleted, matching nothing forever;
#   2. a filter that stops matching a crate's real location is an error, so a
#      rename or a deeper-than-expected move fails in Code Style instead of
#      silently unhooking a lane;
#   3. a filter that matches nothing at all is an error, because every probe
#      set is required to be non-empty and every discovered-file probe is
#      required to have discovered a file.
# ---------------------------------------------------------------------------

CODE_STYLE_WORKFLOW = ".github/workflows/code_style.yml"
PLATFORM_WORKFLOW = ".github/workflows/platform-and-compat.yml"
STRESS_WORKFLOW = ".github/workflows/ironclaw-stress.yml"

# ---------------------------------------------------------------------------
# Docs publication-boundary guard ordering
#
# The guard in the code-style roll-up must run BEFORE the has_code early
# exit: a docs-only PR has has_code=false, so a guard placed after `exit 0`
# can never block. REQUIRED_MARKERS is presence-only and cannot see order —
# relocating the guard below the early exit leaves every marker in the file
# while the gate is fully broken — so the ordering is pinned separately here.
# ---------------------------------------------------------------------------

CODE_STYLE_DOCS_GUARD_MARKER = (
    '"${{ needs.docs-publication-boundary.result }}" != "success"'
)
CODE_STYLE_HAS_CODE_EXIT_MARKER = (
    'echo "No code changes — style checks skipped correctly"'
)


def validate_code_style_docs_guard_order(text: str) -> list[str]:
    """Return every way the docs-gate guard could sit past the early exit.

    Only executable occurrences count: comment lines are stripped first, so a
    commented-out copy of the guard above the early exit (a refactor
    leftover) cannot satisfy the pin, and EVERY live guard occurrence must
    precede the first early-exit occurrence.
    """

    executable = "\n".join(
        line for line in text.splitlines() if not line.lstrip().startswith("#")
    )
    early_exit = executable.find(CODE_STYLE_HAS_CODE_EXIT_MARKER)
    guard_positions: list[int] = []
    cursor = executable.find(CODE_STYLE_DOCS_GUARD_MARKER)
    while cursor != -1:
        guard_positions.append(cursor)
        cursor = executable.find(CODE_STYLE_DOCS_GUARD_MARKER, cursor + 1)
    if not guard_positions or early_exit == -1:
        # Presence itself is REQUIRED_MARKERS' job; report only what this pin
        # cannot delegate — a missing EXECUTABLE anchor makes the order
        # unassertable (a comment-only occurrence lands here on purpose).
        missing = "guard" if not guard_positions else "has_code early-exit"
        return [
            (
                f"{CODE_STYLE_WORKFLOW}: docs-gate guard order unassertable — "
                f"no executable {missing} marker"
            )
        ]
    if any(position > early_exit for position in guard_positions):
        return [
            (
                f"{CODE_STYLE_WORKFLOW}: the docs publication-boundary guard "
                "sits after the has_code early exit — docs-only PRs "
                "(has_code=false) exit 0 before the guard runs, so the gate "
                "cannot block them. Move the guard above the early exit in "
                "the roll-up step"
            )
        ]
    return []


# ---------------------------------------------------------------------------
# libsql-scripted-memory job contract (#7360)
#
# REQUIRED_MARKERS pins tokens across the WHOLE workflow file, so it cannot
# tell which job a token belongs to — `profile = "hosted-single-tenant-volume"`
# appears in more than one job, and a marker satisfied by a neighbouring lane
# reads as green while this lane is broken. This pin is scoped to ONE job:
# the `libsql-scripted-memory` block is extracted from its two-space job key
# through the next two-space job key, and every invariant below is checked
# inside that block only. Exactly-one resolution is the contract: zero
# matches means the lane is gone, two means the key was duplicated — either
# way the pins cannot say which block they validated, so both refuse.
#
# The checked-in workflow is GREEN under this contract. The sabotage tests in
# test_ws12_workflow_contracts.py normalize EITHER source shape — the
# historical RED one (`--operations 2`, an EXIT trap that kills without
# waiting, the server log embedded in every per-script upload, final
# readiness curls without timeout flags) or the checked-in GREEN one — to
# one canonical compliant fixture, then break exactly one piece per test.
# ---------------------------------------------------------------------------

LIBSQL_SCRIPTED_MEMORY_JOB = "libsql-scripted-memory"
LIBSQL_SCRIPTED_SCRIPTS = ("memory_roundtrip", "memory_grow", "memory_mixed")
# The loop line must enumerate exactly the three scripts in order — a
# parametrized list, a dropped scenario, a reordered one, or an extra entry
# all silently change what the matrix measures. `; do` (or end of line)
# anchors the enumeration so a fourth name appended after `memory_mixed`
# cannot pass.
LIBSQL_SCRIPTED_LOOP = re.compile(
    r"for[ \t]+script[ \t]+in[ \t]+memory_roundtrip[ \t]+memory_grow"
    r"[ \t]+memory_mixed[ \t]*(?:;[ \t]*do|$)"
)
# Lookaheads keep the value EXACT: `--operations 14`/`--operations 4.0` and
# `--max-failure-rate 0.5`/`10` must not satisfy their pins.
LIBSQL_SCRIPTED_OPERATIONS = re.compile(r"--operations[ \t]+4(?![0-9.])")
LIBSQL_SCRIPTED_MAX_FAILURE_RATE = re.compile(r"--max-failure-rate[ \t]+0(?![0-9.])")
# The four sizes, then end of line (a trailing ` \` continuation is fine): a
# fifth size appended after `1048576` is rejected by the `(?![0-9,])`
# lookahead, and the argument must stay on one line.
LIBSQL_SCRIPTED_DOC_SIZES = re.compile(
    r"--api-scripted-doc-sizes[ \t]+4096,32768,131072,1048576(?![0-9,])"
    r"[ \t]*(?:\\[ \t]*)?$",
    re.MULTILINE,
)
LIBSQL_SCRIPTED_PROFILE = 'profile = "hosted-single-tenant-volume"'
LIBSQL_SERVER_LOG_PATH = "target/ironclaw-stress/libsql-scripted-server.log"
LIBSQL_SERVER_LOG_NAME = "ironclaw-stress-libsql-scripted-server-log"
LIBSQL_PER_SCRIPT_ARTIFACT_PREFIX = "ironclaw-stress-libsql-scripted-"

# Exact-value pins for the runner flags the matrix is paid to enforce.
# Lookaheads keep the values EXACT: `--api-hot-writers 2.0`/`20`,
# `--mock-llm-bind 127.0.0.1:19091`, `--api-poll-interval-ms 2000`,
# `--api-terminal-timeout-ms 60000`, and `--max-p95-ms 30000`/`1200000`
# must not satisfy their pins.
LIBSQL_SCRIPTED_HOT_WRITERS = re.compile(r"--api-hot-writers[ \t]+2(?![0-9.])")
LIBSQL_SCRIPTED_MOCK_BIND = re.compile(
    r"--mock-llm-bind[ \t]+127\.0\.0\.1:19090(?![0-9])"
)
LIBSQL_SCRIPTED_POLL_INTERVAL = re.compile(
    r"--api-poll-interval-ms[ \t]+10000(?![0-9.])"
)
LIBSQL_SCRIPTED_TERMINAL_TIMEOUT = re.compile(
    r"--api-terminal-timeout-ms[ \t]+120000(?![0-9.])"
)
LIBSQL_SCRIPTED_P95 = re.compile(r"--max-p95-ms[ \t]+120000(?![0-9.])")

# The loop must survive a failed invocation under `set -e`: the runner
# call's `|| failed=1` tail records the failure and lets the next script
# run, so the later scripts still produce and upload their evidence.
# Presence alone cannot see structure, so the exact loop is extracted
# (LIBSQL_SCRIPTED_LOOP_BODY) and the invariants are enforced positionally:
# `failed=0` must initialize the accumulator BEFORE the loop (under `set -u`
# an unset variable aborts the step before any script runs); the outdir
# assignment, `mkdir -p "${outdir}"`, and the guarded invocation must appear
# INSIDE the loop in that order (a mkdir relocated outside the loop leaves
# the upload paths missing exactly when the invocation failed); and the step
# must `exit "$failed"` AFTER the loop's `done` so a recorded failure fails
# the job once every script has had its chance.
LIBSQL_SCRIPTED_FAILED_INIT = re.compile(r"^[ \t]*failed=0[ \t]*$", re.MULTILINE)
LIBSQL_SCRIPTED_OUTDIR_ASSIGN = re.compile(
    r'^[ \t]*outdir="[^"\n]*"[ \t]*$', re.MULTILINE
)
LIBSQL_SCRIPTED_OUTDIR = re.compile(r'mkdir[ \t]+-p[ \t]+"\$\{outdir\}"')
LIBSQL_SCRIPTED_FAILURE_GUARD = re.compile(
    r'2>[ \t]+"\$\{outdir\}/report\.txt"[ \t]*\|\|[ \t]+failed=1[ \t]*$',
    re.MULTILINE,
)
LIBSQL_SCRIPTED_FINAL_EXIT = re.compile(r'exit[ \t]+"\$failed"')
# The exact scripted loop through its own `done`; the captured body is what
# the positional checks above run against.
LIBSQL_SCRIPTED_LOOP_BODY = re.compile(
    r"for[ \t]+script[ \t]+in[ \t]+memory_roundtrip[ \t]+memory_grow"
    r"[ \t]+memory_mixed[ \t]*(?:;[ \t]*do|$)"
    r"(?P<body>.*?)\n[ \t]*done",
    re.DOTALL,
)

# A readiness probe is a `for _ in $(seq 1 N)` loop: finite by construction,
# and capped here so a dead server fails the job instead of burning the whole
# job timeout. `while true` (or a bound past the cap) is the regression.
LIBSQL_PROBE_LOOP = re.compile(
    r"for[ \t]+_[ \t]+in[ \t]+\$\(seq[ \t]+1[ \t]+(?P<bound>[0-9]+)\)"
)
LIBSQL_PROBE_LOOP_BODY = re.compile(
    r"for[ \t]+_[ \t]+in[ \t]+\$\(seq[ \t]+1[ \t]+[0-9]+\);[ \t]*do"
    r"(?P<body>.*?)\n[ \t]*done",
    re.DOTALL,
)
LIBSQL_PROBE_MAX_BOUND = 600
# Every curl inside a probe loop AND after it must carry explicit short
# timeouts: --connect-timeout caps the connect attempt and --max-time caps
# the whole transfer, so a wedged server fails the probe instead of hanging
# past the loop. Values are pinned exactly (5/10 — the same flags the
# in-loop curls use): a missing flag or 50/100 is a regression, not a fix.
LIBSQL_PROBE_TIMEOUT = re.compile(
    r"--connect-timeout[ \t]+5(?![0-9])[ \t]+--max-time[ \t]+10(?![0-9])"
)
# After each bounded loop a final UNCONDITIONAL curl must fail the job when
# the probe never succeeded — otherwise 120 failed tries degrade into a
# matrix run against a dead server. The in-loop `if curl -fsS \` forms do not
# match these (they are prefixed with `if ` / continued differently), and the
# final curl must carry the same explicit timeouts as the loop probes.
LIBSQL_FINAL_HEALTH_PROBE = re.compile(
    r"^[ \t]*curl[ \t]+-fsS[ \t]+"
    r"--connect-timeout[ \t]+5(?![0-9])[ \t]+--max-time[ \t]+10(?![0-9])"
    r"[ \t]+http://127\.0\.0\.1:18080/api/health[ \t]+>/dev/null[ \t]*$",
    re.MULTILINE,
)
LIBSQL_FINAL_SESSION_PROBE = re.compile(
    r"^[ \t]*curl[ \t]+-fsS[ \t]+"
    r"--connect-timeout[ \t]+5(?![0-9])[ \t]+--max-time[ \t]+10(?![0-9])"
    r"[ \t]*\\\n"
    r"[ \t]*-H[ \t]+\"Authorization: Bearer \$IRONCLAW_REBORN_WEBUI_TOKEN\""
    r"[ \t]*\\\n"
    r"[ \t]*http://127\.0\.0\.1:18080/api/webchat/v2/session[ \t]+>/dev/null"
    r"[ \t]*$",
    re.MULTILINE,
)
# Cleanup must kill AND wait: kill alone can leave the port bound when the
# step ends, so the next run collides with a zombie server. The trap must be
# single-quoted so the expansion is DELAYED — `$server_pid` is read when the
# trap fires, not when it is registered — which is what makes the kill/wait
# pair address the process this step actually started.
LIBSQL_SERVER_TRAP = re.compile(r"trap[ \t]+'(?P<cmd>[^']*)'[ \t]+EXIT")

# Two-space job keys (`  name:` at the top level of `jobs:`). Deeper YAML
# keys are indented further and cannot match.
JOB_HEADING = re.compile(r"^  (?P<name>[A-Za-z0-9_-]+):[ \t]*$", re.MULTILINE)
UPLOAD_ARTIFACT_NAME = re.compile(
    r"^[ \t]*name:[ \t]*(?P<name>[^#\n]+?)[ \t]*$", re.MULTILINE
)
UPLOAD_PATH_KEY = re.compile(r"^[ \t]*path:[ \t]*(?:[|>][+-]?)?[ \t]*$", re.MULTILINE)
# Every upload step must run even when the matrix step failed (`if:
# always()`) and must fail the job when its evidence is missing
# (`if-no-files-found: error`) — an upload that silently carries nothing
# hides a lost run behind a green job.
UPLOAD_ALWAYS = re.compile(r"^[ \t]*if:[ \t]+always\(\)[ \t]*$", re.MULTILINE)
UPLOAD_NO_FILES_ERROR = re.compile(
    r"^[ \t]*if-no-files-found:[ \t]+error[ \t]*$", re.MULTILINE
)


def extract_job_block(text: str, job: str) -> tuple[str | None, str]:
    """Return one job's text: from its two-space key line to the next.

    Exactly-one resolution is the contract: zero matches means the lane is
    gone, two means the key was duplicated — either way the scoped pins
    cannot say which block they validated, so both refuse (the same
    fail-closed stance as `extract_scope_regex`).
    """

    headings = list(JOB_HEADING.finditer(text))
    matches = [match for match in headings if match.group("name") == job]
    if len(matches) != 1:
        return None, (
            f"expected exactly one {job!r} job, found {len(matches)} — the "
            "scoped libsql-scripted-memory contract cannot resolve its block"
        )
    start = matches[0].start()
    following = next((m for m in headings if m.start() > start), None)
    return text[start : following.start() if following else len(text)], ""

def extract_continued_commands(text: str, executable: str) -> list[str]:
    """Return shell commands whose first line is `<executable> \\`.

    A command continues while its current line ends in a backslash. This
    keeps flag validation scoped to one invocation instead of allowing a
    same-step `echo` or sibling command to satisfy the contract.
    """

    lines = text.splitlines()
    commands: list[str] = []
    index = 0
    first_line = f"{executable} \\"
    while index < len(lines):
        if lines[index].strip() != first_line:
            index += 1
            continue
        command = [lines[index]]
        while command[-1].rstrip().endswith("\\") and index + 1 < len(lines):
            index += 1
            command.append(lines[index])
        commands.append("\n".join(command))
        index += 1
    return commands



def extract_upload(body: str) -> tuple[str | None, list[str]]:
    """One upload step's artifact name and path list, or (None, []) when the
    step is not a recognizable `actions/upload-artifact` call."""

    name_match = UPLOAD_ARTIFACT_NAME.search(body)
    if name_match is None:
        return None, []
    name = name_match.group("name").strip()
    path_match = UPLOAD_PATH_KEY.search(body)
    if path_match is None:
        return name, []
    indent = len(path_match.group(0)) - len(path_match.group(0).lstrip())
    paths: list[str] = []
    for line in body[path_match.end() :].splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        if len(line) - len(line.lstrip()) <= indent:
            break
        if stripped.startswith("#"):
            continue
        paths.append(stripped)
    return name, paths


def validate_libsql_scripted_memory_job(text: str) -> list[str]:
    """Return every way the libsql-scripted-memory job breaks its contract.

    REQUIRED_MARKERS pins presence file-wide; this pin is scoped to the one
    job block and refuses to resolve it ambiguously. Every error names the
    job and the broken invariant.
    """

    block, detail = extract_job_block(text, LIBSQL_SCRIPTED_MEMORY_JOB)
    if block is None:
        return [f"{STRESS_WORKFLOW}: {detail}"]
    label = f"{STRESS_WORKFLOW} ({LIBSQL_SCRIPTED_MEMORY_JOB} job):"
    errors: list[str] = []

    # Comment lines are inert: a commented-out decoy cannot satisfy a pin
    # (the same stance as validate_code_style_docs_guard_order).
    executable = "\n".join(
        line for line in block.splitlines() if not line.lstrip().startswith("#")
    )

    if not any(
        "'schedule'" in line and "'workflow_dispatch'" in line
        for line in executable.splitlines()
        if line.lstrip().startswith("if:")
    ):
        errors.append(
            f"{label} must gate on both schedule and workflow_dispatch "
            "(if: github.event_name == 'schedule' || github.event_name == "
            "'workflow_dispatch') — the daily scan must stay reachable by hand"
        )
    if LIBSQL_SCRIPTED_PROFILE not in executable:
        errors.append(
            f"{label} must boot the server with {LIBSQL_SCRIPTED_PROFILE!r} — "
            "the libsql volume-backed profile is what the matrix measures"
        )
    if LIBSQL_SCRIPTED_LOOP.search(executable) is None:
        errors.append(
            f"{label} must run the three scripts in one fixed sequential loop — "
            "`for script in memory_roundtrip memory_grow memory_mixed; do` — a "
            "parametrized, reordered, or split loop silently drops a scenario"
        )
    loop = LIBSQL_SCRIPTED_LOOP_BODY.search(executable)
    runner = ""
    if loop is not None:
        commands = extract_continued_commands(
            loop.group("body"), "target/release/ironclaw_stress"
        )
        if len(commands) == 1:
            runner = commands[0]
        else:
            errors.append(
                f"{label} the fixed script loop must contain exactly one "
                "target/release/ironclaw_stress command, found "
                f"{len(commands)}"
            )
    if LIBSQL_SCRIPTED_OPERATIONS.search(runner) is None:
        errors.append(
            f"{label} must run each script with --operations 4 — fewer "
            "operations under-sample the scripted verdicts the matrix is paid "
            "to produce"
        )
    if LIBSQL_SCRIPTED_DOC_SIZES.search(runner) is None:
        errors.append(
            f"{label} must pass exactly the four scripted doc sizes "
            "--api-scripted-doc-sizes 4096,32768,131072,1048576 — the matrix "
            "pins the small-to-large latency curve"
        )
    if LIBSQL_SCRIPTED_MAX_FAILURE_RATE.search(runner) is None:
        errors.append(
            f"{label} must run each script with --max-failure-rate 0 — the "
            "zero-tolerance gate on failed, leaked, or undisclosed scripted "
            "verdicts"
        )
    if LIBSQL_SCRIPTED_HOT_WRITERS.search(runner) is None:
        errors.append(
            f"{label} must run each script with --api-hot-writers 2 — the "
            "hot-writer count is part of what the scripted matrix measures"
        )
    if LIBSQL_SCRIPTED_MOCK_BIND.search(runner) is None:
        errors.append(
            f"{label} must bind the mock LLM sidecar at --mock-llm-bind "
            "127.0.0.1:19090 — that is the address the server's LLM base_url "
            "points at"
        )
    if LIBSQL_SCRIPTED_POLL_INTERVAL.search(runner) is None:
        errors.append(
            f"{label} must poll scripted terminal states every "
            "--api-poll-interval-ms 10000 — the 10s cadence is what the "
            "matrix runs with"
        )
    if LIBSQL_SCRIPTED_TERMINAL_TIMEOUT.search(runner) is None:
        errors.append(
            f"{label} must cap each scripted terminal wait at "
            "--api-terminal-timeout-ms 120000"
        )
    if LIBSQL_SCRIPTED_P95.search(runner) is None:
        errors.append(
            f"{label} must run each script with --max-p95-ms 120000 — the "
            "p95 ceiling the matrix enforces"
        )
    if loop is None:
        errors.append(
            f"{label} the script loop must be one complete "
            "`for script in memory_roundtrip memory_grow memory_mixed; do "
            "... done` block — a split, reordered, or unterminated loop "
            "silently drops a scenario"
        )
    else:
        body = loop.group("body")
        if LIBSQL_SCRIPTED_FAILED_INIT.search(executable[: loop.start()]) is None:
            errors.append(
                f"{label} must initialize `failed=0` before the script loop — "
                "under set -u an unset accumulator aborts the step before any "
                "script runs"
            )
        outdir_assign = LIBSQL_SCRIPTED_OUTDIR_ASSIGN.search(body)
        mkdir = LIBSQL_SCRIPTED_OUTDIR.search(body)
        guard = LIBSQL_SCRIPTED_FAILURE_GUARD.search(body)
        if mkdir is None:
            errors.append(
                f"{label} must create each script's outdir inside the loop "
                '(`mkdir -p "${outdir}"` after the outdir= assignment, before '
                "the invocation) — the per-script upload paths must exist even "
                "when an invocation fails"
            )
        if outdir_assign is None:
            errors.append(
                f"{label} must assign each script's outdir inside the loop "
                '(`outdir="target/ironclaw-stress/…/${script}"`) before the '
                "mkdir and invocation — the per-script evidence paths are "
                "built from it"
            )
        if guard is None:
            errors.append(
                f"{label} a failed invocation must not abort the loop under "
                "set -e — the runner call must end with `|| failed=1` so the "
                "later scripts still run and upload their evidence"
            )
        if (
            outdir_assign is not None
            and mkdir is not None
            and guard is not None
            and not (outdir_assign.start() < mkdir.start() < guard.start())
        ):
            errors.append(
                f"{label} must keep the loop body in order — the outdir= "
                'assignment, then `mkdir -p "${outdir}"`, then the guarded '
                "invocation (`2> \"${outdir}/report.txt\" || failed=1`) — a "
                "relocated or reordered line breaks the per-script evidence "
                "path"
            )
        if LIBSQL_SCRIPTED_FINAL_EXIT.search(executable[loop.end() :]) is None:
            errors.append(
                f'{label} the step must `exit "$failed"` after the loop\'s '
                "done — an exit before done fails the job before the later "
                "scripts run and upload their evidence"
            )

    probes = list(LIBSQL_PROBE_LOOP.finditer(executable))
    if not probes:
        errors.append(
            f"{label} every server readiness probe must be a bounded retry "
            "loop (for _ in $(seq 1 120); do ... curl ...; done) — an "
            "unbounded while loop hangs the job on a dead server"
        )
    else:
        for probe in probes:
            bound = int(probe.group("bound"))
            if bound > LIBSQL_PROBE_MAX_BOUND:
                errors.append(
                    f"{label} readiness probe retry bound is {bound}, not "
                    f"capped at {LIBSQL_PROBE_MAX_BOUND} — a dead server burns "
                    "the whole job timeout; keep the bounded form "
                    "(for _ in $(seq 1 120))"
                )
        for probe in LIBSQL_PROBE_LOOP_BODY.finditer(executable):
            body = probe.group("body")
            if not all(token in body for token in ("curl", "sleep", "break")) or (
                LIBSQL_PROBE_TIMEOUT.search(body) is None
            ):
                errors.append(
                    f"{label} every readiness probe loop must curl the "
                    "endpoint, sleep, and break on success — a probe that "
                    "cannot break keeps hammering a dead server, and a curl "
                    "without explicit --connect-timeout/--max-time flags can "
                    "hang on a wedged server"
                )
    missing_final = [
        name
        for name, pattern in (
            ("health", LIBSQL_FINAL_HEALTH_PROBE),
            ("webchat session", LIBSQL_FINAL_SESSION_PROBE),
        )
        if pattern.search(executable) is None
    ]
    if missing_final:
        errors.append(
            f"{label} after its bounded retry loop the "
            f"{'/'.join(missing_final)} readiness probe must run one final "
            "unconditional curl with explicit --connect-timeout and "
            "--max-time flags — a probe removed entirely starts the matrix "
            "against a dead server, and one without timeouts can hang past "
            "the loop"
        )

    traps = list(LIBSQL_SERVER_TRAP.finditer(executable))
    if not traps:
        errors.append(
            f"{label} must register an EXIT trap that kills the server — a "
            "failed step otherwise leaks the background server across the job"
        )
    elif not any(
        "kill" in trap.group("cmd") and "wait" in trap.group("cmd")
        for trap in traps
    ):
        errors.append(
            f"{label} the EXIT trap must kill the server AND wait for it "
            "(trap 'kill \"$server_pid\" ...; wait \"$server_pid\" ...' EXIT) — "
            "kill alone can leave the port bound when the step ends"
        )

    upload_steps: list[tuple[str, list[str], str]] = []
    for heading in STEP_HEADING.finditer(block):
        if not heading.group("name").strip().startswith("Upload "):
            continue
        following = STEP_HEADING.search(block, heading.end())
        body = block[heading.end() : following.start() if following else len(block)]
        name, paths = extract_upload(body)
        if name is not None:
            upload_steps.append((name, paths, body))
    uploads = [(name, paths) for name, paths, _ in upload_steps]

    not_always = [
        name
        for name, _, body in upload_steps
        if UPLOAD_ALWAYS.search(body) is None
    ]
    if not_always:
        errors.append(
            f"{label} every upload step must run with `if: always()` so a "
            "failed matrix step still uploads its evidence — missing on "
            f"{', '.join(not_always)}"
        )
    not_error_on_missing = [
        name
        for name, _, body in upload_steps
        if UPLOAD_NO_FILES_ERROR.search(body) is None
    ]
    if not_error_on_missing:
        errors.append(
            f"{label} every upload step must set `if-no-files-found: error` "
            "so silently missing evidence fails the job instead of "
            f"uploading nothing — missing on {', '.join(not_error_on_missing)}"
        )

    server_log_uploads = [upload for upload in uploads if upload[0] == LIBSQL_SERVER_LOG_NAME]
    if len(server_log_uploads) != 1:
        errors.append(
            f"{label} must upload the server log as its own artifact step "
            f"named {LIBSQL_SERVER_LOG_NAME!r} (found {len(server_log_uploads)}) "
            "— without a separate artifact the log is split across the "
            "per-script uploads and a failed run has no single log to fetch"
        )
    elif LIBSQL_SERVER_LOG_PATH not in server_log_uploads[0][1]:
        errors.append(
            f"{label} the {LIBSQL_SERVER_LOG_NAME!r} artifact must include "
            f"{LIBSQL_SERVER_LOG_PATH!r}"
        )

    expected_names = {
        f"{LIBSQL_PER_SCRIPT_ARTIFACT_PREFIX}{script.replace('_', '-')}"
        for script in LIBSQL_SCRIPTED_SCRIPTS
    }
    per_script_uploads = [upload for upload in uploads if upload[0] in expected_names]
    actual_names = sorted(upload[0] for upload in per_script_uploads)
    if len(per_script_uploads) != len(expected_names) or len(set(actual_names)) != len(
        expected_names
    ):
        errors.append(
            f"{label} must upload exactly three distinct per-script artifacts "
            f"({', '.join(sorted(expected_names))}) — found {actual_names}; two "
            "scripts sharing one artifact identity makes one of them "
            "unrecoverable after a failure"
        )
    for name, paths in uploads:
        if name == LIBSQL_SERVER_LOG_NAME:
            continue
        if LIBSQL_SERVER_LOG_PATH in paths:
            errors.append(
                f"{label} artifact {name!r} must not include "
                f"{LIBSQL_SERVER_LOG_PATH!r} — the server log is uploaded once, "
                "by its own step; embedding it in every per-script upload "
                "duplicates it and hides which script owns it"
            )

    expected_script_paths = {
        f"{LIBSQL_PER_SCRIPT_ARTIFACT_PREFIX}{script.replace('_', '-')}": [
            f"target/ironclaw-stress/ironclaw-stress-libsql-scripted-{script}/"
            + suffix
            for suffix in ("summary.jsonl", "summary.json", "report.txt")
        ]
        for script in LIBSQL_SCRIPTED_SCRIPTS
    }
    for name, paths in uploads:
        if name not in expected_script_paths:
            continue
        if paths != expected_script_paths[name]:
            errors.append(
                f"{label} artifact {name!r} must include exactly the three "
                "paths for its own script's outdir — "
                + ", ".join(expected_script_paths[name])
                + " — a swapped or missing path leaves that script's "
                "evidence unrecoverable after a failure"
            )
    return errors


# ---------------------------------------------------------------------------
# postgres-api-capacity scripted parity (#7360 Phase 1)
#
# The Postgres job's scripted leg is the parity twin of the libsql matrix:
# the same four doc sizes through the same runner. The runner picks each
# operation's document size by cycling the size list (`doc_size_for`), so N
# operations exercise only the FIRST N sizes. The historical `--operations
# 2` therefore ran only the 4096/32768 buckets and never touched the two
# largest — the Postgres leg measured half the curve. The pin requires at
# least one operation per configured size (>= 4) AND the exact four-size
# list; together they prove every configured bucket is exercised. The
# operations pin is anchored to the `--api-scripted-tool memory_roundtrip`
# line that follows it, so the capacity leg's `--operations 1` cannot
# satisfy it.
# ---------------------------------------------------------------------------

POSTGRES_API_CAPACITY_JOB = "postgres-api-capacity"
POSTGRES_SCRIPTED_OPERATIONS = re.compile(
    r"--operations[ \t]+(?P<count>[0-9]+)(?![0-9.])[ \t]*\\[ \t]*\n"
    r"[ \t]*--api-scripted-tool[ \t]+memory_roundtrip"
)
POSTGRES_SCRIPTED_DOC_SIZES = re.compile(
    r"--api-scripted-doc-sizes[ \t]+4096,32768,131072,1048576(?![0-9,])"
    r"[ \t]*(?:\\[ \t]*)?$",
    re.MULTILINE,
)
POSTGRES_SCRIPTED_MIN_OPERATIONS = 4
POSTGRES_SCRIPTED_MAX_FAILURE_RATE = re.compile(
    r"--max-failure-rate[ \t]+0(?![0-9.])"
)


def validate_postgres_scripted_parity(text: str) -> list[str]:
    """Return every way the Postgres scripted leg stops reaching every
    configured doc size.

    `doc_size_for` cycles the --api-scripted-doc-sizes list by operation
    index, so fewer operations than sizes leave the largest buckets
    unexercised and the parity leg reports green on half the curve.
    """

    block, detail = extract_job_block(text, POSTGRES_API_CAPACITY_JOB)
    if block is None:
        return [f"{STRESS_WORKFLOW}: {detail}"]
    label = f"{STRESS_WORKFLOW} ({POSTGRES_API_CAPACITY_JOB} job):"
    errors: list[str] = []

    # Comment lines are inert, matching the other scoped validators.
    executable = "\n".join(
        line for line in block.splitlines() if not line.lstrip().startswith("#")
    )
    commands = [
        command
        for command in extract_continued_commands(
            executable, "target/release/ironclaw_stress"
        )
        if "--api-scripted-tool memory_roundtrip" in command
    ]
    if len(commands) == 1:
        runner = commands[0]
    else:
        errors.append(
            f"{label} must contain exactly one scripted memory runner command, "
            f"found {len(commands)}"
        )
        runner = ""


    ops = POSTGRES_SCRIPTED_OPERATIONS.search(runner)
    if ops is None:
        errors.append(
            f"{label} the scripted memory leg must pass --operations N "
            f"(N >= {POSTGRES_SCRIPTED_MIN_OPERATIONS}) directly before "
            "--api-scripted-tool memory_roundtrip — the runner cycles the "
            "doc-size list by operation index, so fewer operations than "
            "configured sizes leave the largest buckets unexercised"
        )
    elif int(ops.group("count")) < POSTGRES_SCRIPTED_MIN_OPERATIONS:
        errors.append(
            f"{label} the scripted memory leg runs --operations "
            f"{ops.group('count')}, below the "
            f"{POSTGRES_SCRIPTED_MIN_OPERATIONS} configured doc sizes — "
            "doc_size_for() cycles sizes by operation index, so the "
            "131072/1048576 buckets are never exercised"
        )
    if POSTGRES_SCRIPTED_DOC_SIZES.search(runner) is None:
        errors.append(
            f"{label} the scripted memory leg must pass exactly the four "
            "scripted doc sizes --api-scripted-doc-sizes "
            "4096,32768,131072,1048576 — the Postgres parity leg measures "
            "the same small-to-large latency curve as the libsql matrix"
        )
    if POSTGRES_SCRIPTED_MAX_FAILURE_RATE.search(runner) is None:
        errors.append(
            f"{label} the scripted memory leg must pass --max-failure-rate 0 "
            "on its runner command — one leak or lost write must fail the "
            "32-operation parity leg"
        )
    return errors


# ---------------------------------------------------------------------------
# Per-package clippy target selection (#6965)
#
# `Check production-target lints` runs `cargo clippy -p <changed package> …`,
# so its command has to hold for every package shape in the workspace. Explicit
# target filters do not:
#
#   * `--lib` is a hard error on a bin-only package ("no library targets found
#     in package `ironclaw`"), so a PR whose only changed package is
#     crates/ironclaw_cli fails the lane on the flag, not on a lint;
#   * `--bins` on a lib-only package is "target filter `bins` specified, but no
#     targets matched; this is a no-op" — the lane reports green having linted
#     nothing, which is the worse failure of the two;
#   * `--bin`/`--example`/`--test`/`--bench` and their plurals swap the
#     package's default production targets for a hand-picked set.
#
# Cargo's default target set is already lib + bins, tests/examples/benches
# excluded, so the lane needs no filter at all — and this contract keeps it
# that way.
#
# The check reads the whole step body rather than locating the command and
# parsing its arguments. That is deliberate: a matcher is a thing to fool, and
# every attempt to write one leaked (a wrapper binary, a prefixed command, a
# continuation line). Scanning the body has no match position to displace and
# no formatting to get wrong. The trade is that a command deliberately written
# to look inert — `echo cargo clippy … -- -D warnings` — would pass. This file
# is repo-controlled and changed through reviewed PRs; the regression worth
# catching is a flag added back by hand, not a disguise.
#
# Known gap, deliberately unguarded: a package with neither a lib nor a bin
# target (today only `ironclaw_integration_tests`) lints nothing and
# exits 0 without even the `no targets matched` warning. Unreachable while
# `changed_workspace_packages.py` only selects the root package for a
# `Cargo.toml`/`Cargo.lock` change — which selects every other package too — so
# the assertion would have no failing case to pin.
# ---------------------------------------------------------------------------

PRODUCTION_LINT_STEP = "Check production-target lints"
WINDOWS_CLIPPY_JOB = "clippy-windows"
WEBUI_INSTALL_STEP = "Install WebUI frontend dependencies"
SCCACHE_SETUP_ACTION = ".github/actions/setup-sccache-dist/action.yml"
SCCACHE_INSTALL_STEP = "Install sccache"
SCCACHE_CONFIGURE_STEP = "Configure OVH sccache"
SCCACHE_FALLBACK_STEP = "Fall back to local compilation"

# One `- name:` step heading. The scan is bounded to its own step because the
# neighbouring `Check all-target lints` legitimately passes `--tests
# --examples`; unbounded, this contract would blame this step for them.
STEP_HEADING = re.compile(r"^[ \t]*- name: (?P<name>.+)$", re.MULTILINE)
JOB_HEADING = re.compile(r"^  (?P<name>[a-zA-Z0-9_-]+):[ \t]*$", re.MULTILINE)

# `${{ matrix.flags }}` is the lane's other flag channel: `clippy_matrix` is
# defined in this same workflow and expands into the command, so a target
# filter added there widens the lane exactly as one on the command line would.
# Scoped to the lines defining that matrix — `clippy_matrix` is the only
# `flags`-bearing matrix here today, and an unrelated one that legitimately
# passes `--tests` should not be read as widening this lane.
CLIPPY_MATRIX_ASSIGNMENT = "clippy_matrix"
MATRIX_FLAGS = re.compile(r'"flags"[ \t]*:[ \t]*"(?P<flags>[^"]*)"')

# Ways to keep the command intact while throwing away its verdict. These are
# not the disguised-command case the block above rules out of scope: each is a
# plausible edit someone makes on purpose and for a stated reason ("unblock the
# queue", "this lane is flaky"), and each leaves a lane that runs clippy and
# ignores it — the silent-green failure this contract exists to prevent.
EXIT_STATUS_MASKS = (
    ("|| true", "swallows a failing lint"),
    ("|| :", "swallows a failing lint"),
    ("set +e", "stops the shell failing on a failing lint"),
    ("continue-on-error", "lets the job report success with the lane red"),
)

# Matched on word boundaries so `--bins` is not also reported as `--bin`, and
# so value-bearing forms (`--bin ironclaw`, `--bench=throughput`) are caught.
FORBIDDEN_PRODUCTION_LINT_FLAGS = tuple(
    (flag, why, re.compile(rf"(?<![\w-]){re.escape(flag)}(?![\w-])"))
    for flag, why in (
        ("--lib", "is a hard error on a bin-only package"),
        ("--bins", "silently lints nothing on a lib-only package"),
        ("--bin", "pins the lane to one binary and skips the package's other targets"),
        ("--all-targets", "widens the lane past production targets"),
        ("--tests", "widens the lane past production targets"),
        ("--test", "widens the lane past production targets"),
        ("--examples", "widens the lane past production targets"),
        ("--example", "widens the lane past production targets"),
        ("--benches", "widens the lane past production targets"),
        ("--bench", "widens the lane past production targets"),
    )
)


def step_body(text: str, step_name: str) -> str | None:
    """Return one workflow step's body, bounded by the next step heading."""
    for heading in STEP_HEADING.finditer(text):
        if heading.group("name").strip() != step_name:
            continue
        following = STEP_HEADING.search(text, heading.end())
        return text[heading.end() : following.start() if following else len(text)]
    return None


def validate_sccache_setup_action(text: str) -> list[str]:
    """Keep an optional compiler-cache download from gating CI correctness."""
    errors: list[str] = []

    install = step_body(text, SCCACHE_INSTALL_STEP)
    if install is None:
        return [
            f"{SCCACHE_SETUP_ACTION}: could not find the {SCCACHE_INSTALL_STEP!r} step"
        ]
    if "id: install_sccache" not in install:
        errors.append(
            f"{SCCACHE_SETUP_ACTION}: {SCCACHE_INSTALL_STEP!r} must expose "
            "`id: install_sccache` so later steps can route on its outcome"
        )
    if "continue-on-error: true" not in install:
        errors.append(
            f"{SCCACHE_SETUP_ACTION}: {SCCACHE_INSTALL_STEP!r} must set "
            "`continue-on-error: true` because cache installation is optional"
        )

    configure = step_body(text, SCCACHE_CONFIGURE_STEP)
    if configure is None:
        errors.append(
            f"{SCCACHE_SETUP_ACTION}: could not find the {SCCACHE_CONFIGURE_STEP!r} step"
        )
    elif "steps.install_sccache.outcome == 'success'" not in configure:
        errors.append(
            f"{SCCACHE_SETUP_ACTION}: {SCCACHE_CONFIGURE_STEP!r} must require a "
            "successful installation before configuring or enabling `RUSTC_WRAPPER`"
        )

    fallback = step_body(text, SCCACHE_FALLBACK_STEP)
    if fallback is None:
        errors.append(
            f"{SCCACHE_SETUP_ACTION}: could not find the {SCCACHE_FALLBACK_STEP!r} step"
        )
    elif (
        "steps.install_sccache.outcome == 'failure'" not in fallback
        or "::warning" not in fallback
        or "local compilation" not in fallback
    ):
        errors.append(
            f"{SCCACHE_SETUP_ACTION}: {SCCACHE_FALLBACK_STEP!r} must warn that a "
            "failed installation is falling back to local compilation"
        )

    return errors


def job_body(text: str, job_name: str) -> str | None:
    """Return one workflow job's body, bounded by the next job heading."""
    for heading in JOB_HEADING.finditer(text):
        if heading.group("name") != job_name:
            continue
        following = JOB_HEADING.search(text, heading.end())
        return text[heading.end() : following.start() if following else len(text)]
    return None


def validate_windows_webui_install_shell(text: str) -> list[str]:
    """Keep POSIX WebUI setup commands out of PowerShell on Windows."""
    windows_job = job_body(text, WINDOWS_CLIPPY_JOB)
    if windows_job is None:
        return [
            f"{CODE_STYLE_WORKFLOW}: could not find the {WINDOWS_CLIPPY_JOB!r} job"
        ]
    install_step = step_body(windows_job, WEBUI_INSTALL_STEP)
    if install_step is None:
        return [
            f"{CODE_STYLE_WORKFLOW}: {WINDOWS_CLIPPY_JOB!r} has no "
            f"{WEBUI_INSTALL_STEP!r} step"
        ]
    if re.search(r"^[ \t]*shell:[ \t]*bash[ \t]*$", install_step, re.MULTILINE):
        return []
    return [
        f"{CODE_STYLE_WORKFLOW}: {WINDOWS_CLIPPY_JOB!r} must run "
        f"{WEBUI_INSTALL_STEP!r} with `shell: bash` because its commands use "
        "POSIX shell syntax"
    ]


def validate_production_lint_targets(text: str) -> list[str]:
    """Return every way the per-package clippy lane could error or no-op."""
    body = step_body(text, PRODUCTION_LINT_STEP)
    if body is None:
        return [
            f"{CODE_STYLE_WORKFLOW}: could not find the {PRODUCTION_LINT_STEP!r} step "
            "— it is the only clippy gate on pull requests and must stay assertable"
        ]
    # Comments in the step explain which flags are absent and why, so they name
    # the very strings being rejected.
    command = "\n".join(
        line for line in body.splitlines() if not line.lstrip().startswith("#")
    )
    if "cargo clippy" not in command:
        return [
            f"{CODE_STYLE_WORKFLOW}: {PRODUCTION_LINT_STEP!r} no longer runs "
            "`cargo clippy` — this contract can only pin a command it can see"
        ]
    errors = [
        f"{CODE_STYLE_WORKFLOW}: {PRODUCTION_LINT_STEP!r} must not pass {flag} — it {why}"
        for flag, why, pattern in FORBIDDEN_PRODUCTION_LINT_FLAGS
        if pattern.search(command)
    ]
    errors.extend(
        f"{CODE_STYLE_WORKFLOW}: {PRODUCTION_LINT_STEP!r} must not mask the lint's "
        f"exit status with `{mask}` — it {why}"
        for mask, why in EXIT_STATUS_MASKS
        if mask in command
    )
    errors.extend(
        f"{CODE_STYLE_WORKFLOW}: clippy_matrix flags {match.group('flags')!r} must not "
        f"contain {flag} — it {why}, and the matrix expands into "
        f"{PRODUCTION_LINT_STEP!r}"
        for line in text.splitlines()
        if CLIPPY_MATRIX_ASSIGNMENT in line
        for match in MATRIX_FLAGS.finditer(line)
        for flag, why, pattern in FORBIDDEN_PRODUCTION_LINT_FLAGS
        if pattern.search(match.group("flags"))
    )
    return errors

# Every single-quoted ERE in a workflow that looks like a path scope filter.
# Both spellings in use are covered: `grep -Eq '^(...)'` and the `has_match
# '^(...)'` helper. A filter is selected out of the result by an anchor
# substring that must appear in exactly one of them.
SCOPE_ERE = re.compile(r"'(\^\([^']+\))'")

# The `paths:` block of a workflow trigger: an indented `paths:` key followed by
# `- "<glob>"` items. Comment lines are skipped so the rationale can live inline.
PATHS_BLOCK = re.compile(r"^[ \t]*paths:[ \t]*$", re.MULTILINE)
PATHS_ITEM = re.compile(r"^[ \t]*-[ \t]*\"([^\"]+)\"[ \t]*$")

# Family directory used to build the "does this survive nesting?" probe. It is
# the name PROPOSAL §5 uses in its worked example; nothing depends on the
# spelling, only on it being one level deeper than today.
NESTED_FAMILY = "substrates"


@dataclasses.dataclass(frozen=True)
class CrateScopeFilter:
    """One workflow scope filter, pinned against the real crate inventory."""

    workflow: str
    name: str
    # Substring identifying this filter uniquely within the workflow text.
    anchor: str
    # `regex` = a single-quoted ERE; `globs` = the trigger's `paths:` list.
    kind: str
    # (crate name, in-crate probe path) — the crate must exist, and the filter
    # must match both its real location and a nested equivalent.
    crates: tuple[tuple[str, str], ...] = ()
    # (crate name, in-crate glob) — the glob must discover at least one real
    # file, and every discovered file must be in scope.
    crate_globs: tuple[tuple[str, str], ...] = ()
    # Non-crate paths that must stay in scope.
    in_scope: tuple[str, ...] = ()
    # Paths that must stay OUT of scope: a filter that matches everything is
    # not a fix.
    out_of_scope: tuple[str, ...] = ()


CRATE_SCOPE_FILTERS: tuple[CrateScopeFilter, ...] = (
    CrateScopeFilter(
        workflow=CODE_STYLE_WORKFLOW,
        name="has_code",
        anchor="migrations/",
        kind="regex",
        in_scope=(
            "crates/ironclaw_llm/src/lib.rs",
            f"crates/{NESTED_FAMILY}/ironclaw_event_log/src/lib.rs",
            "crates/extensions/packages/slack/manifest.toml",
            "tests/integration/mod.rs",
        ),
        out_of_scope=("README.md", "docs/internal/plans/whatever.md", "openwiki/index.md"),
    ),
    # The guidance-surface companion to `has_code`: check-guidance.py scans
    # `.claude/` rules and skills, the root AGENTS.md/CLAUDE.md pair, and
    # resolves references into docs/, none of which `has_code` covers (the row
    # above pins docs/ OUT of it on purpose). This filter OR-s into
    # fast-checks' condition only, so a `.claude/`-only PR runs the gate built
    # for exactly that change shape (#7306 review: the gate must run for the
    # files it governs).
    CrateScopeFilter(
        workflow=CODE_STYLE_WORKFLOW,
        name="has_guidance",
        anchor="\\.claude/",
        kind="regex",
        in_scope=(
            ".claude/rules/testing.md",
            ".claude/skills/reborn-feature/SKILL.md",
            "AGENTS.md",
            "CLAUDE.md",
            "docs/internal/reborn/guidance-conventions.md",
            # check-guidance.py scans the docs/ surface (published pages, the
            # zh/ mirror, docs/internal/reborn/contracts/) since 2026-08-07 —
            # a narrowed trigger that drops public docs must fail here, not
            # silently skip the gate.
            "docs/api/responses.mdx",
            "docs/zh/index.mdx",
        ),
        out_of_scope=(
            # Crate-tier guidance rides `has_code`'s `crates/` prefix; this
            # filter must stay the narrow guidance-surface half, and the root
            # README is not a guidance scan surface.
            "crates/AGENTS.md",
            "crates/domains/ironclaw_llm/AGENTS.md",
            "README.md",
            "openwiki/index.md",
        ),
    ),
    CrateScopeFilter(
        workflow=CODE_STYLE_WORKFLOW,
        name="has_reborn_cli",
        anchor="ironclaw_cli",
        kind="regex",
        crates=(
            ("ironclaw_turn_runner", "src/lib.rs"),
            # WS3 runner sheds: the model gateway and the tool-disclosure
            # decorator live here now, so the lane must follow them.
            ("ironclaw_loop_host", "src/model_gateway.rs"),
            ("ironclaw_cli", "src/main.rs"),
            ("ironclaw_config", "src/lib.rs"),
            ("ironclaw_architecture_tests", "tests/reborn_dependency_boundaries.rs"),
        ),
        in_scope=("Cargo.toml", "Cargo.lock", "scripts/ci/smoke-release-binary.py"),
        out_of_scope=(
            # The filter must stay a filter: the dist-build lane is expensive
            # and is deliberately NOT triggered by every crate.
            "crates/ironclaw_llm/src/lib.rs",
            f"crates/{NESTED_FAMILY}/ironclaw_llm/src/lib.rs",
            "crates/ironclaw_architecture_tests/tests/reborn_retired_taxonomy.rs",
            "README.md",
        ),
    ),
    CrateScopeFilter(
        workflow=CODE_STYLE_WORKFLOW,
        name="has_docs",
        # Not crate-keyed: docs/ is deliberately outside the has_code scope,
        # so this trigger is the ONLY thing that runs the publication-boundary
        # gate on a docs-only PR. A narrowed grep here skips the gate with
        # nothing red anywhere — the same silent-skip class as the crate
        # filters, pinned the same way.
        anchor="docs_publication_boundary",
        kind="regex",
        in_scope=(
            "docs/index.mdx",
            # Both halves of the gate's contract: navigation (docs.json) and
            # the fence (.mintignore). A future markdown-only narrowing of
            # the grep would drop them while every .mdx probe stays green.
            "docs/docs.json",
            "docs/.mintignore",
            "docs/internal/plans/whatever.md",
            # The gate's own files (review-discipline.md "Guardrails are
            # code": checks must run when their own files change).
            "scripts/ci/docs_publication_boundary.py",
            "scripts/ci/test_docs_publication_boundary.py",
            ".github/workflows/code_style.yml",
        ),
        out_of_scope=(
            "crates/ironclaw_llm/src/lib.rs",
            f"crates/{NESTED_FAMILY}/ironclaw_llm/src/lib.rs",
            "README.md",
            "openwiki/index.md",
        ),
    ),
    CrateScopeFilter(
        workflow=PLATFORM_WORKFLOW,
        name="has_direct_wasm_abi_risk",
        # Anchored on the build script rather than on a path prefix: the WIT
        # directory moved inside `ironclaw_wasm` (CHECKLIST WS4), so the bare
        # `wit/` alternative that used to anchor this filter is gone.
        anchor="build-wasm-extensions",
        kind="regex",
        crates=(
            ("ironclaw_common", "src/lib.rs"),
            ("ironclaw_wasm", "src/lib.rs"),
        ),
        # Probes derived from reality rather than from a guessed layout: the
        # files are found on disk and every one of them must be in scope.
        #
        #  * the shipped package manifests, anchored on the support crate and
        #    hopping to its sibling `packages/` directory, which is where WS2
        #    put them — if that moves again this stops discovering files or
        #    stops matching them, either way loudly.
        #  * the canonical WASM ABI contracts themselves. Naming them by glob
        #    rather than by literal is the point: the filter's whole job is to
        #    put a `tool.wit`/`channel.wit` edit in scope, and a literal probe
        #    can only assert the path someone typed. `wit/*.wit` discovers
        #    whatever the crate actually ships, so adding a third contract or
        #    moving the directory out of the crate fails here instead of
        #    passing on a stale name. (It replaces two `.../wit/host.wit`
        #    probes: no `host.wit` exists in this repository, so they asserted
        #    the crate-name alternative twice and nothing about the ABI files.)
        crate_globs=(
            ("ironclaw_extension_support", "../packages/*/manifest.toml"),
            ("ironclaw_wasm", "wit/*.wit"),
        ),
        in_scope=(
            "registry/tools/x.json",
            "scripts/build-wasm-extensions.sh",
        ),
        out_of_scope=(
            "crates/ironclaw_llm/src/lib.rs",
            f"crates/{NESTED_FAMILY}/ironclaw_llm/src/lib.rs",
            # Neighbouring crate whose name merely starts with the same text.
            "crates/ironclaw_wasm_limiter/src/lib.rs",
            "README.md",
        ),
    ),
    CrateScopeFilter(
        workflow=STRESS_WORKFLOW,
        name="pull_request paths",
        anchor="paths:",
        kind="globs",
        crates=(
            ("ironclaw_filesystem", "src/lib.rs"),
            ("ironclaw_threads", "src/lib.rs"),
            ("ironclaw_turns", "src/lib.rs"),
            ("ironclaw_resources", "src/lib.rs"),
            ("ironclaw_host_api", "src/lib.rs"),
        ),
        in_scope=(
            "tools/ironclaw_stress/src/main.rs",
            "Cargo.toml",
            "Cargo.lock",
            ".github/workflows/ironclaw-stress.yml",
        ),
        out_of_scope=(
            "crates/ironclaw_llm/src/lib.rs",
            f"crates/{NESTED_FAMILY}/ironclaw_llm/src/lib.rs",
            "README.md",
        ),
    ),
)


def github_glob_to_regex(glob: str) -> re.Pattern[str]:
    """Compile one GitHub `paths:` filter pattern.

    Implements the documented subset actually used by this repository's
    filters: `**` matches any characters including `/`, `*` matches any
    characters except `/`, `?` matches one character except `/`, and everything
    else is literal. A full implementation of GitHub's syntax (`!` negation,
    `+`) is deliberately out of scope — this exists to replay probe paths
    through the filters we write, not to reimplement the platform.
    """

    out = ["^"]
    index = 0
    while index < len(glob):
        char = glob[index]
        if char == "*":
            if glob.startswith("**", index):
                out.append(".*")
                index += 2
                continue
            out.append("[^/]*")
        elif char == "?":
            out.append("[^/]")
        else:
            out.append(re.escape(char))
        index += 1
    out.append("$")
    return re.compile("".join(out))


def extract_scope_regex(text: str, anchor: str) -> tuple[re.Pattern[str] | None, str]:
    """Return the one single-quoted ERE in `text` containing `anchor`."""

    matches = [pattern for pattern in SCOPE_ERE.findall(text) if anchor in pattern]
    if len(matches) != 1:
        return None, (
            f"expected exactly one scope regex containing {anchor!r}, found "
            f"{len(matches)}"
        )
    try:
        return re.compile(matches[0]), matches[0]
    except re.error as error:  # pragma: no cover - a malformed ERE is a typo
        return None, f"scope regex {matches[0]!r} does not compile: {error}"


def extract_paths_globs(text: str) -> tuple[list[str], str | None]:
    """Return the `paths:` trigger filter's glob list.

    Ambiguity is a refusal, matching `extract_scope_regex`: a workflow with two
    `paths:` blocks (a `push:` filter beside the `pull_request:` one, say) would
    otherwise have its FIRST block pinned unconditionally, and the contract would
    read as green while governing a filter nobody asked it to check — the same
    silent-resolution class this module exists to close.
    """

    blocks = PATHS_BLOCK.findall(text)
    if len(blocks) > 1:
        return [], (
            f"found {len(blocks)} `paths:` trigger filters; this pin resolves one "
            "unconditionally, so it cannot say which it validated. Split the contract "
            "entry per filter rather than letting it pick"
        )
    block = PATHS_BLOCK.search(text)
    if block is None:
        return [], "no `paths:` trigger filter found"
    globs: list[str] = []
    for line in text[block.end() :].splitlines()[1:]:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        item = PATHS_ITEM.match(line)
        if item is None:
            break
        globs.append(item.group(1))
    if not globs:
        return [], "the `paths:` trigger filter is empty"
    return globs, None


def _matcher(
    scope: CrateScopeFilter, text: str
) -> tuple[object | None, str | None]:
    if scope.kind == "regex":
        pattern, detail = extract_scope_regex(text, scope.anchor)
        if pattern is None:
            return None, detail
        return (lambda path: bool(pattern.search(path))), None
    globs, detail = extract_paths_globs(text)
    if detail is not None:
        return None, detail
    compiled = [github_glob_to_regex(glob) for glob in globs]
    return (
        lambda path: any(pattern.match(path) for pattern in compiled)
    ), None


def validate_crate_scope_filters(
    workflows: dict[str, str], root: Path = ROOT
) -> list[str]:
    """Return every way a crate-keyed workflow scope filter could go dark."""

    errors: list[str] = []
    try:
        inventory = crate_directories(root)
    except CrateTreeError as error:
        return [f"crate inventory unavailable, scope filters unpinnable: {error}"]
    if not inventory:  # pragma: no cover - crate_directories raises first
        return ["crate inventory is empty, scope filters unpinnable"]

    for scope in CRATE_SCOPE_FILTERS:
        label = f"{scope.workflow}: {scope.name}"
        text = workflows.get(scope.workflow)
        if text is None:
            errors.append(f"{label}: workflow not loaded")
            continue
        matches, detail = _matcher(scope, text)
        if matches is None:
            errors.append(f"{label}: {detail}")
            continue

        probes: list[tuple[str, bool]] = [(path, True) for path in scope.in_scope]
        probes.extend((path, False) for path in scope.out_of_scope)

        for name, relative in scope.crates:
            if name not in text:
                errors.append(
                    f"{label}: no longer enumerates crate {name!r} — a governed "
                    "crate silently dropped out of scope"
                )
                continue
            try:
                directory = crate_directory(name, root)
            except CrateTreeError as error:
                errors.append(
                    f"{label}: names crate {name!r}, which the crate inventory "
                    f"cannot resolve — repoint the filter rather than leaving a "
                    f"term that matches nothing ({error})"
                )
                continue
            probes.append((f"{directory}/{relative}", True))
            probes.append((f"crates/{NESTED_FAMILY}/{name}/{relative}", True))

        for name, pattern in scope.crate_globs:
            try:
                directory = crate_directory(name, root)
            except CrateTreeError as error:
                errors.append(f"{label}: names crate {name!r}: {error}")
                continue
            # `glob.glob` rather than `Path.glob` because a pattern may climb
            # out of the anchor crate with `../`: WS2 moved the extension
            # packages to `extensions/packages/`, a SIBLING of the support
            # crate rather than a subdirectory of it, because a package
            # directory is self-contained and owned by no crate (PROPOSAL §5).
            # Anchoring on the crate name is still what keeps this probe alive
            # across a family move; only the hop changed.
            anchored = os.path.normpath(str(root / directory / pattern))
            discovered = sorted(
                pathlib.Path(candidate).relative_to(root).as_posix()
                for candidate in glob.glob(anchored)
            )
            relative_probe = os.path.normpath(f"{directory}/{pattern}")
            if not discovered:
                errors.append(
                    f"{label}: probe {relative_probe} discovered no files, so "
                    "the filter is pinned against nothing — repoint the probe to "
                    "wherever those files moved"
                )
                continue
            probes.extend((path, True) for path in discovered)
            nested_probe = os.path.normpath(
                f"crates/{NESTED_FAMILY}/{name}/{pattern}"
            ).replace("*", "probe")
            probes.append((nested_probe, True))

        if not probes:  # pragma: no cover - every entry declares probes
            errors.append(f"{label}: no probes declared, the pin asserts nothing")
            continue
        for path, expected in probes:
            if matches(path) != expected:
                verdict = "must be in scope" if expected else "must NOT be in scope"
                errors.append(f"{label}: {path!r} {verdict}")
    return errors


# ---------------------------------------------------------------------------
# WebUI frontend directory sites + crate-name residue (#7155 WS10: "loud
# path-pattern inventory")
#
# 28 sites across seven workflows spelled `crates/ironclaw_webui/frontend`
# directly: a `cache-dependency-path:` value (12), a `cd` inside a `run:`
# block (12), and a `working-directory:` key (4). Two more workflows spelled a
# single crate's Cargo.toml / source path directly: docker.yml's release
# VERSION extraction (`ironclaw_cli`) and nightly-deep-ci.yml's
# mutation-audit target (`ironclaw_capabilities`). All of these break the
# moment their crate moves into a family directory (crates/<family>/
# ironclaw_*, PROPOSAL §5).
#
# `cache-dependency-path` is a static YAML value `actions/setup-node` globs at
# runtime rather than a shell site — verified against the pinned commit's
# bundled dist/setup/index.js: `hashFiles` walks ONE globber built from every
# newline-separated pattern, and `restoreCache` only throws when that COMBINED
# walk finds nothing — so its fix twins the flat lockfile line with a nested
# wildcard sibling instead of resolving dynamically. Every other site (`cd`,
# `working-directory`, docker.yml's VERSION grep, nightly-deep-ci.yml's
# mutation-audit path) resolves once through scripts/ci/crate-dir.sh
# (scripts/ci/lib/crate_tree.py) and must carry no literal trace of the flat
# path it replaced.
#
# Two contracts, one per site shape:
#   `validate_webui_frontend_sites` scans every `.github/workflows/*.yml` for
#   the flat WebUI frontend literal. The ONLY sanctioned shape is the
#   cache-dependency-path pairing (flat line immediately followed by its
#   nested sibling); anything else is the dynamic-site regression.
#   `validate_crate_name_residue` pins docker.yml and nightly-deep-ci.yml to
#   still name the crate they resolve, with that name still resolvable — the
#   same `name in text` + `crate_directory(name)` shape CRATE_SCOPE_FILTERS
#   already uses for its `crates=` tuples.
# ---------------------------------------------------------------------------

DOCKER_WORKFLOW = ".github/workflows/docker.yml"
NIGHTLY_DEEP_CI_WORKFLOW = ".github/workflows/nightly-deep-ci.yml"

WEBUI_FRONTEND_CRATE = "ironclaw_webui"
# One directory level deeper than the crate sits TODAY. WS7 moved the crate
# into `crates/product/`, so the single-`*` form now matches its real location
# and stopped being a depth probe — the gate below rejects exactly that ("not
# depth-tolerant, just broad"). Two `*` segments keep the spare one level below
# wherever the crate actually is; `*` does not cross `/` in a GitHub glob, so
# this cannot collapse back onto the flat line.
WEBUI_NESTED_LOCKFILE_PATTERN = (
    f"crates/*/*/{WEBUI_FRONTEND_CRATE}/frontend/pnpm-lock.yaml"
)


def _yaml_code_portion(line: str) -> str:
    """`line` with any YAML comment suffix removed.

    A `#` starts a comment only when it is at the start of the line or
    preceded by whitespace (the YAML spec rule) — enough to separate workflow
    CODE from an explanatory comment without a full parser.
    """

    for index, char in enumerate(line):
        if char == "#" and (index == 0 or line[index - 1] in " \t"):
            return line[:index]
    return line


def validate_webui_frontend_sites(
    workflows: dict[str, str], root: Path = ROOT
) -> list[str]:
    """Return every way a WebUI frontend directory site could go dark.

    The flat literal is sanctioned in exactly one shape: the
    `cache-dependency-path` flat lockfile line, immediately followed by its
    nested wildcard sibling on the next non-blank line. Anywhere else — a bare
    `cd`, a `working-directory:` key, a cache-dependency-path line missing its
    sibling, or a sibling that has drifted — is the WS10 regression this pin
    exists to catch.
    """

    errors: list[str] = []
    try:
        webui_dir = crate_directory(WEBUI_FRONTEND_CRATE, root)
    except CrateTreeError as error:
        return [
            f"crate inventory cannot resolve {WEBUI_FRONTEND_CRATE!r}, the crate "
            f"every WebUI frontend workflow site is derived from: {error}"
        ]
    flat_frontend_dir = f"{webui_dir}/frontend"
    flat_lockfile = f"{flat_frontend_dir}/pnpm-lock.yaml"

    # The glob machinery itself, independent of any workflow text: the flat
    # line must match today's real tree, the nested line must match a
    # plausible moved tree, and the nested line must not ALREADY match today's
    # tree — a pattern that matches everything is not depth-tolerant, it is
    # just broad (the same principle CRATE_SCOPE_FILTERS enforces on `paths:`).
    if not (root / flat_lockfile).is_file():
        errors.append(
            f"probe {flat_lockfile} does not exist on disk — the WebUI "
            "cache-dependency-path pin is unmeasurable; repoint it to wherever "
            "the frontend lockfile really is"
        )
    flat_pattern = github_glob_to_regex(flat_lockfile)
    nested_pattern = github_glob_to_regex(WEBUI_NESTED_LOCKFILE_PATTERN)
    # Two family segments, matching WEBUI_NESTED_LOCKFILE_PATTERN's depth: the
    # probe has to be one level below where the crate sits today, and today it
    # already sits inside a family directory (WS7).
    nested_probe = (
        f"crates/{NESTED_FAMILY}/{NESTED_FAMILY}/"
        f"{WEBUI_FRONTEND_CRATE}/frontend/pnpm-lock.yaml"
    )
    if not flat_pattern.match(flat_lockfile):
        errors.append(
            f"flat cache-dependency-path line {flat_lockfile!r} does not match itself"
        )
    if not nested_pattern.match(nested_probe):
        errors.append(
            f"nested cache-dependency-path line {WEBUI_NESTED_LOCKFILE_PATTERN!r} "
            f"does not match a plausible moved location ({nested_probe})"
        )
    if nested_pattern.match(flat_lockfile):
        errors.append(
            f"nested cache-dependency-path line {WEBUI_NESTED_LOCKFILE_PATTERN!r} "
            f"already matches today's flat location ({flat_lockfile}) — it must "
            "stay depth-tolerant, not just broad"
        )

    cache_sites = 0
    for path in sorted(workflows):
        if not path.startswith(".github/workflows/") or not path.endswith(".yml"):
            continue
        lines = workflows[path].splitlines()
        for index, raw_line in enumerate(lines):
            code = _yaml_code_portion(raw_line)
            if flat_frontend_dir not in code:
                continue
            stripped = code.strip()
            if stripped == flat_lockfile:
                rest = (candidate.strip() for candidate in lines[index + 1 :])
                following = next((candidate for candidate in rest if candidate), "")
                if following == WEBUI_NESTED_LOCKFILE_PATTERN:
                    cache_sites += 1
                    continue
                errors.append(
                    f"{path}:{index + 1}: cache-dependency-path flat lockfile line "
                    f"is not twinned with {WEBUI_NESTED_LOCKFILE_PATTERN!r} on the "
                    "next non-blank line — the family move will go dark here"
                )
                continue
            errors.append(
                f"{path}:{index + 1}: hardcodes {flat_frontend_dir!r} outside a "
                "comment — resolve it through scripts/ci/crate-dir.sh "
                f"{WEBUI_FRONTEND_CRATE} instead of the flat literal"
            )

    if cache_sites == 0:
        errors.append(
            "no workflow pairs the flat WebUI lockfile line with its nested "
            "sibling — the cache-dependency-path probe set is empty"
        )
    return errors


# (crate-governing workflow, crate name) — the workflow's text must still
# spell the name as a token, and the inventory must still resolve it. Both
# sites already resolve their PATH dynamically through scripts/ci/crate-dir.sh
# once fixed (B1/B2 in #7155); this is the pin that catches the workflow TEXT
# itself going stale — a rename or deletion the workflow never followed.
CRATE_NAME_RESIDUE: tuple[tuple[str, str], ...] = (
    (DOCKER_WORKFLOW, "ironclaw_cli"),
    (NIGHTLY_DEEP_CI_WORKFLOW, "ironclaw_capabilities"),
)


def validate_crate_name_residue(
    workflows: dict[str, str], root: Path = ROOT
) -> list[str]:
    """Return every way a governed crate name could go dark in its workflow."""

    errors: list[str] = []
    for workflow, name in CRATE_NAME_RESIDUE:
        text = workflows.get(workflow)
        if text is None:
            errors.append(f"{workflow}: workflow not loaded")
            continue
        if name not in text:
            errors.append(
                f"{workflow}: no longer names crate {name!r} — the step that "
                "resolves this crate's directory has nothing to resolve"
            )
        try:
            crate_directory(name, root)
        except CrateTreeError as error:
            errors.append(
                f"{workflow}: names crate {name!r}, which the crate inventory "
                f"cannot resolve — repoint it rather than leaving a token that "
                f"matches nothing ({error})"
            )
    return errors


def validate_workflow_texts(
    workflows: dict[str, str], root: Path = ROOT
) -> list[str]:
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
    code_style = workflows.get(CODE_STYLE_WORKFLOW)
    if code_style is not None:
        errors.extend(validate_production_lint_targets(code_style))
        errors.extend(validate_code_style_docs_guard_order(code_style))
        errors.extend(validate_windows_webui_install_shell(code_style))
    stress = workflows.get(STRESS_WORKFLOW)
    if stress is not None:
        errors.extend(validate_libsql_scripted_memory_job(stress))
        errors.extend(validate_postgres_scripted_parity(stress))
    errors.extend(validate_crate_scope_filters(workflows, root))
    errors.extend(validate_crate_name_residue(workflows, root))
    errors.extend(validate_webui_frontend_sites(workflows, root))
    try:
        sccache_action = (root / SCCACHE_SETUP_ACTION).read_text(encoding="utf-8")
    except OSError as error:
        errors.append(f"{SCCACHE_SETUP_ACTION}: could not read action: {error}")
    else:
        errors.extend(validate_sccache_setup_action(sccache_action))
    return errors


def load_workflows(root: Path) -> dict[str, str]:
    """Every `.github/workflows/*.yml` file, repo-relative path -> text.

    A handful of contracts key off one specific known path (REQUIRED_MARKERS,
    CRATE_SCOPE_FILTERS, CRATE_NAME_RESIDUE); `validate_webui_frontend_sites`
    must see EVERY workflow, since the whole point of that pin is to catch the
    flat WebUI literal reappearing somewhere nobody enumerated. Loading every
    (small) workflow file eagerly costs nothing and keeps one loader for every
    consumer — a superset of the old explicit path list, so every existing
    `workflows.get(path)` lookup keeps working unchanged.
    """

    workflows_dir = root / ".github" / "workflows"
    return {
        path.relative_to(root).as_posix(): path.read_text(encoding="utf-8")
        for path in sorted(workflows_dir.glob("*.yml"))
    }


def main() -> int:
    try:
        errors = validate_workflow_texts(load_workflows(ROOT), ROOT)
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
