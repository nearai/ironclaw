"""Shared types for full-path provider operation cases."""

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Literal

BaselineAssertion = Callable[[str], Awaitable[None]]
OutcomeAssertion = Callable[[str, dict], Awaitable[None]]
ProviderService = Literal["google", "github", "slack"]


@dataclass(frozen=True)
class ProviderOperationCase:
    """One capability invocation with provider-observable proof."""

    case_id: str
    provider_service: ProviderService
    capability_id: str
    arguments: dict
    assert_baseline: BaselineAssertion
    assert_outcome: OutcomeAssertion
