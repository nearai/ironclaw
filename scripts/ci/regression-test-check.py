#!/usr/bin/env python3
"""Require meaningful regression evidence for fixes and high-risk changes."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


HIGH_RISK_PATTERNS = (
    "crates/ironclaw_turns/src/coordinator.rs",
    "crates/ironclaw_turns/src/status.rs",
    "crates/ironclaw_run_state/src/",
    "crates/ironclaw_processes/src/journal_store",
    "crates/ironclaw_processes/src/supervisor.rs",
    "crates/ironclaw_llm/src/circuit_breaker.rs",
    "crates/ironclaw_llm/src/retry.rs",
    "crates/ironclaw_llm/src/failover.rs",
    "crates/ironclaw_agent_loop/src/executor",
    "crates/ironclaw_agent_loop/src/state",
    "crates/ironclaw_safety/src/",
)
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
    text = without_comment_lines(text, ("//", "/*", "*"))
    for match in re.finditer(
        r"expect\s*\((.*?)\)\s*\.\s*"
        r"(to[A-Z]\w*|resolves|rejects)(?:\.\w+)?\s*\((.*?)\)",
        text,
        re.DOTALL,
    ):
        actual, _matcher, expected = match.groups()
        if normalized(actual) == normalized(expected) and expected.strip():
            continue
        if normalized(actual) in {"true", "1"} and normalized(expected) in {
            "true",
            "1",
        }:
            continue
        return True
    return bool(re.search(r"\b(assert|strictEqual|deepStrictEqual)\s*\(", text))


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
        pattern
        for pattern in HIGH_RISK_PATTERNS
        if any(pattern in path for path in files)
    ]
    if not is_fix and not high_risk_matches:
        print("Not a fix and no high-risk files changed; regression gate not required.")
        return 0

    if not files:
        print("No changed files; regression gate not required.")
        return 0
    if all(
        path.endswith(".md")
        or path.startswith("crates/ironclaw_webui/frontend/public/")
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
