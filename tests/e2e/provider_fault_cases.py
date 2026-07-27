"""Representative provider fault cases selected by operation equivalence class."""

from dataclasses import dataclass
from typing import Literal

from provider_operation_cases import PROVIDER_OPERATION_CASES
from provider_operation_types import ProviderOperationCase

ProviderFaultOutcome = Literal["unchanged", "committed_without_ack"]


def _operation(case_id: str) -> ProviderOperationCase:
    return next(
        case for case in PROVIDER_OPERATION_CASES if case.case_id == case_id
    )


@dataclass(frozen=True)
class ProviderFaultCase:
    """One fault applied to a representative provider operation class."""

    case_id: str
    operation: ProviderOperationCase
    profile: str
    method: str
    path: str
    expected_tool_result: str
    expected_outcome: ProviderFaultOutcome
    expected_preview_error: str | None = None
    expected_forwarded: bool = False


PROVIDER_FAULT_CASES = (
    ProviderFaultCase(
        case_id="read_forbidden",
        operation=_operation("github_get_issue"),
        profile="http_403",
        method="GET",
        path="/repos/nearai/ironclaw/issues/1",
        expected_tool_result="github_api_error_status_403",
        expected_preview_error="github_api_error_status_403",
        expected_outcome="unchanged",
    ),
    ProviderFaultCase(
        case_id="read_rate_limited",
        operation=_operation("github_get_issue"),
        profile="http_429",
        method="GET",
        path="/repos/nearai/ironclaw/issues/1",
        expected_tool_result="github_api_error_status_429",
        expected_preview_error="github_api_error_status_429",
        expected_outcome="unchanged",
    ),
    ProviderFaultCase(
        case_id="read_unavailable",
        operation=_operation("github_get_issue"),
        profile="http_503",
        method="GET",
        path="/repos/nearai/ironclaw/issues/1",
        expected_tool_result="github_api_error_status_503",
        expected_preview_error="github_api_error_status_503",
        expected_outcome="unchanged",
    ),
    ProviderFaultCase(
        case_id="read_malformed_json",
        operation=_operation("github_get_issue"),
        profile="malformed_json",
        method="GET",
        path="/repos/nearai/ironclaw/issues/1",
        expected_tool_result='"status": "error"',
        expected_outcome="unchanged",
    ),
    ProviderFaultCase(
        case_id="read_connection_reset",
        operation=_operation("github_get_issue"),
        profile="connection_reset",
        method="GET",
        path="/repos/nearai/ironclaw/issues/1",
        expected_tool_result='"status": "error"',
        expected_outcome="unchanged",
    ),
    ProviderFaultCase(
        case_id="read_timeout",
        operation=_operation("github_get_issue"),
        profile="timeout",
        method="GET",
        path="/repos/nearai/ironclaw/issues/1",
        expected_tool_result="github_api_request_failed",
        expected_preview_error="github_api_request_failed",
        expected_outcome="unchanged",
    ),
    ProviderFaultCase(
        case_id="idempotent_write_unavailable",
        operation=_operation("github_update_issue"),
        profile="http_503",
        method="PATCH",
        path="/repos/nearai/ironclaw/issues/1",
        expected_tool_result="github_api_error_status_503",
        expected_preview_error="github_api_error_status_503",
        expected_outcome="unchanged",
    ),
    ProviderFaultCase(
        case_id="non_idempotent_write_lost_acknowledgement",
        operation=_operation("github_create_issue"),
        profile="lost_acknowledgement",
        method="POST",
        path="/repos/nearai/ironclaw/issues",
        expected_tool_result="github_api_request_failed",
        expected_preview_error="github_api_request_failed",
        expected_outcome="committed_without_ack",
        expected_forwarded=True,
    ),
)
