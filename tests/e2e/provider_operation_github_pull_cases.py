"""GitHub pull request full-path provider operation cases."""

import json

from provider_operation_github_common import (
    BRANCH,
    CODE_PATH,
    OWNER,
    PR_TITLE,
    REPO,
    REPO_PATH,
    pull_request,
    github_request,
    seed_branch,
    seed_pull_request,
)
from provider_operation_types import ProviderOperationCase

CREATED_PR = "REBORN_PROVIDER_CASE_CREATED_PR"
UPDATED_PR = "REBORN_PROVIDER_CASE_UPDATED_PR"
REVIEW_BODY = "REBORN_PROVIDER_CASE_REVIEW"
INLINE_COMMENT = "REBORN_PROVIDER_CASE_INLINE_COMMENT"
THREAD_ROOT_COMMENT = "REBORN_PROVIDER_CASE_THREAD_ROOT"
THREAD_REPLY_COMMENT = "REBORN_PROVIDER_CASE_THREAD_REPLY"
BASE_ARGS = {"owner": OWNER, "repo": REPO}

REVIEW_THREADS_QUERY = """
query($owner: String!, $repo: String!, $number: Int!, $first: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: $first) {
        nodes { id isResolved }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}
"""

RESOLVE_THREAD_MUTATION = """
mutation($threadId: ID!) {
  resolveReviewThread(input: { threadId: $threadId }) {
    thread { id isResolved }
  }
}
"""


async def _seeded_branch(emulate_url: str) -> None:
    await seed_branch(emulate_url)
    pulls = await github_request(emulate_url, "GET", f"{REPO_PATH}/pulls")
    assert pulls == [], pulls


async def _seeded_pull(emulate_url: str) -> None:
    await seed_pull_request(emulate_url)


async def _seeded_review(emulate_url: str) -> None:
    await seed_pull_request(emulate_url)
    await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/pulls/1/reviews",
        payload={"body": REVIEW_BODY, "event": "COMMENT"},
        expected_status=201,
    )


async def _seeded_inline_comment(emulate_url: str) -> None:
    await seed_pull_request(emulate_url)
    await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/pulls/1/reviews",
        payload={
            "body": REVIEW_BODY,
            "event": "COMMENT",
            "comments": [
                {
                    "path": CODE_PATH,
                    "position": 1,
                    "body": INLINE_COMMENT,
                }
            ],
        },
        expected_status=201,
    )


async def _seed_review_thread(emulate_url: str) -> None:
    await seed_pull_request(emulate_url)
    await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/pulls/1/reviews",
        payload={
            "body": "Provider contract review",
            "event": "COMMENT",
            "comments": [
                {
                    "path": CODE_PATH,
                    "position": 1,
                    "body": THREAD_ROOT_COMMENT,
                }
            ],
        },
        expected_status=201,
    )
    comments = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/pulls/1/comments"
    )
    assert isinstance(comments, list)
    assert [comment["body"] for comment in comments] == [
        THREAD_ROOT_COMMENT
    ], comments


async def _graphql(
    emulate_url: str, query: str, variables: dict
) -> dict:
    result = await github_request(
        emulate_url,
        "POST",
        "/graphql",
        payload={"query": query, "variables": variables},
    )
    assert isinstance(result, dict)
    assert "errors" not in result, result
    return result


async def _review_threads(emulate_url: str) -> list[dict]:
    result = await _graphql(
        emulate_url,
        REVIEW_THREADS_QUERY,
        {"owner": OWNER, "repo": REPO, "number": 1, "first": 30},
    )
    return result["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"]


async def _thread_arguments(emulate_url: str) -> dict:
    threads = await _review_threads(emulate_url)
    assert len(threads) == 1, threads
    return {"thread_id": threads[0]["id"]}


async def _reply_arguments(emulate_url: str) -> dict:
    comments = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/pulls/1/comments"
    )
    assert isinstance(comments, list)
    assert len(comments) == 1, comments
    return {
        **BASE_ARGS,
        "pr_number": 1,
        "comment_id": comments[0]["id"],
        "body": THREAD_REPLY_COMMENT,
    }


async def _seed_resolved_review_thread(emulate_url: str) -> None:
    await _seed_review_thread(emulate_url)
    arguments = await _thread_arguments(emulate_url)
    result = await _graphql(
        emulate_url, RESOLVE_THREAD_MUTATION, {"threadId": arguments["thread_id"]}
    )
    assert result["data"]["resolveReviewThread"]["thread"]["isResolved"] is True


async def _created_pull_outcome(emulate_url: str, preview: dict) -> None:
    created = await pull_request(emulate_url)
    assert created["title"] == CREATED_PR, created
    assert created["head"]["ref"] == BRANCH, created
    assert CREATED_PR in json.dumps(preview), preview


async def _seeded_pull_outcome(emulate_url: str, preview: dict) -> None:
    seeded = await pull_request(emulate_url)
    assert seeded["title"] == PR_TITLE, seeded
    assert PR_TITLE in json.dumps(preview), preview


async def _updated_pull_outcome(emulate_url: str, preview: dict) -> None:
    updated = await pull_request(emulate_url)
    assert updated["title"] == UPDATED_PR, updated
    assert UPDATED_PR in json.dumps(preview), preview


async def _pull_files_outcome(emulate_url: str, preview: dict) -> None:
    files = await github_request(emulate_url, "GET", f"{REPO_PATH}/pulls/1/files")
    assert isinstance(files, list)
    # Emulate currently models this successful empty state but not generated
    # pull-request diffs. The seeded branch still proves the PR is real.
    assert files == [], files
    assert preview["output_preview"] == "[]", preview


async def _create_review_outcome(emulate_url: str, preview: dict) -> None:
    reviews = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/pulls/1/reviews"
    )
    assert isinstance(reviews, list)
    assert [review["body"] for review in reviews] == [REVIEW_BODY], reviews
    assert REVIEW_BODY in json.dumps(preview), preview


async def _list_comments_outcome(emulate_url: str, preview: dict) -> None:
    comments = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/pulls/1/comments"
    )
    assert isinstance(comments, list)
    assert [comment["body"] for comment in comments] == [
        INLINE_COMMENT
    ], comments
    assert INLINE_COMMENT in json.dumps(preview), preview


async def _merge_outcome(emulate_url: str, preview: dict) -> None:
    merged = await pull_request(emulate_url)
    assert merged["merged"] is True, merged
    assert merged["state"] == "closed", merged
    assert json.loads(preview["output_preview"])["merged"] is True, preview


async def _reply_outcome(emulate_url: str, preview: dict) -> None:
    comments = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/pulls/1/comments"
    )
    assert isinstance(comments, list)
    assert [comment["body"] for comment in comments] == [
        THREAD_ROOT_COMMENT,
        THREAD_REPLY_COMMENT,
    ], comments
    assert comments[1]["in_reply_to_id"] == comments[0]["id"], comments
    assert THREAD_REPLY_COMMENT in json.dumps(preview), preview


async def _list_threads_outcome(emulate_url: str, preview: dict) -> None:
    threads = await _review_threads(emulate_url)
    assert len(threads) == 1, threads
    assert threads[0]["isResolved"] is False, threads
    assert threads[0]["id"] in json.dumps(preview), preview


def _thread_resolution_outcome(expected: bool):
    async def assert_outcome(emulate_url: str, preview: dict) -> None:
        threads = await _review_threads(emulate_url)
        assert len(threads) == 1, threads
        assert threads[0]["isResolved"] is expected, threads
        output = json.loads(preview["output_preview"])
        assert f'"isResolved": {str(expected).lower()}' in json.dumps(
            output, separators=(", ", ": ")
        ), output

    return assert_outcome


GITHUB_PULL_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="github_create_pull_request",
        provider_service="github",
        capability_id="github.create_pull_request",
        arguments={
            **BASE_ARGS,
            "title": CREATED_PR,
            "head": BRANCH,
            "base": "main",
            "body": "Created through the provider operation runner.",
        },
        assert_baseline=_seeded_branch,
        assert_outcome=_created_pull_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_pull_request",
        provider_service="github",
        capability_id="github.get_pull_request",
        arguments={**BASE_ARGS, "pr_number": 1},
        assert_baseline=_seeded_pull,
        assert_outcome=_seeded_pull_outcome,
    ),
    ProviderOperationCase(
        case_id="github_list_pull_requests",
        provider_service="github",
        capability_id="github.list_pull_requests",
        arguments={**BASE_ARGS, "state": "open"},
        assert_baseline=_seeded_pull,
        assert_outcome=_seeded_pull_outcome,
    ),
    ProviderOperationCase(
        case_id="github_update_pull_request",
        provider_service="github",
        capability_id="github.update_pull_request",
        arguments={**BASE_ARGS, "pr_number": 1, "title": UPDATED_PR},
        assert_baseline=_seeded_pull,
        assert_outcome=_updated_pull_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_pull_request_files",
        provider_service="github",
        capability_id="github.get_pull_request_files",
        arguments={**BASE_ARGS, "pr_number": 1},
        assert_baseline=_seeded_pull,
        assert_outcome=_pull_files_outcome,
    ),
    ProviderOperationCase(
        case_id="github_create_pr_review",
        provider_service="github",
        capability_id="github.create_pr_review",
        arguments={
            **BASE_ARGS,
            "pr_number": 1,
            "body": REVIEW_BODY,
            "event": "COMMENT",
        },
        assert_baseline=_seeded_pull,
        assert_outcome=_create_review_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_pull_request_reviews",
        provider_service="github",
        capability_id="github.get_pull_request_reviews",
        arguments={**BASE_ARGS, "pr_number": 1},
        assert_baseline=_seeded_review,
        assert_outcome=_create_review_outcome,
    ),
    ProviderOperationCase(
        case_id="github_list_pull_request_comments",
        provider_service="github",
        capability_id="github.list_pull_request_comments",
        arguments={**BASE_ARGS, "pr_number": 1},
        assert_baseline=_seeded_inline_comment,
        assert_outcome=_list_comments_outcome,
    ),
    ProviderOperationCase(
        case_id="github_merge_pull_request",
        provider_service="github",
        capability_id="github.merge_pull_request",
        arguments={
            **BASE_ARGS,
            "pr_number": 1,
            "merge_method": "squash",
            "commit_title": "Merge provider operation case",
        },
        assert_baseline=_seeded_pull,
        assert_outcome=_merge_outcome,
    ),
    ProviderOperationCase(
        case_id="github_reply_pull_request_comment",
        provider_service="github",
        capability_id="github.reply_pull_request_comment",
        arguments=_reply_arguments,
        assert_baseline=_seed_review_thread,
        assert_outcome=_reply_outcome,
    ),
    ProviderOperationCase(
        case_id="github_list_pull_request_review_threads",
        provider_service="github",
        capability_id="github.list_pull_request_review_threads",
        arguments={**BASE_ARGS, "pr_number": 1},
        assert_baseline=_seed_review_thread,
        assert_outcome=_list_threads_outcome,
    ),
    ProviderOperationCase(
        case_id="github_resolve_review_thread",
        provider_service="github",
        capability_id="github.resolve_review_thread",
        arguments=_thread_arguments,
        assert_baseline=_seed_review_thread,
        assert_outcome=_thread_resolution_outcome(True),
    ),
    ProviderOperationCase(
        case_id="github_unresolve_review_thread",
        provider_service="github",
        capability_id="github.unresolve_review_thread",
        arguments=_thread_arguments,
        assert_baseline=_seed_resolved_review_thread,
        assert_outcome=_thread_resolution_outcome(False),
    ),
)
