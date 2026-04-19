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
- `crate::channels::web::server::` (the `server.rs` compatibility shim —
  routing through it re-creates the same platform → feature back-edge the
  shim is meant to be migrated away from)
- `super::handlers::`, `super::features::`, and `super::server::`
- `super::super::handlers::`, `super::super::features::`, and
  `super::super::server::`

Grouped `use` statements that span multiple lines are matched across the
joined statement, so

    use crate::channels::web::{
        handlers::auth::login_handler,
        platform::state::GatewayState,
    };

is rejected even though `handlers::` never appears on the same line as
`crate::channels::web::`.

Allowed:

- `platform/router.rs` (explicit skip — the composition point).
- Any test module inside a platform file (`#[cfg(test)]` or `mod tests {`):
  the whole `{ ... }` body is blanked before pattern matching, so
  caller-level regression tests in platform files are free to import
  handler/feature modules.
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
    # The `server.rs` compatibility shim hides platform → feature back-edges
    # behind a re-export, so rejecting it is what the reviewer requested
    # while the shim still exists. The allowlist below carries the specific
    # pre-existing call sites that are tracked for follow-up migration.
    re.compile(r"\bcrate::channels::web::server::"),
    # `super::` and `super::super::` resolve differently depending on the
    # file's depth, but any path through them that lands on a handler,
    # feature, or the `server.rs` shim is a back-edge. Match conservatively.
    re.compile(r"\bsuper::handlers::"),
    re.compile(r"\bsuper::features::"),
    re.compile(r"\bsuper::server::"),
    re.compile(r"\bsuper::super::handlers::"),
    re.compile(r"\bsuper::super::features::"),
    re.compile(r"\bsuper::super::server::"),
]

# Header for a grouped import we care about:
# `crate::channels::web::{`, `super::{`, or `super::super::{`. The full
# span of the import is found by depth-tracking the matching `}` in
# `_find_grouped_violations`, which lets us scan imports that contain
# *nested* brace groups (e.g. `web::{ platform::{state}, handlers::... }`)
# — a case a simple `[^{}]*?` regex cannot handle.
GROUPED_IMPORT_HEADER = re.compile(
    r"\b(?:crate::channels::web|super(?:::super)?)::\{"
)

# Segments that are back-edges from non-router platform modules when they
# appear *anywhere inside* a matched grouped-import body.
GROUPED_FORBIDDEN_SEGMENT = re.compile(r"\b(handlers|features|server)::")

# Narrow allowlist of pre-existing back-edges, each tied to a specific
# migration target. Entries are `(platform_file, forbidden_path_prefix)`;
# any line in that file whose matched text starts with that prefix is
# treated as a known pre-existing violation and does not fail the check.
# New entries may only be added with explicit reviewer sign-off documenting
# the migration PR — the intent is for this list to shrink to zero, not
# to grow.
#
# Empty as of ironclaw#2599 stage 4b: the `platform/static_files.rs` widget
# helpers and all seven `ws.rs` → `server.rs` shim references were
# relocated into `platform/` proper. The mechanism stays in place so a
# future migration step can reintroduce narrowly-scoped entries without
# re-adding the surrounding infrastructure.
ALLOWLIST: set[tuple[str, str]] = set()


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


_TEST_MODULE_START = re.compile(
    # `#[cfg(test)]` (optionally with whitespace/newlines) then an optional
    # `pub `, then `mod IDENT {`, *or* a bare `mod tests {`.
    r"(?:#\[cfg\(test\)\]\s*(?:pub\s+)?mod\s+\w+|\bmod\s+tests)\s*\{"
)


def _blank_test_modules(sanitized: str) -> str:
    """Blank the body of every `#[cfg(test)] mod ... { ... }` and
    `mod tests { ... }` block. Line numbers and brace structure outside
    the blanked body are preserved so subsequent scans still report
    accurate locations.
    """
    out = list(sanitized)
    n = len(sanitized)
    last_end = 0
    for m in _TEST_MODULE_START.finditer(sanitized):
        if m.start() < last_end:
            # Nested inside an already-blanked test module.
            continue
        open_pos = m.end() - 1  # position of the opening `{`
        depth = 1
        j = open_pos + 1
        while j < n and depth > 0:
            ch = sanitized[j]
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
            j += 1
        # `j` now points one past the matching `}` (or EOF on malformed
        # input). Blank everything strictly inside the braces so the
        # braces themselves stay visible to any outer tooling.
        for k in range(open_pos + 1, max(open_pos + 1, j - 1)):
            if out[k] != "\n" and out[k] != "\t":
                out[k] = " "
        last_end = j
    return "".join(out)


def _find_violations(path: pathlib.Path) -> list[Violation]:
    text = path.read_text(encoding="utf-8", errors="replace")
    sanitized = _blank_test_modules(_strip_comments_and_strings(text))
    original_lines = text.splitlines()
    sanitized_lines = sanitized.splitlines()
    violations: list[Violation] = []
    flagged_lines: set[int] = set()

    def _original_at(line_number: int) -> str:
        idx = line_number - 1
        if 0 <= idx < len(original_lines):
            return original_lines[idx]
        return ""

    # Per-line scan for same-line forbidden paths.
    for idx, line in enumerate(sanitized_lines, start=1):
        for pattern in FORBIDDEN_PATTERNS:
            if pattern.search(line):
                original = _original_at(idx)
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
                flagged_lines.add(idx)
                break  # one violation per line is enough

    # Grouped-import scan for forbidden paths split across a
    # multi-line `use crate::channels::web::{ ... }` statement. Uses a
    # depth-tracking walk over the brace body so nested groups
    # (`web::{ platform::{state}, handlers::... }`) cannot hide a
    # back-edge by sitting after an inner `{` — see the Copilot review
    # on PR #2647.
    n = len(sanitized)
    for header in GROUPED_IMPORT_HEADER.finditer(sanitized):
        open_pos = header.end() - 1  # position of the header's `{`
        depth = 1
        j = open_pos + 1
        while j < n and depth > 0:
            ch = sanitized[j]
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
            j += 1
        body_end = j - 1 if depth == 0 else n  # exclude the closing `}`
        body = sanitized[open_pos + 1 : body_end]
        for seg in GROUPED_FORBIDDEN_SEGMENT.finditer(body):
            abs_pos = open_pos + 1 + seg.start()
            line_number = sanitized.count("\n", 0, abs_pos) + 1
            if line_number in flagged_lines:
                continue
            original = _original_at(line_number)
            if _is_allowlisted(path, original):
                continue
            violations.append(
                Violation(
                    path=path,
                    line_number=line_number,
                    line=original.rstrip(),
                    pattern=f"grouped-use::{seg.group(1)}",
                )
            )
            flagged_lines.add(line_number)

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

    def test_detects_grouped_crate_web_import(self) -> None:
        # Regression for serrrfirat's review on PR #2647: a grouped
        # `use crate::channels::web::{ handlers::... }` escaped the
        # per-line scan because the forbidden segment lands on a
        # continuation line.
        src = (
            "use crate::channels::web::{\n"
            "    handlers::auth::login_handler,\n"
            "    platform::state::GatewayState,\n"
            "};\n"
        )
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(len(violations), 1)
            # Reported at the continuation line where `handlers::` appears.
            self.assertEqual(violations[0].line_number, 2)
        finally:
            p.unlink()

    def test_detects_grouped_super_import(self) -> None:
        src = "use super::{handlers::auth::login_handler, state::GatewayState};\n"
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(len(violations), 1)
        finally:
            p.unlink()

    def test_detects_nested_brace_grouped_import(self) -> None:
        # Regression for the Copilot review on PR #2647: the original
        # `[^{}]*?` regex stopped at the inner `{` and missed the
        # forbidden segment that follows it.
        src = (
            "use crate::channels::web::{\n"
            "    platform::{state::GatewayState, auth::middleware},\n"
            "    handlers::auth::login_handler,\n"
            "};\n"
        )
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(len(violations), 1)
            # Reported at the line where `handlers::` appears, not at the
            # line with the inner `{`.
            self.assertEqual(violations[0].line_number, 3)
        finally:
            p.unlink()

    def test_detects_server_shim_back_edge(self) -> None:
        # Regression for serrrfirat's review on PR #2647: the `server.rs`
        # compatibility shim still creates a platform → feature dependency,
        # so routing through it must be rejected from non-router platform
        # modules.
        src = "use crate::channels::web::server::chat_send_handler;\n"
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(len(violations), 1)
        finally:
            p.unlink()

    def test_skips_cfg_test_module(self) -> None:
        # Regression for the docstring/impl mismatch called out by both
        # reviewers on PR #2647: `#[cfg(test)] mod tests { ... }` blocks
        # must be exempt so caller-level regression tests in platform
        # files are free to import handler/feature modules.
        src = (
            "use crate::channels::web::platform::state::GatewayState;\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    use crate::channels::web::handlers::auth::login_handler;\n"
            "}\n"
        )
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(violations, [])
        finally:
            p.unlink()

    def test_skips_mod_tests_without_cfg(self) -> None:
        src = (
            "mod tests {\n"
            "    use crate::channels::web::features::oauth::foo;\n"
            "}\n"
        )
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(violations, [])
        finally:
            p.unlink()

    def test_still_flags_production_code_after_test_module(self) -> None:
        # The test-module skip must not blanket-ignore the rest of the file.
        src = (
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    use crate::channels::web::handlers::auth::login_handler;\n"
            "}\n"
            "\n"
            "use crate::channels::web::features::oauth::callback_handler;\n"
        )
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
            f.write(src)
            p = pathlib.Path(f.name)
        try:
            violations = _find_violations(p)
            self.assertEqual(len(violations), 1)
            self.assertEqual(violations[0].line_number, 6)
        finally:
            p.unlink()


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "test":
        unittest.main(argv=sys.argv[:1])
    sys.exit(_main())
