#!/usr/bin/env python3
"""Gate changed testable Reborn production lines and branch arms.

The denominator is mechanical:

* added lines come from ``git diff --unified=0 BASE...HEAD``;
* testable lines are the intersection with LLVM's ``DA`` records;
* changed branch arms are LLVM ``BRDA`` records located on those added lines.

An exact-line exemption is the only way to remove an otherwise-testable line
or branch from the denominator. Exemptions are deliberately review-heavy:
owner, reason, issue, review date, file, and explicit line numbers are
required. Whole files and globs are not accepted.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import json
import pathlib
import re
import subprocess
import sys
import tomllib

PRODUCTION_PATH = re.compile(r"^crates/ironclaw_[^/]+/src/.+\.rs$")
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
RAW_STRING_START = re.compile(r'(?:b|c)?r(#{0,255})"')
TEST_PATH_PARTS = {"/tests/", "/test_support/"}


class GateError(RuntimeError):
    pass


@dataclasses.dataclass(frozen=True)
class Exemption:
    path: str
    lines: frozenset[int]
    branch_lines: frozenset[int]


@dataclasses.dataclass
class Coverage:
    lines: dict[str, dict[int, int]]
    branches: dict[str, dict[tuple[int, str, str], int]]
    saw_branch_records: bool = False


def repo_relative(path: str, repo_root: pathlib.Path) -> str:
    normalized = pathlib.Path(path)
    if normalized.is_absolute():
        rendered = normalized.as_posix()
        root_prefix = repo_root.as_posix().rstrip("/") + "/"
        if rendered.startswith(root_prefix):
            return rendered.removeprefix(root_prefix)
        marker = "/crates/"
        if marker in rendered:
            return "crates/" + rendered.split(marker, 1)[1]
    return normalized.as_posix().lstrip("./")


def parse_lcov(path: pathlib.Path, repo_root: pathlib.Path) -> Coverage:
    if not path.is_file():
        raise GateError(f"coverage lcov file not found: {path}")
    coverage = Coverage(lines={}, branches={})
    current: str | None = None
    for raw in path.read_text(encoding="utf-8").splitlines():
        if raw.startswith("SF:"):
            current = repo_relative(raw[3:], repo_root)
            coverage.lines.setdefault(current, {})
            coverage.branches.setdefault(current, {})
        elif raw.startswith("DA:") and current is not None:
            line, hits, *_checksum = raw[3:].split(",")
            coverage.lines[current][int(line)] = int(hits)
        elif raw.startswith("BRDA:") and current is not None:
            line, block, branch, taken = raw[5:].split(",", 3)
            coverage.saw_branch_records = True
            coverage.branches[current][(int(line), block, branch)] = (
                0 if taken == "-" else int(taken)
            )
        elif raw == "end_of_record":
            current = None
    return coverage


def positive_ints(value: object, field: str) -> frozenset[int]:
    if value is None:
        return frozenset()
    if not isinstance(value, list) or not all(
        isinstance(item, int) and item > 0 for item in value
    ):
        raise GateError(f"{field} must be an array of positive integers")
    if len(set(value)) != len(value):
        raise GateError(f"{field} contains duplicate line numbers")
    return frozenset(value)


def load_manifest(path: pathlib.Path, repo_root: pathlib.Path) -> tuple[dict, list[Exemption]]:
    if not path.is_file():
        raise GateError(f"changed-coverage exemption manifest not found: {path}")
    with path.open("rb") as handle:
        manifest = tomllib.load(handle)
    policy = manifest.get("policy")
    if not isinstance(policy, dict):
        raise GateError("changed-coverage manifest missing required [policy] section")
    expected_policy = {"line_percent", "branch_percent"}
    if set(policy) != expected_policy:
        raise GateError(
            f"[policy] fields must be exactly {sorted(expected_policy)}, got {sorted(policy)}"
        )
    for field in expected_policy:
        value = policy[field]
        if not isinstance(value, (int, float)) or isinstance(value, bool) or not 0 <= value <= 100:
            raise GateError(f"[policy].{field} must be a number in [0, 100]")

    raw_entries = manifest.get("exemption", [])
    if not isinstance(raw_entries, list):
        raise GateError("exemptions must use [[exemption]] array-of-tables")
    exemptions: list[Exemption] = []
    seen: set[tuple[str, int, str]] = set()
    today = dt.date.today()
    for index, entry in enumerate(raw_entries):
        if not isinstance(entry, dict):
            raise GateError(f"exemption #{index + 1} must be a table")
        required = {"path", "owner", "reason", "issue", "review_after"}
        allowed = required | {"lines", "branch_lines"}
        missing = required - set(entry)
        unknown = set(entry) - allowed
        if missing or unknown:
            raise GateError(
                f"exemption #{index + 1} has missing={sorted(missing)} unknown={sorted(unknown)}"
            )
        path_value = entry["path"]
        if not isinstance(path_value, str) or not PRODUCTION_PATH.fullmatch(path_value):
            raise GateError(
                f"exemption #{index + 1} path must name one Reborn production .rs file"
            )
        if not (repo_root / path_value).is_file():
            raise GateError(f"exemption #{index + 1} names stale path: {path_value}")
        for field in ("owner", "reason", "issue"):
            if not isinstance(entry[field], str) or not entry[field].strip():
                raise GateError(f"exemption #{index + 1} missing non-empty {field}")
        if not entry["issue"].startswith("https://github.com/nearai/ironclaw/issues/"):
            raise GateError(f"exemption #{index + 1} issue must be an Ironclaw issue URL")
        try:
            review_after = dt.date.fromisoformat(entry["review_after"])
        except (TypeError, ValueError) as error:
            raise GateError(
                f"exemption #{index + 1} review_after must be YYYY-MM-DD"
            ) from error
        if review_after < today:
            raise GateError(
                f"exemption #{index + 1} review date has expired: {review_after}"
            )
        lines = positive_ints(entry.get("lines"), f"exemption #{index + 1}.lines")
        branch_lines = positive_ints(
            entry.get("branch_lines"), f"exemption #{index + 1}.branch_lines"
        )
        if not lines and not branch_lines:
            raise GateError(
                f"exemption #{index + 1} must name lines and/or branch_lines"
            )
        source_line_count = len((repo_root / path_value).read_text(encoding="utf-8").splitlines())
        if any(line > source_line_count for line in lines | branch_lines):
            raise GateError(
                f"exemption #{index + 1} names a line beyond current EOF ({source_line_count})"
            )
        for line in lines:
            key = (path_value, line, "line")
            if key in seen:
                raise GateError(f"duplicate changed-line exemption: {path_value}:{line}")
            seen.add(key)
        for line in branch_lines:
            key = (path_value, line, "branch")
            if key in seen:
                raise GateError(f"duplicate changed-branch exemption: {path_value}:{line}")
            seen.add(key)
        exemptions.append(Exemption(path_value, lines, branch_lines))
    return policy, exemptions


def parse_diff(text: str) -> dict[str, set[int]]:
    added: dict[str, set[int]] = {}
    current: str | None = None
    new_line: int | None = None
    for raw in text.splitlines():
        if raw.startswith("diff --git "):
            current = None
            new_line = None
        elif new_line is None and raw.startswith("+++ b/"):
            candidate = raw[6:]
            current = candidate if PRODUCTION_PATH.fullmatch(candidate) else None
            if current is not None:
                added.setdefault(current, set())
            new_line = None
        elif raw.startswith("@@ "):
            match = HUNK.match(raw)
            if current is not None and match is None:
                raise GateError(f"malformed diff hunk header: {raw}")
            new_line = int(match.group(1)) if match and current is not None else None
        elif new_line is not None and current is not None:
            marker = raw[:1]
            if marker == "+":
                added[current].add(new_line)
                new_line += 1
            elif marker == "-":
                continue
            elif marker == " ":
                new_line += 1
            elif raw != r"\ No newline at end of file":
                raise GateError(f"malformed diff hunk line: {raw}")
    return added


def test_only_path(path: str) -> bool:
    return (
        any(part in path for part in TEST_PATH_PARTS)
        or path.endswith("/tests.rs")
        or path.endswith("_tests.rs")
    )


def rust_lexical_structure(source: str) -> tuple[list[str], list[int]]:
    """Return comment/literal-free Rust lines and structural brace deltas.

    Braces inside nested comments, quoted strings, raw strings, and character
    literals are ignored and their contents are replaced with spaces. This is
    the small lexer needed by the coverage gate; it deliberately does not
    attempt to parse Rust expressions or types.
    """
    sanitized = list(source)
    deltas = [0] * (source.count("\n") + 1)
    line = 0
    index = 0
    block_comment_depth = 0
    length = len(source)
    while index < length:
        character = source[index]
        following = source[index + 1] if index + 1 < length else ""
        if character == "\n":
            line += 1
            index += 1
            continue
        if block_comment_depth:
            sanitized[index] = " "
            if character == "/" and following == "*":
                sanitized[index + 1] = " "
                block_comment_depth += 1
                index += 2
            elif character == "*" and following == "/":
                sanitized[index + 1] = " "
                block_comment_depth -= 1
                index += 2
            else:
                index += 1
            continue
        if character == "/" and following == "/":
            newline = source.find("\n", index + 2)
            end = length if newline < 0 else newline
            sanitized[index:end] = " " * (end - index)
            index = end
            continue
        if character == "/" and following == "*":
            sanitized[index : index + 2] = [" ", " "]
            block_comment_depth = 1
            index += 2
            continue

        raw_match = None
        if index == 0 or not (source[index - 1].isalnum() or source[index - 1] == "_"):
            raw_match = RAW_STRING_START.match(source, index)
        if raw_match is not None:
            terminator = '"' + raw_match.group(1)
            content_start = raw_match.end()
            content_end = source.find(terminator, content_start)
            end = length if content_end < 0 else content_end + len(terminator)
            for position in range(index, end):
                if source[position] != "\n":
                    sanitized[position] = " "
            line += source[index:end].count("\n")
            index = end
            continue
        if character == '"':
            start = index
            if (
                index > 0
                and source[index - 1] in {"b", "c"}
                and (
                    index == 1
                    or not (
                        source[index - 2].isalnum() or source[index - 2] == "_"
                    )
                )
            ):
                start -= 1
            index += 1
            escaped = False
            while index < length:
                character = source[index]
                if character == "\n":
                    line += 1
                if escaped:
                    escaped = False
                elif character == "\\":
                    escaped = True
                elif character == '"':
                    index += 1
                    break
                index += 1
            for position in range(start, index):
                if source[position] != "\n":
                    sanitized[position] = " "
            continue
        if character == "'":
            end = index + 1
            escaped = False
            while end < length and source[end] != "\n":
                if escaped:
                    escaped = False
                elif source[end] == "\\":
                    escaped = True
                elif source[end] == "'":
                    end += 1
                    break
                end += 1
            if end <= length and end > index + 1 and source[end - 1 : end] == "'":
                sanitized[index:end] = " " * (end - index)
                index = end
                continue
        if character == "{":
            deltas[line] += 1
        elif character == "}":
            deltas[line] -= 1
        index += 1
    return "".join(sanitized).splitlines(), deltas


def mechanically_uninstrumentable_lines(source: str) -> set[int]:
    """Return Rust scaffolding spans that LLVM cannot execute."""
    lexical_lines, _brace_deltas = rust_lexical_structure(source)
    uninstrumentable: set[int] = set()
    attribute_depth = 0
    in_use = False
    for line_number, line in enumerate(lexical_lines, start=1):
        stripped = line.strip()
        if attribute_depth:
            uninstrumentable.add(line_number)
            attribute_depth += stripped.count("[") - stripped.count("]")
            continue
        if stripped.startswith("#["):
            uninstrumentable.add(line_number)
            attribute_depth = stripped.count("[") - stripped.count("]")
            continue
        if in_use:
            uninstrumentable.add(line_number)
            if ";" in stripped:
                in_use = False
            continue
        if re.match(r"^(?:pub(?:\([^)]*\))?\s+)?use\b", stripped):
            uninstrumentable.add(line_number)
            in_use = ";" not in stripped
            continue
        if re.fullmatch(
            r"(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?mod\s+[A-Za-z_]\w*\s*;",
            stripped,
        ):
            uninstrumentable.add(line_number)
            continue
        if not stripped or re.fullmatch(r"[\[\]{}(),;]+", stripped):
            uninstrumentable.add(line_number)
    return uninstrumentable


def rust_brace_deltas(source: str) -> list[int]:
    """Return structural Rust brace deltas per line."""
    _lexical_lines, deltas = rust_lexical_structure(source)
    return deltas


def cfg_test_only_lines(path: pathlib.Path) -> set[int]:
    """Best-effort Rust item span for exact ``#[cfg(test)]`` blocks.

    This keeps inline unit-test modules out of a production changed-code
    denominator. It intentionally fails conservative: if the following item
    has no braced body, only the attribute/item lines are excluded.
    """
    source = path.read_text(encoding="utf-8")
    lines = source.splitlines()
    brace_deltas = rust_brace_deltas(source)
    excluded: set[int] = set()
    pending_start: int | None = None
    depth = 0
    saw_braced_body = False
    for index, line in enumerate(lines, start=1):
        stripped = line.strip()
        if pending_start is None and re.fullmatch(
            r"#\[cfg\(\s*test\s*\)\]", stripped
        ):
            pending_start = index
            depth = 0
            saw_braced_body = False
        if pending_start is None:
            continue
        excluded.add(index)
        delta = brace_deltas[index - 1]
        if delta > 0:
            saw_braced_body = True
        depth += delta
        if saw_braced_body and depth > 0:
            continue
        if saw_braced_body:
            pending_start = None
        elif index > pending_start and stripped and not stripped.startswith("#["):
            pending_start = None
    return excluded


def git_diff(repo_root: pathlib.Path, base: str, head: str) -> str:
    command = [
        "git",
        "diff",
        "--unified=0",
        "--diff-filter=AMR",
        f"{base}...{head}",
        "--",
        "crates/ironclaw_*/src/**/*.rs",
        "crates/ironclaw_*/src/*.rs",
    ]
    result = subprocess.run(
        command, cwd=repo_root, text=True, capture_output=True, check=False
    )
    if result.returncode != 0:
        raise GateError(f"git diff failed: {result.stderr.strip()}")
    return result.stdout


def percent(hit: int, total: int) -> float:
    if total == 0:
        raise GateError("changed coverage denominator is empty")
    return hit / total * 100


def write_json_report(path: pathlib.Path | None, report: dict[str, object]) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lcov", required=True, type=pathlib.Path)
    parser.add_argument("--manifest", required=True, type=pathlib.Path)
    parser.add_argument("--base")
    parser.add_argument("--head")
    parser.add_argument("--diff-file", type=pathlib.Path)
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--json", type=pathlib.Path)
    args = parser.parse_args()
    try:
        if args.diff_file and (args.base or args.head):
            raise GateError("--diff-file cannot be combined with --base/--head")
        if not args.diff_file and (not args.base or not args.head):
            raise GateError("provide --diff-file or both --base and --head")
        repo_root = args.repo_root.resolve()
        policy, exemptions = load_manifest(args.manifest, repo_root)
        coverage = parse_lcov(args.lcov, repo_root)
        diff_text = (
            args.diff_file.read_text(encoding="utf-8")
            if args.diff_file
            else git_diff(repo_root, args.base, args.head)
        )
        changed = {
            path: lines
            for path, lines in parse_diff(diff_text).items()
            if not test_only_path(path)
        }
        if not coverage.saw_branch_records:
            raise GateError(
                "merged lcov contains no BRDA records; branch instrumentation is missing"
            )
        for path in list(changed):
            changed[path] -= cfg_test_only_lines(repo_root / path)
            if not changed[path]:
                del changed[path]
        if not changed:
            write_json_report(
                args.json,
                {
                    "schema_version": 2,
                    "threshold_percent": float(policy["line_percent"]),
                    "coverage_percent": 100.0,
                    "covered_lines": 0,
                    "instrumented_lines": 0,
                    "branch_threshold_percent": float(policy["branch_percent"]),
                    "branch_coverage_percent": 100.0,
                    "covered_branches": 0,
                    "instrumented_branches": 0,
                    "changed_product_files": [],
                    "missing_files": [],
                    "uncovered": [],
                    "uncovered_branches": [],
                    "passed": True,
                },
            )
            print("Changed coverage: no Reborn production lines added")
            return 0

        exempt_lines = {
            (entry.path, line) for entry in exemptions for line in entry.lines
        }
        exempt_branch_lines = {
            (entry.path, line) for entry in exemptions for line in entry.branch_lines
        }
        line_total = line_hit = branch_total = branch_hit = 0
        uncovered_lines: list[str] = []
        uncovered_branches: list[str] = []
        uninstrumented_files: list[str] = []
        empty_denominator_files: list[str] = []
        for path, added_lines in sorted(changed.items()):
            uninstrumentable_lines = mechanically_uninstrumentable_lines(
                (repo_root / path).read_text(encoding="utf-8")
            )
            candidate_lines = {
                line for line in added_lines if (path, line) not in exempt_lines
            }
            instrumented = coverage.lines.get(path)
            if not instrumented:
                if candidate_lines and candidate_lines <= uninstrumentable_lines:
                    continue
                uninstrumented_files.append(path)
                continue
            measured_lines = candidate_lines & set(instrumented)
            first_instrumented = min(instrumented)
            last_instrumented = max(instrumented)
            candidate_lines_in_instrumented_span = {
                line
                for line in candidate_lines
                if first_instrumented <= line <= last_instrumented
                and line not in uninstrumentable_lines
            }
            if candidate_lines_in_instrumented_span and not measured_lines:
                empty_denominator_files.append(path)
                continue
            for line in sorted(measured_lines):
                line_total += 1
                if instrumented[line] > 0:
                    line_hit += 1
                else:
                    uncovered_lines.append(f"{path}:{line}")
            for (line, block, branch), hits in sorted(
                coverage.branches.get(path, {}).items()
            ):
                if line not in added_lines or (path, line) in exempt_branch_lines:
                    continue
                branch_total += 1
                if hits > 0:
                    branch_hit += 1
                else:
                    uncovered_branches.append(f"{path}:{line} branch {block}/{branch}")

        if uninstrumented_files or empty_denominator_files:
            line_pct = 0.0 if line_total == 0 else percent(line_hit, line_total)
        elif line_total == 0:
            # A reviewed exact-line exemption may intentionally remove every
            # changed line. Unmeasured production changes are handled above.
            line_pct = 100.0
        else:
            line_pct = percent(line_hit, line_total)
        branch_pct = 100.0 if branch_total == 0 else percent(branch_hit, branch_total)
        print(f"Changed line coverage: {line_pct:.2f}% ({line_hit}/{line_total})")
        print(f"Changed branch coverage: {branch_pct:.2f}% ({branch_hit}/{branch_total})")
        failures: list[str] = []
        if uninstrumented_files:
            failures.append(
                "changed production files are absent from coverage or contain no DA records: "
                + ", ".join(uninstrumented_files)
            )
        if empty_denominator_files:
            failures.append(
                "changed production files contributed no instrumented lines: "
                + ", ".join(empty_denominator_files)
            )
        if line_pct < float(policy["line_percent"]):
            failures.append(
                f"line coverage {line_pct:.2f}% is below {policy['line_percent']}%"
            )
        if branch_pct < float(policy["branch_percent"]):
            failures.append(
                f"branch coverage {branch_pct:.2f}% is below {policy['branch_percent']}%"
            )
        if uncovered_lines:
            print("Uncovered changed lines:", file=sys.stderr)
            print("\n".join(f"  {item}" for item in uncovered_lines), file=sys.stderr)
        if uncovered_branches:
            print("Uncovered changed branches:", file=sys.stderr)
            print("\n".join(f"  {item}" for item in uncovered_branches), file=sys.stderr)
        write_json_report(
            args.json,
            {
                "schema_version": 2,
                "threshold_percent": float(policy["line_percent"]),
                "coverage_percent": round(line_pct, 2),
                "covered_lines": line_hit,
                "instrumented_lines": line_total,
                "branch_threshold_percent": float(policy["branch_percent"]),
                "branch_coverage_percent": round(branch_pct, 2),
                "covered_branches": branch_hit,
                "instrumented_branches": branch_total,
                "changed_product_files": sorted(changed),
                "missing_files": uninstrumented_files,
                "uncovered": uncovered_lines,
                "uncovered_branches": uncovered_branches,
                "passed": not failures,
            },
        )
        if failures:
            raise GateError("; ".join(failures))
        return 0
    except (GateError, tomllib.TOMLDecodeError, OSError) as error:
        print(f"changed coverage gate failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
