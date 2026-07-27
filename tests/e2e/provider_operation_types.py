"""Shared types for full-path provider operation cases."""

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Literal

BaselineAssertion = Callable[[str], Awaitable[None]]
OutcomeAssertion = Callable[[str, dict], Awaitable[None]]
ArgumentsFactory = Callable[[str], Awaitable[dict]]
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

    async def resolve_arguments(self, emulate_url: str) -> dict:
        """Resolve static arguments or provider-issued values after setup."""
        if callable(self.arguments):
            return await self.arguments(emulate_url)
        return self.arguments
