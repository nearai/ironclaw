"""GitHub empty-result contracts through the real extension boundary."""

from provider_operation_github_common import REPO_PATH, github_request
from provider_operation_types import (
    ProviderOperationCase,
    exact_output,
    exact_text_output,
    static_provider_json_response,
    static_provider_text_response,
)


async def _seeded_repo_baseline(emulate_url: str) -> None:
    repo = await github_request(emulate_url, "GET", REPO_PATH)
    assert isinstance(repo, dict)
    assert repo["full_name"] == "nearai/ironclaw", repo


def _empty_case(
    *,
    case_id: str,
    capability_id: str,
    arguments: dict,
    path: str,
    payload,
    method: str = "GET",
) -> ProviderOperationCase:
    return ProviderOperationCase(
        case_id=case_id,
        provider_service="github",
        capability_id=capability_id,
        arguments=arguments,
        assert_baseline=_seeded_repo_baseline,
        assert_outcome=exact_output(payload),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method=method,
            path=path,
            payload=payload,
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    )


EMPTY_SEARCH = {
    "total_count": 0,
    "incomplete_results": False,
    "items": [],
}

GITHUB_EMPTY_PROVIDER_OPERATION_CASES = (
    _empty_case(
        case_id="github_get_authenticated_user_empty",
        capability_id="github.get_authenticated_user",
        arguments={},
        path="/user",
        payload={},
    ),
    _empty_case(
        case_id="github_get_combined_status_empty",
        capability_id="github.get_combined_status",
        arguments={"owner": "nearai", "repo": "ironclaw", "ref": "main"},
        path=f"{REPO_PATH}/commits/main/status",
        payload={"state": "pending", "total_count": 0, "statuses": []},
    ),
    _empty_case(
        case_id="github_get_file_content_empty",
        capability_id="github.get_file_content",
        arguments={
            "owner": "nearai",
            "repo": "ironclaw",
            "path": "docs/provider-contract-empty.md",
            "ref": "main",
        },
        path=f"{REPO_PATH}/contents/docs/provider-contract-empty.md",
        payload={},
    ),
    _empty_case(
        case_id="github_get_issue_empty",
        capability_id="github.get_issue",
        arguments={"owner": "nearai", "repo": "ironclaw", "issue_number": 999},
        path=f"{REPO_PATH}/issues/999",
        payload={},
    ),
    ProviderOperationCase(
        case_id="github_get_job_logs_empty",
        provider_service="github",
        capability_id="github.get_job_logs",
        arguments={"owner": "nearai", "repo": "ironclaw", "job_id": 999},
        assert_baseline=_seeded_repo_baseline,
        assert_outcome=exact_text_output('{"status":200}'),
        outcome_class="empty",
        setup_provider_proxy=static_provider_text_response(
            method="GET",
            path=f"{REPO_PATH}/actions/jobs/999/logs",
            body="",
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    _empty_case(
        case_id="github_get_pull_request_empty",
        capability_id="github.get_pull_request",
        arguments={"owner": "nearai", "repo": "ironclaw", "pr_number": 999},
        path=f"{REPO_PATH}/pulls/999",
        payload={},
    ),
    _empty_case(
        case_id="github_get_pull_request_files_empty",
        capability_id="github.get_pull_request_files",
        arguments={"owner": "nearai", "repo": "ironclaw", "pr_number": 1},
        path=f"{REPO_PATH}/pulls/1/files",
        payload=[],
    ),
    _empty_case(
        case_id="github_get_pull_request_reviews_empty",
        capability_id="github.get_pull_request_reviews",
        arguments={"owner": "nearai", "repo": "ironclaw", "pr_number": 1},
        path=f"{REPO_PATH}/pulls/1/reviews",
        payload=[],
    ),
    _empty_case(
        case_id="github_get_repo_empty",
        capability_id="github.get_repo",
        arguments={"owner": "nearai", "repo": "provider-contract-empty"},
        path="/repos/nearai/provider-contract-empty",
        payload={},
    ),
    _empty_case(
        case_id="github_get_workflow_run_artifacts_empty",
        capability_id="github.get_workflow_run_artifacts",
        arguments={"owner": "nearai", "repo": "ironclaw", "run_id": 1001},
        path=f"{REPO_PATH}/actions/runs/1001/artifacts",
        payload={"total_count": 0, "artifacts": []},
    ),
    _empty_case(
        case_id="github_get_workflow_run_jobs_empty",
        capability_id="github.get_workflow_run_jobs",
        arguments={"owner": "nearai", "repo": "ironclaw", "run_id": 1001},
        path=f"{REPO_PATH}/actions/runs/1001/jobs",
        payload={"total_count": 0, "jobs": []},
    ),
    _empty_case(
        case_id="github_get_workflow_runs_empty",
        capability_id="github.get_workflow_runs",
        arguments={"owner": "nearai", "repo": "ironclaw"},
        path=f"{REPO_PATH}/actions/runs",
        payload={"total_count": 0, "workflow_runs": []},
    ),
    _empty_case(
        case_id="github_list_branches_empty",
        capability_id="github.list_branches",
        arguments={"owner": "nearai", "repo": "ironclaw"},
        path=f"{REPO_PATH}/branches",
        payload=[],
    ),
    _empty_case(
        case_id="github_list_issue_comments_empty",
        capability_id="github.list_issue_comments",
        arguments={"owner": "nearai", "repo": "ironclaw", "issue_number": 1},
        path=f"{REPO_PATH}/issues/1/comments",
        payload=[],
    ),
    _empty_case(
        case_id="github_list_issues_empty",
        capability_id="github.list_issues",
        arguments={"owner": "nearai", "repo": "ironclaw", "state": "closed"},
        path=f"{REPO_PATH}/issues",
        payload=[],
    ),
    _empty_case(
        case_id="github_list_pull_request_comments_empty",
        capability_id="github.list_pull_request_comments",
        arguments={"owner": "nearai", "repo": "ironclaw", "pr_number": 1},
        path=f"{REPO_PATH}/pulls/1/comments",
        payload=[],
    ),
    _empty_case(
        case_id="github_list_pull_request_review_threads_empty",
        capability_id="github.list_pull_request_review_threads",
        arguments={"owner": "nearai", "repo": "ironclaw", "pr_number": 1},
        path="/graphql",
        method="POST",
        payload={
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "nodes": [],
                            "pageInfo": {
                                "hasNextPage": False,
                                "endCursor": None,
                            },
                        }
                    }
                }
            }
        },
    ),
    _empty_case(
        case_id="github_list_pull_requests_empty",
        capability_id="github.list_pull_requests",
        arguments={"owner": "nearai", "repo": "ironclaw", "state": "closed"},
        path=f"{REPO_PATH}/pulls",
        payload=[],
    ),
    _empty_case(
        case_id="github_list_releases_empty",
        capability_id="github.list_releases",
        arguments={"owner": "nearai", "repo": "ironclaw"},
        path=f"{REPO_PATH}/releases",
        payload=[],
    ),
    _empty_case(
        case_id="github_list_repos_empty",
        capability_id="github.list_repos",
        arguments={"type": "member"},
        path="/user/repos",
        payload=[],
    ),
    _empty_case(
        case_id="github_search_code_empty",
        capability_id="github.search_code",
        arguments={"query": "NO_SUCH_PROVIDER_CONTRACT_TOKEN repo:nearai/ironclaw"},
        path="/search/code",
        payload=EMPTY_SEARCH,
    ),
    _empty_case(
        case_id="github_search_issues_empty",
        capability_id="github.search_issues",
        arguments={
            "query": "NO_SUCH_PROVIDER_CONTRACT_TOKEN",
            "repository": "nearai/ironclaw",
            "type": "issue",
        },
        path="/search/issues",
        payload=EMPTY_SEARCH,
    ),
    _empty_case(
        case_id="github_search_issues_pull_requests_empty",
        capability_id="github.search_issues_pull_requests",
        arguments={
            "query": "NO_SUCH_PROVIDER_CONTRACT_TOKEN",
            "repository": "nearai/ironclaw",
            "type": "pr",
        },
        path="/search/issues",
        payload=EMPTY_SEARCH,
    ),
    _empty_case(
        case_id="github_search_repositories_empty",
        capability_id="github.search_repositories",
        arguments={"query": "NO_SUCH_PROVIDER_CONTRACT_REPOSITORY"},
        path="/search/repositories",
        payload=EMPTY_SEARCH,
    ),
)
