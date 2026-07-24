"""GitHub repository, Contents, branch, release, and search cases."""

import base64
import json

import httpx

from emulate_provider import github_headers
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
CREATED_FILE_PATH = "docs/provider-operation-created.md"
CREATED_FILE_CONTENT = "REBORN_PROVIDER_CASE_CONTENTS_CREATED"
DELETED_FILE_PATH = "docs/provider-operation-delete.md"
DELETED_FILE_CONTENT = "REBORN_PROVIDER_CASE_CONTENTS_DELETE"
STATUS_CONTEXT = "provider-contract"
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


def _decode_content(resource: dict) -> str:
    return base64.b64decode(resource["content"]).decode("utf-8")


async def _missing_file(emulate_url: str, path: str) -> None:
    async with httpx.AsyncClient(
        headers=github_headers(), timeout=15
    ) as client:
        response = await client.get(f"{emulate_url}{REPO_PATH}/contents/{path}")
    assert response.status_code == 404, response.text


async def _seed_delete_file(emulate_url: str) -> None:
    await _missing_file(emulate_url, DELETED_FILE_PATH)
    result = await github_request(
        emulate_url,
        "PUT",
        f"{REPO_PATH}/contents/{DELETED_FILE_PATH}",
        payload={
            "message": "test: seed file for deletion",
            "content": base64.b64encode(DELETED_FILE_CONTENT.encode()).decode(),
            "branch": "main",
        },
        expected_status=201,
    )
    assert isinstance(result, dict)
    assert result["content"]["path"] == DELETED_FILE_PATH, result


async def _delete_file_arguments(emulate_url: str) -> dict:
    resource = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/contents/{DELETED_FILE_PATH}"
    )
    assert isinstance(resource, dict)
    return {
        **BASE_ARGS,
        "path": DELETED_FILE_PATH,
        "message": "test: delete provider operation file",
        "sha": resource["sha"],
        "branch": "main",
    }


async def _seed_status(emulate_url: str) -> None:
    ref = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/git/ref/heads/main"
    )
    assert isinstance(ref, dict)
    status = await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/statuses/{ref['object']['sha']}",
        payload={
            "state": "success",
            "context": STATUS_CONTEXT,
            "description": "Provider contract passed",
        },
        expected_status=201,
    )
    assert isinstance(status, dict)
    assert status["context"] == STATUS_CONTEXT, status


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


async def _get_file_outcome(emulate_url: str, preview: dict) -> None:
    resource = await github_request(
        emulate_url,
        "GET",
        f"{REPO_PATH}/contents/{CODE_PATH}",
        params={"ref": BRANCH},
    )
    assert isinstance(resource, dict)
    assert _decode_content(resource) == CODE_MARKER, resource
    assert (
        base64.b64decode(preview["output_preview"]).decode() == CODE_MARKER
    ), preview


async def _create_file_outcome(emulate_url: str, preview: dict) -> None:
    resource = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/contents/{CREATED_FILE_PATH}"
    )
    assert isinstance(resource, dict)
    assert _decode_content(resource) == CREATED_FILE_CONTENT, resource
    assert CREATED_FILE_PATH in json.dumps(preview), preview


async def _delete_file_outcome(emulate_url: str, preview: dict) -> None:
    await _missing_file(emulate_url, DELETED_FILE_PATH)
    assert json.loads(preview["output_preview"])["content"] is None, preview


async def _status_outcome(emulate_url: str, preview: dict) -> None:
    status = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/commits/main/status"
    )
    assert isinstance(status, dict)
    assert status["state"] == "success", status
    assert [item["context"] for item in status["statuses"]] == [
        STATUS_CONTEXT
    ], status
    assert STATUS_CONTEXT in json.dumps(preview), preview


AUTHENTICATED_LOGIN = "reborn-dev"
LISTED_RELEASE_TAG = "provider-case-listed-v1"


async def _authenticated_user_baseline(emulate_url: str) -> None:
    user = await github_request(emulate_url, "GET", "/user")
    assert isinstance(user, dict)
    assert user["login"] == AUTHENTICATED_LOGIN, user


async def _get_authenticated_user_outcome(emulate_url: str, preview: dict) -> None:
    await _authenticated_user_baseline(emulate_url)
    assert AUTHENTICATED_LOGIN in json.dumps(preview), preview


async def _get_repo_outcome(emulate_url: str, preview: dict) -> None:
    await _repo_baseline(emulate_url)
    assert f"{OWNER}/{REPO}" in json.dumps(preview), preview


async def _seed_listed_release(emulate_url: str) -> None:
    await _repo_baseline(emulate_url)
    release = await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/releases",
        payload={"tag_name": LISTED_RELEASE_TAG, "name": "Provider Case Listed v1"},
        expected_status=201,
    )
    assert isinstance(release, dict)
    assert release["tag_name"] == LISTED_RELEASE_TAG, release


async def _list_releases_outcome(emulate_url: str, preview: dict) -> None:
    releases = await github_request(emulate_url, "GET", f"{REPO_PATH}/releases")
    assert isinstance(releases, list)
    assert LISTED_RELEASE_TAG in [
        release["tag_name"] for release in releases
    ], releases
    assert LISTED_RELEASE_TAG in json.dumps(preview), preview


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
        case_id="github_get_file_content",
        provider_service="github",
        capability_id="github.get_file_content",
        arguments={**BASE_ARGS, "path": CODE_PATH, "ref": BRANCH},
        assert_baseline=seed_branch,
        assert_outcome=_get_file_outcome,
    ),
    ProviderOperationCase(
        case_id="github_create_or_update_file",
        provider_service="github",
        capability_id="github.create_or_update_file",
        arguments={
            **BASE_ARGS,
            "path": CREATED_FILE_PATH,
            "message": "test: create provider operation file",
            "content": CREATED_FILE_CONTENT,
            "branch": "main",
        },
        assert_baseline=lambda emulate_url: _missing_file(
            emulate_url, CREATED_FILE_PATH
        ),
        assert_outcome=_create_file_outcome,
    ),
    ProviderOperationCase(
        case_id="github_delete_file",
        provider_service="github",
        capability_id="github.delete_file",
        arguments=_delete_file_arguments,
        assert_baseline=_seed_delete_file,
        assert_outcome=_delete_file_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_combined_status",
        provider_service="github",
        capability_id="github.get_combined_status",
        arguments={**BASE_ARGS, "ref": "main"},
        assert_baseline=_seed_status,
        assert_outcome=_status_outcome,
    ),
    # Executable evidence for read capabilities whose harvested journeys were
    # quarantined with the retired activation flow (#6520).
    ProviderOperationCase(
        case_id="github_get_authenticated_user",
        provider_service="github",
        capability_id="github.get_authenticated_user",
        arguments={},
        assert_baseline=_authenticated_user_baseline,
        assert_outcome=_get_authenticated_user_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_repo",
        provider_service="github",
        capability_id="github.get_repo",
        arguments=BASE_ARGS,
        assert_baseline=_repo_baseline,
        assert_outcome=_get_repo_outcome,
    ),
    ProviderOperationCase(
        case_id="github_list_releases",
        provider_service="github",
        capability_id="github.list_releases",
        arguments=BASE_ARGS,
        assert_baseline=_seed_listed_release,
        assert_outcome=_list_releases_outcome,
    ),
)
