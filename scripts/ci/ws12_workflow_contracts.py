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
            f"crates/{NESTED_FAMILY}/ironclaw_events/src/lib.rs",
            "crates/extensions/packages/slack/manifest.toml",
            "tests/integration/mod.rs",
        ),
        out_of_scope=("README.md", "docs/plans/whatever.md", "openwiki/index.md"),
    ),
    CrateScopeFilter(
        workflow=CODE_STYLE_WORKFLOW,
        name="has_reborn_cli",
        anchor="ironclaw_reborn_cli",
        kind="regex",
        crates=(
            ("ironclaw_runner", "src/lib.rs"),
            # WS3 runner sheds: the model gateway and the tool-disclosure
            # decorator live here now, so the lane must follow them.
            ("ironclaw_loop_host", "src/model_gateway.rs"),
            ("ironclaw_reborn_cli", "src/main.rs"),
            ("ironclaw_reborn_config", "src/lib.rs"),
            ("ironclaw_architecture", "tests/reborn_dependency_boundaries.rs"),
        ),
        in_scope=("Cargo.toml", "Cargo.lock", "scripts/ci/smoke-release-binary.py"),
        out_of_scope=(
            # The filter must stay a filter: the dist-build lane is expensive
            # and is deliberately NOT triggered by every crate.
            "crates/ironclaw_llm/src/lib.rs",
            f"crates/{NESTED_FAMILY}/ironclaw_llm/src/lib.rs",
            "crates/ironclaw_architecture/tests/reborn_retired_taxonomy.rs",
            "README.md",
        ),
    ),
    CrateScopeFilter(
        workflow=PLATFORM_WORKFLOW,
        name="has_direct_wasm_abi_risk",
        anchor="wit/",
        kind="regex",
        crates=(
            ("ironclaw_common", "src/lib.rs"),
            ("ironclaw_wasm", "src/lib.rs"),
        ),
        # Probe derived from reality rather than from a guessed layout: the
        # shipped package manifests are found on disk and every one of them
        # must be in scope. Anchored on the support crate and hopping to its
        # sibling `packages/` directory, which is where WS2 put them — if that
        # moves again this stops discovering files or stops matching them,
        # either way loudly.
        crate_globs=(("ironclaw_extension_support", "../packages/*/manifest.toml"),),
        in_scope=("wit/host.wit", "registry/tools/x.json", "scripts/build-wasm-extensions.sh"),
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
    errors.extend(validate_crate_scope_filters(workflows, root))
    return errors


def load_workflows(root: Path) -> dict[str, str]:
    paths = dict.fromkeys(
        (*REQUIRED_MARKERS, *(scope.workflow for scope in CRATE_SCOPE_FILTERS))
    )
    return {path: (root / path).read_text(encoding="utf-8") for path in paths}


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
