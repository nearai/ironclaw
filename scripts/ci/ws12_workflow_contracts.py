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
# VERSION extraction (`ironclaw_reborn_cli`) and nightly-deep-ci.yml's
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
WEBUI_NESTED_LOCKFILE_PATTERN = (
    f"crates/*/{WEBUI_FRONTEND_CRATE}/frontend/pnpm-lock.yaml"
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
    nested_probe = (
        f"crates/{NESTED_FAMILY}/{WEBUI_FRONTEND_CRATE}/frontend/pnpm-lock.yaml"
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
    (DOCKER_WORKFLOW, "ironclaw_reborn_cli"),
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
    errors.extend(validate_crate_scope_filters(workflows, root))
    errors.extend(validate_crate_name_residue(workflows, root))
    errors.extend(validate_webui_frontend_sites(workflows, root))
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
