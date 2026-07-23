"""Shared GitHub provider setup and readback helpers."""

import httpx

from emulate_provider import github_json

OWNER = "nearai"
REPO = "ironclaw"
REPO_PATH = f"/repos/{OWNER}/{REPO}"
ISSUE_TITLE = "REBORN_PROVIDER_CASE_SEEDED_ISSUE"
PR_TITLE = "REBORN_PROVIDER_CASE_SEEDED_PR"
BRANCH = "provider-case-branch"
CODE_PATH = "docs/provider-case.md"
CODE_MARKER = "REBORN_PROVIDER_CASE_CODE_MARKER"


async def github_request(
    emulate_url: str,
    method: str,
    path: str,
    *,
    payload: dict | None = None,
    params: dict | None = None,
    expected_status: int = 200,
) -> dict | list:
    async with httpx.AsyncClient(timeout=15) as client:
        return await github_json(
            client,
            emulate_url,
            method,
            path,
            payload=payload,
            params=params,
            expected_status=expected_status,
        )


async def seed_issue(emulate_url: str, *, title: str = ISSUE_TITLE) -> dict:
    result = await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/issues",
        payload={"title": title, "body": "Seeded by the provider operation case."},
        expected_status=201,
    )
    assert isinstance(result, dict)
    assert result["number"] == 1, result
    return result


async def issue(emulate_url: str) -> dict:
    result = await github_request(emulate_url, "GET", f"{REPO_PATH}/issues/1")
    assert isinstance(result, dict)
    return result


async def seed_branch(emulate_url: str, *, branch: str = BRANCH) -> dict:
    main_ref = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/git/ref/heads/main"
    )
    assert isinstance(main_ref, dict)
    main_sha = main_ref["object"]["sha"]
    main_commit = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/git/commits/{main_sha}"
    )
    assert isinstance(main_commit, dict)
    blob = await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/git/blobs",
        payload={"content": CODE_MARKER, "encoding": "utf-8"},
        expected_status=201,
    )
    assert isinstance(blob, dict)
    tree = await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/git/trees",
        payload={
            "base_tree": main_commit["commit"]["tree"]["sha"],
            "tree": [
                {
                    "path": CODE_PATH,
                    "mode": "100644",
                    "type": "blob",
                    "sha": blob["sha"],
                }
            ],
        },
        expected_status=201,
    )
    assert isinstance(tree, dict)
    commit = await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/git/commits",
        payload={
            "message": "test: seed provider operation branch",
            "tree": tree["sha"],
            "parents": [main_sha],
        },
        expected_status=201,
    )
    assert isinstance(commit, dict)
    result = await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/git/refs",
        payload={"ref": f"refs/heads/{branch}", "sha": commit["sha"]},
        expected_status=201,
    )
    assert isinstance(result, dict)
    return result


async def seed_pull_request(emulate_url: str) -> dict:
    await seed_branch(emulate_url)
    result = await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/pulls",
        payload={
            "title": PR_TITLE,
            "head": BRANCH,
            "base": "main",
            "body": "Seeded by the provider operation case.",
        },
        expected_status=201,
    )
    assert isinstance(result, dict)
    assert result["number"] == 1, result
    return result


async def pull_request(emulate_url: str) -> dict:
    result = await github_request(emulate_url, "GET", f"{REPO_PATH}/pulls/1")
    assert isinstance(result, dict)
    return result
