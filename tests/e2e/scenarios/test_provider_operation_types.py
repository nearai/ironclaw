"""Sabotage checks for provider-operation request evidence."""

import copy
import hashlib
from types import SimpleNamespace

import pytest

from provider_operation_types import assert_provider_request_evidence

BEARER = "provider-operation-evidence-token"


def _case():
    return SimpleNamespace(
        case_id="synthetic-provider-operation",
        provider_service="github",
        expected_request_count=1,
        expected_forwarded_request_count=None,
        expect_provider_forward=True,
        expected_profile_request_count=None,
        expected_proxy_profile=None,
    )


def _request():
    return {
        "service": "github",
        "credential_fingerprint": hashlib.sha256(
            f"Bearer {BEARER}".encode()
        ).hexdigest()[:12],
        "fault": None,
        "forwarded": True,
        "upstream_status": 200,
        "responded": True,
    }


def test_provider_request_evidence_accepts_observed_authenticated_success():
    assert_provider_request_evidence(_case(), [_request()], expected_bearer=BEARER)


def test_provider_request_evidence_rejects_wrong_bound_account_fingerprint():
    with pytest.raises(AssertionError):
        assert_provider_request_evidence(
            _case(),
            [_request()],
            expected_credential_fingerprint="wrong-bound-account",
        )


@pytest.mark.parametrize(
    ("field", "sabotaged"),
    [
        ("service", "slack"),
        ("credential_fingerprint", "wrong-account"),
        ("forwarded", False),
        ("responded", False),
        ("upstream_status", 500),
    ],
)
def test_provider_request_evidence_rejects_sabotaged_request(
    field: str, sabotaged
):
    request = copy.deepcopy(_request())
    request[field] = sabotaged
    with pytest.raises(AssertionError):
        assert_provider_request_evidence(
            _case(), [request], expected_bearer=BEARER
        )


@pytest.mark.parametrize("requests", [[], [_request(), _request()]])
def test_provider_request_evidence_rejects_missing_or_duplicate_dispatch(
    requests: list[dict],
):
    with pytest.raises(AssertionError):
        assert_provider_request_evidence(
            _case(), requests, expected_bearer=BEARER
        )
