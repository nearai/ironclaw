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
    #: The vendor-neutral mount every channel extension rides,
    #: `/webhooks/extensions/{extension_id}/{route_suffix}`. Listed separately
    #: from the named channels because it is the surface a *new* extension
    #: arrives on: its evidence has to hold for an extension the host has
    #: never heard of, which is what the acme fixture exists to prove.
    EXTENSION_WEBHOOK = "extension_webhook"


class JourneyExecution(StrEnum):
    STANDALONE_REBORN = "standalone_reborn"
    REBORN_INTEGRATION = "reborn_integration"


class JourneyDeliveryTarget(StrEnum):
    WEBUI = "webui"
    SLACK = "slack"
    TELEGRAM = "telegram"
    #: The journey ends before any reply is delivered. Ingress proofs that
    #: stop at durable turn admission use this rather than naming a target
    #: they never reach -- claiming one would make the inventory read as
    #: delivery evidence it does not have.
    NONE = "none"


class ObservableAssertion(StrEnum):
    TRACE_REPLAY_COMPLETE = "trace_replay_complete"
    CAPABILITY_OUTCOMES = "capability_outcomes"
    PROVIDER_READBACK = "provider_readback"
    DURABLE_STATE = "durable_state"
    EXACT_DESTINATION = "exact_destination"
    EXACT_MUTATION_COUNT = "exact_mutation_count"
    CREDENTIAL_INJECTION = "credential_injection"
    RESTART_IDEMPOTENCY = "restart_idempotency"


class SlackChannelFixture(StrEnum):
    """Provider-side Slack destination selected during trace compilation."""

    SEEDED = "seeded"
    MISSING = "missing"


@dataclass(frozen=True)
class ProviderJourneyReplayFacts:
    """Small typed sidecar for deterministic provider replay compilation."""

    google_spreadsheet_id: str = "sheet_reborn_abc"
    slack_channel: SlackChannelFixture = SlackChannelFixture.SEEDED
    expected_capability_failure: str | None = None
    timeout_seconds: int = 120


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
class LiveEvidence:
    """One scheduled live case and the artifact that carries its result."""

    workflow: str
    job: str
    case_id: str
    artifact: str


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
    live_evidence: LiveEvidence
    replay: ProviderJourneyReplayFacts = ProviderJourneyReplayFacts()
    repeat_after_reset: bool = False


@dataclass(frozen=True)
class ProductJourneyCase(JourneyCaseBase):
    """A trace-less product journey proved by its owning executable test."""

    browser_evidence: PytestEvidence | None = None


JourneyCase: TypeAlias = ProviderJourneyCase | ProductJourneyCase
