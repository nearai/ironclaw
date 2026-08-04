#!/usr/bin/env python3
"""Tree-shape-agnostic discovery of the workspace's crate directories.

CI gates that key off the literal `crates/ironclaw_*` shape all share one
failure mode: when the target-architecture restructure moves crates into
family directories (`crates/<family>/ironclaw_*`, see
docs/reborn/target-architecture/PROPOSAL.md §5), the pattern stops matching,
the gate scans nothing, and it reports success. Coverage goes dark, a scope
classifier buckets everything as out-of-scope, a metric prints 0% — all green.

This module is the one definition of "which directories under `crates/` are
crates", derived from where `Cargo.toml` files actually are rather than from
how deep they sit. It is used by the Python-side gates; the pure-bash
`scripts/ci/classify-test-scope.sh` reimplements the same rule inline (it runs
in a lightweight checkout-only job) and `scripts/ci/test-classify-test-scope.sh`
pins the two inventories equal so they cannot drift.

Deliberately filesystem-based, not `cargo metadata`-based: the consumers run in
jobs that have a checkout but no Rust toolchain (the coverage-report job in
.github/workflows/reborn-tests.yml installs Python only). Gates that *do* have
cargo — `scripts/check_no_panics.py` — use `cargo metadata` directly, which is
strictly better where it is available.

Outermost wins: `crates/ironclaw_safety/fuzz/` is nested inside a crate that
already owns its path, so files under it are attributed to the enclosing crate.
That is exactly what the flat-shaped patterns did (`crates/ironclaw_safety/*`
matched the fuzz subtree too).

Separate workspace roots are not crates of this workspace. A `Cargo.toml` that
declares its own `[workspace]` table is by construction the root of a
*different* workspace: `cargo build` here never compiles it, `cargo metadata`
never lists it, and no test run of this workspace can cover a line inside it.
Those manifests are pruned from the inventory; paths under them are reported by
`nested_workspace_root()` instead, so a caller can tell "excluded by
construction" apart from "the tree and the inventory disagree".

Why the rule had to be stated rather than inherited (WS2 package colocation).
The six `wasm-src/` guest components used to sit under
`crates/ironclaw_first_party_extensions/`, so outermost-wins attributed them to
the enclosing crate and they never surfaced. Once packages moved to
`crates/extensions/packages/<ext>/` — where a data-only package deliberately has
no `Cargo.toml` of its own — five of them became the outermost manifest on their
path and were silently promoted to first-class crates, which put ~12k lines of
workspace-excluded, uncoverable guest code into the changed-coverage and
composition-budget denominators. The `[workspace]` marker is exactly what those
directories have in common with `crates/ironclaw_silk_decoder` (also `exclude`d
from the root workspace, also never built here), so one rule covers both; the
silk-decoder half is a latent-bug fix that arrives with it.

Usage:
    python3 scripts/ci/lib/crate_tree.py [repo_root]
        Print every crate directory, one per line (unchanged, load-bearing
        contract: scripts/build-wasm-extensions.sh and others parse this).

    python3 scripts/ci/lib/crate_tree.py --directory <name> [repo_root]
        Print the single crate directory whose basename is <name>. Exits 1
        with the CrateTreeError message on stderr when the name is absent or
        ambiguous — never falls back to a guessed literal path. The
        `scripts/ci/crate-dir.sh` wrapper shells to this for bash callers.
"""

from __future__ import annotations

import pathlib
import sys

CRATES_ROOT_NAME = "crates"

# Fail-closed floor. Discovery either finds the workspace's crates or it is
# broken (wrong root, truncated checkout, a `crates/` that moved wholesale);
# a handful of results is never a legitimate answer for this repository, and
# silently returning them is the exact class of bug this module exists to kill.
# It is a sanity floor, not a ratchet — it is far below the real count (65 at
# the WS0 baseline) and is never expected to bind.
MIN_CRATE_DIRECTORIES = 20

# Build outputs and dotted directories can contain vendored manifests that are
# not workspace crates. `crates/<x>/target/` appears whenever someone builds
# inside a crate directory (cargo-fuzz does this by default).
_SKIPPED_DIRECTORY_NAMES = ("target",)

# A `[workspace]` table at the start of a line. Cargo only recognizes the
# table at the top level of a manifest, so a line-anchored match is exact;
# `[workspace.dependencies]` and friends are deliberately not matched, since a
# member manifest may inherit from them without being a workspace root itself.
_WORKSPACE_TABLE_HEADER = "[workspace]"


class CrateTreeError(RuntimeError):
    """Discovery could not produce a usable crate inventory."""


def _declares_own_workspace(manifest: pathlib.Path) -> bool:
    """True when ``manifest`` is the root of a separate cargo workspace.

    Read fail-closed: an unreadable manifest is treated as a normal crate so a
    permissions problem shows up as a build error rather than as a directory
    quietly vanishing from every path-keyed gate.
    """

    try:
        text = manifest.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return False
    return any(
        line.strip() == _WORKSPACE_TABLE_HEADER for line in text.splitlines()
    )


def _is_skipped(relative: pathlib.PurePosixPath) -> bool:
    return any(
        part in _SKIPPED_DIRECTORY_NAMES or part.startswith(".")
        for part in relative.parts
    )


def crate_directories(repo_root: str | pathlib.Path = ".") -> list[str]:
    """Return every crate directory under `crates/`, repo-relative and sorted.

    A crate directory is the *outermost* directory under `crates/` that owns a
    `Cargo.toml`. Results are POSIX-style relative paths (`crates/ironclaw_llm`,
    or `crates/substrates/ironclaw_llm` once the family move lands).
    """

    root = pathlib.Path(repo_root)
    crates_root = root / CRATES_ROOT_NAME
    if not crates_root.is_dir():
        raise CrateTreeError(
            f"no {CRATES_ROOT_NAME}/ directory under {root!s} — crate discovery cannot "
            "run. Every path-keyed CI gate resolves its scope through this "
            "inventory; refusing rather than scanning nothing is deliberate "
            "(docs/reborn/target-architecture/CHECKLIST.md WS10)."
        )

    manifests = []
    for manifest in crates_root.rglob("Cargo.toml"):
        relative = pathlib.PurePosixPath(manifest.parent.relative_to(root).as_posix())
        if _is_skipped(relative):
            continue
        # A separate workspace root is not a crate of this workspace. Skipping
        # it here (rather than after the outermost-wins pass) is deliberate: a
        # guest that is not shadowed by an enclosing crate must not become one.
        if _declares_own_workspace(manifest):
            continue
        manifests.append(relative)

    # Shallowest first so the outermost owner of a path is always seen before
    # any manifest nested inside it.
    manifests.sort(key=lambda relative: (len(relative.parts), relative.as_posix()))

    crate_dirs: list[str] = []
    for relative in manifests:
        text = relative.as_posix()
        if any(text == kept or text.startswith(f"{kept}/") for kept in crate_dirs):
            continue
        crate_dirs.append(text)

    if len(crate_dirs) < MIN_CRATE_DIRECTORIES:
        raise CrateTreeError(
            f"crate discovery found only {len(crate_dirs)} crate director(ies) under "
            f"{crates_root!s} (floor is {MIN_CRATE_DIRECTORIES}). Either the crate tree "
            "moved out from under this gate or the repository root is wrong. Failing "
            "closed: a gate that scans nothing must never report success "
            "(docs/reborn/target-architecture/CHECKLIST.md WS10)."
        )

    return sorted(crate_dirs)


def crate_directory(name: str, repo_root: str | pathlib.Path = ".") -> str:
    """Return the crate directory whose basename is ``name``.

    Raises when the crate is absent — a named crate that a gate measures cannot
    silently resolve to "nothing" just because it moved or was renamed.
    """

    matches = [
        directory
        for directory in _crate_directories_cached(repo_root)
        if directory.rsplit("/", 1)[-1] == name
    ]
    if len(matches) != 1:
        raise CrateTreeError(
            f"expected exactly one crate directory named {name!r} under "
            f"{CRATES_ROOT_NAME}/, found {len(matches)}: {matches}. If the crate was "
            "renamed or moved, repoint the gate that names it rather than letting it "
            "measure an empty tree."
        )
    return matches[0]


def owning_crate_directory(
    path: str, repo_root: str | pathlib.Path = "."
) -> str | None:
    """Return the crate directory that owns repo-relative ``path``, else ``None``.

    Outermost wins, matching `crate_directories()` and the inline rule in
    `scripts/ci/classify-test-scope.sh`: `crates/ironclaw_safety/fuzz/src/main.rs`
    is owned by `crates/ironclaw_safety`, not by the nested fuzz manifest.

    ``None`` means "under `crates/` but attributable to no crate" (or not under
    `crates/` at all). Callers that classify production sources must treat the
    first case as an error rather than as "not production" — that fall-through
    is the WS10 silent-dark failure mode
    (docs/reborn/target-architecture/CHECKLIST.md) — *unless*
    `nested_workspace_root()` claims the path, which is the sanctioned
    "excluded by construction" answer.
    """

    normalized = pathlib.PurePosixPath(path).as_posix()
    for directory in _crate_directories_cached(repo_root):
        if normalized.startswith(f"{directory}/"):
            return directory
    return None


def workspace_root_directories(repo_root: str | pathlib.Path = ".") -> list[str]:
    """Directories under `crates/` that are roots of a *separate* workspace.

    These are excluded from `crate_directories()` on purpose (see the module
    docstring). Today: `crates/ironclaw_silk_decoder` and the six
    `crates/extensions/packages/*/wasm-src` guest components.
    """

    root = pathlib.Path(repo_root)
    crates_root = root / CRATES_ROOT_NAME
    if not crates_root.is_dir():
        raise CrateTreeError(
            f"no {CRATES_ROOT_NAME}/ directory under {root!s} — cannot enumerate "
            "separate workspace roots."
        )

    roots: list[str] = []
    for manifest in crates_root.rglob("Cargo.toml"):
        relative = pathlib.PurePosixPath(manifest.parent.relative_to(root).as_posix())
        if _is_skipped(relative):
            continue
        if _declares_own_workspace(manifest):
            roots.append(relative.as_posix())
    return sorted(roots)


def nested_workspace_root(
    path: str, repo_root: str | pathlib.Path = "."
) -> str | None:
    """Return the separate-workspace root containing ``path``, else ``None``.

    A path this claims is excluded from the workspace *by construction* — it is
    never compiled here and no line in it can be covered — which is a different
    answer from "the inventory and the tree disagree". Callers that fail closed
    on unattributable `crates/` paths consult this first.
    """

    normalized = pathlib.PurePosixPath(path).as_posix()
    for directory in _workspace_roots_cached(repo_root):
        if normalized.startswith(f"{directory}/"):
            return directory
    return None


def crate_source_relative(
    path: str, repo_root: str | pathlib.Path = "."
) -> tuple[str, str] | None:
    """Split ``path`` into ``(crate_directory, in-crate remainder)``, else ``None``."""

    owner = owning_crate_directory(path, repo_root)
    if owner is None:
        return None
    return owner, pathlib.PurePosixPath(path).as_posix()[len(owner) + 1 :]


_INVENTORY_CACHE: dict[str, list[str]] = {}
_WORKSPACE_ROOT_CACHE: dict[str, list[str]] = {}


def _workspace_roots_cached(repo_root: str | pathlib.Path = ".") -> list[str]:
    """`workspace_root_directories()` memoized per resolved root."""

    key = str(pathlib.Path(repo_root).resolve())
    cached = _WORKSPACE_ROOT_CACHE.get(key)
    if cached is None:
        cached = sorted(workspace_root_directories(repo_root), key=len)
        _WORKSPACE_ROOT_CACHE[key] = cached
    return cached


def _crate_directories_cached(repo_root: str | pathlib.Path = ".") -> list[str]:
    """`crate_directories()` memoized per resolved root.

    Path classification is a per-file question asked thousands of times per gate
    run; re-walking `crates/` for each one turns an O(tree) gate into O(tree x
    files). The cache is process-local and the tree does not change under a
    running gate. `crate_directories()` already prunes manifests nested inside a
    crate, so no entry is a prefix of another and lookup order cannot change the
    answer; shortest-first is kept anyway so that if that pruning is ever
    relaxed, the first match is still the *outermost* owner — the rule
    `owning_crate_directory` documents.
    """

    key = str(pathlib.Path(repo_root).resolve())
    cached = _INVENTORY_CACHE.get(key)
    if cached is None:
        cached = sorted(crate_directories(repo_root), key=len)
        _INVENTORY_CACHE[key] = cached
    return cached


def reset_inventory_cache() -> None:
    """Drop the memoized inventories.

    Only in-process callers that mutate a crate tree between queries need this
    — self-tests that build a fixture root, assert, then sabotage the same root.
    Gates run once per process and never call it.
    """

    _INVENTORY_CACHE.clear()
    _WORKSPACE_ROOT_CACHE.clear()


def main() -> int:
    args = sys.argv[1:]

    # Manual parsing (not argparse) so the pre-existing positional-root
    # contract above is untouched byte-for-byte: every current caller passes
    # at most one bare positional argument, and that must keep working
    # exactly as before regardless of how `--directory` is implemented.
    directory_name: str | None = None
    if "--directory" in args:
        flag_index = args.index("--directory")
        try:
            directory_name = args[flag_index + 1]
        except IndexError:
            print(
                "crate discovery failed: --directory requires a crate name",
                file=sys.stderr,
            )
            return 1
        del args[flag_index : flag_index + 2]

    root = args[0] if args else "."
    try:
        if directory_name is not None:
            print(crate_directory(directory_name, root))
        else:
            for directory in crate_directories(root):
                print(directory)
    except CrateTreeError as error:
        print(f"crate discovery failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
