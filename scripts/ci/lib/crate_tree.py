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

Outermost wins: `crates/ironclaw_safety/fuzz/` and the six
`crates/ironclaw_first_party_extensions/assets/*/wasm-src/` manifests are
nested inside a crate that already owns their path, so files under them are
attributed to the enclosing crate. That is exactly what the flat-shaped
patterns did (`crates/ironclaw_safety/*` matched the fuzz subtree too), so the
generalization is behavior-free on today's tree.

Usage:
    python3 scripts/ci/lib/crate_tree.py [repo_root]   # one crate dir per line
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


class CrateTreeError(RuntimeError):
    """Discovery could not produce a usable crate inventory."""


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
    (docs/reborn/target-architecture/CHECKLIST.md).
    """

    normalized = pathlib.PurePosixPath(path).as_posix()
    for directory in _crate_directories_cached(repo_root):
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


def _crate_directories_cached(repo_root: str | pathlib.Path = ".") -> list[str]:
    """`crate_directories()` memoized per resolved root.

    Path classification is a per-file question asked thousands of times per gate
    run; re-walking `crates/` for each one turns an O(tree) gate into O(tree x
    files). The cache is process-local and the tree does not change under a
    running gate. `crate_directories()` already prunes manifests nested inside a
    crate, so no entry is a prefix of another and lookup order cannot change the
    answer; longest-first is kept only so the rule stays correct if that pruning
    is ever relaxed.
    """

    key = str(pathlib.Path(repo_root).resolve())
    cached = _INVENTORY_CACHE.get(key)
    if cached is None:
        cached = sorted(crate_directories(repo_root), key=len, reverse=True)
        _INVENTORY_CACHE[key] = cached
    return cached


def reset_inventory_cache() -> None:
    """Drop the memoized inventories.

    Only in-process callers that mutate a crate tree between queries need this
    — self-tests that build a fixture root, assert, then sabotage the same root.
    Gates run once per process and never call it.
    """

    _INVENTORY_CACHE.clear()


def main() -> int:
    root = sys.argv[1] if len(sys.argv) > 1 else "."
    try:
        for directory in crate_directories(root):
            print(directory)
    except CrateTreeError as error:
        print(f"crate discovery failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
