"""GitHub repository, branch, release, search, and Actions cases."""

import json

from provider_operation_github_common import (
    BRANCH,
    CODE_MARKER,
    CODE_PATH,
    OWNER,
    REPO,
    REPO_PATH,
    github_request,
    seed_branch,
)
from provider_operation_types import ProviderOperationCase

CREATED_BRANCH = "provider-case-created-branch"
CREATED_REPO = "reborn-provider-case-repo"
SEEDED_REPO = "reborn-provider-case-seeded-repo"
FORK_NAME = "reborn-provider-case-fork"
RELEASE_TAG = "provider-case-v1"
BASE_ARGS = {"owner": OWNER, "repo": REPO}


async def _repo_baseline(emulate_url: str) -> None:
    repo = await github_request(emulate_url, "GET", REPO_PATH)
    assert isinstance(repo, dict)
    assert repo["full_name"] == f"{OWNER}/{REPO}", repo


async def _empty_release_baseline(emulate_url: str) -> None:
    await _repo_baseline(emulate_url)
    releases = await github_request(emulate_url, "GET", f"{REPO_PATH}/releases")
    assert releases == [], releases


async def _seeded_user_repo(emulate_url: str) -> None:
    await github_request(
        emulate_url,
        "POST",
        "/user/repos",
        payload={"name": SEEDED_REPO, "auto_init": True},
        expected_status=201,
    )


async def _seed_code(emulate_url: str) -> None:
    await seed_branch(emulate_url)


async def _create_branch_outcome(emulate_url: str, preview: dict) -> None:
    branches = await github_request(emulate_url, "GET", f"{REPO_PATH}/branches")
    assert isinstance(branches, list)
    assert {branch["name"] for branch in branches} == {
        "main",
        CREATED_BRANCH,
    }, branches
    assert CREATED_BRANCH in json.dumps(preview), preview


async def _list_branches_outcome(emulate_url: str, preview: dict) -> None:
    branches = await github_request(emulate_url, "GET", f"{REPO_PATH}/branches")
    assert isinstance(branches, list)
    assert {branch["name"] for branch in branches} == {"main", BRANCH}, branches
    rendered = json.dumps(preview)
    assert "main" in rendered, preview
    assert BRANCH in rendered, preview


async def _create_release_outcome(emulate_url: str, preview: dict) -> None:
    releases = await github_request(emulate_url, "GET", f"{REPO_PATH}/releases")
    assert isinstance(releases, list)
    assert [release["tag_name"] for release in releases] == [RELEASE_TAG], releases
    assert RELEASE_TAG in json.dumps(preview), preview


async def _create_repo_outcome(emulate_url: str, preview: dict) -> None:
    repo = await github_request(
        emulate_url, "GET", f"/repos/reborn-dev/{CREATED_REPO}"
    )
    assert isinstance(repo, dict)
    assert repo["private"] is True, repo
    assert CREATED_REPO in json.dumps(preview), preview


async def _fork_outcome(emulate_url: str, preview: dict) -> None:
    fork = await github_request(emulate_url, "GET", f"/repos/reborn-dev/{FORK_NAME}")
    assert isinstance(fork, dict)
    assert fork["fork"] is True, fork
    assert FORK_NAME in json.dumps(preview), preview


async def _list_repos_outcome(emulate_url: str, preview: dict) -> None:
    repos = await github_request(emulate_url, "GET", "/user/repos")
    assert isinstance(repos, list)
    assert any(repo["name"] == SEEDED_REPO for repo in repos), repos
    assert SEEDED_REPO in json.dumps(preview), preview


async def _search_repo_outcome(emulate_url: str, preview: dict) -> None:
    await _repo_baseline(emulate_url)
    rendered = json.dumps(preview)
    assert f"{OWNER}/{REPO}" in rendered, preview
    assert "Emulated IronClaw repo" in rendered, preview


async def _search_code_outcome(emulate_url: str, preview: dict) -> None:
    result = await github_request(
        emulate_url,
        "GET",
        "/search/code",
        params={"q": f"{CODE_MARKER} repo:{OWNER}/{REPO}"},
    )
    assert isinstance(result, dict)
    assert [item["path"] for item in result["items"]] == [CODE_PATH], result
    rendered = json.dumps(preview)
    assert CODE_PATH in rendered, preview


async def _workflow_runs_outcome(emulate_url: str, preview: dict) -> None:
    result = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/actions/runs"
    )
    assert result == {"total_count": 0, "workflow_runs": []}, result
    rendered = json.dumps(preview)
    assert "workflow_runs" in rendered, preview
    assert "total_count" in rendered, preview


GITHUB_REPO_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="github_create_branch",
        provider_service="github",
        capability_id="github.create_branch",
        arguments={
            **BASE_ARGS,
            "branch": CREATED_BRANCH,
            "from_ref": "main",
        },
        assert_baseline=_repo_baseline,
        assert_outcome=_create_branch_outcome,
    ),
    ProviderOperationCase(
        case_id="github_list_branches",
        provider_service="github",
        capability_id="github.list_branches",
        arguments=BASE_ARGS,
        assert_baseline=_seed_code,
        assert_outcome=_list_branches_outcome,
    ),
    ProviderOperationCase(
        case_id="github_create_release",
        provider_service="github",
        capability_id="github.create_release",
        arguments={
            **BASE_ARGS,
            "tag_name": RELEASE_TAG,
            "name": "Provider Case v1",
            "body": "Created through the provider operation runner.",
        },
        assert_baseline=_empty_release_baseline,
        assert_outcome=_create_release_outcome,
    ),
    ProviderOperationCase(
        case_id="github_create_repo",
        provider_service="github",
        capability_id="github.create_repo",
        arguments={
            "name": CREATED_REPO,
            "description": "Created through the provider operation runner.",
            "private": True,
            "auto_init": True,
        },
        assert_baseline=_repo_baseline,
        assert_outcome=_create_repo_outcome,
    ),
    ProviderOperationCase(
        case_id="github_fork_repo",
        provider_service="github",
        capability_id="github.fork_repo",
        arguments={**BASE_ARGS, "name": FORK_NAME},
        assert_baseline=_repo_baseline,
        assert_outcome=_fork_outcome,
    ),
    ProviderOperationCase(
        case_id="github_list_repos",
        provider_service="github",
        capability_id="github.list_repos",
        arguments={"type": "owner"},
        assert_baseline=_seeded_user_repo,
        assert_outcome=_list_repos_outcome,
    ),
    ProviderOperationCase(
        case_id="github_search_repositories",
        provider_service="github",
        capability_id="github.search_repositories",
        arguments={"query": f"org:{OWNER} {REPO}"},
        assert_baseline=_repo_baseline,
        assert_outcome=_search_repo_outcome,
    ),
    ProviderOperationCase(
        case_id="github_search_code",
        provider_service="github",
        capability_id="github.search_code",
        arguments={"query": f"{CODE_MARKER} repo:{OWNER}/{REPO}"},
        assert_baseline=_seed_code,
        assert_outcome=_search_code_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_workflow_runs",
        provider_service="github",
        capability_id="github.get_workflow_runs",
        arguments=BASE_ARGS,
        assert_baseline=_repo_baseline,
        assert_outcome=_workflow_runs_outcome,
    ),
)
