#!/usr/bin/env python3
# Requires Python 3.10+ for PEP 604 union syntax such as `int | None`.

import argparse
import collections
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest
from dataclasses import dataclass


PANIC_PATTERN = re.compile(
    r"\.(?:unwrap|expect)\("
    r"|(?<!_)assert(?:_eq|_ne)?!"
    r"|(?<![A-Za-z0-9_])(?:panic|unreachable|unimplemented|todo)!"
)
SAFETY_RATIONALE_PATTERN = re.compile(r"//\s*safety:\s*\S", re.IGNORECASE)
TEST_ATTR_PATTERN = re.compile(
    r"^\s*#\s*\[\s*(?:"
    r"test"
    r"|tokio::test(?:\s*\([^]]*\))?"
    r"|rstest(?:\s*\([^]]*\))?"
    r"|test_case(?:\s*\([^]]*\))?"
    r"|cfg\s*\([^]]*\btest\b[^]]*\)"
    r")\s*\]"
)
ITEM_PATTERN = re.compile(
    r"^\s*"
    r"(?:(?:pub(?:\([^)]*\))?|crate)\s+)?"
    r"(?:(?:async|unsafe|const)\s+)*"
    r"(fn|mod|struct|enum|trait|union|impl)\b"
    r"(?:\s+([A-Za-z_][A-Za-z0-9_]*))?"
)
OUT_OF_LINE_MOD_PATTERN = re.compile(
    r"^\s*"
    r"(?:(?:pub(?:\([^)]*\))?|crate)\s+)?"
    r"mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
)
PATH_ATTR_PATTERN = re.compile(
    r'^\s*#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]'
)

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
REBORN_BASELINE_PATH = REPO_ROOT / "scripts" / "no_panics_reborn_baseline.txt"
SHIPPING_PACKAGE_MANIFEST = (
    REPO_ROOT / "crates" / "ironclaw_reborn_cli" / "Cargo.toml"
)


@dataclass
class LexerState:
    block_comment_depth: int = 0
    in_string: bool = False
    string_escape: bool = False
    in_char: bool = False
    char_escape: bool = False
    raw_string_hashes: int | None = None


def run_git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout


def sanitize_line(line: str, state: LexerState) -> str:
    chars = list(line)
    out = [" "] * len(chars)
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

    return "".join(out)


def is_test_item(line: str, pending_test_attr: bool) -> tuple[bool, bool]:
    match = ITEM_PATTERN.match(line)
    if not match:
        return False, False

    kind, name = match.groups()
    named_tests_module = kind == "mod" and name == "tests"
    return True, pending_test_attr or named_tests_module


def line_test_contexts(lines: list[str]) -> list[bool]:
    contexts = [False] * len(lines)
    lexer = LexerState()
    block_stack: list[bool] = []
    pending_test_attr = False
    pending_block_context: bool | None = None

    for idx, raw in enumerate(lines):
        code = sanitize_line(raw, lexer)
        stripped = code.strip()
        current_context = block_stack[-1] if block_stack else False

        if TEST_ATTR_PATTERN.match(stripped):
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
    if "tests" in parts or posix_path.name == "tests.rs":
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
    """

    packages = {package["id"]: package for package in metadata["packages"]}
    shipping_manifest = SHIPPING_PACKAGE_MANIFEST.resolve()
    shipping_ids = [
        package_id
        for package_id, package in packages.items()
        if pathlib.Path(package["manifest_path"]).resolve() == shipping_manifest
    ]
    if len(shipping_ids) != 1:
        raise RuntimeError(
            "expected exactly one shipping Reborn package at "
            f"{shipping_manifest}, found {len(shipping_ids)}"
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
    for package_id in reachable:
        package = packages[package_id]
        manifest = pathlib.Path(package["manifest_path"]).resolve()
        if manifest.parent.parent != crates_root:
            continue
        for target in package["targets"]:
            if not ({"lib", "bin"} & set(target["kind"])):
                continue
            source = pathlib.Path(target["src_path"]).resolve()
            if source.suffix == ".rs":
                roots.add(source)
    return sorted(roots)


def default_module_candidates(source: pathlib.Path, name: str) -> tuple[pathlib.Path, ...]:
    if source.name in {"lib.rs", "main.rs", "mod.rs"}:
        module_dir = source.parent
    else:
        module_dir = source.parent / source.stem
    return module_dir / f"{name}.rs", module_dir / name / "mod.rs"


def module_edges(
    source: pathlib.Path,
    inherited_test_only: bool,
) -> list[tuple[pathlib.Path, bool]]:
    lines = source.read_text(encoding="utf-8").splitlines()
    contexts = line_test_contexts(lines)
    lexer = LexerState()
    pending_path: str | None = None
    edges: list[tuple[pathlib.Path, bool]] = []

    for raw, local_test_context in zip(lines, contexts):
        code = sanitize_line(raw, lexer)
        path_attr = PATH_ATTR_PATTERN.match(raw)
        if path_attr and code.lstrip().startswith("#["):
            pending_path = path_attr.group(1)
            continue

        module = OUT_OF_LINE_MOD_PATTERN.match(code)
        if module:
            if pending_path is not None:
                candidates = (source.parent / pending_path,)
            else:
                candidates = default_module_candidates(source, module.group(1))
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
            pending_path = None
            continue

        stripped = code.strip()
        if stripped and not stripped.startswith("#["):
            pending_path = None

    return edges


def discover_reachable_rust_files(
    roots: list[pathlib.Path],
) -> tuple[set[pathlib.Path], set[pathlib.Path]]:
    """Follow Rust module edges, retaining whether each file is test-only."""

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
        pending.extend(module_edges(source, test_only))

    production = {path for path, test_only in states.items() if not test_only}
    tests = {path for path, test_only in states.items() if test_only}
    return production, tests


def repository_relative(path: pathlib.Path) -> str:
    return path.resolve().relative_to(REPO_ROOT).as_posix()


def normalized_source_line(line: str) -> str:
    return " ".join(line.strip().split())


def violation_fingerprint(path: str, line: str) -> tuple[str, str]:
    return path, normalized_source_line(line)


def collect_file_violations(
    paths: set[pathlib.Path],
) -> list[tuple[str, int, str]]:
    violations: list[tuple[str, int, str]] = []
    for path in sorted(paths):
        lines = path.read_text(encoding="utf-8").splitlines()
        contexts = line_test_contexts(lines)
        lexer = LexerState()
        for line_no, (raw, test_context) in enumerate(zip(lines, contexts), 1):
            code = sanitize_line(raw, lexer)
            if test_context or SAFETY_RATIONALE_PATTERN.search(raw):
                continue
            if PANIC_PATTERN.search(code):
                violations.append(
                    (repository_relative(path), line_no, raw.rstrip())
                )
    return violations


def load_reborn_baseline(
    path: pathlib.Path,
) -> collections.Counter[tuple[str, str]]:
    approved: collections.Counter[tuple[str, str]] = collections.Counter()
    for line_no, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        fields = raw.split("\t", 2)
        if len(fields) != 3 or not fields[2].strip():
            raise RuntimeError(
                f"{path}:{line_no}: expected path<TAB>source<TAB>non-empty reason"
            )
        approved[(fields[0], fields[1])] += 1
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
    for path, line_no, line in violations:
        fingerprint = violation_fingerprint(path, line)
        if remaining[fingerprint]:
            remaining[fingerprint] -= 1
            if remaining[fingerprint] == 0:
                del remaining[fingerprint]
        else:
            new.append((path, line_no, line))
    return new, remaining


def changed_rust_files(base: str, head: str) -> list[pathlib.Path]:
    output = run_git("diff", "--name-only", f"{base}...{head}", "--", "src", "crates")
    files = []
    for line in output.splitlines():
        if line.endswith(".rs") and (line.startswith("src/") or line.startswith("crates/")):
            if not is_test_only_path(line):
                files.append(pathlib.Path(line))
    return files


def added_lines_for_file(base: str, head: str, path: pathlib.Path) -> set[int]:
    diff = run_git("diff", "--unified=0", f"{base}...{head}", "--", str(path))
    added: set[int] = set()
    current_line = 0

    for line in diff.splitlines():
        if line.startswith("@@"):
            match = re.search(r"\+(\d+)(?:,(\d+))?", line)
            if not match:
                continue
            current_line = int(match.group(1))
            continue
        if line.startswith("+++ ") or line.startswith("--- "):
            continue
        if line.startswith("+"):
            added.add(current_line)
            current_line += 1
        elif line.startswith("-"):
            continue
        else:
            current_line += 1

    return added


def collect_violations(base: str, head: str) -> list[tuple[str, int, str]]:
    violations: list[tuple[str, int, str]] = []

    for path in changed_rust_files(base, head):
        if not path.exists():
            continue
        added_lines = added_lines_for_file(base, head, path)
        if not added_lines:
            continue

        lines = path.read_text(encoding="utf-8").splitlines()
        contexts = line_test_contexts(lines)
        lexer = LexerState()
        sanitized = [sanitize_line(line, lexer) for line in lines]

        for line_no in sorted(added_lines):
            if line_no < 1 or line_no > len(lines):
                continue
            if contexts[line_no - 1]:
                continue
            if SAFETY_RATIONALE_PATTERN.search(lines[line_no - 1]):
                continue
            if PANIC_PATTERN.search(sanitized[line_no - 1]):
                violations.append((str(path), line_no, lines[line_no - 1].rstrip()))

    return violations


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
        roots = shipping_reborn_source_roots(run_cargo_metadata())
        production, _tests = discover_reachable_rust_files(roots)
        violations = collect_file_violations(production)
        approved = load_reborn_baseline(REBORN_BASELINE_PATH)
        new, stale = compare_reborn_baseline(violations, approved)
        if not new and not stale:
            print(
                "OK: Reborn production panic baseline matches "
                f"({len(production)} files, {len(violations)} reviewed invariant(s))."
            )
            return 0

        print("::error::Reborn production panic baseline changed.")
        if new:
            print("")
            print("New or changed panic-style calls:")
            for path, line_no, line in new[:20]:
                print(f"{path}:{line_no}: {line}")
            if len(new) > 20:
                print(f"... and {len(new) - 20} more")
        if stale:
            print("")
            print("Stale baseline entries (remove them to ratchet downward):")
            for (path, source), count in list(stale.items())[:20]:
                suffix = f" (x{count})" if count > 1 else ""
                print(f"{path}: {source}{suffix}")
            if len(stale) > 20:
                print(f"... and {len(stale) - 20} more")
        return 1

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

    def test_test_only_path_detection(self) -> None:
        self.assertTrue(is_test_only_path("src/channels/web/tests/multi_tenant.rs"))
        self.assertTrue(is_test_only_path("crates/foo/src/tests/helpers.rs"))
        self.assertTrue(is_test_only_path("crates/foo/src/tests.rs"))
        self.assertTrue(is_test_only_path("crates/foo/src/nested/tests.rs"))
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
        self.assertIsNone(SAFETY_RATIONALE_PATTERN.search("panic!(); // safety:"))
        self.assertIsNone(SAFETY_RATIONALE_PATTERN.search("panic!(); // safety:   "))
        self.assertIsNotNone(
            SAFETY_RATIONALE_PATTERN.search(
                "panic!(); // safety: fixed static literal is validated"
            )
        )

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

    def test_baseline_comparison_rejects_new_and_stale_entries(self) -> None:
        approved = collections.Counter(
            {
                (
                    "crates/example/src/lib.rs",
                    'unreachable!("static invariant")',
                ): 1
            }
        )
        matching = [
            (
                "crates/example/src/lib.rs",
                10,
                '    unreachable!("static invariant")',
            )
        ]
        new, stale = compare_reborn_baseline(matching, approved)
        self.assertEqual(new, [])
        self.assertEqual(stale, collections.Counter())

        changed = [
            (
                "crates/example/src/lib.rs",
                10,
                '    panic!("runtime input")',
            )
        ]
        new, stale = compare_reborn_baseline(changed, approved)
        self.assertEqual(new, changed)
        self.assertEqual(stale, approved)


if __name__ == "__main__":
    sys.exit(main())
