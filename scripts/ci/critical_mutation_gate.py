#!/usr/bin/env python3
"""Run cargo-mutants only for changed, explicitly named critical functions."""

from __future__ import annotations

import argparse
import collections
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import tomllib

MANIFEST_PATH = "tests/integration/critical-mutation-functions.toml"
GATE_PATH = "scripts/ci/critical_mutation_gate.py"
SELF_TEST_PATH = "scripts/ci/test-critical-mutation-gate.sh"
PRODUCTION_PATH = re.compile(r"^crates/ironclaw_[^/]+/src/.+\.rs$")
PACKAGE = re.compile(r"^ironclaw_[a-z0-9_]+$")
FUNCTION = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


class GateError(RuntimeError):
    pass


def run(command: list[str], cwd: pathlib.Path) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    # cargo-mutants creates isolated copies of the source tree. A shared target
    # directory can leak incompatible build artifacts into those copies.
    environment.pop("CARGO_TARGET_DIR", None)
    return subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        text=True,
        capture_output=True,
        check=False,
    )


def load_manifest(path: pathlib.Path, repo_root: pathlib.Path) -> list[dict[str, object]]:
    if not path.is_file():
        raise GateError(f"critical mutation manifest not found: {path}")
    with path.open("rb") as handle:
        document = tomllib.load(handle)
    entries = document.get("invariant")
    if not isinstance(entries, list) or not entries:
        raise GateError("manifest must contain at least one [[invariant]] entry")
    required = {"domain", "package", "path", "function", "rationale"}
    allowed = required | {"test_args", "watch_paths"}
    seen_domains: set[str] = set()
    seen_functions: set[tuple[str, str]] = set()
    validated: list[dict[str, object]] = []
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict) or not required <= set(entry) or set(entry) - allowed:
            actual = sorted(entry) if isinstance(entry, dict) else type(entry).__name__
            raise GateError(
                f"invariant #{index + 1} fields must contain {sorted(required)} "
                f"and optional test_args/watch_paths only, got {actual}"
            )
        if not all(isinstance(entry[key], str) and entry[key].strip() for key in required):
            raise GateError(f"invariant #{index + 1} fields must be non-empty strings")
        if entry["domain"] in seen_domains:
            raise GateError(f"duplicate critical invariant domain: {entry['domain']}")
        seen_domains.add(entry["domain"])
        if not PACKAGE.fullmatch(entry["package"]):
            raise GateError(f"invalid package in invariant #{index + 1}: {entry['package']}")
        if not PRODUCTION_PATH.fullmatch(entry["path"]):
            raise GateError(f"invalid production path in invariant #{index + 1}: {entry['path']}")
        if not (repo_root / entry["path"]).is_file():
            raise GateError(f"stale critical invariant path: {entry['path']}")
        if not FUNCTION.fullmatch(entry["function"]):
            raise GateError(f"invalid function name in invariant #{index + 1}: {entry['function']}")
        test_args = entry.get("test_args", [])
        if not isinstance(test_args, list) or not all(
            isinstance(value, str) and value.strip() for value in test_args
        ):
            raise GateError(f"invariant #{index + 1} test_args must be non-empty strings")
        watch_paths = entry.get("watch_paths", [])
        if not isinstance(watch_paths, list) or not all(
            isinstance(value, str) and value.strip() for value in watch_paths
        ):
            raise GateError(f"invariant #{index + 1} watch_paths must be non-empty strings")
        if len(set(watch_paths)) != len(watch_paths):
            raise GateError(f"invariant #{index + 1} watch_paths contains duplicates")
        package_root = f"crates/{entry['package']}/"
        for watch_path in watch_paths:
            if (
                not watch_path.startswith(package_root)
                or "\\" in watch_path
                or ".." in pathlib.PurePosixPath(watch_path).parts
            ):
                raise GateError(
                    f"invariant #{index + 1} watch path must stay inside "
                    f"{package_root}: {watch_path}"
                )
            if not (repo_root / watch_path).is_file():
                raise GateError(f"stale critical invariant watch path: {watch_path}")
        key = (entry["path"], entry["function"])
        if key in seen_functions:
            raise GateError(f"duplicate critical function: {entry['path']}::{entry['function']}")
        seen_functions.add(key)
        validated.append(entry)
    return validated


def exact_mutant_pattern(mutants: list[str]) -> str:
    # Python's re.escape also escapes spaces, which is not portable to every
    # regex engine accepted by cargo-mutants. Escape only regex metacharacters.
    alternatives = "|".join(
        re.sub(r"([\\.^$|?*+()\[\]{}])", r"\\\1", mutant) for mutant in mutants
    )
    return rf"^({alternatives})$"


def changed_files(
    repo_root: pathlib.Path,
    base: str | None,
    head: str | None,
    changed_file: pathlib.Path | None,
) -> set[str]:
    if changed_file is not None:
        if not changed_file.is_file():
            raise GateError(f"changed-files input not found: {changed_file}")
        return {
            line.strip()
            for line in changed_file.read_text(encoding="utf-8").splitlines()
            if line.strip()
        }
    if not base or not head:
        raise GateError("provide --all, --changed-files, or both --base and --head")
    result = run(["git", "diff", "--name-only", f"{base}...{head}"], repo_root)
    if result.returncode != 0:
        raise GateError(f"git diff failed: {result.stderr.strip()}")
    return {line for line in result.stdout.splitlines() if line}


def report_lines(report_dir: pathlib.Path, name: str) -> list[str]:
    path = report_dir / f"{name}.txt"
    if not path.is_file():
        raise GateError(f"cargo-mutants result missing: {path}")
    return [line for line in path.read_text(encoding="utf-8").splitlines() if line]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=pathlib.Path, default=pathlib.Path(MANIFEST_PATH))
    parser.add_argument("--repo-root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--base")
    parser.add_argument("--head")
    parser.add_argument("--changed-files", type=pathlib.Path)
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--selection-only", action="store_true")
    parser.add_argument("--jobs", type=int, default=3)
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args()
    try:
        if args.all and (args.base or args.head or args.changed_files):
            raise GateError("--all cannot be combined with diff-selection arguments")
        if args.jobs < 1 or args.timeout < 1:
            raise GateError("--jobs and --timeout must be positive integers")
        repo_root = args.repo_root.resolve()
        entries = load_manifest(args.manifest, repo_root)
        if args.all:
            selected = entries
        else:
            changed = changed_files(repo_root, args.base, args.head, args.changed_files)
            force_all = bool({MANIFEST_PATH, GATE_PATH, SELF_TEST_PATH} & changed)
            selected = (
                entries
                if force_all
                else [
                    entry
                    for entry in entries
                    if changed
                    & {
                        entry["path"],
                        *entry.get("watch_paths", []),
                    }
                ]
            )
        if args.selection_only:
            print("true" if selected else "false")
            return 0
        if not selected:
            print("Critical mutation gate: no named invariant source or watch path changed")
            return 0

        grouped: dict[tuple[str, str, tuple[str, ...]], list[dict[str, object]]] = (
            collections.defaultdict(list)
        )
        for entry in selected:
            test_args = tuple(entry.get("test_args", []))
            grouped[(entry["package"], entry["path"], test_args)].append(entry)

        for (package, path, test_args), group in sorted(grouped.items()):
            listed = run(
                [
                    "cargo",
                    "mutants",
                    "-p",
                    package,
                    "-f",
                    path,
                    "--list",
                    "--json",
                ],
                repo_root,
            )
            if listed.returncode != 0:
                raise GateError(
                    f"could not enumerate critical functions in {path}: "
                    f"{listed.stderr.strip()}"
                )
            try:
                listed_entries = json.loads(listed.stdout)
            except json.JSONDecodeError as error:
                raise GateError(
                    f"cargo-mutants returned malformed JSON for {path}: {error}"
                ) from error
            if not isinstance(listed_entries, list):
                raise GateError(f"cargo-mutants JSON must be a list for {path}")
            listed_mutants: list[tuple[str, str]] = []
            for index, listed_entry in enumerate(listed_entries):
                if not isinstance(listed_entry, dict):
                    raise GateError(
                        f"cargo-mutants JSON entry #{index + 1} must be an object for {path}"
                    )
                name = listed_entry.get("name")
                function = listed_entry.get("function")
                if not isinstance(name, str) or not name.strip():
                    raise GateError(
                        f"cargo-mutants JSON entry #{index + 1} lacks name for {path}"
                    )
                if function is None:
                    continue
                function_name = (
                    function.get("function_name") if isinstance(function, dict) else None
                )
                if not isinstance(function_name, str) or not function_name.strip():
                    raise GateError(
                        f"cargo-mutants JSON entry #{index + 1} lacks "
                        f"function.function_name for {path}"
                    )
                listed_mutants.append((name, function_name.rsplit("::", 1)[-1]))
            selected_mutants: list[str] = []
            for entry in group:
                function_mutants = [
                    name
                    for name, function_name in listed_mutants
                    if function_name == entry["function"]
                ]
                if not function_mutants:
                    raise GateError(
                        f"named critical function produced zero mutants: "
                        f"{path}::{entry['function']}"
                    )
                selected_mutants.extend(function_mutants)

            pattern = exact_mutant_pattern(selected_mutants)
            print(
                f"Critical mutation gate: {package} {path} "
                f"({', '.join(entry['domain'] for entry in group)})",
                flush=True,
            )
            with tempfile.TemporaryDirectory(prefix="ironclaw-critical-mutants-") as temp:
                result = run(
                    [
                        "cargo",
                        "mutants",
                        "-p",
                        package,
                        "-f",
                        path,
                        "--re",
                        pattern,
                        "--timeout",
                        str(args.timeout),
                        "--jobs",
                        str(args.jobs),
                        "--output",
                        temp,
                        "--annotations",
                        "github",
                        *(
                            argument
                            for test_arg in test_args
                            for argument in ("--cargo-test-arg", test_arg)
                        ),
                    ],
                    repo_root,
                )
                # 0 = all caught, 2 = survivors, and 3 = timeouts. The latter
                # two are verdicts that must be read from mutants.out.
                if result.returncode not in (0, 2, 3):
                    raise GateError(
                        f"cargo-mutants failed for {path} with status {result.returncode}: "
                        f"{result.stderr.strip() or result.stdout.strip()}"
                    )
                report = pathlib.Path(temp) / "mutants.out"
                missed = report_lines(report, "missed")
                caught = report_lines(report, "caught")
                unviable = report_lines(report, "unviable")
                timed_out = report_lines(report, "timeout")
                total = len(missed) + len(caught) + len(unviable) + len(timed_out)
                if total == 0:
                    raise GateError(f"critical mutation run produced zero mutants for {path}")
                reported_mutants = missed + caught + unviable + timed_out
                if set(reported_mutants) != set(selected_mutants) or len(
                    reported_mutants
                ) != len(selected_mutants):
                    raise GateError(
                        "cargo-mutants results did not match the exact named-mutant allowlist"
                    )
                if timed_out:
                    raise GateError(
                        "critical mutants timed out (no passing verdict):\n  "
                        + "\n  ".join(timed_out)
                    )
                if missed:
                    raise GateError(
                        "behavior-changing critical mutants survived:\n  "
                        + "\n  ".join(missed)
                    )
                print(
                    f"  PASS: {len(caught)} caught; {len(unviable)} unviable; "
                    "0 survived; 0 timed out"
                )
        return 0
    except (GateError, tomllib.TOMLDecodeError, OSError) as error:
        print(f"critical mutation gate failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
