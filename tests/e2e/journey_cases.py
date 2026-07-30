"""Typed inventory for harvested provider and representative product journeys."""

import json
import os
from collections.abc import Callable, Iterable
from pathlib import Path
from typing import TypeVar
from urllib.parse import urlparse

import tomllib
from journey_types import (
    CargoEvidence,
    JourneyCase,
    JourneyDeliveryTarget,
    JourneyExecution,
    JourneyIngress,
    ObservableAssertion,
    ProductJourneyCase,
    ProviderJourneyCase,
    ProviderJourneyReplayFacts,
    ProviderWorld,
    PytestEvidence,
    SlackChannelFixture,
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
# The five tools this set used to name by hand. Kept only as a regression
# floor: if the derivation below ever stops finding them, it has broken.
_HISTORICAL_MUTATING_PROVIDER_TOOLS = frozenset(
    {
        "gmail__send_message",
        "google-docs__create_document",
        "google-sheets__create_spreadsheet",
        "google-sheets__append_values",
        "slack__send_message",
    }
)


def _production_mutating_tools() -> dict[str, ProviderWorld]:
    """Provider-world writes, taken from the shipped manifests.

    A journey that mutates a provider world must declare that world so the
    harness resets it afterwards; otherwise whatever the journey created
    survives into the next test. Which tools mutate is not a judgement call --
    production already states it, as the `external_write` effect on each tool
    (`crates/ironclaw_first_party_extensions/assets/*/manifest.toml`).

    This used to be a hand-kept list of five names while production declared
    seventy such tools. Every one of the other sixty-five -- `github__create_issue`,
    `google-drive__upload_file`, and so on -- would have run without marking its
    world mutable, so no reset fired and the leak guards this workstream added
    never got the chance to.
    """
    mutating: dict[str, ProviderWorld] = {}
    for manifest_path in sorted(ASSET_ROOT.glob("*/manifest.toml")):
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        for tool in manifest.get("tools", []) or []:
            if "external_write" not in (tool.get("effects") or []):
                continue
            # Manifest ids are `github.create_issue`; traces record
            # `github__create_issue`.
            tool_name = str(tool["id"]).replace(".", "__", 1)
            world = next(
                (
                    world
                    for prefix, world in _TOOL_WORLD_PREFIXES.items()
                    if tool_name.startswith(prefix)
                ),
                None,
            )
            # Tools outside a world the harness can reset (web-access, nearai)
            # are not skipped silently -- `unreset_mutating_tools()` below is
            # what reports them.
            if world is not None:
                mutating[tool_name] = world
    return mutating


_MUTATING_PROVIDER_TOOLS = _production_mutating_tools()


def unreset_mutating_tools() -> frozenset[str]:
    """Production writes whose provider world no fixture can reset."""
    unreset = set()
    for manifest_path in sorted(ASSET_ROOT.glob("*/manifest.toml")):
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        for tool in manifest.get("tools", []) or []:
            if "external_write" not in (tool.get("effects") or []):
                continue
            tool_name = str(tool["id"]).replace(".", "__", 1)
            if tool_name not in _MUTATING_PROVIDER_TOOLS:
                unreset.add(tool_name)
    return frozenset(unreset)


_REPEAT_AFTER_RESET = {
    "qa_5d_slack_strategy_doc_answer",
    "qa_10f_slack_mention_encoding",
}
_PROVIDER_REPLAY_FACTS = {
    "qa_7c_slack_bug_logger_routine": ProviderJourneyReplayFacts(
        google_spreadsheet_id="sheet_reborn_bug_tracker"
    ),
    "qa_7e_slack_bug_sheet_delivery": ProviderJourneyReplayFacts(
        google_spreadsheet_id="sheet_reborn_bug_tracker"
    ),
    "qa_10e_slack_error_honesty": ProviderJourneyReplayFacts(
        slack_channel=SlackChannelFixture.MISSING,
        expected_capability_failure="channel_not_found",
    ),
}

_PYTEST_PROVIDER_EVIDENCE = PytestEvidence(
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


def _provider_journey_cases() -> tuple[ProviderJourneyCase, ...]:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    excluded = set(manifest["no_model_cases"])
    excluded.update(manifest.get("quarantined_model_cases", []))
    cases = []
    consumed_replay_facts = set()
    for case_id in manifest["selected_cases"]:
        if case_id in excluded:
            continue
        trace_path = TRACE_DIR / f"{case_id}.json"
        trace = json.loads(trace_path.read_text(encoding="utf-8"))
        calls = _tool_calls(trace)
        if not any(call["name"] in EMULATE_SUPPORTED_TOOLS for call in calls):
            continue
        if case_id in _PROVIDER_REPLAY_FACTS:
            consumed_replay_facts.add(case_id)
        cases.append(
            ProviderJourneyCase(
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
                replay=_PROVIDER_REPLAY_FACTS.get(
                    case_id, ProviderJourneyReplayFacts()
                ),
                repeat_after_reset=case_id in _REPEAT_AFTER_RESET,
            )
        )
    assert consumed_replay_facts == set(_PROVIDER_REPLAY_FACTS), (
        "replay facts declared for unknown provider journey cases: "
        f"{sorted(set(_PROVIDER_REPLAY_FACTS) - consumed_replay_facts)}"
    )
    return tuple(cases)


PROVIDER_JOURNEY_CASES = _provider_journey_cases()


JOURNEY_ORDER_ENV = "IRONCLAW_JOURNEY_ORDER"


def journey_order_is_reversed() -> bool:
    """Whether CI explicitly selected the shared-world reverse proof.

    The dedicated scenario owns ordering and provider lifecycle. This selector
    makes that expensive lane fail closed if workflow wiring drops its intent.
    """
    return os.environ.get(JOURNEY_ORDER_ENV, "").strip().lower() == "reverse"


def provider_journey_runs(
    *,
    reverse: bool = False,
) -> tuple[tuple[ProviderJourneyCase, ...], tuple[str, ...]]:
    """Journey runs and their ids, forward or reversed.

    Takes `reverse` explicitly rather than reading the environment so the
    ordering itself is testable without mutating process state — a reversed
    lane that silently ran forward would look exactly like a passing lane, and
    would quietly retire the proof it was added to provide.
    """
    runs = []
    ids = []
    for case in PROVIDER_JOURNEY_CASES:
        runs.append(case)
        ids.append(case.case_id)
        if case.repeat_after_reset:
            runs.append(case)
            ids.append(f"{case.case_id}-isolated-repeat")
    if reverse:
        runs.reverse()
        ids.reverse()
    return tuple(runs), tuple(ids)


def shared_world_provider_journey_runs(
    *,
    reverse: bool = False,
) -> tuple[tuple[ProviderJourneyCase, ...], tuple[str, ...]]:
    """Mutating journeys to replay without provider resets between cases.

    Isolation repeats belong to the ordinary runner: repeating them here would
    deliberately collide with their own mutation rather than expose leakage
    from a different journey.
    """
    runs = [case for case in PROVIDER_JOURNEY_CASES if case.mutable_provider_worlds]
    if reverse:
        runs.reverse()
    return tuple(runs), tuple(case.case_id for case in runs)


# The ordinary replay owns per-case isolation. Reversed ordering is reserved
# for the dedicated shared-world scenario, where ordering can affect outcomes.
PROVIDER_JOURNEY_RUNS, PROVIDER_JOURNEY_RUN_IDS = provider_journey_runs()

PRODUCT_JOURNEY_CASES = (
    ProductJourneyCase(
        case_id="generic_extension_webhook_signed_post_becomes_a_turn",
        provider_worlds=(ProviderWorld.NONE,),
        mutable_provider_worlds=(),
        ingress=JourneyIngress.EXTENSION_WEBHOOK,
        execution=JourneyExecution.REBORN_INTEGRATION,
        # The cited test ends at durable turn admission: its scripted reply is
        # never consulted and nothing is delivered, so naming a delivery
        # target would overstate it.
        delivery_target=JourneyDeliveryTarget.NONE,
        # Admission only. The test registers its ingress secret directly, so
        # the webhook never crosses the runtime credential-injection path --
        # claiming that assertion would credit this row with coverage that
        # lives elsewhere.
        assertions=(ObservableAssertion.DURABLE_STATE,),
        evidence=CargoEvidence(
            source="tests/integration/extension_ingress.rs",
            test="signed_acme_post_flows_through_the_production_mount_into_a_turn",
            target="reborn_integration_extension_ingress",
        ),
    ),
    ProductJourneyCase(
        case_id="webui_text_turn_persists",
        provider_worlds=(ProviderWorld.NONE,),
        mutable_provider_worlds=(),
        ingress=JourneyIngress.WEBUI,
        execution=JourneyExecution.STANDALONE_REBORN,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        assertions=(ObservableAssertion.DURABLE_STATE,),
        evidence=PytestEvidence(
            source="tests/e2e/scenarios/test_reborn_webui_v2_smoke.py",
            test="test_reborn_v2_text_turn_persists",
        ),
    ),
    ProductJourneyCase(
        case_id="slack_inbound_real_turn_reply",
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
        evidence=CargoEvidence(
            source="tests/integration/extension_delivery.rs",
            test="slack_final_reply_flows_through_the_real_delivery_coordinator",
            target="reborn_integration_extension_delivery",
        ),
    ),
    ProductJourneyCase(
        case_id="telegram_inbound_real_turn_reply",
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
        evidence=CargoEvidence(
            source="tests/integration/extension_delivery.rs",
            test="telegram_update_becomes_a_turn_and_a_coordinated_reply",
            target="reborn_integration_extension_delivery",
        ),
    ),
    ProductJourneyCase(
        case_id="scheduled_trigger_slack_delivery_default_and_explicit",
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
        evidence=CargoEvidence(
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
    """Built-in ingress plus every production channel declaring inbound.

    `EXTENSION_WEBHOOK` is the generic mount itself, not a third channel.
    Slack and Telegram both arrive through it, so covering them exercises it
    incidentally -- but only for two vendors the host already ships. The
    surface that matters is the one a *new* extension arrives on, and the only
    way to prove that is with an extension the host has never heard of.
    """
    return {
        JourneyIngress.WEBUI,
        JourneyIngress.SCHEDULED_TRIGGER,
        JourneyIngress.EXTENSION_WEBHOOK,
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
