"""Create the cargo-dist tag for an explicitly approved Ironclaw commit."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from collections.abc import Callable
from pathlib import Path

import tomllib

NUMERIC_IDENTIFIER = r"(?:0|[1-9][0-9]*)"
PRERELEASE_IDENTIFIER = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
# This intentionally excludes Cargo build metadata: the same release publishes a
# Docker image, whose tag grammar does not permit '+'.
VERSION_PATTERN = re.compile(
    rf"{NUMERIC_IDENTIFIER}\.{NUMERIC_IDENTIFIER}\.{NUMERIC_IDENTIFIER}"
    rf"(?:-{PRERELEASE_IDENTIFIER}(?:\.{PRERELEASE_IDENTIFIER})*)?"
)
SHA_PATTERN = re.compile(r"[0-9a-f]{40}")
MAX_ANNOTATED_TAG_DEPTH = 8


class ReleaseTagError(RuntimeError):
    """The requested immutable release tag is unsafe or invalid."""


def ensure_release_tag(
    *,
    requested_version: str,
    requested_sha: str,
    manifest_version: str,
    checked_out_sha: str,
    get_tag_target: Callable[[str], str | None],
    create_tag: Callable[[str, str], None],
) -> str:
    """Validate release identity and create its tag exactly once."""
    if VERSION_PATTERN.fullmatch(requested_version) is None:
        raise ReleaseTagError(f"invalid release version: {requested_version!r}")
    if SHA_PATTERN.fullmatch(requested_sha) is None:
        raise ReleaseTagError("commit_sha must be a full lowercase commit SHA")
    if checked_out_sha != requested_sha:
        raise ReleaseTagError(
            f"checked out {checked_out_sha}, expected approved commit {requested_sha}"
        )
    if manifest_version != requested_version:
        raise ReleaseTagError(
            f"approved commit declares version {manifest_version}, "
            f"not requested version {requested_version}"
        )

    tag = f"ironclaw-v{requested_version}"
    existing_target = get_tag_target(tag)
    if existing_target is not None:
        if existing_target != requested_sha:
            raise ReleaseTagError(
                f"{tag} already points to {existing_target}, not {requested_sha}"
            )
        return f"{tag} already points to approved commit {requested_sha}"

    try:
        create_tag(tag, requested_sha)
    except ReleaseTagError:
        # A retried or concurrently dispatched run is safe only when it created
        # the exact same immutable mapping.
        if get_tag_target(tag) != requested_sha:
            raise
        return f"{tag} was concurrently created at approved commit {requested_sha}"
    return f"created {tag} at approved commit {requested_sha}"


class GitHubTags:
    def __init__(self, repository: str) -> None:
        self.repository = repository

    def get_target(self, tag: str) -> str | None:
        result = subprocess.run(
            ["gh", "api", f"repos/{self.repository}/git/ref/tags/{tag}"],
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            if "HTTP 404" in result.stderr:
                return None
            raise ReleaseTagError(f"failed to read {tag}: {result.stderr.strip()}")
        target = json.loads(result.stdout)["object"]

        for depth in range(MAX_ANNOTATED_TAG_DEPTH + 1):
            object_type = str(target["type"])
            object_sha = str(target["sha"])
            if object_type == "commit":
                return object_sha
            if object_type != "tag":
                raise ReleaseTagError(
                    f"{tag} resolves to unsupported Git object type {object_type!r}"
                )
            if depth == MAX_ANNOTATED_TAG_DEPTH:
                break

            result = subprocess.run(
                [
                    "gh",
                    "api",
                    f"repos/{self.repository}/git/tags/{object_sha}",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            if result.returncode != 0:
                raise ReleaseTagError(
                    f"failed to resolve annotated {tag}: {result.stderr.strip()}"
                )
            target = json.loads(result.stdout)["object"]

        raise ReleaseTagError(f"{tag} exceeds the annotated-tag resolution depth limit")

    def create(self, tag: str, commit_sha: str) -> None:
        result = subprocess.run(
            [
                "gh",
                "api",
                "--method",
                "POST",
                f"repos/{self.repository}/git/refs",
                "-f",
                f"ref=refs/tags/{tag}",
                "-f",
                f"sha={commit_sha}",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise ReleaseTagError(f"failed to create {tag}: {result.stderr.strip()}")


def _checked_out_sha(candidate_root: Path) -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD^{commit}"],
        cwd=candidate_root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def _manifest_version(candidate_root: Path) -> str:
    manifest = candidate_root / "crates/ironclaw_reborn_cli/Cargo.toml"
    with manifest.open("rb") as manifest_file:
        return str(tomllib.load(manifest_file)["package"]["version"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit-sha", required=True)
    parser.add_argument("--candidate-root", required=True, type=Path)
    parser.add_argument("--repository", required=True)
    args = parser.parse_args()

    tags = GitHubTags(args.repository)
    message = ensure_release_tag(
        requested_version=args.version,
        requested_sha=args.commit_sha,
        manifest_version=_manifest_version(args.candidate_root),
        checked_out_sha=_checked_out_sha(args.candidate_root),
        get_tag_target=tags.get_target,
        create_tag=tags.create,
    )
    print(message)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (
        ReleaseTagError,
        subprocess.CalledProcessError,
        KeyError,
        ValueError,
    ) as error:
        raise SystemExit(f"error: {error}") from error
