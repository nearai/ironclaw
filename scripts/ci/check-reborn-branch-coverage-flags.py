#!/usr/bin/env python3
"""Require the crash-safe Rust/LLVM branch coverage command shape."""

from __future__ import annotations

import argparse
import pathlib
import sys

DEFAULT_PATHS = (
    pathlib.Path(".github/workflows/coverage.yml"),
    pathlib.Path(".github/workflows/reborn-tests.yml"),
    pathlib.Path("scripts/ci/reborn-coverage-lane-run.sh"),
)
PINNED_BRANCH_TOOLCHAIN = "nightly-2025-11-01"
COMPATIBILITY_FEATURES = (
    "-Zcrate-attr=feature("
    "array_windows,debug_closure_helpers,slice_as_array)"
)


def logical_lines(text: str) -> list[str]:
    return text.replace("\\\n", " ").splitlines()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("paths", nargs="*", type=pathlib.Path)
    args = parser.parse_args()
    paths = tuple(args.paths) or DEFAULT_PATHS
    exports: list[tuple[pathlib.Path, int, str]] = []
    toolchains: list[tuple[pathlib.Path, int, str]] = []
    bootstraps: list[tuple[pathlib.Path, int]] = []
    compatibility_flags: list[tuple[pathlib.Path, int]] = []
    failures: list[str] = []
    for path in paths:
        if not path.is_file():
            failures.append(f"branch coverage source is missing: {path}")
            continue
        source = path.read_text()
        if path.as_posix().endswith(".github/workflows/coverage.yml"):
            shipping_markers = (
                "cargo build --ignore-rust-version -p ironclaw --bin ironclaw",
                "cargo llvm-cov clean --profraw-only",
                'pytest "${reborn_e2e_tests[@]}"',
                "profraw_count=$(find target/ -name '*.profraw'",
            )
            marker_positions = [source.find(marker) for marker in shipping_markers]
            if any(position < 0 for position in marker_positions):
                failures.append(
                    f"{path}: shipping E2E coverage must build, clear build-time "
                    "profiles, run pytest, and then verify profraw discovery"
                )
            elif marker_positions != sorted(marker_positions):
                failures.append(
                    f"{path}: shipping E2E coverage must clear build-time profiles "
                    "after build and verify new profiles only after pytest"
                )
        for line_number, line in enumerate(logical_lines(source), start=1):
            stripped = line.strip()
            if stripped.startswith("toolchain: nightly-"):
                toolchain = stripped.removeprefix("toolchain: ")
                toolchains.append((path, line_number, toolchain))
                if toolchain != PINNED_BRANCH_TOOLCHAIN:
                    failures.append(
                        f"{path}:{line_number}: branch coverage must pin "
                        f"{PINNED_BRANCH_TOOLCHAIN} (LLVM 21.1.3); newer LLVM "
                        "versions crash on async generic branch mappings"
                    )
            if stripped == 'RUSTC_BOOTSTRAP: "1"':
                bootstraps.append((path, line_number))
            if COMPATIBILITY_FEATURES in stripped:
                compatibility_flags.append((path, line_number))
            if (
                "cargo llvm-cov" in line
                and "--branch" in line
                and "--lcov" in line
            ):
                exports.append((path, line_number, line.strip()))
                if "--skip-functions" not in line:
                    failures.append(
                        f"{path}:{line_number}: branch LCOV export must use "
                        "--skip-functions because the gate consumes only line "
                        "and branch records"
                    )
                if "llvm-cov report" not in line and "--ignore-rust-version" not in line:
                    failures.append(
                        f"{path}:{line_number}: the pinned coverage compiler "
                        "must pass --ignore-rust-version after the test "
                        "subcommand"
                    )
    if not exports:
        failures.append("no branch LCOV exports discovered")
    if len(toolchains) != 4:
        failures.append(
            f"expected 4 pinned branch coverage toolchains, found {len(toolchains)}"
        )
    if len(bootstraps) != 4:
        failures.append(
            f"expected 4 coverage-only RUSTC_BOOTSTRAP envelopes, "
            f"found {len(bootstraps)}"
        )
    if len(compatibility_flags) != 4:
        failures.append(
            f"expected 4 exact coverage compatibility feature sets, "
            f"found {len(compatibility_flags)}; required {COMPATIBILITY_FEATURES}"
        )
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(
        f"Reborn branch coverage export check passed "
        f"({len(exports)} exports, {len(toolchains)} LLVM 21.1.3 toolchains, "
        f"{len(compatibility_flags)} compatibility envelopes)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
