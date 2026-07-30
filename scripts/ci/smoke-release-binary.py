#!/usr/bin/env python3
"""Exercise the exact native IronClaw binary that a release job will package."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tarfile
import tempfile
from collections.abc import Callable
from pathlib import Path

REQUIRED_EVIDENCE = frozenset(
    {
        "version",
        "help",
        "profiles",
        "bundled_extensions",
        "local_libsql_migrations",
        "runtime_assembly",
        "migration_profile",
    }
)
REQUIRED_BUNDLED_RUNTIME_KINDS = frozenset({"first_party", "mcp_server", "wasm_tool"})
_PASSTHROUGH_ENV = (
    "PATH",
    "SystemRoot",
    "WINDIR",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
)


class SmokeFailure(RuntimeError):
    """The release binary did not satisfy the packaged-product smoke contract."""


Runner = Callable[
    [Path, tuple[str, ...], dict[str, str]], subprocess.CompletedProcess[str]
]


def _run_command(
    binary: Path,
    args: tuple[str, ...],
    environment: dict[str, str],
) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            [str(binary), *args],
            check=False,
            capture_output=True,
            text=True,
            env=environment,
            timeout=120,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise SmokeFailure(
            f"could not execute {binary.name} {' '.join(args)}: {error}"
        ) from error


def _checked_output(
    binary: Path,
    args: tuple[str, ...],
    environment: dict[str, str],
    runner: Runner,
) -> str:
    result = runner(binary, args, environment)
    if result.returncode != 0:
        stdout = result.stdout[-4000:]
        stderr = result.stderr[-4000:]
        raise SmokeFailure(
            f"{binary.name} {' '.join(args)} exited {result.returncode}\n"
            f"stdout:\n{stdout}\nstderr:\n{stderr}"
        )
    return result.stdout


def _parse_json_object(output: str, label: str) -> dict[str, object]:
    try:
        value = json.loads(output)
    except json.JSONDecodeError as error:
        raise SmokeFailure(f"{label} did not emit valid JSON: {error}") from error
    if not isinstance(value, dict):
        raise SmokeFailure(f"{label} must emit a JSON object")
    return value


def _validate_profiles(output: str) -> None:
    payload = _parse_json_object(output, "profile list --json")
    profiles = payload.get("profiles")
    if not isinstance(profiles, list) or not profiles:
        raise SmokeFailure("profile list --json emitted no supported profiles")
    names = {
        profile.get("name")
        for profile in profiles
        if isinstance(profile, dict) and isinstance(profile.get("name"), str)
    }
    required = {"local-dev", "production", "migration-dry-run"}
    missing = sorted(required - names)
    if missing:
        raise SmokeFailure("profile list --json is missing: " + ", ".join(missing))


def _validate_bundled_extensions(output: str) -> None:
    payload = _parse_json_object(output, "extension search --json")
    lifecycle_payload = payload.get("payload")
    if not isinstance(lifecycle_payload, dict):
        raise SmokeFailure("extension search --json has no lifecycle payload")
    extensions = lifecycle_payload.get("extensions")
    if not isinstance(extensions, list) or not extensions:
        raise SmokeFailure("shipping binary exposed no bundled extensions")

    ids: list[str] = []
    runtime_kinds: set[str] = set()
    for extension in extensions:
        if not isinstance(extension, dict):
            raise SmokeFailure("extension search returned a non-object entry")
        if extension.get("source") != "host_bundled":
            raise SmokeFailure("extension search returned a non-bundled package")
        package_ref = extension.get("package_ref")
        extension_id = package_ref.get("id") if isinstance(package_ref, dict) else None
        if not isinstance(extension_id, str) or not extension_id.strip():
            raise SmokeFailure(
                "extension search returned an entry without a package id"
            )
        runtime_kind = extension.get("runtime_kind")
        if not isinstance(runtime_kind, str) or not runtime_kind.strip():
            raise SmokeFailure(
                f"bundled extension {extension_id!r} has no runtime kind"
            )
        ids.append(extension_id)
        runtime_kinds.add(runtime_kind)
    if len(ids) != len(set(ids)):
        raise SmokeFailure("extension search returned duplicate package ids")
    missing_runtime_kinds = sorted(REQUIRED_BUNDLED_RUNTIME_KINDS - runtime_kinds)
    if missing_runtime_kinds:
        raise SmokeFailure(
            "shipping binary is missing bundled runtime kinds: "
            + ", ".join(missing_runtime_kinds)
        )


def _isolated_environment(root: Path) -> dict[str, str]:
    environment = {
        key: value for key in _PASSTHROUGH_ENV if (value := os.environ.get(key))
    }
    home = root / "home"
    reborn_home = root / "reborn-home"
    workspace = root / "workspace"
    home.mkdir()
    workspace.mkdir()
    environment.update(
        {
            "HOME": str(home),
            "USERPROFILE": str(home),
            "IRONCLAW_REBORN_HOME": str(reborn_home),
            "IRONCLAW_DISABLE_OS_KEYCHAIN": "1",
            "TZ": "UTC",
            "LANG": environment.get("LANG", "C.UTF-8"),
        }
    )
    return environment


def smoke_release_binary(binary: Path, runner: Runner = _run_command) -> set[str]:
    binary = binary.resolve()
    if not binary.is_file():
        raise SmokeFailure(f"shipping binary does not exist: {binary}")

    evidence: set[str] = set()
    with tempfile.TemporaryDirectory(prefix="ironclaw-release-smoke-") as temp:
        root = Path(temp)
        environment = _isolated_environment(root)

        version = _checked_output(binary, ("--version",), environment, runner)
        if "ironclaw" not in version.lower():
            raise SmokeFailure("--version did not identify IronClaw")
        evidence.add("version")

        help_output = _checked_output(binary, ("--help",), environment, runner)
        for command in ("serve", "run", "extension", "profile"):
            if command not in help_output:
                raise SmokeFailure(f"--help is missing the {command!r} command")
        evidence.add("help")

        profiles = _checked_output(
            binary, ("profile", "list", "--json"), environment, runner
        )
        _validate_profiles(profiles)
        evidence.add("profiles")

        extensions = _checked_output(
            binary, ("extension", "search", "--json"), environment, runner
        )
        _validate_bundled_extensions(extensions)
        evidence.add("bundled_extensions")
        evidence.add("runtime_assembly")
        databases = list((root / "reborn-home").rglob("*.db"))
        if len(databases) != 1 or databases[0].stat().st_size == 0:
            raise SmokeFailure(
                "runtime assembly did not create exactly one non-empty local libSQL database"
            )
        evidence.add("local_libsql_migrations")

        migration_environment = dict(environment)
        migration_environment["IRONCLAW_REBORN_PROFILE"] = "migration-dry-run"
        migration = _checked_output(
            binary, ("run", "--dry-run"), migration_environment, runner
        )
        if "profile: migration-dry-run" not in migration:
            raise SmokeFailure(
                "migration dry-run did not report the migration-dry-run profile"
            )
        evidence.add("migration_profile")

    missing = sorted(REQUIRED_EVIDENCE - evidence)
    if missing:
        raise SmokeFailure(
            "release smoke skipped required evidence: " + ", ".join(missing)
        )
    return evidence


def smoke_release_archive(
    archive: Path,
    binary_name: str,
    runner: Runner = _run_command,
) -> set[str]:
    archive = archive.resolve()
    if not archive.is_file():
        raise SmokeFailure(f"release archive does not exist: {archive}")
    if Path(binary_name).name != binary_name or binary_name in {"", ".", ".."}:
        raise SmokeFailure(f"invalid release binary name: {binary_name!r}")

    try:
        with tarfile.open(archive, mode="r:gz") as package:
            matches = [
                member
                for member in package.getmembers()
                if member.isfile() and Path(member.name).name == binary_name
            ]
            if len(matches) != 1:
                raise SmokeFailure(
                    f"{archive.name} must contain exactly one {binary_name}; "
                    f"found {len(matches)}"
                )
            source = package.extractfile(matches[0])
            if source is None:
                raise SmokeFailure(f"could not read {binary_name} from {archive.name}")
            with tempfile.TemporaryDirectory(
                prefix="ironclaw-release-archive-"
            ) as temp:
                extracted = Path(temp) / binary_name
                extracted.write_bytes(source.read())
                extracted.chmod(0o755)
                return smoke_release_binary(extracted, runner)
    except (tarfile.TarError, OSError) as error:
        raise SmokeFailure(
            f"could not read release archive {archive}: {error}"
        ) from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--binary", type=Path, help="exact native binary to package")
    source.add_argument("--archive", type=Path, help="cargo-dist .tar.gz to extract")
    parser.add_argument(
        "--binary-name",
        choices=("ironclaw", "ironclaw.exe"),
        help="shipping binary basename inside --archive",
    )
    args = parser.parse_args()
    if args.archive:
        if not args.binary_name:
            parser.error("--binary-name is required with --archive")
        evidence = smoke_release_archive(args.archive, args.binary_name)
    else:
        if args.binary_name:
            parser.error("--binary-name is only valid with --archive")
        evidence = smoke_release_binary(args.binary)
    print("release binary smoke passed: " + ", ".join(sorted(evidence)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
