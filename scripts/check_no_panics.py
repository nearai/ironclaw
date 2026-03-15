#!/usr/bin/env python3
# Requires Python 3.10+ for PEP 604 union syntax such as `int | None`.

import argparse
import pathlib
import re
import subprocess
import sys
import unittest
from dataclasses import dataclass


PANIC_PATTERN = re.compile(r"\.(?:unwrap|expect)\(|(?<!_)assert(?:_eq|_ne)?!")
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
            # This can misclassify lifetimes like `'a` as char literals. That only
            # risks false negatives by masking later code on the same line.
            state.in_char = True
            i += 1
            continue

        out[i] = ch
        i += 1

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


def changed_rust_files(base: str, head: str) -> list[pathlib.Path]:
    output = run_git("diff", "--name-only", f"{base}...{head}", "--", "src", "crates")
    files = []
    for line in output.splitlines():
        if line.endswith(".rs") and (line.startswith("src/") or line.startswith("crates/")):
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
            if "// safety:" in lines[line_no - 1]:
                continue
            if PANIC_PATTERN.search(sanitized[line_no - 1]):
                violations.append((str(path), line_no, lines[line_no - 1].rstrip()))

    return violations


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=False, default="origin/staging")
    parser.add_argument("--head", required=False, default="HEAD")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(CheckNoPanicsTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1

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


if __name__ == "__main__":
    sys.exit(main())
