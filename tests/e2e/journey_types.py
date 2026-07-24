"""Typed vocabulary shared by whole-path journey registries and runners."""

from dataclasses import dataclass
from enum import StrEnum
from typing import TypeAlias


class ProviderWorld(StrEnum):
    NONE = "none"
    GOOGLE = "google"
    GITHUB = "github"
    SLACK = "slack"
    TELEGRAM = "telegram"


class JourneyIngress(StrEnum):
    WEBUI = "webui"
    SLACK = "slack"
    TELEGRAM = "telegram"
    SCHEDULED_TRIGGER = "scheduled_trigger"


class JourneyExecution(StrEnum):
    STANDALONE_REBORN = "standalone_reborn"
    REBORN_INTEGRATION = "reborn_integration"


class JourneyDeliveryTarget(StrEnum):
    WEBUI = "webui"
    SLACK = "slack"
    TELEGRAM = "telegram"


class ObservableAssertion(StrEnum):
    TRACE_REPLAY_COMPLETE = "trace_replay_complete"
    CAPABILITY_OUTCOMES = "capability_outcomes"
    PROVIDER_READBACK = "provider_readback"
    DURABLE_STATE = "durable_state"
    EXACT_DESTINATION = "exact_destination"
    EXACT_MUTATION_COUNT = "exact_mutation_count"
    CREDENTIAL_INJECTION = "credential_injection"
    RESTART_IDEMPOTENCY = "restart_idempotency"


@dataclass(frozen=True)
class PytestEvidence:
    """One exact Pytest declaration that CI can execute."""

    source: str
    test: str


@dataclass(frozen=True)
class CargoEvidence:
    """One exact Cargo test declaration that CI can execute."""

    source: str
    test: str
    target: str
    manifest: str | None = None


@dataclass(frozen=True)
class JourneyCaseBase:
    """Shared metadata for one declarative whole-path proof."""

    case_id: str
    provider_worlds: tuple[ProviderWorld, ...]
    mutable_provider_worlds: tuple[ProviderWorld, ...]
    ingress: JourneyIngress
    execution: JourneyExecution
    delivery_target: JourneyDeliveryTarget
    assertions: tuple[ObservableAssertion, ...]
    evidence: PytestEvidence | CargoEvidence


@dataclass(frozen=True)
class ProviderJourneyCase(JourneyCaseBase):
    """A harvested provider journey whose full-path runner requires a trace."""

    trace: str
    repeat_after_reset: bool = False


@dataclass(frozen=True)
class ProductJourneyCase(JourneyCaseBase):
    """A trace-less product journey proved by its owning executable test."""


JourneyCase: TypeAlias = ProviderJourneyCase | ProductJourneyCase
