"""GitHub Contents, status, review-thread, and Actions operation cases."""

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
    seed_pull_request,
)
from provider_operation_types import ProviderOperationCase

BASE_ARGS = {"owner": OWNER, "repo": REPO}
CREATED_FILE_PATH = "docs/provider-operation-created.md"
CREATED_FILE_CONTENT = "REBORN_PROVIDER_CASE_CONTENTS_CREATED"
DELETED_FILE_PATH = "docs/provider-operation-delete.md"
DELETED_FILE_CONTENT = "REBORN_PROVIDER_CASE_CONTENTS_DELETE"
STATUS_CONTEXT = "provider-contract"
INLINE_COMMENT = "REBORN_PROVIDER_CASE_THREAD_ROOT"
REPLY_COMMENT = "REBORN_PROVIDER_CASE_THREAD_REPLY"
RUN_ID = 1001
JOB_ID = 2001
ARTIFACT_ID = 3001
JOB_LOG = "REBORN_PROVIDER_CASE_JOB_LOG"
ARTIFACT_NAME = "provider-results"

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


def _preview(preview: dict) -> str:
    return json.dumps(preview)


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
                    "body": INLINE_COMMENT,
                }
            ],
        },
        expected_status=201,
    )
    comments = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/pulls/1/comments"
    )
    assert isinstance(comments, list)
    assert [comment["body"] for comment in comments] == [INLINE_COMMENT], comments


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
        "body": REPLY_COMMENT,
    }


async def _seed_resolved_review_thread(emulate_url: str) -> None:
    await _seed_review_thread(emulate_url)
    arguments = await _thread_arguments(emulate_url)
    result = await _graphql(
        emulate_url, RESOLVE_THREAD_MUTATION, {"threadId": arguments["thread_id"]}
    )
    assert result["data"]["resolveReviewThread"]["thread"]["isResolved"] is True


async def _actions_baseline(emulate_url: str) -> None:
    run = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/actions/runs/{RUN_ID}"
    )
    assert isinstance(run, dict)
    assert (run["status"], run["conclusion"], run["run_attempt"]) == (
        "completed",
        "failure",
        1,
    ), run
    jobs = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/actions/runs/{RUN_ID}/jobs"
    )
    assert isinstance(jobs, dict)
    assert [job["id"] for job in jobs["jobs"]] == [JOB_ID], jobs
    artifacts = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/actions/runs/{RUN_ID}/artifacts"
    )
    assert isinstance(artifacts, dict)
    assert [artifact["id"] for artifact in artifacts["artifacts"]] == [
        ARTIFACT_ID
    ], artifacts


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
    assert CREATED_FILE_PATH in _preview(preview), preview


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
    assert STATUS_CONTEXT in _preview(preview), preview


async def _reply_outcome(emulate_url: str, preview: dict) -> None:
    comments = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/pulls/1/comments"
    )
    assert isinstance(comments, list)
    assert [comment["body"] for comment in comments] == [
        INLINE_COMMENT,
        REPLY_COMMENT,
    ], comments
    assert comments[1]["in_reply_to_id"] == comments[0]["id"], comments
    assert REPLY_COMMENT in _preview(preview), preview


async def _list_threads_outcome(emulate_url: str, preview: dict) -> None:
    threads = await _review_threads(emulate_url)
    assert len(threads) == 1, threads
    assert threads[0]["isResolved"] is False, threads
    assert threads[0]["id"] in _preview(preview), preview


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


async def _trigger_workflow_outcome(
    emulate_url: str, preview: dict
) -> None:
    runs = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/actions/runs"
    )
    assert isinstance(runs, dict)
    queued = [
        run
        for run in runs["workflow_runs"]
        if run["id"] != RUN_ID and run["event"] == "workflow_dispatch"
    ]
    assert len(queued) == 1, runs
    assert queued[0]["status"] == "queued", queued[0]
    assert json.loads(preview["output_preview"]) == {"status": 204}, preview


async def _jobs_outcome(emulate_url: str, preview: dict) -> None:
    await _actions_baseline(emulate_url)
    assert JOB_ID in [
        job["id"]
        for job in json.loads(preview["output_preview"])["jobs"]
    ], preview


async def _job_logs_outcome(emulate_url: str, preview: dict) -> None:
    async with httpx.AsyncClient(
        headers=github_headers(), timeout=15
    ) as client:
        response = await client.get(
            f"{emulate_url}{REPO_PATH}/actions/jobs/{JOB_ID}/logs"
        )
    response.raise_for_status()
    assert response.text == JOB_LOG, response.text
    assert JOB_LOG in _preview(preview), preview


async def _artifacts_outcome(emulate_url: str, preview: dict) -> None:
    await _actions_baseline(emulate_url)
    assert ARTIFACT_NAME in _preview(preview), preview


async def _rerun_outcome(emulate_url: str, preview: dict) -> None:
    run = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/actions/runs/{RUN_ID}"
    )
    assert isinstance(run, dict)
    assert (run["status"], run["conclusion"], run["run_attempt"]) == (
        "queued",
        None,
        2,
    ), run
    jobs = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/actions/runs/{RUN_ID}/jobs"
    )
    assert isinstance(jobs, dict)
    assert (jobs["jobs"][0]["status"], jobs["jobs"][0]["conclusion"]) == (
        "queued",
        None,
    ), jobs
    assert json.loads(preview["output_preview"]) == {"status": 201}, preview


GITHUB_GAP_PROVIDER_OPERATION_CASES = (
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
    ProviderOperationCase(
        case_id="github_trigger_workflow",
        provider_service="github",
        capability_id="github.trigger_workflow",
        arguments={
            **BASE_ARGS,
            "workflow_id": "101",
            "ref": "main",
            "inputs": {"suite": "provider-contract"},
        },
        assert_baseline=_actions_baseline,
        assert_outcome=_trigger_workflow_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_workflow_run_jobs",
        provider_service="github",
        capability_id="github.get_workflow_run_jobs",
        arguments={**BASE_ARGS, "run_id": RUN_ID},
        assert_baseline=_actions_baseline,
        assert_outcome=_jobs_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_job_logs",
        provider_service="github",
        capability_id="github.get_job_logs",
        arguments={**BASE_ARGS, "job_id": JOB_ID},
        assert_baseline=_actions_baseline,
        assert_outcome=_job_logs_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_workflow_run_artifacts",
        provider_service="github",
        capability_id="github.get_workflow_run_artifacts",
        arguments={**BASE_ARGS, "run_id": RUN_ID},
        assert_baseline=_actions_baseline,
        assert_outcome=_artifacts_outcome,
    ),
    ProviderOperationCase(
        case_id="github_rerun_failed_workflow_run_jobs",
        provider_service="github",
        capability_id="github.rerun_failed_workflow_run_jobs",
        arguments={
            **BASE_ARGS,
            "run_id": RUN_ID,
            "enable_debug_logging": True,
        },
        assert_baseline=_actions_baseline,
        assert_outcome=_rerun_outcome,
    ),
    ProviderOperationCase(
        case_id="github_rerun_workflow_job",
        provider_service="github",
        capability_id="github.rerun_workflow_job",
        arguments={
            **BASE_ARGS,
            "job_id": JOB_ID,
            "enable_debugger": True,
        },
        assert_baseline=_actions_baseline,
        assert_outcome=_rerun_outcome,
    ),
)
