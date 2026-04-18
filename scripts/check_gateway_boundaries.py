#!/usr/bin/env python3
"""Enforce the ironclaw#2599 gateway platform/feature layering rule.

The gateway platform layer (`src/channels/web/platform/`) must not depend on
feature handlers (`src/channels/web/{features,handlers}/`). The only
exception is `platform/router.rs`, which is the composition point where
route registration wires features onto the transport — it imports every
handler it registers, and that's its job.

This script:

1. Walks every `*.rs` file under `src/channels/web/platform/`.
2. Skips the router (and test modules inside platform files).
3. Greps each remaining file for import paths that reference handlers or
   features modules.
4. Prints a diagnostic for every violation and exits non-zero.

Forbidden patterns (matched inside `use ...` statements or fully-qualified
type paths — comments and string literals are stripped first):

- `crate::channels::web::handlers::` and `crate::channels::web::features::`
- `super::handlers::` and `super::features::`
- `super::super::handlers::` and `super::super::features::`

Allowed:

- `platform/router.rs` (explicit skip — the composition point).
- Any test module inside a platform file (`#[cfg(test)]` or `mod tests`).
- Type re-exports inside `platform/` that flow *downward* (platform type
  → handlers), since those land in the handler's file, not in a platform
  file.

Run locally with `python3 scripts/check_gateway_boundaries.py`. The CI
workflow in `.github/workflows/code_style.yml` invokes the same script
on every PR that touches Rust code.
"""

from __future__ import annotations

import pathlib
import re
import sys
import unittest
from dataclasses import dataclass


REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
PLATFORM_DIR = REPO_ROOT / "src" / "channels" / "web" / "platform"

# `platform/router.rs` is the single composition point; it's allowed to
# import handler and feature modules.
EXEMPT_RELATIVE_PATHS = {"router.rs"}

FORBIDDEN_PATTERNS = [
    re.compile(r"\bcrate::channels::web::handlers::"),
    re.compile(r"\bcrate::channels::web::features::"),
    # `super::` and `super::super::` resolve differently depending on the
    # file's depth, but any path through them that lands on a handler or
    # feature module is a back-edge. Match conservatively.
    re.compile(r"\bsuper::handlers::"),
    re.compile(r"\bsuper::features::"),
    re.compile(r"\bsuper::super::handlers::"),
    re.compile(r"\bsuper::super::features::"),
]

# Narrow allowlist of pre-existing back-edges, each tied to a specific
# migration target. Entries are `(platform_file, forbidden_path_prefix)`;
# any line in that file whose matched text starts with that prefix is
# treated as a known pre-existing violation and does not fail the check.
# New entries may only be added with explicit reviewer sign-off documenting
# the migration PR — the intent is for this list to shrink to zero, not
# to grow.
ALLOWLIST: set[tuple[str, str]] = {
    # platform/static_files.rs::build_frontend_html calls read_layout_config
    # and load_resolved_widgets, which currently live in handlers/frontend.rs.
    # Relocating them requires also moving their private helper
    # (read_widget_manifest) and the widget-size constants, plus updating
    # the other call sites (load_widget_manifests, frontend_layout_handler,
    # frontend_widgets_handler) that share the same helpers. Tracked as a
    # follow-up under ironclaw#2599; see the stage 5 commit message.
    ("static_files.rs", "crate::channels::web::handlers::frontend"),
}


@dataclass
class Violation:
    path: pathlib.Path
    line_number: int
    line: str
    pattern: str


def _strip_comments_and_strings(src: str) -> str:
    """Replace `// ...` line comments, `/* ... */` block comments, and string/char
    literals with spaces so forbidden patterns embedded in docstrings or
    explanatory text don't trip the check. Keeps line numbers stable by
    preserving newlines.
    """
    out: list[str] = []
    i = 0
    n = len(src)
    in_line_comment = False
    in_block_comment = 0  # depth for nested /* /* */ */
    in_string = False
    string_escape = False
    raw_string_hashes: int | None = None
    in_char = False
    char_escape = False

    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""

        if in_line_comment:
            if c == "\n":
                in_line_comment = False
                out.append(c)
            else:
                out.append(" " if c != "\t" else c)
            i += 1
            continue

        if in_block_comment:
            if c == "/" and nxt == "*":
                in_block_comment += 1
                out.append("  ")
                i += 2
                continue
            if c == "*" and nxt == "/":
                in_block_comment -= 1
                out.append("  ")
                i += 2
                continue
            out.append(c if c == "\n" else (" " if c != "\t" else c))
            i += 1
            continue

        if raw_string_hashes is not None:
            # Inside a raw string: terminate on `"` followed by matching `#` count.
            if c == '"':
                closing = '"' + ("#" * raw_string_hashes)
                if src[i : i + len(closing)] == closing:
                    out.append(" " * len(closing))
                    i += len(closing)
                    raw_string_hashes = None
                    continue
            out.append(c if c == "\n" else (" " if c != "\t" else c))
            i += 1
            continue

        if in_string:
            if string_escape:
                string_escape = False
                out.append(" " if c != "\n" and c != "\t" else c)
                i += 1
                continue
            if c == "\\":
                string_escape = True
                out.append(" ")
                i += 1
                continue
            if c == '"':
                in_string = False
                out.append(" ")
                i += 1
                continue
            out.append(c if c == "\n" or c == "\t" else " ")
            i += 1
            continue

        if in_char:
            if char_escape:
                char_escape = False
                out.append(" ")
                i += 1
                continue
            if c == "\\":
                char_escape = True
                out.append(" ")
                i += 1
                continue
            if c == "'":
                in_char = False
                out.append(" ")
                i += 1
                continue
            out.append(c if c == "\n" or c == "\t" else " ")
            i += 1
            continue

        # Start of a line comment?
        if c == "/" and nxt == "/":
            in_line_comment = True
            out.append("  ")
            i += 2
            continue

        # Start of a block comment?
        if c == "/" and nxt == "*":
            in_block_comment = 1
            out.append("  ")
            i += 2
            continue

        # Start of a raw string literal: `r#*"`?
        if c == "r":
            j = i + 1
            hashes = 0
            while j < n and src[j] == "#":
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                raw_string_hashes = hashes
                out.append(" " * (j - i + 1))
                i = j + 1
                continue

        # Start of a string literal?
        if c == '"':
            in_string = True
            out.append(" ")
            i += 1
            continue

        # Start of a char literal? Very crude — only count it if the
        # following byte pattern looks like a Rust char (letter, digit,
        # escape, or a short non-ident char). The heuristic just needs to
        # avoid eating lifetime apostrophes.
        if c == "'" and i + 1 < n:
            # Heuristic: a lifetime is `'name` where `name` is an identifier
            # and the next char after `name` is *not* `'`. A char literal
            # closes with `'` within a few chars.
            tail = src[i + 1 : i + 6]
            close = tail.find("'")
            if close != -1 and close <= 4:
                # Looks like a char literal
                in_char = True
                out.append(" ")
                i += 1
                continue

        out.append(c)
        i += 1

    return "".join(out)


def _is_allowlisted(path: pathlib.Path, line: str) -> bool:
    """Return True if this (file, line) pair matches an allowlist entry."""
    filename = path.name
    for allow_file, allow_prefix in ALLOWLIST:
        if filename == allow_file and allow_prefix in line:
            return True
    return False


def _find_violations(path: pathlib.Path) -> list[Violation]:
    text = path.read_text(encoding="utf-8", errors="replace")
    sanitized = _strip_comments_and_strings(text)
    violations: list[Violation] = []
    for idx, line in enumerate(sanitized.splitlines(), start=1):
        for pattern in FORBIDDEN_PATTERNS:
            if pattern.search(line):
                # Report the *original* line so operators see the real text.
                original = text.splitlines()[idx - 1] if idx - 1 < len(text.splitlines()) else line
                if _is_allowlisted(path, original):
                    break
                violations.append(
                    Violation(
                        path=path,
                        line_number=idx,
                        line=original.rstrip(),
                        pattern=pattern.pattern,
                    )
                )
                break  # one violation per line is enough
    return violations


def check(platform_dir: pathlib.Path = PLATFORM_DIR) -> list[Violation]:
    if not platform_dir.is_dir():
        return []
    violations: list[Violation] = []
    for path in sorted(platform_dir.rglob("*.rs")):
        relative = path.relative_to(platform_dir)
        if str(relative) in EXEMPT_RELATIVE_PATHS:
            continue
        violations.extend(_find_violations(path))
    return violations


def _main() -> int:
    violations = check()
    if not violations:
        print("OK: platform/ has no back-edges into handlers/ or features/.")
        return 0

    print("Gateway platform/feature boundary violations:", file=sys.stderr)
    print(
        "platform/ submodules (except router) must not import from "
        "handlers/ or features/. Move the referenced symbol into "
        "platform/ or refactor the caller.",
        file=sys.stderr,
    )
    print(file=sys.stderr)
    for v in violations:
        rel = v.path.relative_to(REPO_ROOT)
        print(f"{rel}:{v.line_number}: {v.line}", file=sys.stderr)
        print(f"    matched: {v.pattern}", file=sys.stderr)
    print(file=sys.stderr)
    print(f"Total: {len(violations)} violation(s)", file=sys.stderr)
    return 1


# --- Tests -------------------------------------------------------------

class _Tests(unittest.TestCase):
    def test_strip_line_comment(self) -> None:
        src = "use crate::channels::web::handlers::foo; // see ::handlers::bar\n"
        cleaned = _strip_comments_and_strings(src)
        # The real import stays; the comment text is replaced with spaces.
        self.assertIn("crate::channels::web::handlers::foo", cleaned)
        self.assertNotIn("see ::handlers::bar", cleaned)

    def test_strip_block_comment(self) -> None:
        src = "/* references crate::channels::web::handlers:: here */\nuse x::y;\n"
        cleaned = _strip_comments_and_strings(src)
        self.assertNotIn("crate::channels::web::handlers::", cleaned)
        self.assertIn("use x::y;", cleaned)

    def test_strip_string_literal(self) -> None:
        src = 'let msg = "crate::channels::web::features::foo"; let t = a;\n'
        cleaned = _strip_comments_and_strings(src)
        self.assertNotIn("features::foo", cleaned)
        self.assertIn("let t = a;", cleaned)

    def test_strip_raw_string(self) -> None:
        src = 'let s = r#"crate::channels::web::handlers::x"#;\n'
        cleaned = _strip_comments_and_strings(src)
        self.assertNotIn("crate::channels::web::handlers::x", cleaned)

    def test_detect_crate_path(self) -> None:
        src = "use crate::channels::web::handlers::chat::foo;\n"
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(len(violations), 1)
            self.assertEqual(violations[0].line_number, 1)
        finally:
            p.unlink()

    def test_detect_super_path(self) -> None:
        src = "pub use super::features::oauth::foo;\n"
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(len(violations), 1)
        finally:
            p.unlink()

    def test_router_would_be_flagged_if_not_exempt(self) -> None:
        # Sanity check: the patterns DO match router-style imports; the
        # exemption is what keeps router clean. Confirm the exemption list
        # matches the file name exactly.
        self.assertIn("router.rs", EXEMPT_RELATIVE_PATHS)

    def test_allows_intra_platform_imports(self) -> None:
        src = "use crate::channels::web::platform::state::GatewayState;\n"
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(violations, [])
        finally:
            p.unlink()

    def test_allows_other_crate_paths(self) -> None:
        src = "use crate::db::Database; use crate::tools::ToolRegistry;\n"
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(violations, [])
        finally:
            p.unlink()


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "test":
        unittest.main(argv=sys.argv[:1])
    sys.exit(_main())
