#!/usr/bin/env python3
"""Agent guidance must reference the tree that exists.

The restructure left guidance drifted in ways that were purely mechanical:
`.claude/rules/skills.md` carried a `paths:` trigger naming a file that did not
exist (so the rule never fired for the code it governs), and `crates/AGENTS.md`
routed readers to `crates/<crate>/…` paths that stopped existing when crates
moved into family directories. A misled agent does not complain the way a
misled human does — it silently produces a bad change — so guidance drift is a
correctness bug, and the detectable classes belong in CI
(`docs/internal/reborn/guidance-conventions.md` closes with "the guidance half is this
convention's job"; this gate is that job's mechanical part).

Five claims are checked, all against **git-tracked** state so local build
artifacts can never satisfy a reference and CI and a laptop agree:

  1. **References resolve.** Every repo path a guidance document names —
     backticked inline paths and relative markdown links in the root
     `AGENTS.md`/`CLAUDE.md`, every `AGENTS.md`/`CLAUDE.md`/`CONTRACT.md`/
     `README.md` under `crates/`, `.claude/rules/*.md`, and
     `.claude/skills/*/SKILL.md` — must resolve to a tracked file or
     directory. Published `docs/` pages (everything outside `docs/internal/`,
     `zh/` mirror included) are scanned for backticked paths only — their
     markdown links are Mintlify site routes, not repo paths. The
     `docs/internal/` archive is excluded as a class, except the living spec
     pages in `INTERNAL_GUIDANCE_PREFIXES`, which are scanned as full
     guidance files (never published, so their links are repo-path claims).
  2. **Rule triggers are live.** Every `paths:` glob in a rule's (or skill's)
     frontmatter must match at least one tracked file. A rule whose glob
     matches nothing is a rule that never loads — the exact
     `skills.md` failure.
  3. **Family tables are complete.** Every crate directory (per
     `scripts/ci/lib/crate_tree.py`, the workspace's one discovery rule) must
     appear in the identity (first) column of a row in its family's
     `crates/<family>/AGENTS.md` crate table — an incidental mention in
     another row's prose does not count — the guidance half of what
     `check-target-tree.py` does for PROPOSAL §5.
  4. **Every crate has a `README.md`.** The convention requires one per crate
     and `crates/AGENTS.md` states "Every crate has one"; measured 2026-08-06
     the tree is at 62/62, so this gates rather than warns.
  5. **The `CLAUDE.md` alias rule holds.** Beside the root `AGENTS.md` and
     every `AGENTS.md` under `crates/`, a `CLAUDE.md` must be tracked — and,
     outside the two named real-file exceptions in
     `ALIAS_REAL_FILE_EXCEPTIONS` below, tracked *as a symlink* (index mode
     `120000`) whose target is exactly `AGENTS.md`. Checked against the git
     **index**, not the working tree: a committed symlink deletion, or a link
     silently replaced by a copied regular file, is the drift this catches
     (the 2026-08-06 audit proved a committed deletion left this gate green;
     a working-tree-only deletion was caught, but only accidentally, by the
     "cannot read guidance file" refusal). Standalone `CLAUDE.md` guides with
     no `AGENTS.md` sibling — the sanctioned `ironclaw_agent_loop` sub-module
     guides — are not alias sites and are not checked here.

**What is deliberately NOT a reference** (the false-positive design — a noisy
gate gets disabled, which is worse than no gate):

  * anything inside a fenced code block — commands and illustrative trees, not
    references;
  * inline tokens containing glob or placeholder characters
    (`*`, `?`, `<`, `>`, `{`, `…`, `$`, quotes, spaces) or symbol-path
    separators (`::`);
  * inline tokens whose first path segment is neither a top-level directory of
    the *live* tree (derived from `git ls-files`, never hardcoded) nor a
    tracked subdirectory of the citing document's own directory — prose about
    long-deleted trees ("the v1 `src/auth/…` was deleted") is historical
    narration, not a claim about today's tree. The known blind spot this
    buys: a dead reference whose first segment died *with its whole tree* is
    indistinguishable from narration and passes even when written as a live
    instruction — two `src/…/README.md` rows phrased as "read these files"
    hid in `.claude/skills/architecture-video/SKILL.md` until 2026-08-06.
    Only review catches that class; this gate structurally cannot;
  * wire-protocol identifiers that lexically collide with repo roots
    (`NON_PATH_IDENTIFIERS`: the MCP JSON-RPC method names such as
    `tools/list`);
  * any line carrying the repo's dated-correction glyph `✎` — the convention's
    own marker for historical/corrected prose. The glyph is deliberately
    line-scoped (a dated correction is a whole line of prose);
  * the one reference immediately preceding a `check-guidance: path-ok`
    marker — put the marker in an HTML comment right after the deliberate
    example: `` `crates/…/myprovider.rs` <!-- check-guidance: path-ok -->``.
    In `.mdx` files (where raw HTML comments are not MDX syntax) use the MDX
    comment form: `` {/* check-guidance: path-ok */}`` — both comment
    syntaxes are recognized everywhere. Unlike the glyph, the marker is
    per-reference, not per-line: other references on the same line are still
    checked, so a fresh dangling path cannot ride into the tree beside a
    vouched-for one.

**How a reference resolves** — the ways this repository's guidance actually
cites paths, each verified against the live tree before being modeled:

  * from the repo root (`crates/domains/ironclaw_llm/CLAUDE.md`);
  * relative to the citing document's directory (a crate README citing its own
    `tests/module_charter.rs` or `prompts/skill_extraction.md`);
  * as a name prefix, when the token ends in `_` or `-` ("vocabulary confined
    to `src/hosted_mcp_`") — resolves when any tracked path starts with it;
  * relative to a crate named in backticks on the same or previous line
    ("`ironclaw_loop_contracts` (`src/driver.rs`)" — both the directory
    basename and the Cargo package name count as naming the crate);
  * as a module-relative path elsewhere inside the citing document's own crate
    subtree ("a test for `policy.rs` goes in `tests/policy.rs`" in a spec
    section about `src/contribution/`) — resolves when a tracked file under
    the document's directory ends with `/<token>`.

Known-but-unresolvable references live in `KNOWN_MISSING` below, one row per
reference with a reason. The table is **shrink-only**: a row whose reference
no longer dangles (fixed, marked in-file, or no longer cited) fails the gate
until the row is deleted, so a row cannot outlive the debt it excuses, and
every run prints the surviving rows as warnings so the debt stays visible.
This gate's author must not edit guidance content, so lines that *assert a
path's absence* ("the v1 `src/config/llm.rs` is gone") sit here until the
content owner adds the `✎` / `path-ok` marker those lines should carry.

The gate fails closed on its own errors — unreadable file, unterminated or
unparseable frontmatter, broken crate discovery, an extraction pass that finds
almost nothing (floor constants below), a docs.json navigation page missing
from the docs scan — because "the checker crashed" must never be reported as
"the guidance is fine".

    python3 scripts/ci/check-guidance.py           # verify
    python3 scripts/ci/check-guidance.py --json    # machine-readable report

Test overrides: `--repo-root` and `--tracked-files` (a file holding one
repo-relative path per line — `<path> -> <target>` marks a tracked symlink —
standing in for `git ls-files -s`, so the self-test needs no git repository).
Self-tests patch the floor constants; production runs never do.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import pathlib
import re
import subprocess
import sys

_CI = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(_CI / "lib"))
sys.path.insert(0, str(_CI))

import crate_tree  # noqa: E402  (scripts/ci/lib — the one discovery rule)

# Owns the Mintlify navigation model; the docs-coverage check reuses it.
import docs_publication_boundary  # noqa: E402  (scripts/ci)

ROOT_GUIDANCE = ("AGENTS.md", "CLAUDE.md")
CRATE_GUIDANCE_BASENAMES = ("AGENTS.md", "CLAUDE.md", "CONTRACT.md", "README.md")
RULES_PREFIX = ".claude/rules/"
SKILLS_PREFIX = ".claude/skills/"
# The docs scan reads the git tree, not the publication set (`.mintignore`
# plays no part). docs/internal/ is excluded as a class: its dated plans and
# ADRs describe the tree as it stood when written (measured 2026-08-07: 705
# of 709 dangling docs references sat there).
DOCS_PREFIX = "docs/"
DOCS_EXCLUDED_PREFIX = "docs/internal/"
# Living spec pages inside the archive, still cited as authoritative, so
# they get the full guidance treatment — links included, since they are
# never published. The extension-runtime siblings checklist.md and
# implementation.md stay archival. Each prefix must match a tracked page or
# discovery refuses (a zero-match means the corpus moved).
INTERNAL_GUIDANCE_PREFIXES = (
    "docs/internal/reborn/contracts/",
    "docs/internal/reborn/extension-runtime/overview.md",
    "docs/internal/reborn/extension-runtime/standard-operations.md",
    "docs/internal/reborn/guidance-conventions.md",
)

CORRECTION_GLYPH = "✎"
SUPPRESS_MARKER = "check-guidance: path-ok"

# Wire-protocol identifiers that lexically collide with a top-level directory
# of this repository (`tools/`, `prompts/`) but are method names, not paths —
# the MCP JSON-RPC method families guidance legitimately talks about.
NON_PATH_IDENTIFIERS = frozenset(
    {
        "tools/list",
        "tools/call",
        "prompts/list",
        "prompts/get",
        "resources/list",
        "resources/read",
    }
)

# The `CLAUDE.md` alias rule (docs/internal/reborn/guidance-conventions.md, "How the
# files actually load"): beside the root `AGENTS.md` and every `AGENTS.md`
# under `crates/`, a `CLAUDE.md` symlink — index mode `120000`, target exactly
# this string — keeps Claude Code auto-injection and AGENTS.md-reading tools
# on the same bytes. A regular-file copy would drift; a deleted or retargeted
# link silently splits the two audiences again.
ALIAS_TARGET = "AGENTS.md"

# Named real-file exceptions to the alias rule — the only two, each carrying
# the reason it must stay a regular file. A row here is a live claim about
# the tree, not a skip: the gate fails when the named file goes missing,
# becomes a symlink, or its sibling `AGENTS.md` disappears, so a stale row
# cannot outlive the exception it encodes.
ALIAS_REAL_FILE_EXCEPTIONS: dict[str, str] = {
    # The root adapter carries a genuine Claude-only tail after its
    # `@AGENTS.md` import (skills routing, knowledge-graph recipes, the REPL
    # logging rule), so it cannot be a plain symlink to AGENTS.md.
    "CLAUDE.md": "root adapter: `@AGENTS.md` import plus a Claude-only tail",
    # `reborn_composition_boundaries.rs::composition_root_embeds_no_prompt_content`
    # refuses symlinks anywhere under this crate — its ownership walks do not
    # follow links — so this alias is a real file (which says so itself).
    "crates/app/ironclaw_composition/CLAUDE.md": (
        "the composition ownership walks reject symlinks "
        "(reborn_composition_boundaries.rs)"
    ),
}

# Fail-closed floors. A scan that discovers almost no guidance files, or an
# extraction pass that finds almost no path references, means the discovery
# globs or the extractor broke — not that the repository stopped documenting
# itself. Refuse rather than report an empty scan as clean. (Measured
# 2026-08-07 on the shipped tree, after the docs/ surface joined the scan:
# 364 guidance files, ~2276 path references, 57 rule globs, 65
# AGENTS.md/CLAUDE.md alias pairs. Re-measure with `--json` and re-date this
# comment when the numbers move materially.) Each floor sits roughly half of
# its measured value: low enough that legitimate consolidation never trips
# it, high enough that a parser or discovery pass silently degrading to a
# handful of hits refuses instead of passing — a floor of 1 catches only
# total loss, not the degraded-but-nonzero shape. The docs surface has no
# count floor: `check_docs_nav_coverage` validates it against docs.json
# navigation instead, and the internal spec prefixes have their own
# zero-match refusal.
MIN_GUIDANCE_FILES = 180
MIN_PATH_REFERENCES = 1100
MIN_RULE_GLOBS = 20
MIN_ALIAS_PAIRS = 40

_INLINE_CODE = re.compile(r"`([^`\n]+)`")
_MARKDOWN_LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
_LINE_SUFFIX = re.compile(r":\d+(?:-\d+)?$")
_FRONTMATTER_KEY = re.compile(r"^([A-Za-z][\w-]*):\s*(.*)$")
_LIST_ITEM = re.compile(r"^\s*-\s+(.*)$")
_PACKAGE_NAME = re.compile(r'^name\s*=\s*"([^"]+)"', re.MULTILINE)
# Characters that mark an inline token as a pattern, placeholder, or prose
# fragment rather than one concrete path.
_NOT_A_PATH = re.compile(r'[\s*?<>{}|$()"\'`\\=,;!…]|::|://')


class GuidanceError(RuntimeError):
    """The gate could not run. Never reported as "the guidance is fine"."""


@dataclasses.dataclass(frozen=True)
class Suppression:
    """One known, owned dangling reference.

    `doc` is the guidance file (repo-relative), `token` the exact normalized
    reference it names, `reason` why the reference is allowed to stay dangling
    and what deletes the row. Shrink-only: the gate fails when the row stops
    matching a live dangling reference, so the row cannot outlive the debt.
    """

    doc: str
    token: str
    reason: str


# ---------------------------------------------------------------------------
# The suppression table — shrink-only. Every row is a reference that is
# *accurate while unresolvable* and merely lacks the in-file marker this
# gate's author may not add (guidance content has its own owner). The content
# owner deletes a row by marking the line with `✎` (line-scoped), by putting
# `<!-- check-guidance: path-ok -->` immediately after the reference, or by
# rewording it. Real drift never belongs here.
# ---------------------------------------------------------------------------
KNOWN_MISSING: tuple[Suppression, ...] = (
)


# ---------------------------------------------------------------------------
# Tracked state
# ---------------------------------------------------------------------------
def load_tracked(
    repo_root: pathlib.Path, tracked_file: pathlib.Path | None
) -> tuple[list[str], dict[str, str]]:
    """Every tracked file (repo-relative POSIX paths), plus the link target of
    every tracked *symlink*.

    Both come from the git **index** (`git ls-files -s`: mode `120000` marks a
    symlink, its blob holds the target), so the committed state is what the
    alias check judges — a checkout quirk can neither hide a committed
    deletion nor forge a link that is not really tracked. The
    `--tracked-files` override marks a symlink as `<path> -> <target>`; a
    plain line is a regular file.
    """
    if tracked_file is not None:
        try:
            raw = tracked_file.read_text(encoding="utf-8")
        except OSError as error:
            raise GuidanceError(f"cannot read --tracked-files: {error}") from error
        paths: list[str] = []
        symlinks: dict[str, str] = {}
        for line in raw.splitlines():
            if not line:
                continue
            if " -> " in line:
                path, target = line.split(" -> ", 1)
                paths.append(path)
                symlinks[path] = target
            else:
                paths.append(line)
        return paths, symlinks
    try:
        completed = subprocess.run(
            ["git", "ls-files", "-s", "-z"],
            cwd=repo_root,
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise GuidanceError("git is not on PATH; the tracked set cannot be read") from error
    except subprocess.CalledProcessError as error:
        raise GuidanceError(
            f"`git ls-files -s` failed, so the tracked set cannot be read: "
            f"{error.stderr.strip()}"
        ) from error
    paths = []
    link_blobs: dict[str, list[str]] = {}
    for entry in completed.stdout.split("\0"):
        if not entry:
            continue
        try:
            header, path = entry.split("\t", 1)
            mode, blob, _stage = header.split(" ")
        except ValueError as error:
            raise GuidanceError(
                f"unparseable `git ls-files -s` entry: {entry!r}"
            ) from error
        paths.append(path)
        if mode == "120000":
            link_blobs.setdefault(blob, []).append(path)
    symlinks = {}
    for blob, blob_paths in link_blobs.items():
        try:
            shown = subprocess.run(
                ["git", "cat-file", "blob", blob],
                cwd=repo_root,
                capture_output=True,
                text=True,
                check=True,
            )
        except subprocess.CalledProcessError as error:
            raise GuidanceError(
                f"`git cat-file blob {blob}` failed, so a tracked symlink's "
                f"target cannot be read: {error.stderr.strip()}"
            ) from error
        for path in blob_paths:
            symlinks[path] = shown.stdout
    return paths, symlinks


@dataclasses.dataclass(frozen=True)
class Tree:
    """The tracked tree, indexed for path-resolution queries."""

    files: frozenset[str]
    directories: frozenset[str]
    roots: frozenset[str]
    sorted_files: tuple[str, ...]

    @classmethod
    def build(cls, tracked: list[str]) -> "Tree":
        files = frozenset(tracked)
        directories: set[str] = set()
        for path in tracked:
            parts = path.split("/")
            for depth in range(1, len(parts)):
                directories.add("/".join(parts[:depth]))
        roots = frozenset(d for d in directories if "/" not in d)
        return cls(
            files=files,
            directories=frozenset(directories),
            roots=roots,
            sorted_files=tuple(sorted(files)),
        )

    def resolves(self, token: str) -> bool:
        target = token.rstrip("/")
        return target in self.files or target in self.directories

    def any_startswith(self, prefix: str) -> bool:
        return any(path.startswith(prefix) for path in self.sorted_files)


@dataclasses.dataclass(frozen=True)
class Crate:
    directory: str
    basename: str
    package: str | None


def load_crates(repo_root: pathlib.Path) -> list[Crate]:
    """The workspace crate inventory, via the shared discovery rule."""
    try:
        directories = crate_tree.crate_directories(repo_root)
    except crate_tree.CrateTreeError as error:
        raise GuidanceError(str(error)) from error
    crates: list[Crate] = []
    for directory in directories:
        manifest = repo_root / directory / "Cargo.toml"
        try:
            text = manifest.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            raise GuidanceError(f"cannot read {directory}/Cargo.toml: {error}") from error
        # The first `name = "…"` of a crate manifest is the [package] name;
        # crate_tree has already pruned separate-workspace roots, which are
        # the only manifests here where that would not hold.
        match = _PACKAGE_NAME.search(text)
        crates.append(
            Crate(
                directory=directory,
                basename=directory.rsplit("/", 1)[-1],
                package=match.group(1) if match else None,
            )
        )
    return crates


# ---------------------------------------------------------------------------
# Check 1 — every referenced path resolves
# ---------------------------------------------------------------------------
def discover_guidance(tree: Tree) -> list[str]:
    docs: list[str] = []
    for name in ROOT_GUIDANCE:
        if name not in tree.files:
            raise GuidanceError(
                f"root {name} is not tracked — the scan seed is gone, which is a "
                "finding in itself, not a reason to scan less."
            )
        docs.append(name)
    for path in sorted(tree.files):
        if path.startswith("crates/") and path.rsplit("/", 1)[-1] in CRATE_GUIDANCE_BASENAMES:
            docs.append(path)
        elif path.startswith(RULES_PREFIX) and path.endswith(".md") and "/" not in path[len(RULES_PREFIX):]:
            docs.append(path)
        elif path.startswith(SKILLS_PREFIX) and path.endswith("/SKILL.md"):
            docs.append(path)
        elif (
            path.startswith(DOCS_PREFIX)
            and path.endswith((".md", ".mdx"))
            and (
                not path.startswith(DOCS_EXCLUDED_PREFIX)
                or path.startswith(INTERNAL_GUIDANCE_PREFIXES)
            )
        ):
            docs.append(path)
    for prefix in INTERNAL_GUIDANCE_PREFIXES:
        if not any(path.startswith(prefix) for path in docs):
            raise GuidanceError(
                f"living internal spec prefix {prefix!r} matched no tracked "
                "pages — the corpus moved or was renamed, and leaving the "
                "prefix stale would silently drop it from the scan. Update "
                "INTERNAL_GUIDANCE_PREFIXES to the new location."
            )
    return docs


def check_docs_nav_coverage(repo_root: pathlib.Path, docs: list[str]) -> int:
    """Every page docs.json navigation publishes must be in the scan.

    Validates docs discovery against the published surface itself instead of
    a count floor: a broken filter refuses on the first missing nav page.
    OpenAPI pseudo-pages ("GET /users") have no source file and are skipped,
    matching docs_publication_boundary.py. Zero covered pages also refuses —
    an empty navigation would validate nothing.
    """
    docs_json = repo_root / "docs" / "docs.json"
    try:
        manifest = json.loads(docs_json.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise GuidanceError(
            f"cannot read docs/docs.json ({error}) — the published-page set "
            "cannot be derived, so docs discovery cannot be validated."
        ) from error
    scanned = frozenset(docs)
    covered = 0
    uncovered: list[str] = []
    for page in sorted(
        docs_publication_boundary.collect_nav_pages(manifest.get("navigation", {}))
    ):
        if docs_publication_boundary.OPENAPI_PAGE_RE.match(page):
            continue
        if any(
            f"docs/{page}{suffix}" in scanned
            for suffix in docs_publication_boundary.PAGE_SUFFIXES
        ):
            covered += 1
        else:
            uncovered.append(page)
    if uncovered:
        shown = ", ".join(uncovered[:10])
        more = f" (and {len(uncovered) - 10} more)" if len(uncovered) > 10 else ""
        raise GuidanceError(
            f"docs.json navigation publishes pages the reference scan does "
            f"not cover: {shown}{more}. Either the docs branch of discovery "
            "broke (fix DOCS_PREFIX/DOCS_EXCLUDED_PREFIX) or a navigation "
            "entry has no tracked source file (docs_publication_boundary.py "
            "fails on that too); refusing rather than leaving published "
            "prose unchecked."
        )
    if covered == 0:
        raise GuidanceError(
            "docs.json navigation names no source-backed pages — the "
            "coverage check has nothing to validate docs discovery against; "
            "refusing rather than passing vacuously."
        )
    return covered


def _read_guidance(path: pathlib.Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise GuidanceError(f"cannot read guidance file {path}: {error}") from error


# Comment syntaxes stripped from prose before extraction. HTML comments are
# markdown's native form; MDX (the `.mdx` pages under `docs/`) rejects raw
# HTML comments and uses JSX comment braces instead. Both are recognized in
# every file — a marker's meaning must not change when a page is renamed
# between `.md` and `.mdx`.
_COMMENT_SYNTAXES: tuple[tuple[str, str], ...] = (("<!--", "-->"), ("{/*", "*/}"))


def _reference_lines(text: str, doc: str) -> list[tuple[int, str, tuple[int, ...]]]:
    """Prose lines eligible for extraction: fences, comments, and `✎` lines out.

    Each entry carries the positions, in the comment-stripped line, where a
    `path-ok` marker comment stood — `extract_references` suppresses only the
    one reference immediately preceding each marker. The `✎` glyph, by
    contrast, skips its whole line: a dated correction is a line of prose,
    and that line-level meaning is the glyph's documented contract.
    """
    lines: list[tuple[int, str, tuple[int, ...]]] = []
    in_fence = False
    # The closing delimiter of a comment that opened on an earlier line, or
    # None outside comments. Which syntax opened decides which closer ends it.
    pending_close: str | None = None
    for number, raw in enumerate(text.splitlines(), start=1):
        stripped = raw.strip()
        if pending_close is None and stripped.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if CORRECTION_GLYPH in raw:
            continue
        line = raw
        markers: list[int] = []
        if pending_close is not None:
            end = line.find(pending_close)
            if end == -1:
                continue
            # A marker inside a comment that opened on an earlier line has no
            # same-line reference before it; position 0 suppresses nothing.
            if SUPPRESS_MARKER in line[: end + len(pending_close)]:
                markers.append(0)
            line = line[end + len(pending_close) :]
            pending_close = None
        # Comments are collapsed left to right, so a recorded position stays
        # valid in the final string: everything before it is comment-free.
        while True:
            openings = [
                (position, opener, closer)
                for opener, closer in _COMMENT_SYNTAXES
                if (position := line.find(opener)) != -1
            ]
            if not openings:
                break
            start, opener, closer = min(openings)
            end = line.find(closer, start + len(opener))
            if end == -1:
                if SUPPRESS_MARKER in line[start:]:
                    markers.append(start)
                line = line[:start]
                pending_close = closer
                break
            if SUPPRESS_MARKER in line[start : end + len(closer)]:
                markers.append(start)
            line = line[:start] + line[end + len(closer) :]
        if line.strip():
            lines.append((number, line, tuple(markers)))
    if in_fence:
        raise GuidanceError(
            f"{doc}: unterminated ``` fence — the extractor cannot tell prose "
            "from code, so it refuses rather than scanning the wrong half."
        )
    if pending_close is not None:
        opener = next(o for o, c in _COMMENT_SYNTAXES if c == pending_close)
        raise GuidanceError(
            f"{doc}: unterminated {opener} comment — every line after it "
            "would be silently un-scanned, the same fail-open shape the "
            "fence refusal above exists to prevent."
        )
    return lines


def _normalize_inline(token: str) -> str | None:
    """A backticked span -> one concrete repo-relative path claim, or None."""
    t = token.strip()
    if t.startswith("./"):
        t = t[2:]
    if "#" in t:
        t = t.split("#", 1)[0]
    t = _LINE_SUFFIX.sub("", t)
    if not t or _NOT_A_PATH.search(t):
        return None
    if t.startswith("-") or t.startswith("/") or ".." in t.split("/"):
        return None
    if "/" not in t.rstrip("/"):
        # Bare names (`AGENTS.md`, `Cargo.toml`) and single words are prose,
        # not location claims — except an explicit top-level dir (`crates/`).
        if not t.endswith("/"):
            return None
    return t


def _resolve_link(doc: str, target: str) -> str | None:
    """A markdown link target -> repo-relative path claim, or None for externals."""
    if re.match(r"^[a-z][a-z0-9+.-]*:", target):  # http:, https:, mailto:, …
        return None
    if target.startswith("#") or not target:
        return None
    t = target.split("#", 1)[0]
    if not t:
        return None
    if t.startswith("/"):
        return None  # site-absolute — not a repo path claim
    base = pathlib.PurePosixPath(doc).parent
    joined = pathlib.PurePosixPath(*(base / t).parts)  # normalizes ./
    parts: list[str] = []
    for part in joined.parts:
        if part == "..":
            if not parts:
                return t  # escaped the repo — leave as-is; it will not resolve
            parts.pop()
        elif part != ".":
            parts.append(part)
    normalized = "/".join(parts)
    return normalized + ("/" if target.endswith("/") and normalized else "")


@dataclasses.dataclass(frozen=True)
class Reference:
    doc: str
    line: int
    token: str
    # Crate directories named in backticks on this or the previous line —
    # the "`ironclaw_loop_contracts` (`src/driver.rs`)" citation shape.
    context_crates: tuple[str, ...] = ()


def extract_references(
    doc: str, text: str, tree: Tree, crates_by_name: dict[str, tuple[str, ...]]
) -> list[Reference]:
    references: list[Reference] = []
    base = str(pathlib.PurePosixPath(doc).parent)
    # Under docs/, markdown link targets are Mintlify site routes
    # (extensionless page paths, `/using/cli` site-absolute forms) — a
    # different namespace than the tracked tree, so the link extractor is off
    # there by design. Backticked repo paths remain checked. The living
    # internal spec pages are the exception: they are fenced out of
    # publication (never a site route), and their relative links
    # (`](kernel-boundary.md)`) are genuine repo-path claims — renaming one
    # contract file would leave every cross-reference dangling with the gate
    # green.
    include_links = not doc.startswith(DOCS_PREFIX) or doc.startswith(
        INTERNAL_GUIDANCE_PREFIXES
    )
    previous_spans: list[str] = []
    for number, line, markers in _reference_lines(text, doc):
        inline_matches = list(_INLINE_CODE.finditer(line))
        spans = [match.group(1) for match in inline_matches]
        context = tuple(
            directory
            for span in previous_spans + spans
            for directory in crates_by_name.get(span.strip().strip("/"), ())
        )
        candidates: list[tuple[int, Reference]] = []
        for match in inline_matches:
            token = _normalize_inline(match.group(1))
            if token is None or token in NON_PATH_IDENTIFIERS:
                continue
            first = token.split("/", 1)[0]
            is_root_claim = first in tree.roots
            is_local_claim = base != "." and f"{base}/{first}" in tree.directories
            if is_root_claim or is_local_claim:
                candidates.append(
                    (match.end(), Reference(doc, number, token, context))
                )
        if include_links:
            for match in _MARKDOWN_LINK.finditer(line):
                token = _resolve_link(doc, match.group(1))
                if token is None or _NOT_A_PATH.search(token):
                    continue
                candidates.append(
                    (match.end(), Reference(doc, number, token, context))
                )
        candidates.sort(key=lambda item: item[0])
        # A `path-ok` marker vouches for the one reference immediately before
        # it — never the whole line, so a fresh dangling path beside a
        # vouched-for one still fails. A marker with nothing before it
        # suppresses nothing.
        suppressed: set[int] = set()
        for marker in markers:
            preceding = [
                index
                for index, (end, _) in enumerate(candidates)
                if end <= marker
            ]
            if preceding:
                suppressed.add(preceding[-1])
        references.extend(
            reference
            for index, (_, reference) in enumerate(candidates)
            if index not in suppressed
        )
        previous_spans = spans
    return references


def _reference_resolves(reference: Reference, tree: Tree) -> bool:
    token = reference.token
    base = str(pathlib.PurePosixPath(reference.doc).parent)
    candidates = [token]
    if base != ".":
        candidates.append(f"{base}/{token}")
    candidates.extend(f"{directory}/{token}" for directory in reference.context_crates)

    if token.endswith(("_", "-")):
        # A name-prefix citation ("vocabulary confined to `src/hosted_mcp_`").
        return any(tree.any_startswith(candidate) for candidate in candidates)
    if any(tree.resolves(candidate) for candidate in candidates):
        return True
    # Module-relative citation inside the document's own crate subtree
    # ("a test for `policy.rs` goes in `tests/policy.rs`" in a section about
    # `src/contribution/`). Bounded to the citing document's directory.
    if base != "." and "/" in token:
        suffix = "/" + token.rstrip("/")
        prefix = f"{base}/"
        target_dir = token.endswith("/")
        for path in tree.sorted_files:
            if not path.startswith(prefix):
                continue
            if not target_dir and path.endswith(suffix):
                return True
            if target_dir and (suffix + "/") in path:
                return True
    return False


def check_references(
    repo_root: pathlib.Path,
    docs: list[str],
    tree: Tree,
    crates: list[Crate],
) -> tuple[list[str], list[str], int]:
    crates_by_name: dict[str, tuple[str, ...]] = {}
    for crate in crates:
        for name in {crate.basename, crate.package or crate.basename}:
            crates_by_name.setdefault(name, ())
            crates_by_name[name] = crates_by_name[name] + (crate.directory,)

    problems: list[str] = []
    warnings: list[str] = []
    total = 0
    dangling: dict[tuple[str, str], Reference] = {}
    for doc in docs:
        text = _read_guidance(repo_root / doc)
        for reference in extract_references(doc, text, tree, crates_by_name):
            total += 1
            if not _reference_resolves(reference, tree):
                dangling.setdefault((reference.doc, reference.token), reference)

    suppressed = {(row.doc, row.token): row for row in KNOWN_MISSING}
    if len(suppressed) != len(KNOWN_MISSING):
        problems.append(
            "  KNOWN_MISSING holds two rows for the same reference; one debt, one row."
        )
    for key, reference in sorted(dangling.items()):
        row = suppressed.get(key)
        if row is not None:
            warnings.append(
                f"  ⚠ grandfathered dangling reference: {reference.doc}:{reference.line} "
                f"names `{reference.token}` ({row.reason})"
            )
            continue
        problems.append(
            f"  {reference.doc}:{reference.line} references `{reference.token}`, "
            "which is tracked neither from the repo root nor by any of this "
            "gate's documented relative-citation forms"
        )
    for key, row in sorted(suppressed.items()):
        if key not in dangling:
            problems.append(
                f"  KNOWN_MISSING excuses `{row.token}` in {row.doc}, but that "
                "reference no longer dangles (fixed, marked, or no longer cited). "
                "Delete the row — it cannot outlive the debt it excuses."
            )
    if total < MIN_PATH_REFERENCES:
        raise GuidanceError(
            f"extracted only {total} path references across {len(docs)} guidance "
            f"files (floor is {MIN_PATH_REFERENCES}). The extractor or the "
            "discovery globs broke; refusing rather than verifying almost nothing."
        )
    return problems, warnings, total


# ---------------------------------------------------------------------------
# Check 2 — frontmatter `paths:` globs match tracked files
# ---------------------------------------------------------------------------
def parse_frontmatter_paths(doc: str, text: str) -> list[str] | None:
    """The `paths:` globs of a frontmatter block; None when there is none.

    Deliberately a strict subset of YAML: a leading `---` fence, unindented
    `key:` lines, and a block list under `paths:`. Anything else *for the
    paths key* is an error, not a skip — an unparseable trigger is a trigger
    that silently never fires, which is the failure this check exists for.
    """
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return None
    try:
        closing = next(
            index for index, line in enumerate(lines[1:], start=1) if line.strip() == "---"
        )
    except StopIteration:
        raise GuidanceError(f"{doc}: frontmatter opened with `---` but never closed.") from None

    globs: list[str] | None = None
    collecting = False
    for line in lines[1:closing]:
        if not line.strip() or line.strip().startswith("#"):
            continue
        key_match = _FRONTMATTER_KEY.match(line)
        if key_match:
            if collecting:
                collecting = False
            if key_match.group(1) == "paths":
                if key_match.group(2).strip():
                    raise GuidanceError(
                        f"{doc}: `paths:` carries an inline value "
                        f"({key_match.group(2).strip()!r}); this gate reads block "
                        "lists only — extend the parser rather than skipping the file."
                    )
                globs = []
                collecting = True
            continue
        item = _LIST_ITEM.match(line)
        if item and collecting:
            value = item.group(1).strip().strip("\"'")
            if not value:
                raise GuidanceError(f"{doc}: empty entry under `paths:`.")
            assert globs is not None
            globs.append(value)
            continue
        if collecting:
            raise GuidanceError(
                f"{doc}: unrecognized line inside `paths:` block: {line.strip()!r}"
            )
    if globs is not None and not globs:
        raise GuidanceError(
            f"{doc}: `paths:` is present but lists no globs — the rule can never fire."
        )
    return globs


def glob_to_regex(glob: str) -> re.Pattern[str]:
    return re.compile("^" + _glob_body(glob) + "$")


def _glob_body(glob: str) -> str:
    """One glob (or brace alternative) -> regex body.

    `{a,b}` brace alternation becomes `(?:a|b)`, nesting supported, each
    alternative translated recursively — a legitimate `crates/**/*.{rs,toml}`
    trigger must count as live, not be escaped into a literal that matches
    nothing and gets reported as a dead rule. An unmatched `{` or `}` stays a
    literal character, preserving the pre-alternation behavior for degenerate
    globs.
    """
    out: list[str] = []
    i = 0
    while i < len(glob):
        if glob.startswith("**/", i) and (i == 0 or glob[i - 1] == "/"):
            out.append("(?:[^/]+/)*")
            i += 3
        elif glob.startswith("**", i) and i + 2 == len(glob):
            out.append(".+")
            i += 2
        elif glob.startswith("**", i):
            out.append(".*")
            i += 2
        elif glob[i] == "*":
            out.append("[^/]*")
            i += 1
        elif glob[i] == "?":
            out.append("[^/]")
            i += 1
        elif glob[i] == "{":
            closing = _matching_brace(glob, i)
            if closing is None:
                out.append(re.escape(glob[i]))
                i += 1
                continue
            alternatives = _split_alternatives(glob[i + 1 : closing])
            out.append(
                "(?:" + "|".join(_glob_body(part) for part in alternatives) + ")"
            )
            i = closing + 1
        else:
            out.append(re.escape(glob[i]))
            i += 1
    return "".join(out)


def _matching_brace(glob: str, start: int) -> int | None:
    depth = 0
    for i in range(start, len(glob)):
        if glob[i] == "{":
            depth += 1
        elif glob[i] == "}":
            depth -= 1
            if depth == 0:
                return i
    return None


def _split_alternatives(body: str) -> list[str]:
    """Split a brace body on top-level commas only (`{a,{b,c}}` keeps nesting)."""
    parts: list[str] = []
    current: list[str] = []
    depth = 0
    for char in body:
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
        if char == "," and depth == 0:
            parts.append("".join(current))
            current = []
        else:
            current.append(char)
    parts.append("".join(current))
    return parts


def check_rule_globs(
    repo_root: pathlib.Path, docs: list[str], tree: Tree
) -> tuple[list[str], int]:
    problems: list[str] = []
    checked = 0
    scoped = [doc for doc in docs if doc.startswith((RULES_PREFIX, SKILLS_PREFIX))]
    for doc in scoped:
        globs = parse_frontmatter_paths(doc, _read_guidance(repo_root / doc))
        if globs is None:
            continue
        for glob in globs:
            checked += 1
            pattern = glob_to_regex(glob)
            if not any(pattern.match(path) for path in tree.sorted_files):
                problems.append(
                    f"  {doc}: `paths:` glob {glob!r} matches no tracked file — "
                    "this trigger never fires, so the rule is dead for the code "
                    "it governs"
                )
    if checked < MIN_RULE_GLOBS:
        raise GuidanceError(
            f"found only {checked} `paths:` globs under {RULES_PREFIX} and "
            f"{SKILLS_PREFIX} (floor is {MIN_RULE_GLOBS}). The rules moved or the "
            "frontmatter parser broke; refusing rather than checking nothing."
        )
    return problems, checked


# ---------------------------------------------------------------------------
# Checks 3 and 4 — family crate tables and per-crate READMEs
# ---------------------------------------------------------------------------
def _table_rows(text: str) -> list[str]:
    return [line for line in text.splitlines() if line.strip().startswith("|")]


def _row_identity_cell(row: str) -> str:
    """The first cell of a table row — the crate-identity column.

    Matching the whole row would let an *incidental* mention carry a crate:
    delete `ironclaw_beta`'s row and, so long as another row's charter prose
    says "routes to `ironclaw_beta`", the table still reads as covering it
    (#7306 review). Coverage means a row whose identity column names the
    crate, so only that cell counts.
    """
    cells = row.strip().strip("|").split("|")
    return cells[0].strip() if cells else ""


def _word_pattern(name: str) -> re.Pattern[str]:
    return re.compile(rf"(?<![\w-]){re.escape(name)}(?![\w-])")


def check_family_tables(
    repo_root: pathlib.Path, tree: Tree, crates: list[Crate]
) -> tuple[list[str], int]:
    problems: list[str] = []
    rows_by_family: dict[str, list[str]] = {}
    tabled = 0
    for crate in crates:
        family = crate.directory.split("/")[1]
        family_agents = f"crates/{family}/AGENTS.md"
        if family not in rows_by_family:
            if family_agents not in tree.files:
                problems.append(
                    f"  crates/{family}/ has crates but no tracked {family_agents} — "
                    "every family carries its boundary document "
                    "(docs/internal/reborn/guidance-conventions.md)"
                )
                rows_by_family[family] = []
            else:
                rows_by_family[family] = _table_rows(
                    _read_guidance(repo_root / family_agents)
                )
        rows = rows_by_family[family]
        names = list(dict.fromkeys([crate.basename] + ([crate.package] if crate.package else [])))
        patterns = [_word_pattern(name) for name in names]
        identity_cells = [_row_identity_cell(row) for row in rows]
        if rows and not any(p.search(cell) for cell in identity_cells for p in patterns):
            if len(names) == 1:
                spelled = f"`{names[0]}` appears in no row's identity (first) column"
            else:
                joined = " nor ".join(f"`{name}`" for name in names)
                spelled = f"neither {joined} appears in any row's identity (first) column"
            problems.append(
                f"  {crate.directory} is a crate of family `{family}`, but "
                f"{spelled} of {family_agents} — the family table no longer "
                "covers its own crates (a mention in another row's description "
                "does not count as coverage)"
            )
        elif rows:
            tabled += 1

        if f"{crate.directory}/README.md" not in tree.files:
            problems.append(
                f"  {crate.directory} has no tracked README.md — the convention "
                "requires one per crate (docs/internal/reborn/guidance-conventions.md), "
                "and crates/AGENTS.md states every crate has one"
            )
    return problems, tabled


# ---------------------------------------------------------------------------
# Check 5 — a CLAUDE.md symlink alias sits beside every AGENTS.md
# ---------------------------------------------------------------------------
def check_claude_aliases(
    tree: Tree, symlinks: dict[str, str]
) -> tuple[list[str], int]:
    """The alias rule, judged from the index (see `ALIAS_TARGET` above)."""
    problems: list[str] = []
    verified = 0
    agents_docs = ["AGENTS.md"] + sorted(
        path
        for path in tree.files
        if path.startswith("crates/") and path.endswith("/AGENTS.md")
    )
    if len(agents_docs) < MIN_ALIAS_PAIRS:
        raise GuidanceError(
            f"found only {len(agents_docs)} AGENTS.md alias sites (floor is "
            f"{MIN_ALIAS_PAIRS}). The discovery prefix broke or the checkout "
            "is partial; refusing rather than verifying almost nothing."
        )
    seen_aliases: set[str] = set()
    for agents in agents_docs:
        alias = agents[: -len("AGENTS.md")] + "CLAUDE.md"
        seen_aliases.add(alias)
        reason = ALIAS_REAL_FILE_EXCEPTIONS.get(alias)
        if reason is not None:
            if alias not in tree.files:
                problems.append(
                    f"  {alias} is not tracked, yet it is a named real-file "
                    f"exception to the alias rule ({reason}) — restore the real "
                    "file, or delete its ALIAS_REAL_FILE_EXCEPTIONS row in "
                    "scripts/ci/check-guidance.py if the exception no longer "
                    "applies"
                )
            elif alias in symlinks:
                problems.append(
                    f"  {alias} is tracked as a symlink, yet it is a named "
                    f"real-file exception to the alias rule ({reason}) — restore "
                    "the real file, or delete its ALIAS_REAL_FILE_EXCEPTIONS row "
                    "now that the reason no longer holds"
                )
            else:
                verified += 1
            continue
        if alias not in tree.files:
            problems.append(
                f"  {agents} has no tracked CLAUDE.md beside it — every "
                "AGENTS.md at the root and under crates/ carries a "
                "`CLAUDE.md -> AGENTS.md` symlink alias so Claude Code "
                "auto-injection and AGENTS.md readers see the same bytes "
                "(docs/internal/reborn/guidance-conventions.md). Restore it: "
                f"`ln -s AGENTS.md {alias} && git add {alias}`"
            )
        elif alias not in symlinks:
            problems.append(
                f"  {alias} is tracked as a regular file, not a symlink — the "
                "alias must stay index mode 120000 targeting `AGENTS.md`, or "
                "the two copies drift apart. Replace it: `git rm "
                f"{alias} && ln -s AGENTS.md {alias} && git add {alias}` "
                "(a real-file alias is legal only as a named "
                "ALIAS_REAL_FILE_EXCEPTIONS row in "
                "scripts/ci/check-guidance.py, with its reason)"
            )
        elif symlinks[alias] != ALIAS_TARGET:
            problems.append(
                f"  {alias} is a symlink to {symlinks[alias]!r} — the alias "
                f"must target the sibling `{ALIAS_TARGET}` exactly (a bare "
                "one-segment relative link), nothing else"
            )
        else:
            verified += 1
    for alias in sorted(ALIAS_REAL_FILE_EXCEPTIONS):
        if alias not in seen_aliases:
            problems.append(
                f"  ALIAS_REAL_FILE_EXCEPTIONS names {alias}, but no AGENTS.md "
                "sits beside it, so the row excuses nothing — delete it from "
                "scripts/ci/check-guidance.py; a stale exception cannot "
                "outlive the pair it encodes"
            )
    return problems, verified


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------
def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Verify guidance references, rule globs, and family tables."
    )
    parser.add_argument("--repo-root", default=None, help="repository root (test override)")
    parser.add_argument(
        "--tracked-files",
        default=None,
        help="file listing tracked paths, one per line (test override for `git ls-files`)",
    )
    parser.add_argument("--json", action="store_true", help="machine-readable report")
    args = parser.parse_args(argv)

    repo_root = pathlib.Path(
        args.repo_root or pathlib.Path(__file__).resolve().parents[2]
    )
    tracked_file = pathlib.Path(args.tracked_files) if args.tracked_files else None

    try:
        tracked, symlinks = load_tracked(repo_root, tracked_file)
        tree = Tree.build(tracked)
        docs = discover_guidance(tree)
        if len(docs) < MIN_GUIDANCE_FILES:
            raise GuidanceError(
                f"discovered only {len(docs)} guidance files (floor is "
                f"{MIN_GUIDANCE_FILES}). The discovery globs or the checkout broke; "
                "refusing rather than scanning almost nothing."
            )
        # All scanned docs/ files; published-surface health is nav_pages_covered.
        docs_files = sum(1 for doc in docs if doc.startswith(DOCS_PREFIX))
        nav_covered = check_docs_nav_coverage(repo_root, docs)
        crates = load_crates(repo_root)
        reference_problems, warnings, references = check_references(
            repo_root, docs, tree, crates
        )
        glob_problems, globs_checked = check_rule_globs(repo_root, docs, tree)
        table_problems, crates_tabled = check_family_tables(repo_root, tree, crates)
        alias_problems, aliases_verified = check_claude_aliases(tree, symlinks)
    except GuidanceError as error:
        print(f"guidance: {error}", file=sys.stderr)
        return 1

    problems = reference_problems + glob_problems + table_problems + alias_problems
    report = {
        "guidance_files": len(docs),
        "docs_files": docs_files,
        "nav_pages_covered": nav_covered,
        "path_references": references,
        "rule_globs": globs_checked,
        "crates_tabled": crates_tabled,
        "aliases_verified": aliases_verified,
        "grandfathered": [dataclasses.asdict(row) for row in KNOWN_MISSING],
        "warnings": warnings,
        "problems": problems,
    }

    if args.json:
        print(json.dumps(report, indent=2, ensure_ascii=False))
        return 1 if problems else 0

    for warning in warnings:
        print(warning, file=sys.stderr)
    if problems:
        print(
            "✗ guidance references drifted from the tree:\n"
            + "\n".join(problems)
            + "\n\nFix: repoint the reference (or glob, or table row) at today's "
            "tree. For a deliberately historical mention, put it in a fenced "
            "block, mark the whole line with the dated-correction glyph `✎`, or "
            "put `<!-- check-guidance: path-ok -->` immediately after the one "
            "reference it vouches for. KNOWN_MISSING in "
            "scripts/ci/check-guidance.py is the last resort and shrinks only.",
            file=sys.stderr,
        )
        return 1

    print(
        "guidance: OK "
        f"({len(docs)} guidance files, {references} path references verified, "
        f"{nav_covered} published docs pages nav-covered, "
        f"{globs_checked} frontmatter globs live, {crates_tabled} crates tabled "
        f"in their family AGENTS.md with README.md present, "
        f"{aliases_verified} CLAUDE.md aliases verified, "
        f"{len(KNOWN_MISSING)} grandfathered)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
