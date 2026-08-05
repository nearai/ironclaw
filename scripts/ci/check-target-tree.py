#!/usr/bin/env python3
"""The crate tree on disk must match the tree PROPOSAL §5 documents.

WS7 moved 57 crates into ten family directories. Nothing checked the result.
Every other gate in this repository answers "where is crate X?" by *discovery*
(`scripts/ci/lib/crate_tree.py`), which is exactly right for a gate that must
survive a move — and exactly why none of them can tell you the move went where
the design said. A crate landing in `substrates/` instead of `domains/` is
invisible to all of them; it shows up only when a reader trusts §5 and finds it
wrong.

So this gate compares the two directly:

  * the documented tree — parsed out of the fenced `text` block under
    `## 5. Recommended target directory tree` in
    `docs/reborn/target-architecture/PROPOSAL.md`, which is the *only* copy;
    this script deliberately embeds no second copy that could drift from it;
  * the real tree — `cargo metadata --no-deps`, i.e. every workspace member's
    manifest directory and package name.

Three claims are checked:

  1. **Placement.** Every workspace member sits at the directory §5 draws for
     it, and §5 draws no crate that does not exist.
  2. **Naming.** A crate's directory carries its full package name (§5.1's
     directory rule). The two written exceptions are read out of the tree
     itself: `app/ironclaw_cli` holds the package named `ironclaw` (the
     annotation says so), and package directories under `extensions/packages/`
     are named by extension identity with the crate name written beside the
     `▣` marker.
  3. **Exclusions.** A `◇` entry is a package this workspace deliberately does
     not build. It must exist on disk and must NOT be a workspace member —
     otherwise "excluded" is a claim nothing enforces.

Known, owned deltas live in `EXCEPTIONS` below. That table is **shrink-only**:
an uncovered delta fails, and so does an exception row that no longer describes
a real delta. Closing a disposition therefore means deleting its row, and the
row cannot outlive the thing it excuses.

    python3 scripts/ci/check-target-tree.py           # verify
    python3 scripts/ci/check-target-tree.py --json    # machine-readable report

Test overrides: `--repo-root`, `--proposal`, and `--metadata` (a file holding
`cargo metadata --format-version 1 --no-deps` output, so the self-test does not
need a cargo toolchain).
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import pathlib
import re
import subprocess
import sys

PROPOSAL_RELATIVE = "docs/reborn/target-architecture/PROPOSAL.md"
SECTION_HEADING = "## 5. Recommended target directory tree"

PACKAGE_MARKER = "▣"
DIRECTORY_MARKER = "▢"
EXCLUDED_MARKER = "◇"
MARKERS = PACKAGE_MARKER + DIRECTORY_MARKER + EXCLUDED_MARKER

# A tree that parses to a handful of packages means the block moved, was
# reformatted, or the heading changed — not that the workspace shrank by 50
# crates. Refuse rather than report a tree that matches a tree nobody wrote.
MIN_DOCUMENTED_PACKAGES = 50

_CONNECTOR = re.compile(r"^(?P<indent>[│ ]*)(?:├──|└──) (?P<rest>.*)$")
_CLI_PACKAGE = re.compile(r"package name stays `(?P<name>[^`]+)`")
_ROOT_PACKAGE_LINE = "(workspace root package)"


class TargetTreeError(RuntimeError):
    """The gate could not run. Never reported as "the tree matches"."""


@dataclasses.dataclass(frozen=True)
class Exception_:
    """One known, owned divergence between the tree on disk and §5.

    `documented` is the §5 path (``None`` when §5 draws the crate nowhere,
    because its disposition is deletion or a merge). `actual` is where the
    crate really is (``None`` when §5 draws a crate that has not been built
    yet). `owner` names the row that closes this exception — deleting the row
    here is part of landing that row, and this gate fails if the row is deleted
    early or left behind late.
    """

    package: str
    actual: str | None
    documented: str | None
    owner: str
    why: str


# ---------------------------------------------------------------------------
# The exceptions table — shrink-only. Every row names the row that closes it.
# ---------------------------------------------------------------------------
#
# ✎ 2026-08-05: the `ironclaw_projects` row is GONE, not edited — its
# disposition closed. WS10's merge row landed the §12.10 consolidation verdict
# (`projects` → `identity`, as the `projects` module), the package stopped being
# a workspace member, and this gate said so by name: *"EXCEPTIONS carries a row
# for a delta that no longer exists … Delete the row."* That is the table
# working as designed — a row cannot outlive what it excuses — and it is why
# closing a disposition is a deletion here rather than a rewrite. One row left.
EXCEPTIONS: tuple[Exception_, ...] = (
    Exception_(
)


# ---------------------------------------------------------------------------
# Parsing §5
# ---------------------------------------------------------------------------
@dataclasses.dataclass(frozen=True)
class DocumentedEntry:
    directory: str
    package: str
    excluded: bool


def read_section(proposal: pathlib.Path) -> str:
    """The fenced `text` block under §5's heading."""
    try:
        text = proposal.read_text(encoding="utf-8")
    except OSError as error:
        raise TargetTreeError(
            f"cannot read {proposal} — §5 is the only copy of the target tree, so "
            f"without it this gate has nothing to compare against: {error}"
        ) from error
    if SECTION_HEADING not in text:
        raise TargetTreeError(
            f"{proposal} has no {SECTION_HEADING!r} heading. If §5 was renumbered, "
            "repoint this gate in the same change rather than letting it fail open."
        )
    tail = text.split(SECTION_HEADING, 1)[1]
    opener = "```text"
    if opener not in tail:
        raise TargetTreeError(
            f"no ```text block follows {SECTION_HEADING!r} in {proposal}; the tree is "
            "not where this gate reads it from."
        )
    body = tail.split(opener, 1)[1]
    if "```" not in body:
        raise TargetTreeError(f"unterminated ```text block under §5 in {proposal}.")
    return body.split("```", 1)[0]


def _split_marker(rest: str) -> tuple[str, str, str]:
    """`"ironclaw_wasm  ▣ [runtimes] …"` -> (name, marker, annotation)."""
    positions = [rest.index(marker) for marker in MARKERS if marker in rest]
    if not positions:
        return rest.strip(), "", ""
    index = min(positions)
    return rest[:index].strip(), rest[index], rest[index + 1 :].strip()


def parse_tree(block: str) -> list[DocumentedEntry]:
    """Every `▣`/`◇` leaf of §5's tree, as (directory, package, excluded)."""
    entries: list[DocumentedEntry] = []
    root = ""
    stack: dict[int, str] = {}

    for raw in block.splitlines():
        line = raw.rstrip()
        if not line.strip():
            continue

        match = _CONNECTOR.match(line)
        if match is None:
            # Flush-left lines start a new top-level tree (`crates/`, `tools/`);
            # anything else at this indentation is a continuation of the entry
            # above and carries no `├──`/`└──` of its own.
            if line[0] in "│ ":
                continue
            if line.startswith(_ROOT_PACKAGE_LINE):
                _, marker, annotation = _split_marker(line)
                if marker == PACKAGE_MARKER and annotation:
                    entries.append(
                        DocumentedEntry(".", annotation.split()[0], excluded=False)
                    )
                continue
            first = line.split()[0]
            root = first.rstrip("/") if first.endswith("/") else ""
            stack = {}
            continue

        depth = len(match.group("indent")) // 4
        name, marker, annotation = _split_marker(match.group("rest"))
        if not name:
            continue

        parents = [stack[level] for level in sorted(stack) if level < depth]
        bare = name.rstrip("/")
        segments = ([root] if root else []) + parents + [bare]
        directory = "/".join(segments)

        if name.endswith("/"):
            stack = {level: value for level, value in stack.items() if level < depth}
            stack[depth] = bare

        if marker == PACKAGE_MARKER:
            # A package *directory* (`slack/`) writes its crate name beside the
            # marker; a crate directory carries its own package name, except
            # `app/ironclaw_cli`, whose annotation says which name it holds.
            if name.endswith("/"):
                if not annotation:
                    raise TargetTreeError(
                        f"§5 marks {directory!r} as a package but names no crate; a "
                        "package directory must write its crate name beside `▣`."
                    )
                package = annotation.split()[0]
            else:
                override = _CLI_PACKAGE.search(annotation)
                package = override.group("name") if override else bare
            entries.append(DocumentedEntry(directory, package, excluded=False))
        elif marker == EXCLUDED_MARKER:
            entries.append(DocumentedEntry(directory, bare, excluded=True))

    packages = [entry for entry in entries if not entry.excluded]
    if len(packages) < MIN_DOCUMENTED_PACKAGES:
        raise TargetTreeError(
            f"§5 parsed to only {len(packages)} packages (floor is "
            f"{MIN_DOCUMENTED_PACKAGES}). The block moved or its shape changed; "
            "refusing rather than declaring a tree that matches almost nothing."
        )
    return entries


# ---------------------------------------------------------------------------
# The real tree
# ---------------------------------------------------------------------------
def load_members(
    repo_root: pathlib.Path, metadata_file: pathlib.Path | None
) -> dict[str, str]:
    """Workspace member directory (repo-relative, POSIX) -> package name."""
    if metadata_file is not None:
        raw = metadata_file.read_text(encoding="utf-8")
    else:
        try:
            raw = subprocess.run(
                ["cargo", "metadata", "--format-version", "1", "--no-deps"],
                cwd=repo_root,
                capture_output=True,
                text=True,
                check=True,
            ).stdout
        except FileNotFoundError as error:
            raise TargetTreeError(
                "cargo is not on PATH, so the real tree cannot be read"
            ) from error
        except subprocess.CalledProcessError as error:
            raise TargetTreeError(
                f"`cargo metadata` failed, so the real tree cannot be read: "
                f"{error.stderr.strip()}"
            ) from error

    metadata = json.loads(raw)
    workspace_root = pathlib.Path(metadata["workspace_root"])
    members: dict[str, str] = {}
    for package in metadata["packages"]:
        directory = pathlib.Path(package["manifest_path"]).parent
        if directory == workspace_root:
            relative = "."
        else:
            relative = directory.relative_to(workspace_root).as_posix()
        members[relative] = package["name"]
    return members


# ---------------------------------------------------------------------------
# Comparison
# ---------------------------------------------------------------------------
def compare(
    repo_root: pathlib.Path,
    documented: list[DocumentedEntry],
    members: dict[str, str],
) -> tuple[list[str], dict[str, object]]:
    problems: list[str] = []
    documented_packages = {e.package: e for e in documented if not e.excluded}
    documented_excluded = {e.package: e for e in documented if e.excluded}
    actual_by_package: dict[str, str] = {
        name: directory for directory, name in members.items()
    }

    by_package = {exception.package: exception for exception in EXCEPTIONS}
    if len(by_package) != len(EXCEPTIONS):
        problems.append(
            "  EXCEPTIONS holds two rows for the same package; one delta, one row."
        )

    used: set[str] = set()
    misplaced: list[str] = []
    undocumented: list[str] = []
    unbuilt: list[str] = []

    for package, directory in sorted(actual_by_package.items()):
        entry = documented_packages.get(package)
        exception = by_package.get(package)
        if entry is not None and entry.directory == directory:
            if exception is not None:
                problems.append(
                    f"  {package}: EXCEPTIONS still excuses it, but it now sits at "
                    f"{directory} exactly as §5 draws. Delete the row "
                    f"(owner: {exception.owner})."
                )
                used.add(package)
            continue
        if exception is not None:
            used.add(package)
            if exception.actual != directory or exception.documented != (
                entry.directory if entry else None
            ):
                problems.append(
                    f"  {package}: EXCEPTIONS records actual={exception.actual!r} / "
                    f"documented={exception.documented!r}, but the tree says "
                    f"actual={directory!r} / documented="
                    f"{(entry.directory if entry else None)!r}. The row describes a "
                    "different delta than the one that exists."
                )
            continue
        if entry is None:
            undocumented.append(f"  {package} is at {directory}, and §5 draws no home for it")
        else:
            misplaced.append(
                f"  {package} is at {directory}, but §5 draws it at {entry.directory}"
            )

    for package, entry in sorted(documented_packages.items()):
        if package in actual_by_package:
            continue
        exception = by_package.get(package)
        if exception is not None:
            used.add(package)
            continue
        unbuilt.append(
            f"  {package} is drawn at {entry.directory} by §5, but no workspace member "
            "has that name"
        )

    for package, exception in sorted(by_package.items()):
        if package not in used:
            problems.append(
                f"  {package}: EXCEPTIONS carries a row for a delta that no longer "
                f"exists (the package is not a workspace member at all). Delete the "
                f"row (owner: {exception.owner})."
            )

    for package, entry in sorted(documented_excluded.items()):
        if package in actual_by_package:
            problems.append(
                f"  {package}: §5 marks it `◇` (excluded from this workspace) but it "
                f"is a workspace member at {actual_by_package[package]}. Either the "
                "exclusion or the tree is wrong."
            )
        manifest = repo_root / entry.directory / "Cargo.toml"
        if not manifest.is_file():
            problems.append(
                f"  {package}: §5 draws it at {entry.directory}, but there is no "
                f"{entry.directory}/Cargo.toml. An excluded package nothing builds is "
                "the easiest thing in the repository to lose track of."
            )

    problems.extend(misplaced)
    problems.extend(undocumented)
    problems.extend(unbuilt)

    report: dict[str, object] = {
        "documented_packages": len(documented_packages),
        "documented_excluded": len(documented_excluded),
        "workspace_members": len(members),
        "exceptions": [dataclasses.asdict(e) for e in EXCEPTIONS],
        "problems": problems,
    }
    return problems, report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Compare the crate tree to PROPOSAL §5.")
    parser.add_argument("--repo-root", default=None, help="repository root (test override)")
    parser.add_argument("--proposal", default=None, help="PROPOSAL.md path (test override)")
    parser.add_argument(
        "--metadata",
        default=None,
        help="file holding `cargo metadata --no-deps` output (test override)",
    )
    parser.add_argument("--json", action="store_true", help="machine-readable report")
    args = parser.parse_args(argv)

    repo_root = pathlib.Path(
        args.repo_root or pathlib.Path(__file__).resolve().parents[2]
    )
    proposal = pathlib.Path(args.proposal) if args.proposal else repo_root / PROPOSAL_RELATIVE
    metadata_file = pathlib.Path(args.metadata) if args.metadata else None

    try:
        documented = parse_tree(read_section(proposal))
        members = load_members(repo_root, metadata_file)
        problems, report = compare(repo_root, documented, members)
    except TargetTreeError as error:
        print(f"target tree: {error}", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(report, indent=2, ensure_ascii=False))
        return 1 if problems else 0

    if problems:
        print(
            "✗ the crate tree does not match PROPOSAL §5:\n"
            + "\n".join(problems)
            + "\n\nFix: move the crate to its §5 path, or — if the divergence is "
            "intended and owned — amend §5 and add a row to EXCEPTIONS in "
            "scripts/ci/check-target-tree.py naming the row that closes it.",
            file=sys.stderr,
        )
        return 1

    print(
        "target tree: OK "
        f"({report['workspace_members']} workspace members against "
        f"{report['documented_packages']} documented packages, "
        f"{report['documented_excluded']} documented exclusion(s), "
        f"{len(EXCEPTIONS)} owned exception(s))"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
