#!/usr/bin/env python3
"""Require meaningful regression evidence for fixes and high-risk changes."""

from __future__ import annotations

import argparse
import functools
import re
import subprocess
import sys
from pathlib import Path

# The crate inventory, not a `crates/ironclaw_*` path shape, decides where a
# high-risk area lives. The literal-prefix list this replaced matched nothing
# once crates move into family directories (`crates/<family>/ironclaw_*`,
# PROPOSAL §5): every high-risk path stopped matching, the gate reported "no
# high-risk files changed", and the regression-test requirement relaxed
# silently. See docs/internal/reborn/target-architecture/CHECKLIST.md WS10 and #6963.
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
try:
    from crate_tree import CrateTreeError, crate_directory  # noqa: E402
except ImportError as error:  # pragma: no cover - deployment error, not logic
    # This gate is copied around (the commit-msg hook runs it from the repo, CI
    # runs it from a second checkout). Say which file is missing instead of
    # showing an import traceback — but still refuse to run, because without the
    # inventory the gate cannot tell "no high-risk change" from "found nothing".
    raise SystemExit(
        "regression-test-check: cannot import scripts/ci/lib/crate_tree.py "
        f"({error}). The gate resolves high-risk paths through the crate "
        "inventory and will not run without it."
    ) from error


# High-risk areas as (crate name, path inside the crate). The crate name is the
# identity; where its `Cargo.toml` sits is discovered. `ironclaw_run_state` used
# to be here and was deleted with #6696 — the entry outlived the crate and, being
# a never-matching string, cost nothing to leave behind. Resolution now refuses
# an entry that names no crate, so that cannot recur.
HIGH_RISK_ENTRIES = (
    ("ironclaw_turns", "src/coordinator.rs"),
    ("ironclaw_turns", "src/status.rs"),
    ("ironclaw_processes", "src/journal_store"),
    ("ironclaw_processes", "src/supervisor.rs"),
    ("ironclaw_llm", "src/circuit_breaker.rs"),
    ("ironclaw_llm", "src/retry.rs"),
    ("ironclaw_llm", "src/failover.rs"),
    ("ironclaw_agent_loop", "src/executor"),
    ("ironclaw_agent_loop", "src/state"),
    ("ironclaw_safety", "src/"),
)

# Built frontend assets carry no behavior, so a diff touching only these (and
# markdown) does not owe a regression test. Same discovery rule as above.
STATIC_ASSET_ENTRIES = (("ironclaw_webui", "frontend/public/"),)

FIX_RE = re.compile(
    r"^(fix(\(.*\))?|hotfix|bugfix):", re.IGNORECASE | re.MULTILINE
)
MARKER_PREFIX = "[skip-regression-check"
MARKER_RE = re.compile(
    r"\[skip-regression-check:\s*"
    r"deterministic reproduction is impossible because\s+"
    r"([^\]\r\n]{20,})\]",
    re.IGNORECASE,
)
BODY_REASON_RE = re.compile(
    r"^Regression-test exemption:\s*"
    r"deterministic reproduction is impossible because\s+(.{20,})$",
    re.IGNORECASE | re.MULTILINE,
)
PLACEHOLDER_RE = re.compile(
    r"^(n/?a|none|todo|tbd|unknown|not applicable|because impossible)[.!]?$",
    re.IGNORECASE,
)

# `node:assert`, in the three shapes a test can call it. `assert.equal(…)` is
# unambiguous, so the qualified form accepts every method. The bare form (a
# named import, `import { strictEqual } from "node:assert/strict"`) only accepts
# names nothing else plausibly answers to: bare `match(…)` is `url.match(/x/)`
# and bare `ok(…)` is anybody's helper, neither of which is evidence of anything.
_ASSERT_QUALIFIED_METHODS = (
    "strictEqual|deepStrictEqual|notStrictEqual|notDeepStrictEqual"
    "|equal|deepEqual|notEqual|notDeepEqual"
    "|ok|match|doesNotMatch|throws|rejects|doesNotThrow|doesNotReject|fail"
)
_ASSERT_BARE_METHODS = (
    "strictEqual|deepStrictEqual|notStrictEqual|notDeepStrictEqual"
    "|deepEqual|notDeepEqual"
)
_ASSERT_METHOD_START = re.compile(
    rf"(?:\bassert\s*\.\s*(?P<qualified>{_ASSERT_QUALIFIED_METHODS})"
    rf"|(?<![.\w$])(?P<bare>{_ASSERT_BARE_METHODS})"
    r"|\bassert(?=\s*\())\s*\(",
)

# `expect(actual).toEqual(expected)`, split so both operands can be read with
# the balanced parser. A single regex cannot: `(.*?)` stops at the first `)`
# unless something after it forces backtracking, so `expected` loses its last
# character whenever it nests — and `expect(f(1)).toEqual(f(1))` then looks like
# two *different* operands and passes the tautology filter.
_EXPECT_START = re.compile(r"\bexpect\s*\(")
_EXPECT_MATCHER = re.compile(
    r"\s*\.\s*(?:to[A-Z]\w*|resolves|rejects)(?:\s*\.\s*\w+)?\s*\(",
)

_ASSERT_COMPARISON_METHODS = {
    "strictEqual",
    "deepStrictEqual",
    "notStrictEqual",
    "notDeepStrictEqual",
    "equal",
    "deepEqual",
    "notEqual",
    "notDeepEqual",
}

# Literal interiors are replaced character-for-character with this filler rather
# than blanked. Offsets stay valid for the argument parser, and two identical
# literals still normalize equal, so `expect("fixed").toBe("fixed")` is still
# caught as a tautology. Two *different* literals of the same length collide and
# are rejected too, which is the safe direction for a guardrail.
_LITERAL_FILLER = "~"

# A `/` following any of these cannot be division, so it opens a regex literal.
# `>` covers the arrow in `() => /fixed/.test(value)`. Everything else —
# identifiers, numbers, `)`, `]` — is division, which is the safe default: a
# misread regex would mask real code.
_REGEX_PRECEDING_PUNCTUATION = frozenset("(,=:[!&|?{};>")
_REGEX_PRECEDING_KEYWORDS = frozenset(
    {
        "return",
        "typeof",
        "instanceof",
        "in",
        "of",
        "new",
        "delete",
        "void",
        "throw",
        "case",
        "do",
        "else",
        "yield",
        "await",
    }
)


# Both argument parsers below run on masked text, where every literal interior
# is filler: no bracket, comma, or quote survives inside a string, a regex, or a
# comment. Bracket depth is therefore the whole grammar they need.
_BRACKET_CLOSERS = {"(": ")", "[": "]", "{": "}"}


def balanced_arguments(text: str, start: int) -> str | None:
    """Return the contents of a call whose opening parenthesis is at start."""

    stack = [")"]
    for index in range(start + 1, len(text)):
        character = text[index]
        if character in _BRACKET_CLOSERS:
            stack.append(_BRACKET_CLOSERS[character])
        elif character == stack[-1]:
            stack.pop()
            if not stack:
                return text[start + 1 : index]
    return None


def top_level_operands(expression: str) -> list[str]:
    """Split call arguments without treating nested commas as separators."""

    stack: list[str] = []
    operands: list[str] = []
    operand_start = 0
    for index, character in enumerate(expression):
        if character in _BRACKET_CLOSERS:
            stack.append(_BRACKET_CLOSERS[character])
        elif stack and character == stack[-1]:
            stack.pop()
        elif character == "," and not stack:
            operands.append(expression[operand_start:index])
            operand_start = index + 1
    operands.append(expression[operand_start:])
    return operands


def line_end(text: str, index: int) -> int:
    newline = text.find("\n", index)
    return len(text) if newline == -1 else newline


def mask_span(masked: list[str], start: int, end: int, filler: str) -> None:
    """Overwrite [start, end) with filler, keeping newlines so offsets hold."""

    for index in range(start, end):
        if masked[index] != "\n":
            masked[index] = filler


def string_literal_end(text: str, start: int, quote: str) -> int | None:
    """Index just past the closing quote, or None if the literal never closes.

    Only a template literal may span lines. An unclosed `'` or `"` on a line is
    an apostrophe in prose, not a string.
    """

    escaped = False
    for index in range(start + 1, len(text)):
        character = text[index]
        if escaped:
            escaped = False
        elif character == "\\":
            escaped = True
        elif character == quote:
            return index + 1
        elif character == "\n" and quote != "`":
            return None
    return None


def regex_literal_end(text: str, start: int) -> int | None:
    """Index just past the closing slash, or None if it never closes.

    A `/` inside a character class does not terminate the literal, and neither
    does an escaped one — which is the whole point: `/^https?:\\/\\//` must not
    be read as code followed by a `//` comment.
    """

    escaped = False
    in_class = False
    for index in range(start + 1, len(text)):
        character = text[index]
        if escaped:
            escaped = False
        elif character == "\\":
            escaped = True
        elif character == "\n":
            return None
        elif in_class:
            in_class = character != "]"
        elif character == "[":
            in_class = True
        elif character == "/":
            return index + 1
    return None


def opens_regex_literal(masked: list[str], index: int) -> bool:
    """Whether the `/` at index starts a regex literal rather than a division."""

    cursor = index - 1
    while cursor >= 0 and masked[cursor].isspace():
        cursor -= 1
    if cursor < 0:
        return True
    if masked[cursor] in _REGEX_PRECEDING_PUNCTUATION:
        return True
    word_end = cursor + 1
    while cursor >= 0 and (masked[cursor].isalnum() or masked[cursor] in "_$"):
        cursor -= 1
    return "".join(masked[cursor + 1 : word_end]) in _REGEX_PRECEDING_KEYWORDS


def without_typescript_comments_and_strings(text: str) -> str:
    """Mask comments, string bodies, and regex bodies, preserving offsets.

    The input is `added_text`: the `+` lines of a diff spliced together, so its
    quoting is not guaranteed to balance. An unterminated literal is therefore
    left alone rather than allowed to swallow the rest of the text — otherwise
    one apostrophe in JSX prose, or a hunk landing inside a template literal,
    hides every assertion after it and the gate rejects a legitimate fix.
    """

    masked = list(text)
    index = 0
    while index < len(text):
        character = text[index]
        # Comments are checked before the regex heuristic and always win: no
        # regex literal can start with `/` or `*`.
        if text.startswith("//", index):
            end = line_end(text, index)
            mask_span(masked, index, end, " ")
            index = end
            continue
        if text.startswith("/*", index):
            close = text.find("*/", index + 2)
            end = close + 2 if close != -1 else line_end(text, index)
            mask_span(masked, index, end, " ")
            index = end
            continue
        end = None
        if character == "/" and opens_regex_literal(masked, index):
            end = regex_literal_end(text, index)
        elif character in {"'", '"', "`"}:
            end = string_literal_end(text, index, character)
        if end is None:
            index += 1
            continue
        # Keep the delimiters: `expect("x").toBe("x")` stays visible as a
        # comparison of two literals, so the tautology filter still sees it.
        mask_span(masked, index + 1, end - 1, _LITERAL_FILLER)
        index = end
    return "".join(masked)


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ("git", "-C", str(repo), *args),
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "git command failed")
    return result.stdout


def diff_args(base: str, head: str) -> list[str]:
    if head == "INDEX":
        return ["diff", "--cached"]
    if head == "WORKTREE":
        return ["diff", base]
    return ["diff", f"{base}...{head}"]


def changed_files(repo: Path, base: str, head: str) -> list[str]:
    output = git(
        repo,
        *diff_args(base, head),
        "--name-only",
        "--diff-filter=ACMR",
        "-z",
    )
    return [path for path in output.split("\0") if path]


def all_touched_paths(repo: Path, base: str, head: str) -> list[str]:
    """Every path the diff touches, deletions and renames included.

    `changed_files` filters to ACMR because only surviving files can carry a
    regression assertion. Staleness tolerance needs the unfiltered set: see
    `resolve_prefixes`.
    """

    output = git(repo, *diff_args(base, head), "--name-only", "-z")
    return [path for path in output.split("\0") if path]


def is_workspace_checkout(repo: Path) -> bool:
    """True when `repo` is the IronClaw workspace root.

    The discriminator is the root manifest's `[workspace]` table. It decides
    whether a missing crate tree is a broken checkout (hard error) or simply a
    repository that has no crates — the hermetic fixtures in
    `scripts/ci/test-regression-test-check.sh` are the latter, and so is any
    caller pointing the gate at an unrelated tree.
    """

    try:
        return "[workspace]" in (repo / "Cargo.toml").read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return False


@functools.lru_cache(maxsize=None)
def _crate_directory(crate: str, repo: Path) -> str:
    """`crate_directory` memoized: each call re-walks `crates/` from scratch."""

    return crate_directory(crate, repo)


def resolve_prefixes(
    repo: Path, entries: tuple[tuple[str, str], ...], touched: list[str]
) -> list[str] | None:
    """Resolve (crate, subpath) entries against the checkout's real crate tree.

    Returns repo-relative path prefixes, or `None` when `repo` is not the
    IronClaw workspace and there is therefore no inventory to resolve against.

    Fails closed in both directions a path-keyed list can rot:

    * a named crate that no longer exists (or moved so no directory carries the
      name) raises, naming the entry — the failure mode that let
      `ironclaw_run_state` sit here unmatched after #6696 deleted it;
    * a resolved prefix that exists on disk nowhere raises too, so deleting the
      last file under a high-risk subpath forces the entry out instead of
      leaving a string that can never match again.

    The one tolerated case is a prefix the diff itself touches. In CI the gate
    executes from the *trusted base* checkout (`$GATE_ROOT`, base.sha) while
    judging the PR *head* tree, so a PR deleting a high-risk file cannot also
    fix the base copy of this list. Without that escape such a PR would be
    permanently red with no in-PR remedy; with it, the entry disappears from the
    list on the same merge that deletes the file.
    """

    if not is_workspace_checkout(repo):
        return None

    prefixes: list[str] = []
    for crate, subpath in entries:
        try:
            directory = _crate_directory(crate, repo)
        except CrateTreeError as error:
            raise RuntimeError(
                f"high-risk entry ({crate!r}, {subpath!r}) names a crate this "
                f"checkout does not have: {error} Repoint or drop the entry in "
                "scripts/ci/regression-test-check.py — an entry that resolves to "
                "nothing silently relaxes the regression-test requirement."
            ) from error
        prefix = f"{directory}/{subpath}"
        if not (repo / prefix).exists() and not any(
            path.startswith(prefix) for path in touched
        ):
            raise RuntimeError(
                f"high-risk entry ({crate!r}, {subpath!r}) resolves to {prefix!r}, "
                "which this checkout does not contain and this diff does not "
                "touch. Repoint or drop the entry in "
                "scripts/ci/regression-test-check.py."
            )
        prefixes.append(prefix)
    return prefixes


def added_text(repo: Path, base: str, head: str, path: str) -> str:
    diff = git(repo, *diff_args(base, head), "--unified=40", "--", path)
    added: list[str] = []
    for line in diff.splitlines():
        if line.startswith("+++") or not line.startswith("+"):
            continue
        added.append(line[1:])
    return "\n".join(added)


def target_file_text(repo: Path, head: str, path: str) -> str | None:
    if head == "WORKTREE":
        try:
            return (repo / path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            return None
    revision = "" if head == "INDEX" else head
    try:
        return git(repo, "show", f"{revision}:{path}")
    except RuntimeError:
        return None


def is_test_path(path: str) -> bool:
    name = Path(path).name
    return (
        path.startswith("tests/")
        or "/tests/" in path
        or re.search(r"(^|/)test_[^/]*\.py$", path) is not None
        or re.search(r"(^|/)[^/]*_test\.py$", path) is not None
        or re.search(r"^scripts/(ci/)?test-[^/]*\.py$", path) is not None
        or re.search(r"\.(test|spec)\.(ts|tsx|mts|js|jsx)$", name) is not None
        or re.search(r"^scripts/(ci/)?test-[^/]*\.sh$", path) is not None
    )


def normalized(expression: str) -> str:
    return re.sub(r"\s+", "", expression).rstrip(",;")


def without_comment_lines(text: str, prefixes: tuple[str, ...]) -> str:
    return "\n".join(
        line for line in text.splitlines() if not line.lstrip().startswith(prefixes)
    )


def has_meaningful_rust_assertion(text: str) -> bool:
    text = without_comment_lines(text, ("//", "/*", "*"))
    for match in re.finditer(
        r"\b(assert|assert_eq|assert_ne|matches)!\s*\((.*?)\)\s*;",
        text,
        re.DOTALL,
    ):
        macro, expression = match.groups()
        compact = normalized(expression)
        if macro == "assert" and re.fullmatch(
            r"(true|false|0|1)(,[^,]+)?", compact
        ):
            continue
        if macro in {"assert_eq", "assert_ne"}:
            operands = expression.split(",", 2)
            if len(operands) >= 2 and normalized(operands[0]) == normalized(
                operands[1]
            ):
                continue
        return True
    return bool(
        re.search(
            r"\bassert_(?:ok|err|matches)!\s*\(|"
            r"\.(?:unwrap_err|expect_err)\s*\(",
            text,
            re.DOTALL,
        )
    )


def has_meaningful_python_assertion(text: str) -> bool:
    text = without_comment_lines(text, ("#",))
    for line in text.splitlines():
        stripped = line.strip()
        if re.match(r"assert\s+(True|False|0|1)(\s*(,|$))", stripped):
            continue
        comparison = re.match(r"assert\s+(.+?)\s*(==|!=)\s*(.+?)(?:\s*,.*)?$", stripped)
        if comparison and normalized(comparison.group(1)) == normalized(
            comparison.group(3)
        ):
            continue
        if stripped.startswith("assert ") and len(stripped) > len("assert "):
            return True
        if re.search(
            r"\b(pytest\.raises|assert_called|assert_awaited|assert_has_calls)\b",
            stripped,
        ):
            return True
    for match in re.finditer(
        r"\bself\.assert([A-Z]\w*)\s*\((.*?)\)",
        text,
        re.DOTALL,
    ):
        method, expression = match.groups()
        operands = expression.split(",", 2)
        if method in {"Equal", "NotEqual", "Is", "IsNot"}:
            if len(operands) >= 2 and normalized(operands[0]) == normalized(
                operands[1]
            ):
                continue
        elif method == "True" and normalized(operands[0]) in {"True", "1"}:
            continue
        elif method == "False" and normalized(operands[0]) in {"False", "0"}:
            continue
        return True
    return False


def has_meaningful_typescript_assertion(text: str) -> bool:
    text = without_typescript_comments_and_strings(text)
    for match in _EXPECT_START.finditer(text):
        actual = balanced_arguments(text, match.end() - 1)
        if actual is None:
            continue
        matcher = _EXPECT_MATCHER.match(text, match.end() + len(actual) + 1)
        if matcher is None:
            continue
        expected = balanced_arguments(text, matcher.end() - 1)
        if expected is None:
            continue
        if normalized(actual) == normalized(expected) and expected.strip():
            continue
        if normalized(actual) in {"true", "1"} and normalized(expected) in {
            "true",
            "1",
        }:
            continue
        return True
    for match in _ASSERT_METHOD_START.finditer(text):
        # None for a plain `assert(value)`, which has no method to dispatch on.
        method = match.group("qualified") or match.group("bare")
        expression = balanced_arguments(text, match.end() - 1)
        if expression is None:
            continue
        operands = top_level_operands(expression)
        if method in _ASSERT_COMPARISON_METHODS:
            # A comparison missing its second operand is malformed, and a
            # comparison of a value against itself holds whether or not the bug
            # is fixed. Neither is regression evidence.
            if len(operands) < 2 or normalized(operands[0]) == normalized(
                operands[1]
            ):
                continue
        elif normalized(operands[0]) in {"true", "false", "1", "0", ""}:
            continue
        return True
    return False


def has_meaningful_shell_assertion(text: str) -> bool:
    text = without_comment_lines(text, ("#",))
    meaningful_patterns = (
        r"\bexpect_(?:pass|fail|success|failure)\b",
        r"\bassert_(?:eq|ne|contains|matches|success|failure)\b",
        r"\bgrep\s+(?:-[A-Za-z]*q[A-Za-z]*\s+|--quiet\s+)",
        r"(^|[;&|]\s*)!\s+\S+",
    )
    return any(re.search(pattern, text, re.MULTILINE) for pattern in meaningful_patterns)


def has_substantive_test_change(text: str, suffix: str) -> bool:
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith(("#", "//", "/*", "*")):
            continue
        if stripped in {"{", "}", "};", "pass", "..."}:
            continue
        if suffix == ".rs" and (
            stripped.startswith(("#[", "fn ", "mod ", "use "))
            or re.fullmatch(r"(async\s+)?fn\s+\w+\([^)]*\)\s*\{\s*\}", stripped)
        ):
            continue
        if suffix == ".py" and stripped.startswith(
            ("def test_", "async def test_", "@pytest.", "import ", "from ")
        ):
            continue
        if suffix in {".ts", ".tsx", ".mts", ".js", ".jsx"} and re.match(
            r"(test|it|describe)\s*\(", stripped
        ):
            continue
        if re.fullmatch(
            r"(assert\s+(True|1)|assert!\s*\(\s*(true|1)\s*\)\s*;?|"
            r"expect\s*\(\s*(true|1)\s*\)\.to\w+\(\s*(true|1)\s*\)\s*;?)",
            stripped,
            re.IGNORECASE,
        ):
            continue
        return True
    return False


def meaningful_test_changed(
    repo: Path, base: str, head: str, files: list[str]
) -> tuple[bool, str | None]:
    for path in files:
        suffix = Path(path).suffix
        text = added_text(repo, base, head, path)
        if not text.strip():
            continue
        if suffix == ".rs":
            full_text = target_file_text(repo, head, path)
            if full_text is None:
                continue
            inline_test_file = (
                "#[test]" in full_text
                or "#[tokio::test]" in full_text
                or "#[cfg(test)]" in full_text
            )
            if (
                (inline_test_file or is_test_path(path))
                and has_substantive_test_change(text, suffix)
                and has_meaningful_rust_assertion(text)
            ):
                return True, path
        elif suffix == ".py" and is_test_path(path):
            if has_substantive_test_change(
                text, suffix
            ) and has_meaningful_python_assertion(text):
                return True, path
        elif suffix in {".ts", ".tsx", ".mts", ".js", ".jsx"} and is_test_path(path):
            if has_substantive_test_change(
                text, suffix
            ) and has_meaningful_typescript_assertion(text):
                return True, path
        elif suffix == ".sh" and is_test_path(path):
            if has_substantive_test_change(
                text, suffix
            ) and has_meaningful_shell_assertion(text):
                return True, path
    return False, None


def reason_is_valid(match: re.Match[str] | None) -> bool:
    if match is None:
        return False
    reason = match.group(1).strip()
    return len(reason) >= 20 and PLACEHOLDER_RE.fullmatch(reason) is None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--title", default="")
    parser.add_argument("--labels", default="")
    parser.add_argument("--body", default="")
    parser.add_argument("--author", default="")
    parser.add_argument("--approving-reviewers", default="")
    parser.add_argument("--commit-bodies")
    parser.add_argument(
        "--allow-unreviewed-reasoned-marker",
        action="store_true",
        help="Local commit-hook escape hatch; CI must never set this.",
    )
    args = parser.parse_args()

    repo = args.repo.resolve()
    files = changed_files(repo, args.base, args.head)
    # Discovery runs against --repo (the tree being judged), never against this
    # script's own location: in CI the script comes from the trusted base
    # checkout while --repo is the PR head (.github/workflows/regression-test-check.yml).
    touched = all_touched_paths(repo, args.base, args.head)
    # `resolve_prefixes` owns the workspace probe and returns None for a
    # non-workspace checkout; `main` reads that result rather than re-probing,
    # so there is exactly one place that decides "is this the IronClaw tree?".
    high_risk_prefixes = resolve_prefixes(repo, HIGH_RISK_ENTRIES, touched)
    static_asset_prefixes = resolve_prefixes(repo, STATIC_ASSET_ENTRIES, touched)
    if high_risk_prefixes is None:
        print(
            f"No [workspace] manifest at {repo}; high-risk path detection is "
            "inactive for this checkout and only the fix-commit trigger applies.",
            file=sys.stderr,
        )
        high_risk_prefixes = []
        static_asset_prefixes = []
    if args.commit_bodies is None:
        if args.head in {"INDEX", "WORKTREE"}:
            commit_bodies = ""
        else:
            commit_bodies = git(
                repo, "log", "--format=%B%x00", f"{args.base}..{args.head}"
            )
    else:
        commit_bodies = args.commit_bodies

    commit_subjects = "\n".join(
        body.strip().splitlines()[0]
        for body in commit_bodies.split("\0")
        if body.strip()
    )
    is_fix = bool(FIX_RE.search(args.title) or FIX_RE.search(commit_subjects))
    high_risk_matches = [
        prefix
        for prefix in high_risk_prefixes
        if any(prefix in path for path in files)
    ]
    if not is_fix and not high_risk_matches:
        print("Not a fix and no high-risk files changed; regression gate not required.")
        return 0

    if not files:
        print("No changed files; regression gate not required.")
        return 0
    if all(
        path.endswith(".md")
        or any(path.startswith(prefix) for prefix in static_asset_prefixes)
        for path in files
    ):
        print("Only documentation or static assets changed; regression gate not required.")
        return 0

    labels = {label.strip() for label in args.labels.split(",") if label.strip()}
    has_label = "skip-regression-check" in labels
    has_marker = MARKER_PREFIX in commit_bodies

    if has_label:
        if not reason_is_valid(BODY_REASON_RE.search(args.body)):
            print(
                "Regression exemption denied: add an explicit impossibility reason "
                "to the PR body using:\n"
                "Regression-test exemption: deterministic reproduction is impossible "
                "because <specific reason>",
                file=sys.stderr,
            )
            return 1
        reviewers = {
            reviewer.strip().casefold()
            for reviewer in args.approving_reviewers.split(",")
            if reviewer.strip()
        }
        reviewers.discard(args.author.strip().casefold())
        if not reviewers:
            print(
                "Regression exemption denied: the skip-regression-check label "
                "requires a non-author approving review.",
                file=sys.stderr,
            )
            return 1
        print("Reviewed regression exemption accepted.")
        return 0

    if has_marker:
        if not reason_is_valid(MARKER_RE.search(commit_bodies)):
            print(
                "Regression exemption denied: [skip-regression-check] requires "
                "an explicit impossibility reason using "
                "[skip-regression-check: deterministic reproduction is impossible "
                "because <specific reason>].",
                file=sys.stderr,
            )
            return 1
        if not args.allow_unreviewed_reasoned_marker:
            reviewers = {
                reviewer.strip().casefold()
                for reviewer in args.approving_reviewers.split(",")
                if reviewer.strip()
            }
            reviewers.discard(args.author.strip().casefold())
            if not reviewers:
                print(
                    "Regression exemption denied: a reasoned commit marker "
                    "requires a non-author approving review.",
                    file=sys.stderr,
                )
                return 1
        print("Reasoned regression exemption accepted.")
        return 0

    meaningful, path = meaningful_test_changed(
        repo, args.base, args.head, files
    )
    if meaningful:
        print(f"Meaningful changed regression assertion found in {path}.")
        return 0

    trigger = "fix" if is_fix else "high-risk change"
    if high_risk_matches:
        trigger += f"; high-risk paths: {', '.join(high_risk_matches)}"
    print(
        f"Regression test required ({trigger}): no meaningful changed regression "
        "assertion was found. Add a deterministic test that fails on the bug, or "
        "provide a justified exemption.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"regression-test-check: {error}", file=sys.stderr)
        raise SystemExit(2) from error
