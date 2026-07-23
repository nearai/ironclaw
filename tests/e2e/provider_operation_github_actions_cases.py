"""GitHub Actions full-path provider operation cases."""

import json

import httpx

from emulate_provider import github_headers
from provider_operation_github_common import OWNER, REPO, REPO_PATH, github_request
from provider_operation_types import ProviderOperationCase

BASE_ARGS = {"owner": OWNER, "repo": REPO}
RUN_ID = 1001
JOB_ID = 2001
ARTIFACT_ID = 3001
JOB_LOG = "REBORN_PROVIDER_CASE_JOB_LOG"
ARTIFACT_NAME = "provider-results"


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


async def _workflow_runs_outcome(emulate_url: str, preview: dict) -> None:
    result = await github_request(
        emulate_url, "GET", f"{REPO_PATH}/actions/runs"
    )
    assert isinstance(result, dict)
    assert result["total_count"] == 1, result
    assert result["workflow_runs"][0]["id"] == RUN_ID, result
    output = json.loads(preview["output_preview"])
    assert output["workflow_runs"][0]["name"] == "CI", output
    assert output["workflow_runs"][0]["conclusion"] == "failure", output


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
    assert JOB_LOG in json.dumps(preview), preview


async def _artifacts_outcome(emulate_url: str, preview: dict) -> None:
    await _actions_baseline(emulate_url)
    assert ARTIFACT_NAME in json.dumps(preview), preview


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


GITHUB_ACTIONS_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="github_get_workflow_runs",
        provider_service="github",
        capability_id="github.get_workflow_runs",
        arguments=BASE_ARGS,
        assert_baseline=_actions_baseline,
        assert_outcome=_workflow_runs_outcome,
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
