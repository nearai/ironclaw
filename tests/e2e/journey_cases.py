"""Typed inventory for harvested provider and representative product journeys."""

import json
import tomllib
from collections.abc import Callable, Iterable
from pathlib import Path
from typing import TypeVar
from urllib.parse import urlparse

from journey_types import (
    EvidenceRunner,
    ExecutableEvidence,
    JourneyCase,
    JourneyDeliveryTarget,
    JourneyExecution,
    JourneyIngress,
    ObservableAssertion,
    ProviderWorld,
)
from provider_capability_inventory import EMULATE_SUPPORTED_TOOLS

ROOT = Path(__file__).resolve().parents[2]
TRACE_DIR = ROOT / "tests/fixtures/llm_traces/reborn_qa/live_canary"
MANIFEST_PATH = TRACE_DIR / "case-manifest.json"
ASSET_ROOT = ROOT / "crates/ironclaw_first_party_extensions/assets"

_TOOL_WORLD_PREFIXES = {
    "gmail__": ProviderWorld.GOOGLE,
    "google-calendar__": ProviderWorld.GOOGLE,
    "google-docs__": ProviderWorld.GOOGLE,
    "google-drive__": ProviderWorld.GOOGLE,
    "google-sheets__": ProviderWorld.GOOGLE,
    "google-slides__": ProviderWorld.GOOGLE,
    "github__": ProviderWorld.GITHUB,
    "slack__": ProviderWorld.SLACK,
}
_HTTP_WORLD_HOSTS = {
    "api.github.com": ProviderWorld.GITHUB,
}
_MUTATING_PROVIDER_TOOLS = {
    "gmail__send_message": ProviderWorld.GOOGLE,
    "google-docs__create_document": ProviderWorld.GOOGLE,
    "google-sheets__create_spreadsheet": ProviderWorld.GOOGLE,
    "google-sheets__append_values": ProviderWorld.GOOGLE,
    "slack__send_message": ProviderWorld.SLACK,
}
_REPEAT_AFTER_RESET = {
    "qa_5d_slack_strategy_doc_answer",
    "qa_10f_slack_mention_encoding",
}

_PYTEST_PROVIDER_EVIDENCE = ExecutableEvidence(
    runner=EvidenceRunner.PYTEST,
    source="tests/e2e/scenarios/test_reborn_qa_trace_full_path.py",
    test="test_qa_journey_provider_leg_replays_through_emulate",
)


def _tool_calls(trace: dict) -> list[dict]:
    return [
        call
        for step in trace["steps"]
        for call in step["response"].get("tool_calls", [])
    ]


def _provider_worlds(calls: Iterable[dict]) -> tuple[ProviderWorld, ...]:
    worlds = set()
    for call in calls:
        worlds.update(
            world
            for prefix, world in _TOOL_WORLD_PREFIXES.items()
            if call["name"].startswith(prefix)
        )
        if call["name"] == "builtin__http":
            host = urlparse(call["arguments"].get("url", "")).hostname
            if (world := _HTTP_WORLD_HOSTS.get(host)) is not None:
                worlds.add(world)
    return tuple(sorted(worlds, key=str)) or (ProviderWorld.NONE,)


def _mutable_provider_worlds(calls: Iterable[dict]) -> tuple[ProviderWorld, ...]:
    worlds = {
        world
        for call in calls
        if (world := _MUTATING_PROVIDER_TOOLS.get(call["name"])) is not None
    }
    return tuple(sorted(worlds, key=str))


def _provider_journey_cases() -> tuple[JourneyCase, ...]:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    excluded = set(manifest["no_model_cases"])
    excluded.update(manifest.get("quarantined_model_cases", []))
    cases = []
    for case_id in manifest["selected_cases"]:
        if case_id in excluded:
            continue
        trace_path = TRACE_DIR / f"{case_id}.json"
        trace = json.loads(trace_path.read_text(encoding="utf-8"))
        calls = _tool_calls(trace)
        if not any(call["name"] in EMULATE_SUPPORTED_TOOLS for call in calls):
            continue
        cases.append(
            JourneyCase(
                case_id=case_id,
                trace=str(trace_path.relative_to(ROOT)),
                provider_worlds=_provider_worlds(calls),
                mutable_provider_worlds=_mutable_provider_worlds(calls),
                ingress=JourneyIngress.WEBUI,
                execution=JourneyExecution.STANDALONE_REBORN,
                delivery_target=JourneyDeliveryTarget.WEBUI,
                assertions=(
                    ObservableAssertion.TRACE_REPLAY_COMPLETE,
                    ObservableAssertion.CAPABILITY_OUTCOMES,
                    ObservableAssertion.PROVIDER_READBACK,
                ),
                evidence=_PYTEST_PROVIDER_EVIDENCE,
                repeat_after_reset=case_id in _REPEAT_AFTER_RESET,
            )
        )
    return tuple(cases)


PROVIDER_JOURNEY_CASES = _provider_journey_cases()


def _provider_journey_runs() -> tuple[
    tuple[JourneyCase, ...],
    tuple[str, ...],
]:
    runs = []
    ids = []
    for case in PROVIDER_JOURNEY_CASES:
        runs.append(case)
        ids.append(case.case_id)
        if case.repeat_after_reset:
            runs.append(case)
            ids.append(f"{case.case_id}-isolated-repeat")
    return tuple(runs), tuple(ids)


PROVIDER_JOURNEY_RUNS, PROVIDER_JOURNEY_RUN_IDS = _provider_journey_runs()

PRODUCT_JOURNEY_CASES = (
    JourneyCase(
        case_id="webui_text_turn_persists",
        trace=None,
        provider_worlds=(ProviderWorld.NONE,),
        mutable_provider_worlds=(),
        ingress=JourneyIngress.WEBUI,
        execution=JourneyExecution.STANDALONE_REBORN,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        assertions=(ObservableAssertion.DURABLE_STATE,),
        evidence=ExecutableEvidence(
            runner=EvidenceRunner.PYTEST,
            source="tests/e2e/scenarios/test_reborn_webui_v2_smoke.py",
            test="test_reborn_v2_text_turn_persists",
        ),
    ),
    JourneyCase(
        case_id="slack_inbound_real_turn_reply",
        trace=None,
        provider_worlds=(ProviderWorld.SLACK,),
        mutable_provider_worlds=(ProviderWorld.SLACK,),
        ingress=JourneyIngress.SLACK,
        execution=JourneyExecution.REBORN_INTEGRATION,
        delivery_target=JourneyDeliveryTarget.SLACK,
        assertions=(
            ObservableAssertion.DURABLE_STATE,
            ObservableAssertion.EXACT_DESTINATION,
            ObservableAssertion.CREDENTIAL_INJECTION,
        ),
        evidence=ExecutableEvidence(
            runner=EvidenceRunner.CARGO,
            source="tests/integration/extension_delivery.rs",
            test="slack_final_reply_flows_through_the_real_delivery_coordinator",
            target="reborn_integration_extension_delivery",
        ),
    ),
    JourneyCase(
        case_id="telegram_inbound_real_turn_reply",
        trace=None,
        provider_worlds=(ProviderWorld.TELEGRAM,),
        mutable_provider_worlds=(ProviderWorld.TELEGRAM,),
        ingress=JourneyIngress.TELEGRAM,
        execution=JourneyExecution.REBORN_INTEGRATION,
        delivery_target=JourneyDeliveryTarget.TELEGRAM,
        assertions=(
            ObservableAssertion.DURABLE_STATE,
            ObservableAssertion.EXACT_DESTINATION,
            ObservableAssertion.CREDENTIAL_INJECTION,
        ),
        evidence=ExecutableEvidence(
            runner=EvidenceRunner.CARGO,
            source="tests/integration/extension_delivery.rs",
            test="telegram_update_becomes_a_turn_and_a_coordinated_reply",
            target="reborn_integration_extension_delivery",
        ),
    ),
    JourneyCase(
        case_id="scheduled_trigger_default_slack_delivery",
        trace=None,
        provider_worlds=(ProviderWorld.SLACK,),
        mutable_provider_worlds=(ProviderWorld.SLACK,),
        ingress=JourneyIngress.SCHEDULED_TRIGGER,
        execution=JourneyExecution.REBORN_INTEGRATION,
        delivery_target=JourneyDeliveryTarget.SLACK,
        assertions=(
            ObservableAssertion.DURABLE_STATE,
            ObservableAssertion.EXACT_DESTINATION,
            ObservableAssertion.EXACT_MUTATION_COUNT,
            ObservableAssertion.CREDENTIAL_INJECTION,
            ObservableAssertion.RESTART_IDEMPOTENCY,
        ),
        evidence=ExecutableEvidence(
            runner=EvidenceRunner.CARGO,
            source=("crates/ironclaw_reborn_composition/tests/trigger_poller_e2e.rs"),
            test=(
                "scheduled_trigger_results_reach_exact_slack_targets_once_"
                "across_restart"
            ),
            target="trigger_poller_e2e",
            manifest="crates/ironclaw_reborn_composition/Cargo.toml",
        ),
    ),
    JourneyCase(
        case_id="scheduled_trigger_explicit_slack_delivery",
        trace=None,
        provider_worlds=(ProviderWorld.SLACK,),
        mutable_provider_worlds=(ProviderWorld.SLACK,),
        ingress=JourneyIngress.SCHEDULED_TRIGGER,
        execution=JourneyExecution.REBORN_INTEGRATION,
        delivery_target=JourneyDeliveryTarget.SLACK,
        assertions=(
            ObservableAssertion.DURABLE_STATE,
            ObservableAssertion.EXACT_DESTINATION,
            ObservableAssertion.EXACT_MUTATION_COUNT,
            ObservableAssertion.CREDENTIAL_INJECTION,
            ObservableAssertion.RESTART_IDEMPOTENCY,
        ),
        evidence=ExecutableEvidence(
            runner=EvidenceRunner.CARGO,
            source=("crates/ironclaw_reborn_composition/tests/trigger_poller_e2e.rs"),
            test=(
                "scheduled_trigger_results_reach_exact_slack_targets_once_"
                "across_restart"
            ),
            target="trigger_poller_e2e",
            manifest="crates/ironclaw_reborn_composition/Cargo.toml",
        ),
    ),
)

ALL_JOURNEY_CASES = (*PROVIDER_JOURNEY_CASES, *PRODUCT_JOURNEY_CASES)


def _production_channel_surfaces(direction: str) -> set[str]:
    surfaces = set()
    for manifest_path in sorted(ASSET_ROOT.glob("*/manifest.toml")):
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        channel = manifest.get("channel")
        if channel is not None and channel.get(direction) is True:
            surfaces.add(manifest["id"])
    return surfaces


def required_ingresses() -> set[str]:
    """Built-in ingress plus every production channel declaring inbound."""
    return {
        JourneyIngress.WEBUI,
        JourneyIngress.SCHEDULED_TRIGGER,
        *_production_channel_surfaces("inbound"),
    }


def required_delivery_targets() -> set[str]:
    """Built-in WebUI delivery plus every production outbound channel."""
    return {
        JourneyDeliveryTarget.WEBUI,
        *_production_channel_surfaces("outbound"),
    }


T = TypeVar("T")


def uncovered_surfaces(
    required: Iterable[str],
    cases: Iterable[JourneyCase],
    selector: Callable[[JourneyCase], T],
) -> set[str]:
    """Return required surface IDs with no typed journey evidence."""
    covered = {str(selector(case)) for case in cases}
    return {str(surface) for surface in required} - covered
