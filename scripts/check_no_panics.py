#!/usr/bin/env python3
# Requires Python 3.10+ for PEP 604 union syntax such as `int | None`.

import argparse
import collections
import contextlib
import io
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest
from dataclasses import dataclass
from unittest import mock


PANIC_PATTERN = re.compile(
    r"\.(?:unwrap|expect)\("
    r"|(?<!_)assert(?:_eq|_ne)?!"
    r"|(?<![A-Za-z0-9_])(?:panic|unreachable|unimplemented|todo)!"
)
SAFETY_RATIONALE_PATTERN = re.compile(r"^\s*safety:\s*\S", re.IGNORECASE)
TEST_ATTR_PATTERN = re.compile(
    r"^\s*#\s*\[\s*(?:"
    r"test"
    r"|tokio::test(?:\s*\([^]]*\))?"
    r"|rstest(?:\s*\([^]]*\))?"
    r"|test_case(?:\s*\([^]]*\))?"
    r")\s*\]"
)
CFG_ATTR_PATTERN = re.compile(r"^\s*#\s*\[\s*cfg\s*\((.*)\)\s*\]\s*$")
ITEM_PATTERN = re.compile(
    r"^\s*"
    r"(?:(?:pub(?:\([^)]*\))?|crate)\s+)?"
    r"(?:(?:async|unsafe|const)\s+)*"
    r"(fn|mod|struct|enum|trait|union|impl)\b"
    r"(?:\s+([A-Za-z_][A-Za-z0-9_]*))?"
)
CONST_STATIC_ITEM_PATTERN = re.compile(
    r"^\s*"
    r"(?:(?:pub(?:\([^)]*\))?|crate)\s+)?"
    r"(const|static)\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\b"
)
OUT_OF_LINE_MOD_PATTERN = re.compile(
    r"^\s*"
    r"(?:(?:pub(?:\([^)]*\))?|crate)\s+)?"
    r"mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
)
INLINE_MOD_PATTERN = re.compile(
    r"^\s*"
    r"(?:(?:pub(?:\([^)]*\))?|crate)\s+)?"
    r"mod\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)
PATH_ATTR_PATTERN = re.compile(
    r'^\s*#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]'
)

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
REBORN_BASELINE_PATH = REPO_ROOT / "scripts" / "no_panics_reborn_baseline.txt"
# The scope anchor is the shipping package's *name*, not its directory. The
# package is `ironclaw` whether its manifest sits at crates/ironclaw_reborn_cli/
# (today) or at crates/app/ironclaw_cli/ (the target layout, which keeps the
# package name and only renames the directory — PROPOSAL §5.1). Keying on the
# path meant a move turned the whole gate into a crash or, worse, a shrunken
# scan. See docs/reborn/target-architecture/CHECKLIST.md WS10.
SHIPPING_PACKAGE_NAME = "ironclaw"


@dataclass
class LexerState:
    block_comment_depth: int = 0
    in_string: bool = False
    string_escape: bool = False
    in_char: bool = False
    char_escape: bool = False
    raw_string_hashes: int | None = None


@dataclass(frozen=True)
class SourceScan:
    path: pathlib.Path
    lines: tuple[str, ...]
    code: tuple[str, ...]
    line_comments: tuple[str | None, ...]
    test_contexts: tuple[bool, ...]
    item_contexts: tuple[str, ...]
    module_directories: tuple[pathlib.Path, ...]


class RustScanner:
    """Read and lex each Rust source once for discovery and violation checks."""

    def __init__(self) -> None:
        self._cache: dict[pathlib.Path, SourceScan] = {}

    def scan(self, path: pathlib.Path) -> SourceScan:
        resolved = path.resolve()
        cached = self._cache.get(resolved)
        if cached is not None:
            return cached

        lines = tuple(resolved.read_text(encoding="utf-8").splitlines())
        code, comments = lex_lines(lines)
        scan = SourceScan(
            path=resolved,
            lines=lines,
            code=code,
            line_comments=comments,
            test_contexts=tuple(line_test_contexts_from_code(code, lines)),
            item_contexts=tuple(line_item_contexts(code)),
            module_directories=tuple(line_module_directories(resolved, code)),
        )
        self._cache[resolved] = scan
        return scan


def run_git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout


def lex_line(line: str, state: LexerState) -> tuple[str, str | None]:
    chars = list(line)
    out = [" "] * len(chars)
    line_comment: str | None = None
    i = 0

    while i < len(chars):
        ch = chars[i]
        nxt = chars[i + 1] if i + 1 < len(chars) else ""

        if state.block_comment_depth:
            if ch == "/" and nxt == "*":
                state.block_comment_depth += 1
                i += 2
                continue
            if ch == "*" and nxt == "/":
                state.block_comment_depth -= 1
                i += 2
                continue
            i += 1
            continue

        if state.raw_string_hashes is not None:
            if ch == '"':
                hashes = 0
                j = i + 1
                while j < len(chars) and chars[j] == "#":
                    hashes += 1
                    j += 1
                if hashes == state.raw_string_hashes:
                    state.raw_string_hashes = None
                    i = j
                    continue
            i += 1
            continue

        if state.in_string:
            if state.string_escape:
                state.string_escape = False
            elif ch == "\\":
                state.string_escape = True
            elif ch == '"':
                state.in_string = False
            i += 1
            continue

        if state.in_char:
            if state.char_escape:
                state.char_escape = False
            elif ch == "\\":
                state.char_escape = True
            elif ch == "'":
                state.in_char = False
            i += 1
            continue

        if ch == "/" and nxt == "/":
            line_comment = line[i + 2 :]
            break
        if ch == "/" and nxt == "*":
            state.block_comment_depth += 1
            i += 2
            continue
        if ch == "r":
            j = i + 1
            while j < len(chars) and chars[j] == "#":
                j += 1
            if j < len(chars) and chars[j] == '"':
                state.raw_string_hashes = j - i - 1
                i = j + 1
                continue
        if ch == '"':
            state.in_string = True
            i += 1
            continue
        if ch == "'":
            # Distinguish char literals ('x', '\n') from lifetime annotations
            # ('a, 'static).  Lifetimes are an apostrophe followed by an ASCII
            # letter or underscore and then a non-apostrophe (identifiers, not
            # closing-quote).  Misclassifying a lifetime as a char literal
            # blanks the rest of the line, hiding braces and causing the
            # brace-depth tracker to desync across the whole file.
            if nxt and (nxt.isalpha() or nxt == "_"):
                # Peek past the identifier to see if it's 'x' (char) or 'ident (lifetime).
                j = i + 2
                while j < len(chars) and (chars[j].isalnum() or chars[j] == "_"):
                    j += 1
                if j < len(chars) and chars[j] == "'":
                    # Closing quote found -> char literal like 'a' or 'ab' (invalid but safe to skip).
                    state.in_char = True
                else:
                    # No closing quote -> lifetime annotation; skip the apostrophe.
                    out[i] = " "
            else:
                state.in_char = True
            i += 1
            continue

        out[i] = ch
        i += 1

    # Rust char literals cannot span lines; reset if still open at EOL.
    if state.in_char:
        state.in_char = False
        state.char_escape = False

    return "".join(out), line_comment


def sanitize_line(line: str, state: LexerState) -> str:
    return lex_line(line, state)[0]


def lex_lines(
    lines: tuple[str, ...] | list[str],
) -> tuple[tuple[str, ...], tuple[str | None, ...]]:
    lexer = LexerState()
    code: list[str] = []
    comments: list[str | None] = []
    for line in lines:
        sanitized, comment = lex_line(line, lexer)
        code.append(sanitized)
        comments.append(comment)
    return tuple(code), tuple(comments)


class CfgParser:
    """Parse enough cfg syntax to identify test-only Reborn source."""

    TOKEN_PATTERN = re.compile(
        r'\s*(?:([A-Za-z_][A-Za-z0-9_]*)|("(?:\\.|[^"])*")|([(),=]))'
    )

    def __init__(self, source: str) -> None:
        self.tokens: list[str] = []
        position = 0
        while position < len(source):
            match = self.TOKEN_PATTERN.match(source, position)
            if match is None:
                self.tokens = []
                break
            self.tokens.append(next(group for group in match.groups() if group))
            position = match.end()
        self.position = 0

    def parse(self) -> tuple | None:
        expression = self._expression()
        if expression is None or self.position != len(self.tokens):
            return None
        return expression

    def _expression(self) -> tuple | None:
        if self.position >= len(self.tokens):
            return None
        name = self.tokens[self.position]
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
            return None
        self.position += 1

        if self._consume("="):
            if self.position >= len(self.tokens):
                return None
            value = self.tokens[self.position]
            self.position += 1
            if name == "feature" and value == '"test-support"':
                return ("test-support",)
            return ("unknown",)

        if not self._consume("("):
            return ("test",) if name == "test" else ("unknown",)

        arguments: list[tuple] = []
        if not self._peek(")"):
            while True:
                argument = self._expression()
                if argument is None:
                    return None
                arguments.append(argument)
                if not self._consume(","):
                    break
        if not self._consume(")"):
            return None
        if name in {"all", "any", "not"}:
            if name == "not" and len(arguments) != 1:
                return None
            return (name, tuple(arguments))
        return ("unknown",)

    def _peek(self, token: str) -> bool:
        return self.position < len(self.tokens) and self.tokens[self.position] == token

    def _consume(self, token: str) -> bool:
        if not self._peek(token):
            return False
        self.position += 1
        return True


def cfg_possible_values(expression: tuple) -> set[bool]:
    kind = expression[0]
    if kind == "test":
        return {False}
    if kind == "test-support":
        return {False}
    if kind == "unknown":
        return {False, True}

    arguments = expression[1]
    values = [cfg_possible_values(argument) for argument in arguments]
    if kind == "not":
        return {not value for value in values[0]}
    if kind == "all":
        possible: set[bool] = set()
        if all(True in value for value in values):
            possible.add(True)
        if any(False in value for value in values):
            possible.add(False)
        return possible
    if kind == "any":
        possible = set()
        if any(True in value for value in values):
            possible.add(True)
        if all(False in value for value in values):
            possible.add(False)
        return possible
    return {False, True}


def attribute_is_test_only(line: str) -> bool:
    if TEST_ATTR_PATTERN.match(line):
        return True
    match = CFG_ATTR_PATTERN.match(line)
    if match is None:
        return False
    expression = CfgParser(match.group(1)).parse()
    return expression is not None and True not in cfg_possible_values(expression)


def item_kind_and_name(line: str) -> tuple[str, str | None] | None:
    match = ITEM_PATTERN.match(line)
    if match is not None:
        return match.groups()
    const_or_static = CONST_STATIC_ITEM_PATTERN.match(line)
    if const_or_static is not None:
        return const_or_static.groups()
    return None


def is_test_item(line: str, pending_test_attr: bool) -> tuple[bool, bool]:
    item = item_kind_and_name(line)
    if item is None:
        return False, False

    kind, name = item
    named_tests_module = kind == "mod" and name == "tests"
    return True, pending_test_attr or named_tests_module


def line_test_contexts_from_code(
    code_lines: tuple[str, ...],
    raw_lines: tuple[str, ...] | list[str],
) -> list[bool]:
    contexts = [False] * len(code_lines)
    block_stack: list[bool] = []
    pending_test_attr = False
    pending_block_context: bool | None = None

    for idx, code in enumerate(code_lines):
        stripped = code.strip()
        current_context = block_stack[-1] if block_stack else False

        if stripped.startswith("#[") and attribute_is_test_only(raw_lines[idx].strip()):
            pending_test_attr = True

        item_found, item_is_test = is_test_item(code, pending_test_attr)
        if item_found:
            pending_block_context = item_is_test or current_context
            pending_test_attr = False
        elif stripped and not stripped.startswith("#[") and pending_test_attr:
            pending_test_attr = False

        contexts[idx] = current_context or bool(pending_block_context)

        for ch in code:
            if ch == "{":
                if pending_block_context is not None:
                    block_stack.append(pending_block_context)
                    pending_block_context = None
                else:
                    block_stack.append(block_stack[-1] if block_stack else False)
            elif ch == "}" and block_stack:
                block_stack.pop()

        if stripped.endswith(";"):
            pending_block_context = None

    return contexts


def has_cfg_test_module_declaration(
    path: str, repository_root: pathlib.Path = pathlib.Path(".")
) -> bool:
    """Return whether ``path`` is included by an exact ``#[cfg(test)] mod``.

    Filename suffixes are only a convention. Some ``*_tests.rs`` modules are
    also compiled by ``feature = "test-support"``, so exempting them by name
    alone can hide panics from non-test builds.
    """
    posix_path = pathlib.PurePosixPath(path)
    module_name = posix_path.stem
    parent = pathlib.Path(*posix_path.parent.parts)
    candidates = [
        repository_root / parent / "mod.rs",
        repository_root / parent.with_suffix(".rs"),
    ]
    source_parent = repository_root / parent
    if source_parent.is_dir():
        candidates.extend(source_parent.glob("*.rs"))
    if posix_path.parent.name == "src":
        candidates.extend(
            [
                repository_root / parent / "lib.rs",
                repository_root / parent / "main.rs",
            ]
        )
    declaration = re.compile(
        rf"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*"
        rf"(?:pub(?:\([^)]*\))?\s+)?mod\s+{re.escape(module_name)}\s*;"
    )
    path_declaration = re.compile(
        rf"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*"
        rf"#\s*\[\s*path\s*=\s*[\"']{re.escape(posix_path.name)}[\"']\s*\]\s*"
        r"(?:pub(?:\([^)]*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\s*;"
    )
    for candidate in candidates:
        try:
            source = candidate.read_text(encoding="utf-8")
        except (FileNotFoundError, OSError, UnicodeError):
            continue
        if declaration.search(source) or path_declaration.search(source):
            return True
    return False


def line_test_contexts(lines: list[str]) -> list[bool]:
    code, _comments = lex_lines(lines)
    return line_test_contexts_from_code(code, lines)


def root_module_directory(source: pathlib.Path) -> pathlib.Path:
    if source.name in {"lib.rs", "main.rs", "mod.rs"}:
        return source.parent
    return source.parent / source.stem


def line_module_directories(
    source: pathlib.Path,
    code_lines: tuple[str, ...],
) -> list[pathlib.Path]:
    root = root_module_directory(source)
    directories: list[pathlib.Path] = []
    block_stack: list[pathlib.Path] = []
    pending_inline: pathlib.Path | None = None

    for code in code_lines:
        current = block_stack[-1] if block_stack else root
        directories.append(current)
        module = INLINE_MOD_PATTERN.match(code)
        if module is not None and OUT_OF_LINE_MOD_PATTERN.match(code) is None:
            pending_inline = current / module.group(1)

        for char in code:
            if char == "{":
                block_stack.append(pending_inline or current)
                pending_inline = None
            elif char == "}" and block_stack:
                block_stack.pop()
                current = block_stack[-1] if block_stack else root

        stripped = code.strip()
        if pending_inline is not None and stripped.endswith(";"):
            pending_inline = None

    return directories


def line_item_contexts(code_lines: tuple[str, ...]) -> list[str]:
    contexts: list[str] = []
    block_stack: list[str | None] = []
    pending_item: str | None = None

    for code in code_lines:
        item = item_kind_and_name(code)
        if item is not None:
            kind, name = item
            pending_item = (
                f"{kind} {name}"
                if name is not None
                else normalized_source_line(code.split("{", 1)[0])
            )

        active = [label for label in block_stack if label is not None]
        if pending_item is not None:
            active.append(pending_item)
        contexts.append("::".join(active) if active else "<module>")

        for char in code:
            if char == "{":
                block_stack.append(pending_item)
                pending_item = None
            elif char == "}" and block_stack:
                block_stack.pop()

        if code.strip().endswith(";"):
            pending_item = None

    return contexts


def is_test_only_path(path: str) -> bool:
    """Return True for files that live in Rust test-only locations.

    Files under ``src/**/tests/*.rs`` and ``src/**/tests.rs`` are Rust test
    sub-modules, typically included behind ``#[cfg(test)]`` and never compiled
    in production. Top-level ``tests/*.rs`` integration test files are already
    outside ``src/`` / ``crates/`` and therefore never checked.

    ``src/**/test_support.rs`` is the repo-wide convention for a
    ``#[cfg(feature = "test-support")] pub mod test_support;`` module
    (used by ``ironclaw_agent_loop``, ``ironclaw_host_runtime``,
    ``ironclaw_product``, ``ironclaw_reborn_composition``). The
    ``test-support`` feature is enabled
    only via ``[dev-dependencies]``, so these modules ship zero bytes in
    production binaries — the same "never compiled in production" rationale that
    exempts ``tests.rs``. Their fixtures legitimately ``.unwrap()`` constant
    literals, so they are exempt whether the module is a single
    ``src/test_support.rs`` file or a ``src/test_support/**`` directory module
    (so growing test-support coverage needs no further changes here). **The
    ``src/`` path component is required**: a ``bin/test_support.rs`` or any
    ``test_support`` outside ``src/`` would be compiled into production binaries
    and must NOT be exempt. A file merely *containing* the substring, e.g.
    ``my_test_support.rs``, is NOT exempt — the match is on the exact filename
    or an exact path component.

    NOTE: the scanner only ever looks at ``src/`` and ``crates/`` (see
    ``changed_rust_files``). Top-level ``tests/**`` integration tests and their
    support trees are never scanned at all, so they need no exemption here.
    """
    posix_path = pathlib.PurePosixPath(path)
    parts = posix_path.parts
    if (
        "tests" in parts
        or (
            (
                posix_path.name == "tests.rs"
                or posix_path.name.endswith("_test.rs")
                or posix_path.name.endswith("_tests.rs")
            )
            and has_cfg_test_module_declaration(path)
        )
    ):
        return True
    # Exempt only the canonical feature-gated module root: `.../src/test_support.rs`
    # or `.../src/test_support/**`. The component immediately after `src` must be
    # `test_support` — `src/bin/test_support.rs` (compiled into a binary) and a
    # nested `src/foo/test_support.rs` are NOT exempt.
    try:
        src_idx = parts.index("src")
    except ValueError:
        return False
    suffix = parts[src_idx + 1 :]
    return bool(suffix) and (suffix[0] == "test_support" or suffix == ("test_support.rs",))


def run_cargo_metadata() -> dict:
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--all-features",
            "--locked",
        ],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def shipping_reborn_source_roots(metadata: dict) -> list[pathlib.Path]:
    """Return production target roots in the shipping Reborn dependency closure.

    The shipping binary's normal-dependency graph is the owned scope. This is
    deliberately derived from Cargo instead of a hand-maintained crate list so
    a newly wired runtime/persistence/transport crate enters the gate
    automatically. External packages and non-production targets (tests,
    examples, benches, build scripts) are excluded.

    Workspace membership plus "lives somewhere under ``crates/``" is the
    ownership test, at any depth. The previous rule — manifest exactly one
    level under ``crates/`` — silently dropped every crate the moment the
    target-architecture restructure nests them in family directories
    (``crates/<family>/ironclaw_*``): fewer files scanned, panics in the moved
    crates unreviewed, and the gate still green for any crate that happened to
    carry no baseline entries. See
    docs/reborn/target-architecture/CHECKLIST.md WS10.

    Fail-closed: every reachable workspace member must resolve to a crate
    directory *and* contribute at least one production target root. A member
    that resolves to neither is reported rather than skipped — "the discovery
    found nothing here" is never allowed to look like "there was nothing here".
    """

    packages = {package["id"]: package for package in metadata["packages"]}
    workspace_members = set(metadata.get("workspace_members", ()))
    shipping_ids = [
        package_id
        for package_id, package in packages.items()
        if package["name"] == SHIPPING_PACKAGE_NAME and package_id in workspace_members
    ]
    if len(shipping_ids) != 1:
        raise RuntimeError(
            f"expected exactly one workspace package named {SHIPPING_PACKAGE_NAME!r}, "
            f"found {len(shipping_ids)}"
        )

    resolve = metadata.get("resolve")
    if not resolve:
        raise RuntimeError("cargo metadata did not return a dependency graph")
    nodes = {node["id"]: node for node in resolve["nodes"]}
    reachable: set[str] = set()
    pending = list(shipping_ids)
    while pending:
        package_id = pending.pop()
        if package_id in reachable:
            continue
        reachable.add(package_id)
        node = nodes.get(package_id)
        if node is None:
            continue
        for dependency in node["deps"]:
            if any(kind.get("kind") is None for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])

    roots: set[pathlib.Path] = set()
    crates_root = (REPO_ROOT / "crates").resolve()
    scanned: list[str] = []
    outside_crates: list[str] = []
    without_roots: list[str] = []
    for package_id in sorted(reachable):
        package = packages[package_id]
        if package_id not in workspace_members:
            continue
        manifest = pathlib.Path(package["manifest_path"]).resolve()
        if crates_root not in manifest.parents:
            outside_crates.append(f"{package['name']} ({manifest})")
            continue
        package_roots = {
            source
            for target in package["targets"]
            if {"lib", "bin"} & set(target["kind"])
            for source in (pathlib.Path(target["src_path"]).resolve(),)
            if source.suffix == ".rs"
        }
        if not package_roots:
            without_roots.append(package["name"])
            continue
        roots |= package_roots
        scanned.append(package["name"])

    if outside_crates:
        raise RuntimeError(
            "these workspace crates ship in the binary but their manifests are not "
            f"under {crates_root}: {', '.join(outside_crates)}. The panic gate scans "
            "the crate tree; a shipping crate outside it would be skipped silently. "
            "Move it under crates/ or widen this scope deliberately."
        )
    if without_roots:
        raise RuntimeError(
            "these shipping workspace crates declare no lib/bin target for the panic "
            f"gate to scan: {', '.join(without_roots)}. A crate that contributes no "
            "source root contributes no coverage either."
        )
    # `scanned` is non-empty by construction: the shipping package is a
    # workspace member, and it either resolved under crates/ with a source root
    # or one of the two errors above already fired.
    return sorted(roots)


def default_module_candidates(
    module_dir: pathlib.Path,
    name: str,
) -> tuple[pathlib.Path, ...]:
    return module_dir / f"{name}.rs", module_dir / name / "mod.rs"


def module_edges(
    scan: SourceScan,
    inherited_test_only: bool,
) -> list[tuple[pathlib.Path, bool]]:
    pending_path: str | None = None
    pending_path_directory: pathlib.Path | None = None
    edges: list[tuple[pathlib.Path, bool]] = []

    for raw, code, local_test_context, module_dir in zip(
        scan.lines,
        scan.code,
        scan.test_contexts,
        scan.module_directories,
    ):
        path_attr = PATH_ATTR_PATTERN.match(raw)
        if path_attr and code.lstrip().startswith("#["):
            pending_path = path_attr.group(1)
            pending_path_directory = module_dir
            continue

        module = OUT_OF_LINE_MOD_PATTERN.match(code)
        if module:
            if pending_path is not None:
                path_directory = pending_path_directory or module_dir
                if path_directory == root_module_directory(scan.path):
                    path_directory = scan.path.parent
                candidates = (path_directory / pending_path,)
            else:
                candidates = default_module_candidates(module_dir, module.group(1))
            for candidate in candidates:
                if candidate.is_file():
                    edges.append(
                        (
                            candidate.resolve(),
                            inherited_test_only
                            or local_test_context
                            or is_test_only_path(candidate.as_posix()),
                        )
                    )
                    break
            else:
                raise RuntimeError(
                    f"{scan.path}: could not resolve `mod {module.group(1)};`; "
                    f"tried {[str(candidate) for candidate in candidates]}"
                )
            pending_path = None
            pending_path_directory = None
            continue

        stripped = code.strip()
        if stripped and not stripped.startswith("#["):
            pending_path = None
            pending_path_directory = None

    return edges


def discover_reachable_rust_files(
    roots: list[pathlib.Path],
    scanner: RustScanner | None = None,
) -> tuple[set[pathlib.Path], set[pathlib.Path]]:
    """Follow Rust module edges, retaining whether each file is test-only."""

    scanner = scanner or RustScanner()
    states: dict[pathlib.Path, bool] = {}
    pending: list[tuple[pathlib.Path, bool]] = [
        (path.resolve(), False) for path in roots
    ]
    while pending:
        source, test_only = pending.pop()
        previous = states.get(source)
        if previous is False or previous == test_only:
            continue
        states[source] = test_only
        pending.extend(module_edges(scanner.scan(source), test_only))

    production = {path for path, test_only in states.items() if not test_only}
    tests = {path for path, test_only in states.items() if test_only}
    return production, tests


def repository_relative(path: pathlib.Path) -> str:
    return path.resolve().relative_to(REPO_ROOT).as_posix()


def normalized_source_line(line: str) -> str:
    return " ".join(line.strip().split())


def invocation_source(
    scan: SourceScan,
    start_line: int,
    start_column: int,
    match_end: int,
) -> str:
    open_line = start_line
    open_column = match_end
    if match_end > 0 and scan.code[start_line][match_end - 1] == "(":
        open_column = match_end - 1
    while open_line < len(scan.code):
        found = scan.code[open_line].find("(", open_column)
        if found >= 0:
            open_column = found
            break
        open_line += 1
        open_column = 0
    else:
        return scan.lines[start_line][start_column:]

    depth = 0
    end_line = open_line
    end_column = len(scan.lines[open_line]) - 1
    found_end = False
    for line_index in range(open_line, len(scan.code)):
        column = open_column if line_index == open_line else 0
        for column_index in range(column, len(scan.code[line_index])):
            char = scan.code[line_index][column_index]
            if char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
                if depth == 0:
                    end_line = line_index
                    end_column = column_index
                    found_end = True
                    break
        if found_end:
            break

    pieces: list[str] = []
    for line_index in range(start_line, end_line + 1):
        left = start_column if line_index == start_line else 0
        right = end_column + 1 if line_index == end_line else len(scan.lines[line_index])
        pieces.append(scan.lines[line_index][left:right])
    return "\n".join(pieces)


def violation_fingerprint(path: str, context: str, invocation: str) -> tuple[str, str]:
    return path, f"{context} :: {normalized_source_line(invocation)}"


def scan_violations(
    scan: SourceScan,
    included_lines: set[int] | None = None,
) -> list[tuple[str, int, str]]:
    path = repository_relative(scan.path)
    violations: list[tuple[str, int, str]] = []
    for line_index, code in enumerate(scan.code):
        line_no = line_index + 1
        if included_lines is not None and line_no not in included_lines:
            continue
        if scan.test_contexts[line_index]:
            continue
        for match in PANIC_PATTERN.finditer(code):
            first_code_column = len(scan.lines[line_index]) - len(
                scan.lines[line_index].lstrip()
            )
            invocation = invocation_source(
                scan,
                line_index,
                first_code_column,
                match.end(),
            )
            span_end = line_index + invocation.count("\n")
            if any(
                comment is not None and SAFETY_RATIONALE_PATTERN.search(comment)
                for comment in scan.line_comments[line_index : span_end + 1]
            ):
                continue
            fingerprint = violation_fingerprint(
                path,
                scan.item_contexts[line_index],
                invocation,
            )[1]
            violations.append((path, line_no, fingerprint))
    return violations


def collect_file_violations(
    paths: set[pathlib.Path],
    scanner: RustScanner | None = None,
) -> list[tuple[str, int, str]]:
    scanner = scanner or RustScanner()
    violations: list[tuple[str, int, str]] = []
    for path in sorted(paths):
        violations.extend(scan_violations(scanner.scan(path)))
    return violations


def load_reborn_baseline(
    path: pathlib.Path,
) -> collections.Counter[tuple[str, str]]:
    approved: collections.Counter[tuple[str, str]] = collections.Counter()
    for line_no, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        fields = raw.split("\t")
        if len(fields) != 3 or any(not field.strip() for field in fields):
            raise RuntimeError(
                f"{path}:{line_no}: expected non-empty "
                "path<TAB>fingerprint<TAB>reason"
            )
        approved[(fields[0], normalized_source_line(fields[1]))] += 1
    return approved


def compare_reborn_baseline(
    violations: list[tuple[str, int, str]],
    approved: collections.Counter[tuple[str, str]],
) -> tuple[
    list[tuple[str, int, str]],
    collections.Counter[tuple[str, str]],
]:
    remaining = approved.copy()
    new: list[tuple[str, int, str]] = []
    for path, line_no, fingerprint_source in violations:
        fingerprint = (path, fingerprint_source)
        if remaining[fingerprint]:
            remaining[fingerprint] -= 1
            if remaining[fingerprint] == 0:
                del remaining[fingerprint]
        else:
            new.append((path, line_no, fingerprint_source))
    return new, remaining


def changed_rust_files(base: str, head: str) -> list[pathlib.Path]:
    output = run_git("diff", "--name-only", "-M", f"{base}...{head}", "--", "src", "crates")
    files = []
    for line in output.splitlines():
        if line.endswith(".rs") and (line.startswith("src/") or line.startswith("crates/")):
            if not is_test_only_path(line):
                files.append(pathlib.Path(line))
    return files


def added_lines_by_file(base: str, head: str) -> dict[str, set[int]]:
    """Added line numbers per changed file, computed once, RENAME-AWARE.

    `-M` is what makes this correct for a move: without it, a relocated file
    has no pre-image on its new path, so every one of its lines reads as added
    and a pure `git mv` of a file carrying justified `unreachable!()` calls
    fails the delta scan wholesale. With `-M`, a 100%-similar rename yields a
    rename header and no `+` lines at all (measured on the WS2 package
    colocation: 1,754 spurious added lines for
    `memory-native/src/repo/filesystem.rs` before, 0 after), while a
    rename-with-edits still reports exactly the lines that actually changed.
    That is the property this scan wants — "did THIS change introduce a panic",
    not "did this change touch a line near one".

    One `git diff` for the whole range rather than one per file: pairing a
    rename needs both sides in the same invocation, which a per-file pathspec
    on the destination cannot provide.
    """

    diff = run_git(
        "diff", "--unified=0", "-M", f"{base}...{head}", "--", "src", "crates"
    )
    return parse_added_lines(diff)


def parse_added_lines(diff: str) -> dict[str, set[int]]:
    """Parse `git diff --unified=0` output into per-file added line numbers."""

    per_file: dict[str, set[int]] = {}
    current: set[int] | None = None
    current_line = 0

    for line in diff.splitlines():
        if line.startswith("+++ "):
            target = line[4:]
            if target == "/dev/null":
                current = None
            else:
                name = target[2:] if target.startswith("b/") else target
                current = per_file.setdefault(name, set())
            continue
        if line.startswith("--- ") or line.startswith("diff --git "):
            continue
        if current is None:
            continue
        if line.startswith("@@"):
            match = re.search(r"\+(\d+)(?:,(\d+))?", line)
            if not match:
                continue
            current_line = int(match.group(1))
            continue
        if line.startswith("+"):
            current.add(current_line)
            current_line += 1
        elif line.startswith("-"):
            continue
        else:
            current_line += 1

    return per_file


def collect_violations(base: str, head: str) -> list[tuple[str, int, str]]:
    violations: list[tuple[str, int, str]] = []
    scanner = RustScanner()

    added_by_file = added_lines_by_file(base, head)
    for path in changed_rust_files(base, head):
        if not path.exists():
            continue
        added_lines = added_by_file.get(path.as_posix(), set())
        if not added_lines:
            continue

        scan = scanner.scan(path)
        violations.extend(scan_violations(scan, added_lines))

    return violations


def report_reborn_baseline(
    production_count: int,
    violations: list[tuple[str, int, str]],
    approved: collections.Counter[tuple[str, str]],
) -> int:
    new, stale = compare_reborn_baseline(violations, approved)
    if not new and not stale:
        print(
            "OK: Reborn production panic baseline matches "
            f"({production_count} files, {len(violations)} reviewed invariant(s))."
        )
        return 0

    print("::error::Reborn production panic baseline changed.")
    if new:
        print("")
        print("New or changed panic-style calls:")
        for path, line_no, fingerprint in new[:20]:
            print(f"{path}:{line_no}: {fingerprint}")
        if len(new) > 20:
            print(f"... and {len(new) - 20} more")
    if stale:
        print("")
        print("Stale baseline entries (remove them to ratchet downward):")
        for (path, fingerprint), count in list(stale.items())[:20]:
            suffix = f" (x{count})" if count > 1 else ""
            print(f"{path}: {fingerprint}{suffix}")
        if len(stale) > 20:
            print(f"... and {len(stale) - 20} more")
    return 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=False, default="origin/staging")
    parser.add_argument("--head", required=False, default="HEAD")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--reborn-baseline",
        action="store_true",
        help=(
            "scan the complete production module graph in the shipping Reborn "
            "normal-dependency closure and compare it with the audited baseline"
        ),
    )
    args = parser.parse_args()

    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(CheckNoPanicsTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1

    if args.reborn_baseline:
        scanner = RustScanner()
        roots = shipping_reborn_source_roots(run_cargo_metadata())
        production, _tests = discover_reachable_rust_files(roots, scanner)
        violations = collect_file_violations(production, scanner)
        approved = load_reborn_baseline(REBORN_BASELINE_PATH)
        return report_reborn_baseline(len(production), violations, approved)

    violations = collect_violations(args.base, args.head)
    if not violations:
        print("OK: No panic-inducing calls in changed production code.")
        return 0

    print("::error::Found panic-style calls outside test-only Rust code.")
    print("Production code must use proper error handling instead of panicking.")
    print("Suppress false positives with an inline '// safety: <reason>' comment.")
    print("")
    for path, line_no, line in violations[:20]:
        print(f"{path}:{line_no}: {line}")
    print("")
    print(f"Total: {len(violations)} violation(s)")
    return 1


class CheckNoPanicsTests(unittest.TestCase):
    def test_cfg_test_module_marks_inner_lines(self) -> None:
        lines = [
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    assert!(true);\n",
            "}\n",
            "fn prod() {\n",
            "    value.expect(\"boom\");\n",
            "}\n",
        ]

        contexts = line_test_contexts(lines)

        self.assertTrue(contexts[1])
        self.assertTrue(contexts[2])
        self.assertFalse(contexts[4])
        self.assertFalse(contexts[5])

    def test_test_function_marks_body_only(self) -> None:
        lines = [
            "#[test]\n",
            "fn it_works(\n",
            ") {\n",
            "    assert_eq!(2 + 2, 4);\n",
            "}\n",
            "fn prod() {\n",
            "    assert!(ready);\n",
            "}\n",
        ]

        contexts = line_test_contexts(lines)

        self.assertTrue(contexts[1])
        self.assertTrue(contexts[2])
        self.assertTrue(contexts[3])
        self.assertFalse(contexts[5])
        self.assertFalse(contexts[6])

    def test_proc_macro_test_attrs_mark_body_only(self) -> None:
        attrs = [
            "tokio::test",
            'tokio::test(flavor = "multi_thread", worker_threads = 4)',
            "rstest",
            "test_case(1, 2)",
            "cfg(all(test, unix))",
        ]

        for attr in attrs:
            with self.subTest(attr=attr):
                lines = [
                    f"#[{attr}]\n",
                    "fn it_works() {\n",
                    '    value.expect("allowed in test");\n',
                    "}\n",
                    "fn prod() {\n",
                    '    value.expect("boom");\n',
                    "}\n",
                ]

                contexts = line_test_contexts(lines)

                self.assertTrue(contexts[1])
                self.assertTrue(contexts[2])
                self.assertFalse(contexts[4])
                self.assertFalse(contexts[5])

    def test_cfg_only_excludes_items_that_logically_require_test(self) -> None:
        cases = {
            "cfg(test)": True,
            "cfg(all(test, unix))": True,
            "cfg(not(not(test)))": True,
            "cfg(not(test))": False,
            "cfg(any(test, unix))": False,
            'cfg(all(feature = "test-support", unix))': True,
            'cfg(all(feature = "production-mode", unix))': False,
        }

        for attribute, expected in cases.items():
            with self.subTest(attribute=attribute):
                lines = [
                    f"#[{attribute}]\n",
                    "fn candidate() {\n",
                    '    value.expect("classified by cfg");\n',
                    "}\n",
                ]
                contexts = line_test_contexts(lines)
                self.assertEqual(contexts[2], expected)

    def test_test_only_path_detection(self) -> None:
        self.assertTrue(is_test_only_path("src/channels/web/tests/multi_tenant.rs"))
        self.assertTrue(is_test_only_path("crates/foo/src/tests/helpers.rs"))
        self.assertFalse(is_test_only_path("crates/foo/src/tests.rs"))
        self.assertFalse(is_test_only_path("crates/foo/src/nested/tests.rs"))
        self.assertFalse(is_test_only_path("crates/foo/src/auth_test.rs"))
        self.assertFalse(is_test_only_path("crates/foo/src/auth_tests.rs"))
        self.assertTrue(
            is_test_only_path(
                "crates/ironclaw_processes/src/journal_store/state_tests.rs"
            )
        )
        self.assertFalse(is_test_only_path("src/channels/web/mod.rs"))
        self.assertFalse(is_test_only_path("src/channels/web/test_helpers.rs"))
        self.assertFalse(is_test_only_path("crates/foo/src/lib.rs"))
        # `#[cfg(feature = "test-support")] pub mod test_support;` — dev-dep
        # gated, ships zero bytes in production. Exempt by exact filename only.
        self.assertTrue(
            is_test_only_path("crates/ironclaw_reborn_composition/src/test_support.rs")
        )
        # Directory-module form: src/test_support/**.rs is also exempt, so
        # growing a test_support module never needs another change here.
        self.assertTrue(is_test_only_path("crates/foo/src/test_support/oauth.rs"))
        self.assertTrue(is_test_only_path("crates/foo/src/test_support/mod.rs"))
        self.assertFalse(is_test_only_path("src/channels/web/my_test_support.rs"))
        self.assertFalse(is_test_only_path("crates/foo/src/test_supportish/x.rs"))
        # test_support outside src/ (e.g. bin/) is NOT exempt — it is compiled
        # into production binaries and must be checked for panics.
        self.assertFalse(is_test_only_path("crates/foo/bin/test_support.rs"))
        self.assertFalse(is_test_only_path("crates/foo/bin/test_support/x.rs"))
        # `src/bin/test_support*` IS compiled into a binary — must not be exempt.
        self.assertFalse(is_test_only_path("crates/foo/src/bin/test_support.rs"))
        self.assertFalse(is_test_only_path("crates/foo/src/bin/test_support/mod.rs"))
        # A nested test_support.rs (not the canonical `src/test_support.rs` root)
        # is not the blessed feature-gated module either.
        self.assertFalse(is_test_only_path("crates/foo/src/auth/test_support.rs"))
        self.assertFalse(
            is_test_only_path("crates/extensions/packages/memory-native/src/contract_tests.rs")
        )

    def test_lifetime_annotations_do_not_desync_braces(self) -> None:
        """Lifetime annotations ('a, 'static) must not be parsed as char literals.

        If they are, the sanitizer blanks the rest of the line — including any
        opening brace — and the brace-depth tracker desyncs.  This caused
        false positives in large test modules (e.g. server.rs).
        """
        lines = [
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    fn set_env_var(key: &'static str) -> Guard {\n",
            "        let original = std::env::var(key).ok();\n",
            "        Guard { key, original }\n",
            "    }\n",
            "    fn later_helper() {\n",
            '        value.expect("should be test context");\n',
            "    }\n",
            "}\n",
        ]

        contexts = line_test_contexts(lines)

        # All lines inside mod tests must be test context, even after
        # a function signature containing a lifetime annotation.
        self.assertTrue(contexts[2], "fn with 'static should be test context")
        self.assertTrue(contexts[7], "later helper should still be test context")

    def test_named_tests_module_marks_context(self) -> None:
        lines = [
            "mod tests {\n",
            "    fn helper() {\n",
            "        assert!(true);\n",
            "    }\n",
            "}\n",
        ]

        contexts = line_test_contexts(lines)

        self.assertTrue(all(contexts))

    def test_all_panic_style_macros_are_detected(self) -> None:
        sources = [
            'value.unwrap();',
            'value.expect("reason");',
            'assert!(ready);',
            'assert_eq!(left, right);',
            'assert_ne!(left, right);',
            'panic!("boom");',
            'unreachable!("invariant");',
            'unimplemented!("missing");',
            'todo!("later");',
        ]
        for source in sources:
            with self.subTest(source=source):
                lexer = LexerState()
                self.assertIsNotNone(PANIC_PATTERN.search(sanitize_line(source, lexer)))

        lexer = LexerState()
        self.assertIsNone(
            PANIC_PATTERN.search(sanitize_line("debug_assert!(ready);", lexer))
        )

    def test_safety_suppression_requires_a_reason(self) -> None:
        self.assertIsNone(SAFETY_RATIONALE_PATTERN.search(" safety:"))
        self.assertIsNone(SAFETY_RATIONALE_PATTERN.search(" safety:   "))
        self.assertIsNotNone(
            SAFETY_RATIONALE_PATTERN.search(
                " safety: fixed static literal is validated"
            )
        )

    def test_safety_rationale_must_be_an_actual_line_comment(self) -> None:
        with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
            root = pathlib.Path(directory)
            source = root / "lib.rs"
            source.write_text(
                'fn string_is_not_a_rationale() {\n'
                '    let note = "// safety: misleading string"; panic!("caught");\n'
                "}\n"
                "fn comment_is_a_rationale() {\n"
                '    panic!("suppressed"); // safety: fixed invariant\n'
                "}\n",
                encoding="utf-8",
            )

            violations = collect_file_violations({source})

            self.assertEqual(len(violations), 1)
            self.assertIn('panic!("caught")', violations[0][2])

    def test_multiline_safety_rationale_on_closing_line_suppresses(self) -> None:
        with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
            source = pathlib.Path(directory) / "lib.rs"
            source.write_text(
                "fn invariant() {\n"
                "    value.expect(\n"
                '        "validated invariant",\n'
                "    ); // safety: construction validates the invariant\n"
                "}\n",
                encoding="utf-8",
            )

            self.assertEqual(collect_file_violations({source}), [])

    def test_nested_inline_module_resolves_out_of_line_children(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source_root = root / "src"
            nested_root = source_root / "outer"
            nested_root.mkdir(parents=True)
            (source_root / "lib.rs").write_text(
                "mod outer {\n"
                "    mod nested;\n"
                "}\n",
                encoding="utf-8",
            )
            (nested_root / "nested.rs").write_text(
                "pub fn reachable() {}\n",
                encoding="utf-8",
            )

            production, _tests = discover_reachable_rust_files(
                [source_root / "lib.rs"]
            )

            self.assertIn((nested_root / "nested.rs").resolve(), production)

    def test_unresolved_default_module_edge_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = pathlib.Path(directory) / "lib.rs"
            source.write_text("mod missing;\n", encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, r"mod missing;"):
                discover_reachable_rust_files([source])

    def test_unresolved_nested_path_module_edge_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = pathlib.Path(directory) / "lib.rs"
            source.write_text(
                "mod outer {\n"
                '    #[path = "missing.rs"]\n'
                "    mod missing;\n"
                "}\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(RuntimeError, r"outer/missing.rs"):
                discover_reachable_rust_files([source])

    def test_out_of_line_test_modules_are_excluded_transitively(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source_root = root / "src"
            source_root.mkdir()
            (source_root / "lib.rs").write_text(
                "mod production;\n"
                '#[cfg(feature = "test-support")]\n'
                "mod test_support;\n"
                "#[cfg(test)]\n"
                "#[path = \"tests_out.rs\"]\n"
                "mod tests_out;\n",
                encoding="utf-8",
            )
            (source_root / "production.rs").write_text(
                "pub fn value() -> u8 { 1 }\n",
                encoding="utf-8",
            )
            (source_root / "test_support.rs").write_text(
                'fn fixture() { panic!("feature-gated test support"); }\n',
                encoding="utf-8",
            )
            (source_root / "tests_out.rs").write_text(
                "#[path = \"nested_fixture.rs\"]\n"
                "mod nested_fixture;\n",
                encoding="utf-8",
            )
            (source_root / "nested_fixture.rs").write_text(
                'fn fixture() { panic!("test-only"); }\n',
                encoding="utf-8",
            )

            production, tests = discover_reachable_rust_files(
                [source_root / "lib.rs"]
            )

            self.assertIn((source_root / "production.rs").resolve(), production)
            self.assertNotIn((source_root / "test_support.rs").resolve(), production)
            self.assertNotIn((source_root / "tests_out.rs").resolve(), production)
            self.assertNotIn(
                (source_root / "nested_fixture.rs").resolve(), production
            )
            self.assertIn((source_root / "test_support.rs").resolve(), tests)
            self.assertIn((source_root / "tests_out.rs").resolve(), tests)
            self.assertIn((source_root / "nested_fixture.rs").resolve(), tests)

    def test_production_reachability_reclassifies_transitive_children(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source_root = pathlib.Path(directory)
            (source_root / "lib.rs").write_text(
                '#[path = "shared.rs"]\n'
                "mod production_shared;\n"
                "#[cfg(test)]\n"
                '#[path = "shared.rs"]\n'
                "mod test_shared;\n",
                encoding="utf-8",
            )
            (source_root / "shared.rs").write_text(
                "mod child;\n",
                encoding="utf-8",
            )
            (source_root / "shared" / "child.rs").parent.mkdir()
            (source_root / "shared" / "child.rs").write_text(
                "pub fn reachable() {}\n",
                encoding="utf-8",
            )

            production, tests = discover_reachable_rust_files(
                [source_root / "lib.rs"]
            )

            shared = (source_root / "shared.rs").resolve()
            child = (source_root / "shared" / "child.rs").resolve()
            self.assertIn(shared, production)
            self.assertIn(child, production)
            self.assertNotIn(shared, tests)
            self.assertNotIn(child, tests)

    def test_pure_rename_contributes_no_added_lines(self) -> None:
        """A relocated file must not read as wholly new.

        Regression for the WS2 package colocation: without `-M` the diff has no
        pre-image on the destination path, so every line of a moved file counts
        as added and a `git mv` of a file carrying justified `unreachable!()`
        calls fails the delta scan wholesale. `git diff -M` emits a rename
        header and no `+` lines for a 100%-similar rename, which is what this
        parser must reflect.
        """

        rename_only = (
            "diff --git a/crates/old/src/lib.rs b/crates/new/src/lib.rs\n"
            "similarity index 100%\n"
            "rename from crates/old/src/lib.rs\n"
            "rename to crates/new/src/lib.rs\n"
        )
        self.assertEqual(parse_added_lines(rename_only), {})

    def test_rename_with_edits_reports_only_the_edited_lines(self) -> None:
        rename_with_edit = (
            "diff --git a/crates/old/src/lib.rs b/crates/new/src/lib.rs\n"
            "similarity index 98%\n"
            "rename from crates/old/src/lib.rs\n"
            "rename to crates/new/src/lib.rs\n"
            "--- a/crates/old/src/lib.rs\n"
            "+++ b/crates/new/src/lib.rs\n"
            "@@ -7,0 +8,2 @@\n"
            "+    let x = value.unwrap();\n"
            "+    let y = other.expect(\"boom\");\n"
        )
        self.assertEqual(
            parse_added_lines(rename_with_edit),
            {"crates/new/src/lib.rs": {8, 9}},
        )

    def test_created_file_reports_every_line_and_deletion_reports_none(self) -> None:
        created = (
            "diff --git a/crates/new/src/added.rs b/crates/new/src/added.rs\n"
            "--- /dev/null\n"
            "+++ b/crates/new/src/added.rs\n"
            "@@ -0,0 +1,2 @@\n"
            "+fn a() {}\n"
            "+fn b() {}\n"
            "diff --git a/crates/old/src/gone.rs b/crates/old/src/gone.rs\n"
            "--- a/crates/old/src/gone.rs\n"
            "+++ /dev/null\n"
            "@@ -1,2 +0,0 @@\n"
            "-fn a() {}\n"
            "-fn b() {}\n"
        )
        parsed = parse_added_lines(created)
        self.assertEqual(parsed, {"crates/new/src/added.rs": {1, 2}})

    def test_baseline_comparison_rejects_new_and_stale_entries(self) -> None:
        fingerprint = 'fn invariant :: unreachable!("static invariant")'
        approved = collections.Counter(
            {
                (
                    "crates/example/src/lib.rs",
                    fingerprint,
                ): 1
            }
        )
        matching = [
            (
                "crates/example/src/lib.rs",
                10,
                fingerprint,
            )
        ]
        new, stale = compare_reborn_baseline(matching, approved)
        self.assertEqual(new, [])
        self.assertEqual(stale, collections.Counter())

        changed = [
            (
                "crates/example/src/lib.rs",
                10,
                'fn invariant :: panic!("runtime input")',
            )
        ]
        new, stale = compare_reborn_baseline(changed, approved)
        self.assertEqual(new, changed)
        self.assertEqual(stale, approved)

    def test_baseline_loader_normalizes_and_counts_duplicate_fingerprints(self) -> None:
        path = "crates/example/src/lib.rs"
        normalized = 'fn invariant :: panic!("static invariant")'
        record = (
            f"{path}\t  fn   invariant ::   panic!(\"static invariant\")  "
            "\treviewed invariant\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            baseline = pathlib.Path(directory) / "baseline.txt"
            baseline.write_text(record + record, encoding="utf-8")

            approved = load_reborn_baseline(baseline)

        self.assertEqual(approved[(path, normalized)], 2)
        violations = [
            (path, 10, normalized),
            (path, 20, normalized),
        ]
        new, stale = compare_reborn_baseline(violations, approved)
        self.assertEqual(new, [])
        self.assertEqual(stale, collections.Counter())

        _new, stale = compare_reborn_baseline(violations[:1], approved)
        self.assertEqual(stale[(path, normalized)], 1)

    def test_multiline_fingerprint_uses_complete_call_and_item_context(self) -> None:
        with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
            source = pathlib.Path(directory) / "lib.rs"
            source.write_text(
                "fn first() {\n"
                "    value.expect(\n"
                '        "same reason",\n'
                "    );\n"
                "}\n"
                "fn second() {\n"
                "    value.expect(\n"
                '        "same reason",\n'
                "    );\n"
                "}\n",
                encoding="utf-8",
            )

            violations = collect_file_violations({source})

            self.assertEqual(len(violations), 2)
            first = violations[0]
            second = violations[1]
            self.assertIn('.expect( "same reason", )', first[2])
            self.assertIn("fn first", first[2])
            self.assertIn("fn second", second[2])
            approved = collections.Counter({(first[0], first[2]): 1})
            new, stale = compare_reborn_baseline([second], approved)
            self.assertEqual(new, [second])
            self.assertEqual(stale, approved)

    def test_fingerprint_includes_receiver_and_match_arm_prefix(self) -> None:
        with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
            source = pathlib.Path(directory) / "lib.rs"
            source.write_text(
                "fn same_item(value: Option<u8>) {\n"
                '    left.unwrap();\n'
                '    right.unwrap();\n'
                "    match value {\n"
                "        None => unreachable!(),\n"
                '        Some(_) => unreachable!("different arm"),\n'
                "    }\n"
                "}\n",
                encoding="utf-8",
            )

            fingerprints = [
                violation[2] for violation in collect_file_violations({source})
            ]

            self.assertEqual(len(fingerprints), 4)
            self.assertIn("left.unwrap()", fingerprints[0])
            self.assertIn("right.unwrap()", fingerprints[1])
            self.assertIn("None => unreachable!()", fingerprints[2])
            self.assertIn('Some(_) => unreachable!("different arm")', fingerprints[3])
            self.assertEqual(len(set(fingerprints)), 4)

    def test_module_level_const_and_static_are_named_fingerprint_contexts(self) -> None:
        with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
            source = pathlib.Path(directory) / "lib.rs"
            source.write_text(
                'const DEFAULT: u8 = value.expect("const");\n'
                'static FALLBACK: u8 = value.expect("static");\n',
                encoding="utf-8",
            )

            fingerprints = [
                violation[2] for violation in collect_file_violations({source})
            ]

            self.assertIn("const DEFAULT ::", fingerprints[0])
            self.assertIn("static FALLBACK ::", fingerprints[1])

    def test_scanner_reuses_lexed_source_for_discovery_and_detection(self) -> None:
        with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
            root = pathlib.Path(directory)
            source = root / "lib.rs"
            source.write_text('fn invariant() { panic!("one scan"); }\n', encoding="utf-8")
            scanner = RustScanner()
            original_read_text = pathlib.Path.read_text
            with mock.patch.object(
                pathlib.Path,
                "read_text",
                autospec=True,
                side_effect=original_read_text,
            ) as read_text:
                production, _tests = discover_reachable_rust_files([source], scanner)
                violations = collect_file_violations(production, scanner)

            source_reads = [
                call
                for call in read_text.call_args_list
                if pathlib.Path(call.args[0]).resolve() == source.resolve()
            ]
            self.assertEqual(len(source_reads), 1)
            self.assertEqual(len(violations), 1)

    @staticmethod
    def _shipping_metadata(
        normal_dependency_dir: str = "crates/normal_dependency",
        members: tuple[str, ...] | None = None,
        shipping_dir: str = "crates/ironclaw_reborn_cli",
    ) -> dict:
        shipping_id = "shipping"
        normal_id = "normal"
        dev_id = "dev"
        build_id = "build"

        def package(
            package_id: str,
            crate_dir: str,
            target_kinds: list[str],
            name: str | None = None,
        ) -> dict:
            crate_root = REPO_ROOT / crate_dir
            return {
                "id": package_id,
                "name": name or crate_dir.rsplit("/", 1)[-1],
                "manifest_path": str(crate_root / "Cargo.toml"),
                "targets": [
                    {
                        "kind": [kind],
                        "src_path": str(
                            crate_root / "src" / ("main.rs" if kind == "bin" else f"{kind}.rs")
                        ),
                    }
                    for kind in target_kinds
                ],
            }

        return {
            "workspace_members": list(
                members
                if members is not None
                else (shipping_id, normal_id, dev_id, build_id)
            ),
            "packages": [
                package(
                    shipping_id,
                    shipping_dir,
                    ["lib", "bin", "test", "custom-build"],
                    name=SHIPPING_PACKAGE_NAME,
                ),
                package(normal_id, normal_dependency_dir, ["lib", "bin"]),
                package(dev_id, "crates/dev_dependency", ["lib"]),
                package(build_id, "crates/build_dependency", ["lib"]),
            ],
            "resolve": {
                "nodes": [
                    {
                        "id": shipping_id,
                        "deps": [
                            {"pkg": normal_id, "dep_kinds": [{"kind": None}]},
                            {"pkg": dev_id, "dep_kinds": [{"kind": "dev"}]},
                            {"pkg": build_id, "dep_kinds": [{"kind": "build"}]},
                        ],
                    },
                    {"id": normal_id, "deps": []},
                    {"id": dev_id, "deps": []},
                    {"id": build_id, "deps": []},
                ]
            },
        }

    def test_shipping_roots_follow_normal_dependencies_and_lib_bin_targets(self) -> None:
        roots = shipping_reborn_source_roots(self._shipping_metadata())

        self.assertEqual(
            roots,
            sorted(
                [
                    (REPO_ROOT / "crates/ironclaw_reborn_cli/src/lib.rs").resolve(),
                    (REPO_ROOT / "crates/ironclaw_reborn_cli/src/main.rs").resolve(),
                    (REPO_ROOT / "crates/normal_dependency/src/lib.rs").resolve(),
                    (REPO_ROOT / "crates/normal_dependency/src/main.rs").resolve(),
                ]
            ),
        )

    def test_shipping_roots_find_crates_nested_in_family_directories(self) -> None:
        """A crate in `crates/<family>/<crate>/` is in scope, not skipped.

        The flat-tree rule (manifest exactly one level under `crates/`) dropped
        these silently, which is how the target-architecture family move would
        have shrunk this gate's scope without failing it.
        """
        roots = shipping_reborn_source_roots(
            self._shipping_metadata(
                normal_dependency_dir="crates/substrates/ironclaw_events"
            )
        )

        self.assertIn(
            (REPO_ROOT / "crates/substrates/ironclaw_events/src/lib.rs").resolve(),
            roots,
        )

    def test_shipping_package_is_found_by_name_after_a_directory_rename(self) -> None:
        """The scope anchor survives `crates/ironclaw_reborn_cli` -> `crates/app/ironclaw_cli`.

        The package keeps its name (`ironclaw`) across that rename, so resolving
        by name keeps the whole gate pointed at the right dependency closure.
        """
        roots = shipping_reborn_source_roots(
            self._shipping_metadata(shipping_dir="crates/app/ironclaw_cli")
        )

        self.assertIn((REPO_ROOT / "crates/app/ironclaw_cli/src/main.rs").resolve(), roots)

    def test_shipping_crate_outside_the_crate_tree_fails_closed(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "not under"):
            shipping_reborn_source_roots(
                self._shipping_metadata(normal_dependency_dir="tools/ironclaw_stress")
            )

    def test_missing_shipping_package_fails_closed(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "exactly one workspace package"):
            shipping_reborn_source_roots(self._shipping_metadata(members=()))

    def test_shipping_crate_without_a_source_root_fails_closed(self) -> None:
        metadata = self._shipping_metadata()
        for package in metadata["packages"]:
            if package["id"] == "normal":
                package["targets"] = [{"kind": ["test"], "src_path": "x.rs"}]

        with self.assertRaisesRegex(RuntimeError, "no lib/bin target"):
            shipping_reborn_source_roots(metadata)

    def test_malformed_baseline_records_are_rejected(self) -> None:
        malformed = [
            "missing-tabs",
            "\tfingerprint\treason",
            "crates/example/src/lib.rs\t\treason",
            "crates/example/src/lib.rs\tfingerprint\t",
            "crates/example/src/lib.rs\tfingerprint\treason\textra",
        ]
        for record in malformed:
            with self.subTest(record=record):
                with tempfile.TemporaryDirectory() as directory:
                    baseline = pathlib.Path(directory) / "baseline.txt"
                    baseline.write_text(record + "\n", encoding="utf-8")
                    with self.assertRaises(RuntimeError):
                        load_reborn_baseline(baseline)

    def test_reborn_report_returns_nonzero_for_new_and_stale_entries(self) -> None:
        violation = (
            "crates/example/src/lib.rs",
            7,
            'fn new_call :: panic!("new")',
        )
        with contextlib.redirect_stdout(io.StringIO()) as output:
            new_result = report_reborn_baseline(
                1,
                [violation],
                collections.Counter(),
            )
        self.assertEqual(new_result, 1)
        self.assertIn("New or changed panic-style calls", output.getvalue())

        approved = collections.Counter(
            {
                (
                    "crates/example/src/lib.rs",
                    'fn removed :: panic!("old")',
                ): 1
            }
        )
        with contextlib.redirect_stdout(io.StringIO()) as output:
            stale_result = report_reborn_baseline(1, [], approved)
        self.assertEqual(stale_result, 1)
        self.assertIn("Stale baseline entries", output.getvalue())

    def test_has_code_classifier_includes_reborn_baseline(self) -> None:
        workflow = (REPO_ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("scripts/no_panics_reborn_baseline\\.txt$", workflow)


if __name__ == "__main__":
    sys.exit(main())
