#!/usr/bin/env python3
"""Enforce the docs/ publication boundary.

docs/ holds both the public Mintlify site and internal engineering docs.
Mintlify builds every .md/.mdx it can see: a page omitted from docs.json
navigation is only "hidden" — still deployed, reachable by URL, and
indexable. docs/.mintignore is the only thing that stops Mintlify from
processing and serving a page, so it is the real publication boundary.

This check fails when a docs/ page is in neither bucket:

  * published:  referenced from docs.json navigation, or
  * fenced:     matched by docs/.mintignore (or Mintlify's built-in
                ignores), or
  * deliberate: frontmatter contains `hidden: true`, marking a page that is
                intentionally public-but-unlisted (reachable by URL only).

It also fails when navigation references a page whose source file does not
exist, because that ships a broken public page.

The fence list itself is frozen: all new internal material goes under
docs/internal/, which is already fenced. .mintignore entries outside
FROZEN_MINTIGNORE_PATTERNS are rejected — the legacy directories in that
set are kept until they are consolidated into internal/, and entries may
only ever be removed.

Only the .mintignore syntax this repo uses is supported (comments, blank
lines, `dir/` patterns, glob patterns, literal file names). Negation
patterns (`!`) are rejected loudly rather than silently misinterpreted.
"""

from __future__ import annotations

import fnmatch
import json
import re
import sys
from pathlib import Path, PurePosixPath

REPO_ROOT = Path(__file__).resolve().parents[2]
DOCS_ROOT = REPO_ROOT / "docs"

PAGE_SUFFIXES = (".mdx", ".md")

# Mintlify skips these regardless of .mintignore: dot-directories
# (.git, .github, .claude, .agents, .idea, ...), node_modules, the reserved
# snippets/ directory (reusable content, never standalone pages), and the
# conventional repo files below.
BUILTIN_IGNORED_DIRS = frozenset({"node_modules", "snippets"})
BUILTIN_IGNORED_FILES = frozenset(
    {"README.md", "LICENSE.md", "CHANGELOG.md", "CONTRIBUTING.md"}
)

# Navigation strings that name auto-generated OpenAPI endpoint pages, not
# source files ("GET /users"). No file existence to assert for those.
OPENAPI_PAGE_RE = re.compile(
    r"^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|WEBHOOK)\s", re.IGNORECASE
)

HIDDEN_FRONTMATTER_RE = re.compile(r"^hidden:\s*true\s*$")

# docs/.mintignore may only ever shrink. internal/ is the one growing home for
# internal docs; reborn/ is the last legacy location, kept until its
# load-bearing consumers (architecture tests reading contract files, the
# reborn-e2e scope filters) can move with it.
FROZEN_MINTIGNORE_PATTERNS = frozenset(
    {
        "drafts/",
        "*.draft.mdx",
        "internal/",
        "reborn/",
    }
)


class MintignoreSyntaxError(ValueError):
    """A .mintignore pattern uses syntax this checker does not model."""


def parse_mintignore(text: str) -> list[str]:
    patterns: list[str] = []
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("!"):
            raise MintignoreSyntaxError(
                f"negation pattern {line!r} is not supported by this checker; "
                "restructure .mintignore without negations or extend "
                "scripts/ci/docs_publication_boundary.py first"
            )
        patterns.append(line)
    return patterns


def is_ignored(rel_path: PurePosixPath, patterns: list[str]) -> bool:
    """Gitignore-style match for the pattern subset .mintignore uses."""
    parts = rel_path.parts
    if any(part.startswith(".") for part in parts):
        return True
    if any(part in BUILTIN_IGNORED_DIRS for part in parts):
        return True
    if rel_path.name in BUILTIN_IGNORED_FILES:
        return True

    for pattern in patterns:
        if pattern.endswith("/"):
            dir_pattern = pattern[:-1]
            if "/" in dir_pattern:
                if str(rel_path).startswith(dir_pattern + "/"):
                    return True
            elif any(fnmatch.fnmatch(part, dir_pattern) for part in parts[:-1]):
                return True
        elif "/" in pattern:
            if fnmatch.fnmatch(str(rel_path), pattern):
                return True
        elif any(fnmatch.fnmatch(part, pattern) for part in parts):
            return True
    return False


def collect_nav_pages(node: object) -> set[str]:
    pages: set[str] = set()
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "pages" and isinstance(value, list):
                for entry in value:
                    if isinstance(entry, str):
                        pages.add(entry)
                    else:
                        pages |= collect_nav_pages(entry)
            else:
                pages |= collect_nav_pages(value)
    elif isinstance(node, list):
        for entry in node:
            pages |= collect_nav_pages(entry)
    return pages


def frontmatter_marks_hidden(page_file: Path) -> bool:
    try:
        lines = page_file.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError):
        return False
    if not lines or lines[0].strip() != "---":
        return False
    for line in lines[1:200]:
        if line.strip() == "---":
            return False
        if HIDDEN_FRONTMATTER_RE.match(line.strip()):
            return True
    return False


def find_violations(docs_root: Path) -> tuple[list[str], list[str], list[str]]:
    """Return (unfenced page files, nav pages with no source file,
    .mintignore entries outside the frozen list)."""
    docs_json = json.loads((docs_root / "docs.json").read_text(encoding="utf-8"))
    nav_pages = collect_nav_pages(docs_json.get("navigation", {}))

    mintignore_file = docs_root / ".mintignore"
    patterns = (
        parse_mintignore(mintignore_file.read_text(encoding="utf-8"))
        if mintignore_file.exists()
        else []
    )
    unexpected_patterns = sorted(
        pattern for pattern in patterns if pattern not in FROZEN_MINTIGNORE_PATTERNS
    )

    unfenced: list[str] = []
    for page_file in sorted(docs_root.rglob("*")):
        if not page_file.is_file() or page_file.suffix not in PAGE_SUFFIXES:
            continue
        rel = PurePosixPath(page_file.relative_to(docs_root).as_posix())
        if is_ignored(rel, patterns):
            continue
        page_id = str(rel.with_suffix(""))
        if page_id in nav_pages:
            continue
        if frontmatter_marks_hidden(page_file):
            continue
        unfenced.append(str(rel))

    missing: list[str] = []
    for page in sorted(nav_pages):
        if OPENAPI_PAGE_RE.match(page):
            continue
        if not any((docs_root / (page + suffix)).is_file() for suffix in PAGE_SUFFIXES):
            missing.append(page)

    return unfenced, missing, unexpected_patterns


def main() -> int:
    try:
        unfenced, missing, unexpected = find_violations(DOCS_ROOT)
    except MintignoreSyntaxError as err:
        print(f"docs/.mintignore: {err}", file=sys.stderr)
        return 1

    if unfenced:
        print(
            "The following docs/ pages are neither in docs.json navigation nor "
            "excluded by docs/.mintignore. Mintlify will publish them as hidden "
            "pages: reachable by URL and indexable, just absent from the sidebar.",
            file=sys.stderr,
        )
        for path in unfenced:
            print(f"  docs/{path}", file=sys.stderr)
        print(
            "\nFix one of three ways:\n"
            "  * public page      -> add it to docs/docs.json navigation\n"
            "  * internal doc     -> move it under docs/internal/ (already\n"
            "    fenced; docs/.mintignore is frozen)\n"
            "  * deliberately     -> add `hidden: true` to its frontmatter to\n"
            "    unlisted page       mark it intentionally public-but-unlisted",
            file=sys.stderr,
        )

    if missing:
        print(
            "\nThe following docs.json navigation entries have no .md/.mdx "
            "source file — they ship as broken public pages:",
            file=sys.stderr,
        )
        for page in missing:
            print(f"  {page}", file=sys.stderr)

    if unexpected:
        print(
            "\ndocs/.mintignore is frozen — new internal docs belong under "
            "docs/internal/, which is already fenced. Remove these entries and "
            "move the content instead:",
            file=sys.stderr,
        )
        for pattern in unexpected:
            print(f"  {pattern}", file=sys.stderr)

    if unfenced or missing or unexpected:
        return 1
    print("docs/ publication boundary: every page is published or fenced")
    return 0


if __name__ == "__main__":
    sys.exit(main())
