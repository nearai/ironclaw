#!/usr/bin/env python3
"""Committed `.wasm` artifacts must be no older than the sources beside them.

Every package under `extensions/packages/` that ships a WASM tool keeps two
things in the repository: the built component (`wasm/<name>.wasm`) and the
guest crate that produced it (`wasm-src/`). Nothing linked them. A PR could
edit `wasm-src/` and never rebuild, and CI stayed green: the rebuild job runs
`scripts/build-wasm-extensions.sh`, which *overwrites* the committed artifact
in the working tree and then tests the freshly built one, without ever
comparing it to what is committed. The stale artifact is what ships.

The obvious check — rebuild and `git diff --exit-code` — does not work here.
The guest builds are not reproducible: they pin no toolchain, resolve their own
`Cargo.lock` at build time, and `wit-bindgen` versions differ per guest, so two
honest builds of the same source differ in bytes. Pinning artifact bytes would
be a new and unkeepable obligation.

So this gate records the *source* digest instead. A guest's `wasm-src/` tree
hashes deterministically; the recorded digest says "the committed artifact was
produced from this source". Editing `wasm-src/` without rebuilding changes the
source digest and fails; rebuilding and re-recording passes. That is exactly
the invariant that was missing, and it costs no build.

    python3 scripts/ci/check-wasm-artifact-freshness.py            # verify
    python3 scripts/ci/check-wasm-artifact-freshness.py --update   # re-record

Re-record only after `./scripts/build-wasm-extensions.sh --first-party` and
committing the rebuilt artifact — the digest asserts a claim about the
artifact, and updating it without rebuilding launders a stale one.

Test override: IRONCLAW_REPO_ROOT selects the tree to check.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "lib"))
from crate_tree import CrateTreeError, crate_directory  # noqa: E402

# The support crate anchors discovery by NAME so a family move cannot make this
# gate scan nothing; `packages/` is its sibling (PROPOSAL §5). Same anchor and
# same hop as `scripts/build-wasm-extensions.sh`.
ANCHOR_CRATE = "ironclaw_extension_support"
DIGEST_FILE = "scripts/ci/wasm-src-digests.toml"

# Build outputs and lockfiles are not source: `target/` is generated, and the
# guests do not commit `Cargo.lock` (they resolve fresh, which is also why
# artifact bytes are not reproducible).
_SKIPPED_NAMES = {"target", "Cargo.lock"}


class FreshnessError(RuntimeError):
    """The gate could not run, or found a stale artifact."""


def packages_root(repo_root: pathlib.Path) -> pathlib.Path:
    try:
        anchor = crate_directory(ANCHOR_CRATE, repo_root)
    except CrateTreeError as error:
        raise FreshnessError(
            f"cannot resolve the {ANCHOR_CRATE} crate, so the package root is unknown "
            f"and this gate would check nothing: {error}"
        ) from error
    root = repo_root / pathlib.PurePosixPath(anchor).parent / "packages"
    if not root.is_dir():
        raise FreshnessError(
            f"{root} is not a directory — the package tree moved out from under this "
            "gate. Refusing rather than reporting every artifact fresh."
        )
    return root


def source_digest(wasm_src: pathlib.Path) -> str:
    """Deterministic digest of a guest crate's source tree.

    Path and content both feed the hash, so a rename is a change. Paths are
    POSIX-normalized and sorted so the result does not depend on filesystem
    ordering or platform.
    """
    digest = hashlib.sha256()
    files = []
    for path in wasm_src.rglob("*"):
        relative = path.relative_to(wasm_src)
        if any(part in _SKIPPED_NAMES for part in relative.parts):
            continue
        if path.is_file():
            files.append((relative.as_posix(), path))
    for relative, path in sorted(files):
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def committed_artifact(package_dir: pathlib.Path) -> pathlib.Path | None:
    """The single committed `.wasm` under `<package>/wasm/`, if any."""
    wasm_dir = package_dir / "wasm"
    if not wasm_dir.is_dir():
        return None
    artifacts = sorted(wasm_dir.glob("*.wasm"))
    if len(artifacts) != 1:
        raise FreshnessError(
            f"{wasm_dir} holds {len(artifacts)} `.wasm` files; expected exactly one so "
            "the recorded digest names an unambiguous artifact."
        )
    return artifacts[0]


def measure(repo_root: pathlib.Path) -> dict[str, str]:
    """Package id -> source digest, for every package shipping a WASM guest."""
    root = packages_root(repo_root)
    measured: dict[str, str] = {}
    for package_dir in sorted(root.iterdir()):
        wasm_src = package_dir / "wasm-src"
        if not package_dir.is_dir() or not wasm_src.is_dir():
            continue
        artifact = committed_artifact(package_dir)
        if artifact is None:
            raise FreshnessError(
                f"{package_dir.name} ships `wasm-src/` but no `wasm/*.wasm`. Either the "
                "artifact was deleted without its source, or a build was never "
                "committed — both mean the package cannot serve its declared tools."
            )
        measured[package_dir.name] = source_digest(wasm_src)
    if not measured:
        raise FreshnessError(
            f"no package under {root} ships a `wasm-src/` guest. Six do today; finding "
            "none means discovery is broken, not that the obligation went away."
        )
    return measured


def load_recorded(path: pathlib.Path) -> dict[str, str]:
    if not path.is_file():
        return {}
    import tomllib

    with path.open("rb") as handle:
        data = tomllib.load(handle)
    return {str(k): str(v) for k, v in data.get("packages", {}).items()}


def render(measured: dict[str, str]) -> str:
    lines = [
        "# Source digests for the committed WASM artifacts under",
        "# `crates/extensions/packages/*/wasm/`. Each value is a sha256 over the",
        "# package's `wasm-src/` tree (paths + contents, `target/` and `Cargo.lock`",
        "# excluded).",
        "#",
        "# Regenerate ONLY after rebuilding and committing the artifacts:",
        "#   ./scripts/build-wasm-extensions.sh --first-party",
        "#   python3 scripts/ci/check-wasm-artifact-freshness.py --update",
        "#",
        "# Updating this file without rebuilding launders a stale artifact past the",
        "# gate, which is the exact failure it exists to catch.",
        "",
        "[packages]",
    ]
    lines.extend(f'{name} = "{digest}"' for name, digest in sorted(measured.items()))
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--update",
        action="store_true",
        help="re-record the digests instead of verifying them",
    )
    args = parser.parse_args()

    repo_root = pathlib.Path(
        os.environ.get("IRONCLAW_REPO_ROOT", pathlib.Path(__file__).resolve().parents[2])
    )
    digest_path = repo_root / DIGEST_FILE

    try:
        measured = measure(repo_root)
    except FreshnessError as error:
        print(f"wasm artifact freshness: {error}", file=sys.stderr)
        return 1

    if args.update:
        digest_path.parent.mkdir(parents=True, exist_ok=True)
        digest_path.write_text(render(measured), encoding="utf-8")
        print(f"recorded {len(measured)} wasm source digest(s) in {DIGEST_FILE}")
        return 0

    recorded = load_recorded(digest_path)
    problems: list[str] = []
    for name, digest in sorted(measured.items()):
        if name not in recorded:
            problems.append(
                f"  {name}: no recorded digest. A package that ships a committed "
                f"artifact must record the source it was built from."
            )
        elif recorded[name] != digest:
            problems.append(
                f"  {name}: wasm-src changed but wasm/ was not rebuilt\n"
                f"      recorded {recorded[name]}\n"
                f"      measured {digest}"
            )
    for name in sorted(set(recorded) - set(measured)):
        problems.append(
            f"  {name}: recorded, but no such package ships a `wasm-src/` guest. A "
            f"stale entry pins nothing — delete it in the change that removed the "
            f"package."
        )

    if problems:
        print(
            "✗ committed WASM artifacts are out of date with their sources:\n"
            + "\n".join(problems)
            + "\n\nFix: rebuild and commit the artifact, then re-record:\n"
            "    ./scripts/build-wasm-extensions.sh --first-party\n"
            "    python3 scripts/ci/check-wasm-artifact-freshness.py --update",
            file=sys.stderr,
        )
        return 1

    print(f"wasm artifact freshness: OK ({len(measured)} package(s) checked)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
