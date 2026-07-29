"""Shared types for full-path provider operation cases."""

import hashlib
import json
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any, Literal

from provider_fault_proxy import ProviderFaultProfile

BaselineAssertion = Callable[[str], Awaitable[None]]
OutcomeAssertion = Callable[[str, dict], Awaitable[None]]
ArgumentsFactory = Callable[[str], Awaitable[dict]]
ProviderProxySetup = Callable[[Any], None]
ProviderService = Literal["google", "github", "slack"]

# Which provider-observable outcome this case pins. A capability covered only
# by `success` is a happy path, not a contract: `empty` is what proves the
# runtime distinguishes "no results" from "call failed". Status/transport
# failures belong to the reusable fault profiles, not here.
OutcomeClass = Literal["success", "empty"]


@dataclass(frozen=True)
class ProviderOperationCase:
    """One capability invocation with provider-observable proof."""

    case_id: str
    provider_service: ProviderService
    capability_id: str
    arguments: dict | ArgumentsFactory
    assert_baseline: BaselineAssertion
    assert_outcome: OutcomeAssertion
    outcome_class: OutcomeClass = "success"
    expected_request_count: int = 1
    setup_provider_proxy: ProviderProxySetup | None = None
    expect_provider_forward: bool = True
    expected_proxy_profile: str | None = None
    expected_forwarded_request_count: int | None = None
    expected_profile_request_count: int | None = None

    async def resolve_arguments(self, emulate_url: str) -> dict:
        """Resolve static arguments or provider-issued values after setup."""
        if callable(self.arguments):
            return await self.arguments(emulate_url)
        return self.arguments


def static_provider_json_response(
    *,
    method: str,
    path: str,
    payload: Any,
) -> ProviderProxySetup:
    """Return setup for one provider-authentic empty success response."""
    profile = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body=json.dumps(payload, separators=(",", ":")),
    )

    def setup(proxy: Any) -> None:
        proxy.arm(profile, method=method, path=path)

    return setup


def static_provider_text_response(
    *,
    method: str,
    path: str,
    body: str,
) -> ProviderProxySetup:
    """Return setup for one provider-authentic plain-text success response."""
    profile = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body=body,
        content_type="text/plain",
    )

    def setup(proxy: Any) -> None:
        proxy.arm(profile, method=method, path=path)

    return setup


def exact_output(expected: Any) -> OutcomeAssertion:
    """Require a complete, non-truncated capability result with exact JSON."""

    async def assert_outcome(_emulate_url: str, preview: dict) -> None:
        assert preview["truncated"] is False, preview
        assert json.loads(preview["output_preview"]) == expected, preview

    return assert_outcome


def exact_text_output(expected: str) -> OutcomeAssertion:
    """Require a complete text result without JSON coercion."""

    async def assert_outcome(_emulate_url: str, preview: dict) -> None:
        assert preview["truncated"] is False, preview
        assert preview["output_kind"] == "text", preview
        assert preview.get("output_preview", "") == expected, preview

    return assert_outcome


def exact_provider_http_output(body: Any) -> OutcomeAssertion:
    """Require the first-party HTTP executor's complete provider response."""
    return exact_output(
        {
            "body": body,
            "redaction_applied": True,
            "status": 200,
        }
    )


def provider_http_body(preview: dict) -> Any:
    """Return a fully observed first-party HTTP body after pinning its envelope."""
    assert preview["truncated"] is False, preview
    output = json.loads(preview["output_preview"])
    assert set(output) == {"body", "redaction_applied", "status"}, output
    assert output["redaction_applied"] is True, output
    assert output["status"] == 200, output
    return output["body"]


def assert_provider_request_evidence(
    operation_case: ProviderOperationCase,
    requests: list[dict],
    *,
    expected_bearer: str | None,
    excluded_bearers: tuple[str, ...] = (),
) -> None:
    """Require exact, authenticated provider request evidence for a case."""
    excluded_fingerprints = {
        hashlib.sha256(f"Bearer {bearer}".encode()).hexdigest()[:12]
        for bearer in excluded_bearers
    }
    requests = [
        request
        for request in requests
        if request["credential_fingerprint"] not in excluded_fingerprints
    ]
    assert len(requests) == operation_case.expected_request_count, (
        operation_case.case_id,
        requests,
    )
    expected_fingerprint = (
        hashlib.sha256(f"Bearer {expected_bearer}".encode()).hexdigest()[:12]
        if expected_bearer is not None
        else None
    )
    expected_forwarded_count = operation_case.expected_forwarded_request_count
    if expected_forwarded_count is None:
        expected_forwarded_count = (
            operation_case.expected_request_count
            if operation_case.expect_provider_forward
            else 0
        )
    assert sum(request["forwarded"] for request in requests) == (
        expected_forwarded_count
    ), requests
    expected_profile_count = operation_case.expected_profile_request_count
    if expected_profile_count is None:
        expected_profile_count = (
            operation_case.expected_request_count
            if operation_case.expected_proxy_profile is not None
            else 0
        )
    assert all(
        request["fault"] in {None, operation_case.expected_proxy_profile}
        for request in requests
    ), requests
    assert sum(request["fault"] is not None for request in requests) == (
        expected_profile_count
    ), requests
    observed_fingerprints = {
        request["credential_fingerprint"] for request in requests
    }
    assert None not in observed_fingerprints, requests
    assert len(observed_fingerprints) == 1, requests
    if expected_fingerprint is not None:
        assert observed_fingerprints == {expected_fingerprint}, requests
    for request in requests:
        assert request["service"] == operation_case.provider_service, request
        assert request["responded"] is True, request
        if request["forwarded"]:
            assert 200 <= request["upstream_status"] < 300, request
