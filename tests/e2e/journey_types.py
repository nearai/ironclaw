"""Typed vocabulary shared by whole-path journey registries and runners."""

from dataclasses import dataclass
from enum import StrEnum


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


class EvidenceRunner(StrEnum):
    PYTEST = "pytest"
    CARGO = "cargo"


@dataclass(frozen=True)
class ExecutableEvidence:
    """One exact test declaration that CI can execute."""

    runner: EvidenceRunner
    source: str
    test: str
    target: str | None = None
    manifest: str | None = None


@dataclass(frozen=True)
class JourneyCase:
    """One declarative whole-path proof; runners retain execution logic."""

    case_id: str
    trace: str | None
    provider_worlds: tuple[ProviderWorld, ...]
    mutable_provider_worlds: tuple[ProviderWorld, ...]
    ingress: JourneyIngress
    execution: JourneyExecution
    delivery_target: JourneyDeliveryTarget
    assertions: tuple[ObservableAssertion, ...]
    evidence: ExecutableEvidence
    repeat_after_reset: bool = False
