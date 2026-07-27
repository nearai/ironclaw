"""GitHub issue full-path provider operation cases."""

import json

from provider_operation_github_common import (
    ISSUE_TITLE,
    OWNER,
    REPO,
    REPO_PATH,
    issue,
    github_request,
    seed_issue,
)
from provider_operation_types import ProviderOperationCase

CREATED_ISSUE = "REBORN_PROVIDER_CASE_CREATED_ISSUE"
UPDATED_ISSUE = "REBORN_PROVIDER_CASE_UPDATED_ISSUE"
COMMENT = "REBORN_PROVIDER_CASE_ISSUE_COMMENT"
SEEDED_COMMENT = "REBORN_PROVIDER_CASE_SEEDED_COMMENT"
LABEL = "provider-case"
ASSIGNEE = "reborn-reviewer"
BASE_ARGS = {"owner": OWNER, "repo": REPO}


async def _empty_issues(emulate_url: str) -> None:
    items = await github_request(emulate_url, "GET", f"{REPO_PATH}/issues")
    assert items == [], items


async def _seeded_issue(emulate_url: str) -> None:
    await seed_issue(emulate_url)


async def _seeded_label_issue(emulate_url: str, *, assigned: bool) -> None:
    await seed_issue(emulate_url)
    await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/labels",
        payload={"name": LABEL, "color": "3367d6"},
        expected_status=201,
    )
    if assigned:
        await github_request(
            emulate_url,
            "POST",
            f"{REPO_PATH}/issues/1/labels",
            payload={"labels": [LABEL]},
        )


async def _label_add_baseline(emulate_url: str) -> None:
    await _seeded_label_issue(emulate_url, assigned=False)


async def _label_remove_baseline(emulate_url: str) -> None:
    await _seeded_label_issue(emulate_url, assigned=True)


async def _seeded_assignee_issue(emulate_url: str, *, assigned: bool) -> None:
    await seed_issue(emulate_url)
    if assigned:
        await github_request(
            emulate_url,
            "POST",
            f"{REPO_PATH}/issues/1/assignees",
            payload={"assignees": [ASSIGNEE]},
        )


async def _assignee_add_baseline(emulate_url: str) -> None:
    await _seeded_assignee_issue(emulate_url, assigned=False)


async def _assignee_remove_baseline(emulate_url: str) -> None:
    await _seeded_assignee_issue(emulate_url, assigned=True)


async def _seeded_comment_baseline(emulate_url: str) -> None:
    await seed_issue(emulate_url)
    await github_request(
        emulate_url,
        "POST",
        f"{REPO_PATH}/issues/1/comments",
        payload={"body": SEEDED_COMMENT},
        expected_status=201,
    )


async def _created_issue_outcome(emulate_url: str, preview: dict) -> None:
    created = await issue(emulate_url)
    assert created["title"] == CREATED_ISSUE, created
    assert CREATED_ISSUE in json.dumps(preview), preview


async def _seeded_issue_read_outcome(emulate_url: str, preview: dict) -> None:
    created = await issue(emulate_url)
    assert created["title"] == ISSUE_TITLE, created
    assert ISSUE_TITLE in json.dumps(preview), preview


async def _updated_issue_outcome(emulate_url: str, preview: dict) -> None:
    updated = await issue(emulate_url)
    assert updated["title"] == UPDATED_ISSUE, updated
    assert updated["state"] == "closed", updated
    assert UPDATED_ISSUE in json.dumps(preview), preview


def _comment_outcome(marker: str):
    async def assert_outcome(emulate_url: str, preview: dict) -> None:
        comments = await github_request(
            emulate_url, "GET", f"{REPO_PATH}/issues/1/comments"
        )
        assert isinstance(comments, list)
        assert [comment["body"] for comment in comments] == [marker], comments
        assert marker in json.dumps(preview), preview

    return assert_outcome


async def _label_add_outcome(emulate_url: str, preview: dict) -> None:
    updated = await issue(emulate_url)
    assert [label["name"] for label in updated["labels"]] == [LABEL], updated
    assert LABEL in json.dumps(preview), preview


async def _label_remove_outcome(emulate_url: str, preview: dict) -> None:
    updated = await issue(emulate_url)
    assert updated["labels"] == [], updated
    assert LABEL in json.dumps(preview), preview


async def _assignee_add_outcome(emulate_url: str, preview: dict) -> None:
    updated = await issue(emulate_url)
    assert [assignee["login"] for assignee in updated["assignees"]] == [
        ASSIGNEE
    ], updated
    assert ASSIGNEE in json.dumps(preview), preview


async def _assignee_remove_outcome(emulate_url: str, preview: dict) -> None:
    updated = await issue(emulate_url)
    assert updated["assignees"] == [], updated
    assert ASSIGNEE in json.dumps(preview), preview


GITHUB_ISSUE_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="github_create_issue",
        provider_service="github",
        capability_id="github.create_issue",
        arguments={
            **BASE_ARGS,
            "title": CREATED_ISSUE,
            "body": "Created through the provider operation runner.",
        },
        assert_baseline=_empty_issues,
        assert_outcome=_created_issue_outcome,
    ),
    ProviderOperationCase(
        case_id="github_get_issue",
        provider_service="github",
        capability_id="github.get_issue",
        arguments={**BASE_ARGS, "issue_number": 1},
        assert_baseline=_seeded_issue,
        assert_outcome=_seeded_issue_read_outcome,
    ),
    ProviderOperationCase(
        case_id="github_list_issues",
        provider_service="github",
        capability_id="github.list_issues",
        arguments={**BASE_ARGS, "state": "open"},
        assert_baseline=_seeded_issue,
        assert_outcome=_seeded_issue_read_outcome,
    ),
    ProviderOperationCase(
        case_id="github_search_issues",
        provider_service="github",
        capability_id="github.search_issues",
        arguments={
            "query": ISSUE_TITLE,
            "repository": f"{OWNER}/{REPO}",
            "type": "issue",
        },
        assert_baseline=_seeded_issue,
        assert_outcome=_seeded_issue_read_outcome,
    ),
    ProviderOperationCase(
        case_id="github_search_issues_pull_requests",
        provider_service="github",
        capability_id="github.search_issues_pull_requests",
        arguments={
            "query": ISSUE_TITLE,
            "repository": f"{OWNER}/{REPO}",
            "type": "issue",
        },
        assert_baseline=_seeded_issue,
        assert_outcome=_seeded_issue_read_outcome,
    ),
    ProviderOperationCase(
        case_id="github_update_issue",
        provider_service="github",
        capability_id="github.update_issue",
        arguments={
            **BASE_ARGS,
            "issue_number": 1,
            "title": UPDATED_ISSUE,
            "state": "closed",
        },
        assert_baseline=_seeded_issue,
        assert_outcome=_updated_issue_outcome,
    ),
    ProviderOperationCase(
        case_id="github_create_issue_comment",
        provider_service="github",
        capability_id="github.create_issue_comment",
        arguments={**BASE_ARGS, "issue_number": 1, "body": COMMENT},
        assert_baseline=_seeded_issue,
        assert_outcome=_comment_outcome(COMMENT),
    ),
    ProviderOperationCase(
        case_id="github_comment_issue",
        provider_service="github",
        capability_id="github.comment_issue",
        arguments={**BASE_ARGS, "issue_number": 1, "body": COMMENT},
        assert_baseline=_seeded_issue,
        assert_outcome=_comment_outcome(COMMENT),
    ),
    ProviderOperationCase(
        case_id="github_list_issue_comments",
        provider_service="github",
        capability_id="github.list_issue_comments",
        arguments={**BASE_ARGS, "issue_number": 1},
        assert_baseline=_seeded_comment_baseline,
        assert_outcome=_comment_outcome(SEEDED_COMMENT),
    ),
    ProviderOperationCase(
        case_id="github_add_issue_labels",
        provider_service="github",
        capability_id="github.add_issue_labels",
        arguments={**BASE_ARGS, "issue_number": 1, "labels": [LABEL]},
        assert_baseline=_label_add_baseline,
        assert_outcome=_label_add_outcome,
    ),
    ProviderOperationCase(
        case_id="github_remove_issue_label",
        provider_service="github",
        capability_id="github.remove_issue_label",
        arguments={**BASE_ARGS, "issue_number": 1, "name": LABEL},
        assert_baseline=_label_remove_baseline,
        assert_outcome=_label_remove_outcome,
    ),
    ProviderOperationCase(
        case_id="github_add_issue_assignees",
        provider_service="github",
        capability_id="github.add_issue_assignees",
        arguments={**BASE_ARGS, "issue_number": 1, "assignees": [ASSIGNEE]},
        assert_baseline=_assignee_add_baseline,
        assert_outcome=_assignee_add_outcome,
    ),
    ProviderOperationCase(
        case_id="github_remove_issue_assignees",
        provider_service="github",
        capability_id="github.remove_issue_assignees",
        arguments={**BASE_ARGS, "issue_number": 1, "assignees": [ASSIGNEE]},
        assert_baseline=_assignee_remove_baseline,
        assert_outcome=_assignee_remove_outcome,
    ),
)
