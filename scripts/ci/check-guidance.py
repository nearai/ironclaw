#!/usr/bin/env python3
"""Agent guidance must reference the tree that exists.

The restructure left guidance drifted in ways that were purely mechanical:
`.claude/rules/skills.md` carried a `paths:` trigger naming a file that did not
exist (so the rule never fired for the code it governs), and `crates/AGENTS.md`
routed readers to `crates/<crate>/…` paths that stopped existing when crates
moved into family directories. A misled agent does not complain the way a
misled human does — it silently produces a bad change — so guidance drift is a
correctness bug, and the detectable classes belong in CI
(`docs/reborn/guidance-conventions.md` closes with "the guidance half is this
convention's job"; this gate is that job's mechanical part).

Four claims are checked, all against **git-tracked** state so local build
artifacts can never satisfy a reference and CI and a laptop agree:

  1. **References resolve.** Every repo path a guidance document names —
     backticked inline paths and relative markdown links in the root
     `AGENTS.md`/`CLAUDE.md`, every `AGENTS.md`/`CLAUDE.md`/`CONTRACT.md`/
     `README.md` under `crates/`, `.claude/rules/*.md`, and
     `.claude/skills/*/SKILL.md` — must resolve to a tracked file or
     directory.
  2. **Rule triggers are live.** Every `paths:` glob in a rule's (or skill's)
     frontmatter must match at least one tracked file. A rule whose glob
     matches nothing is a rule that never loads — the exact
     `skills.md` failure.
  3. **Family tables are complete.** Every crate directory (per
     `scripts/ci/lib/crate_tree.py`, the workspace's one discovery rule) must
     appear in its family's `crates/<family>/AGENTS.md` crate table — the
     guidance half of what `check-target-tree.py` does for PROPOSAL §5.
  4. **Every crate has a `README.md`.** The convention requires one per crate
     and `crates/AGENTS.md` states "Every crate has one"; measured 2026-08-06
     the tree is at 62/62, so this gates rather than warns.

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
    narration, not a claim about today's tree;
  * wire-protocol identifiers that lexically collide with repo roots
    (`NON_PATH_IDENTIFIERS`: the MCP JSON-RPC method names such as
    `tools/list`);
  * any line carrying the repo's dated-correction glyph `✎` — the convention's
    own marker for historical/corrected prose;
  * any line carrying `check-guidance: path-ok` (put it in an HTML comment
    beside a deliberate example: `<!-- check-guidance: path-ok -->`).

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
almost nothing (floor constants below) — because "the checker crashed" must
never be reported as "the guidance is fine".

    python3 scripts/ci/check-guidance.py           # verify
    python3 scripts/ci/check-guidance.py --json    # machine-readable report

Test overrides: `--repo-root` and `--tracked-files` (a file holding one
repo-relative path per line, standing in for `git ls-files`, so the self-test
needs no git repository). Self-tests patch the floor constants; production
runs never do.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import pathlib
import re
import subprocess
import sys

_LIB = pathlib.Path(__file__).resolve().parent / "lib"
sys.path.insert(0, str(_LIB))

import crate_tree  # noqa: E402  (scripts/ci/lib — the one discovery rule)

ROOT_GUIDANCE = ("AGENTS.md", "CLAUDE.md")
CRATE_GUIDANCE_BASENAMES = ("AGENTS.md", "CLAUDE.md", "CONTRACT.md", "README.md")
RULES_PREFIX = ".claude/rules/"
SKILLS_PREFIX = ".claude/skills/"

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

# Fail-closed floors. A scan that discovers almost no guidance files, or an
# extraction pass that finds almost no path references, means the discovery
# globs or the extractor broke — not that the repository stopped documenting
# itself. Refuse rather than report an empty scan as clean. (Measured
# 2026-08-06: 174 guidance files, ~800 path references, 30 rule globs.)
MIN_GUIDANCE_FILES = 40
MIN_PATH_REFERENCES = 80
MIN_RULE_GLOBS = 1

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
# owner deletes a row by adding `✎` or `<!-- check-guidance: path-ok -->` to
# the line — or by rewording it. Real drift never belongs here.
# ---------------------------------------------------------------------------
KNOWN_MISSING: tuple[Suppression, ...] = (
)


# ---------------------------------------------------------------------------
# Tracked state
# ---------------------------------------------------------------------------
def load_tracked(
    repo_root: pathlib.Path, tracked_file: pathlib.Path | None
) -> list[str]:
    """Repo-relative POSIX paths of every tracked file."""
    if tracked_file is not None:
        try:
            raw = tracked_file.read_text(encoding="utf-8")
        except OSError as error:
            raise GuidanceError(f"cannot read --tracked-files: {error}") from error
        return [line for line in raw.splitlines() if line]
    try:
        completed = subprocess.run(
            ["git", "ls-files", "-z"],
            cwd=repo_root,
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise GuidanceError("git is not on PATH; the tracked set cannot be read") from error
    except subprocess.CalledProcessError as error:
        raise GuidanceError(
            f"`git ls-files` failed, so the tracked set cannot be read: "
            f"{error.stderr.strip()}"
        ) from error
    return [path for path in completed.stdout.split("\0") if path]


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
    return docs


def _read_guidance(path: pathlib.Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise GuidanceError(f"cannot read guidance file {path}: {error}") from error


def _reference_lines(text: str, doc: str) -> list[tuple[int, str]]:
    """Prose lines eligible for extraction: fences, comments, and marked lines out."""
    lines: list[tuple[int, str]] = []
    in_fence = False
    in_comment = False
    for number, raw in enumerate(text.splitlines(), start=1):
        stripped = raw.strip()
        if not in_comment and stripped.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        # Suppression markers are honored before comment stripping — the
        # `path-ok` marker is *expected* to live inside an HTML comment.
        if CORRECTION_GLYPH in raw or SUPPRESS_MARKER in raw:
            continue
        line = raw
        if in_comment:
            end = line.find("-->")
            if end == -1:
                continue
            line = line[end + 3 :]
            in_comment = False
        while True:
            start = line.find("<!--")
            if start == -1:
                break
            end = line.find("-->", start)
            if end == -1:
                line = line[:start]
                in_comment = True
                break
            line = line[:start] + line[end + 3 :]
        if line.strip():
            lines.append((number, line))
    if in_fence:
        raise GuidanceError(
            f"{doc}: unterminated ``` fence — the extractor cannot tell prose "
            "from code, so it refuses rather than scanning the wrong half."
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
    previous_spans: list[str] = []
    for number, line in _reference_lines(text, doc):
        spans = _INLINE_CODE.findall(line)
        context = tuple(
            directory
            for span in previous_spans + spans
            for directory in crates_by_name.get(span.strip().strip("/"), ())
        )
        for span in spans:
            token = _normalize_inline(span)
            if token is None or token in NON_PATH_IDENTIFIERS:
                continue
            first = token.split("/", 1)[0]
            is_root_claim = first in tree.roots
            is_local_claim = base != "." and f"{base}/{first}" in tree.directories
            if is_root_claim or is_local_claim:
                references.append(Reference(doc, number, token, context))
        for target in _MARKDOWN_LINK.findall(line):
            token = _resolve_link(doc, target)
            if token is None or _NOT_A_PATH.search(token):
                continue
            references.append(Reference(doc, number, token, context))
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
        else:
            out.append(re.escape(glob[i]))
            i += 1
    return re.compile("^" + "".join(out) + "$")


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
                    "(docs/reborn/guidance-conventions.md)"
                )
                rows_by_family[family] = []
            else:
                rows_by_family[family] = _table_rows(
                    _read_guidance(repo_root / family_agents)
                )
        rows = rows_by_family[family]
        names = list(dict.fromkeys([crate.basename] + ([crate.package] if crate.package else [])))
        patterns = [_word_pattern(name) for name in names]
        if rows and not any(p.search(row) for row in rows for p in patterns):
            if len(names) == 1:
                spelled = f"`{names[0]}` appears in no table row"
            else:
                joined = " nor ".join(f"`{name}`" for name in names)
                spelled = f"neither {joined} appears in any table row"
            problems.append(
                f"  {crate.directory} is a crate of family `{family}`, but "
                f"{spelled} of {family_agents} — the family table no longer "
                "covers its own crates"
            )
        elif rows:
            tabled += 1

        if f"{crate.directory}/README.md" not in tree.files:
            problems.append(
                f"  {crate.directory} has no tracked README.md — the convention "
                "requires one per crate (docs/reborn/guidance-conventions.md), "
                "and crates/AGENTS.md states every crate has one"
            )
    return problems, tabled


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
        tree = Tree.build(load_tracked(repo_root, tracked_file))
        docs = discover_guidance(tree)
        if len(docs) < MIN_GUIDANCE_FILES:
            raise GuidanceError(
                f"discovered only {len(docs)} guidance files (floor is "
                f"{MIN_GUIDANCE_FILES}). The discovery globs or the checkout broke; "
                "refusing rather than scanning almost nothing."
            )
        crates = load_crates(repo_root)
        reference_problems, warnings, references = check_references(
            repo_root, docs, tree, crates
        )
        glob_problems, globs_checked = check_rule_globs(repo_root, docs, tree)
        table_problems, crates_tabled = check_family_tables(repo_root, tree, crates)
    except GuidanceError as error:
        print(f"guidance: {error}", file=sys.stderr)
        return 1

    problems = reference_problems + glob_problems + table_problems
    report = {
        "guidance_files": len(docs),
        "path_references": references,
        "rule_globs": globs_checked,
        "crates_tabled": crates_tabled,
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
            "block, mark the line with the dated-correction glyph `✎`, or add "
            "`<!-- check-guidance: path-ok -->` on the line. KNOWN_MISSING in "
            "scripts/ci/check-guidance.py is the last resort and shrinks only.",
            file=sys.stderr,
        )
        return 1

    print(
        "guidance: OK "
        f"({len(docs)} guidance files, {references} path references verified, "
        f"{globs_checked} frontmatter globs live, {crates_tabled} crates tabled "
        f"in their family AGENTS.md with README.md present, "
        f"{len(KNOWN_MISSING)} grandfathered)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
