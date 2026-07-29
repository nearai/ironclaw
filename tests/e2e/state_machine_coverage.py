"""Typed, representative coverage for Reborn lifecycle equivalence classes.

This is an evidence inventory, not a second state machine.  Each row points at
an executable test owned by the relevant boundary.  The selected rows cover
every supported value and the interactions most likely to hide lifecycle
bugs, without constructing the full seven-dimensional Cartesian product.
"""

from dataclasses import dataclass
from enum import StrEnum
from typing import Literal, TypeAlias

from journey_cases import ALL_JOURNEY_CASES
from journey_types import (
    CargoEvidence,
    JourneyDeliveryTarget,
    JourneyIngress,
    ObservableAssertion,
    PytestEvidence,
)
from provider_fault_cases import PROVIDER_FAULT_CASES, ProviderFaultOutcome
from provider_operation_cases import PROVIDER_OPERATION_CASES
from provider_operation_types import OutcomeClass


class AuthState(StrEnum):
    AUTHENTICATED = "authenticated"
    AUTH_REQUIRED = "auth_required"
    WRONG_SCOPE = "wrong_scope"


class PolicyState(StrEnum):
    ALLOWED = "allowed"
    APPROVAL_REQUIRED = "approval_required"
    DENIED = "denied"


class OperationClass(StrEnum):
    READ = "read"
    IDEMPOTENT_WRITE = "idempotent_write"
    NON_IDEMPOTENT_WRITE = "non_idempotent_write"


# Keep the provider vocabulary already exercised by provider operation and
# fault cases.  ``uncertain`` is the additional scheduler/capability boundary
# class proven by lease_wedge: the host cannot assert that the wedged external
# operation did not happen, but it must not report a false success.
ProviderOutcome: TypeAlias = OutcomeClass | ProviderFaultOutcome | Literal["uncertain"]
SUPPORTED_PROVIDER_OUTCOMES: tuple[ProviderOutcome, ...] = (
    "success",
    "empty",
    "unchanged",
    "committed_without_ack",
    "uncertain",
)


class LifecycleState(StrEnum):
    RUNNING = "running"
    BLOCKED_AUTH = "blocked_auth"
    BLOCKED_APPROVAL = "blocked_approval"
    COMPLETED = "completed"
    DENIED = "denied"
    CANCELLED = "cancelled"
    FAILED = "failed"


class SequenceClass(StrEnum):
    TRIGGER = "trigger"
    RETRY = "retry"
    CANCEL = "cancel"
    DUPLICATE = "duplicate"
    RESTART = "restart"
    CONCURRENT_DOUBLE_SUBMIT = "concurrent_double_submit"


class StateMachineInvariant(StrEnum):
    TERMINAL_STABILITY = "terminal_stability"
    AT_MOST_ONCE_EFFECT = "at_most_once_effect"
    ACTOR_ISOLATION = "actor_isolation"
    TRUTHFUL_UNCERTAIN_OUTCOME = "truthful_uncertain_outcome"
    NO_ORPHAN_RUNS_OR_RESERVATIONS = "no_orphan_runs_or_reservations"


Evidence: TypeAlias = PytestEvidence | CargoEvidence


@dataclass(frozen=True)
class StateMachineCoverageCase:
    """One representative crossing of the seven supported dimensions."""

    case_id: str
    ingress: JourneyIngress
    auth_state: AuthState
    policy_state: PolicyState
    operation_class: OperationClass
    provider_outcome: ProviderOutcome
    lifecycle_state: LifecycleState
    delivery_target: JourneyDeliveryTarget
    sequences: tuple[SequenceClass, ...]
    invariants: tuple[StateMachineInvariant, ...]
    evidence: Evidence


def _cargo(source: str, test: str, target: str, manifest: str | None = None):
    return CargoEvidence(
        source=source,
        test=test,
        target=target,
        manifest=manifest,
    )


def _operation(case_id: str):
    """Select a typed provider operation; fail at import if it disappears."""
    return next(case for case in PROVIDER_OPERATION_CASES if case.case_id == case_id)


def _fault(case_id: str):
    """Select a typed provider fault; fail at import if it disappears."""
    return next(case for case in PROVIDER_FAULT_CASES if case.case_id == case_id)


_PROVIDER_PYTEST = PytestEvidence(
    source="tests/e2e/scenarios/test_reborn_qa_trace_full_path.py",
    test="test_provider_operation_case_executes_with_provider_readback",
)
_FAULT_PYTEST = PytestEvidence(
    source="tests/e2e/scenarios/test_reborn_qa_trace_full_path.py",
    test="test_provider_fault_profile_preserves_safe_operation_outcomes",
)
_GENERATED_GATE_EVIDENCE = _cargo(
    "tests/integration/generated_gate_sequences.rs",
    "generated_gate_sequences_preserve_lifecycle_invariants",
    "reborn_generated_gate_sequences",
)


def _journey_cases() -> tuple[StateMachineCoverageCase, ...]:
    """Project the existing journey registry into the shared seven axes."""
    projected = []
    for journey in ALL_JOURNEY_CASES:
        lifecycle = (
            LifecycleState.COMPLETED
            if ObservableAssertion.TRACE_REPLAY_COMPLETE in journey.assertions
            else LifecycleState.RUNNING
        )
        is_trigger = journey.ingress is JourneyIngress.SCHEDULED_TRIGGER
        sequences = []
        if is_trigger:
            sequences.append(SequenceClass.TRIGGER)
        if ObservableAssertion.RESTART_IDEMPOTENCY in journey.assertions:
            sequences.append(SequenceClass.RESTART)
        invariants = []
        if (
            ObservableAssertion.EXACT_MUTATION_COUNT in journey.assertions
            or ObservableAssertion.RESTART_IDEMPOTENCY in journey.assertions
        ):
            invariants.append(StateMachineInvariant.AT_MOST_ONCE_EFFECT)
        projected.append(
            StateMachineCoverageCase(
                case_id=f"journey_{journey.case_id}",
                ingress=journey.ingress,
                auth_state=AuthState.AUTHENTICATED,
                policy_state=PolicyState.ALLOWED,
                operation_class=OperationClass.READ,
                provider_outcome="success",
                lifecycle_state=lifecycle,
                delivery_target=journey.delivery_target,
                sequences=tuple(sequences),
                invariants=tuple(invariants),
                evidence=journey.evidence,
            )
        )
    return tuple(projected)


def _provider_case(
    case_id: str,
    operation_class: OperationClass,
) -> StateMachineCoverageCase:
    operation = _operation(case_id)
    return StateMachineCoverageCase(
        case_id=f"provider_{case_id}",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTHENTICATED,
        policy_state=PolicyState.ALLOWED,
        operation_class=operation_class,
        provider_outcome=operation.outcome_class,
        lifecycle_state=LifecycleState.COMPLETED,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(),
        invariants=(StateMachineInvariant.AT_MOST_ONCE_EFFECT,),
        evidence=_PROVIDER_PYTEST,
    )


def _fault_case(
    case_id: str,
    *,
    auth_state: AuthState,
    operation_class: OperationClass,
    sequences: tuple[SequenceClass, ...] = (),
) -> StateMachineCoverageCase:
    fault = _fault(case_id)
    return StateMachineCoverageCase(
        case_id=f"provider_fault_{case_id}",
        ingress=JourneyIngress.WEBUI,
        auth_state=auth_state,
        policy_state=(
            PolicyState.DENIED
            if auth_state is AuthState.WRONG_SCOPE
            else PolicyState.ALLOWED
        ),
        operation_class=operation_class,
        provider_outcome=fault.expected_outcome,
        lifecycle_state=LifecycleState.FAILED,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=sequences,
        invariants=(StateMachineInvariant.AT_MOST_ONCE_EFFECT,),
        evidence=_FAULT_PYTEST,
    )


STATE_MACHINE_COVERAGE_CASES = (
    *_journey_cases(),
    _provider_case("github_get_issue", OperationClass.READ),
    _provider_case("google_sheets_read_values_empty", OperationClass.READ),
    _provider_case("github_update_issue", OperationClass.IDEMPOTENT_WRITE),
    _provider_case("github_create_issue", OperationClass.NON_IDEMPOTENT_WRITE),
    _fault_case(
        "idempotent_write_wrong_scope",
        auth_state=AuthState.WRONG_SCOPE,
        operation_class=OperationClass.IDEMPOTENT_WRITE,
    ),
    _fault_case(
        "non_idempotent_write_lost_acknowledgement",
        auth_state=AuthState.AUTHENTICATED,
        operation_class=OperationClass.NON_IDEMPOTENT_WRITE,
        sequences=(SequenceClass.DUPLICATE,),
    ),
    StateMachineCoverageCase(
        case_id="auth_gate_requires_resolution",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTH_REQUIRED,
        policy_state=PolicyState.APPROVAL_REQUIRED,
        operation_class=OperationClass.READ,
        provider_outcome="unchanged",
        lifecycle_state=LifecycleState.BLOCKED_AUTH,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(SequenceClass.RETRY,),
        invariants=(StateMachineInvariant.TERMINAL_STABILITY,),
        evidence=_cargo(
            "tests/integration/group_journeys/main.rs",
            "journeys_group_auth_convergence_e2e",
            "reborn_group_journeys",
        ),
    ),
    StateMachineCoverageCase(
        case_id="approval_gate_is_blocked_before_resolution",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTHENTICATED,
        policy_state=PolicyState.APPROVAL_REQUIRED,
        operation_class=OperationClass.IDEMPOTENT_WRITE,
        provider_outcome="unchanged",
        lifecycle_state=LifecycleState.BLOCKED_APPROVAL,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(SequenceClass.CANCEL, SequenceClass.DUPLICATE),
        invariants=(
            StateMachineInvariant.TERMINAL_STABILITY,
            StateMachineInvariant.AT_MOST_ONCE_EFFECT,
            StateMachineInvariant.NO_ORPHAN_RUNS_OR_RESERVATIONS,
        ),
        evidence=_GENERATED_GATE_EVIDENCE,
    ),
    StateMachineCoverageCase(
        case_id="approval_denial_is_terminal",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTHENTICATED,
        policy_state=PolicyState.DENIED,
        operation_class=OperationClass.IDEMPOTENT_WRITE,
        provider_outcome="unchanged",
        lifecycle_state=LifecycleState.DENIED,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(SequenceClass.DUPLICATE,),
        invariants=(
            StateMachineInvariant.TERMINAL_STABILITY,
            StateMachineInvariant.AT_MOST_ONCE_EFFECT,
        ),
        evidence=_GENERATED_GATE_EVIDENCE,
    ),
    StateMachineCoverageCase(
        case_id="approval_cancel_is_terminal",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTHENTICATED,
        policy_state=PolicyState.APPROVAL_REQUIRED,
        operation_class=OperationClass.IDEMPOTENT_WRITE,
        provider_outcome="unchanged",
        lifecycle_state=LifecycleState.CANCELLED,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(SequenceClass.CANCEL,),
        invariants=(
            StateMachineInvariant.TERMINAL_STABILITY,
            StateMachineInvariant.NO_ORPHAN_RUNS_OR_RESERVATIONS,
        ),
        evidence=_GENERATED_GATE_EVIDENCE,
    ),
    StateMachineCoverageCase(
        case_id="generated_actor_isolation",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTHENTICATED,
        policy_state=PolicyState.APPROVAL_REQUIRED,
        operation_class=OperationClass.IDEMPOTENT_WRITE,
        provider_outcome="success",
        lifecycle_state=LifecycleState.COMPLETED,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(SequenceClass.DUPLICATE,),
        invariants=(StateMachineInvariant.ACTOR_ISOLATION,),
        evidence=_cargo(
            "tests/integration/generated_gate_sequences.rs",
            "generated_actions_on_one_actor_never_disturb_another",
            "reborn_generated_gate_sequences",
        ),
    ),
    StateMachineCoverageCase(
        case_id="lease_wedge_truthful_uncertainty",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTHENTICATED,
        policy_state=PolicyState.ALLOWED,
        operation_class=OperationClass.READ,
        provider_outcome="uncertain",
        lifecycle_state=LifecycleState.FAILED,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(SequenceClass.RETRY,),
        invariants=(StateMachineInvariant.TRUTHFUL_UNCERTAIN_OUTCOME,),
        evidence=_cargo(
            "tests/integration/lease_wedge.rs",
            "wedged_tool_call_is_reaped_by_lease_expiry_not_left_running_forever",
            "reborn_integration_lease_wedge",
        ),
    ),
    StateMachineCoverageCase(
        case_id="generated_same_thread_double_submit",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTHENTICATED,
        policy_state=PolicyState.ALLOWED,
        operation_class=OperationClass.READ,
        provider_outcome="success",
        lifecycle_state=LifecycleState.RUNNING,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(SequenceClass.CONCURRENT_DOUBLE_SUBMIT,),
        invariants=(
            StateMachineInvariant.AT_MOST_ONCE_EFFECT,
            StateMachineInvariant.NO_ORPHAN_RUNS_OR_RESERVATIONS,
        ),
        evidence=_cargo(
            "tests/integration/generated_gate_sequences.rs",
            "generated_same_thread_double_submit_interleavings_admit_one_run",
            "reborn_generated_gate_sequences",
        ),
    ),
    StateMachineCoverageCase(
        case_id="generated_restart_sequence",
        ingress=JourneyIngress.WEBUI,
        auth_state=AuthState.AUTHENTICATED,
        policy_state=PolicyState.APPROVAL_REQUIRED,
        operation_class=OperationClass.IDEMPOTENT_WRITE,
        provider_outcome="success",
        lifecycle_state=LifecycleState.COMPLETED,
        delivery_target=JourneyDeliveryTarget.WEBUI,
        sequences=(SequenceClass.RESTART,),
        invariants=(
            StateMachineInvariant.TERMINAL_STABILITY,
            StateMachineInvariant.AT_MOST_ONCE_EFFECT,
            StateMachineInvariant.NO_ORPHAN_RUNS_OR_RESERVATIONS,
        ),
        evidence=_cargo(
            "tests/integration/generated_restart_sequences.rs",
            "generated_restart_sequences_preserve_gate_lifecycle_and_effect_count",
            "reborn_generated_restart_sequences",
        ),
    ),
)


SUPPORTED_DIMENSIONS = {
    "ingress": frozenset(JourneyIngress),
    "auth_state": frozenset(AuthState),
    "policy_state": frozenset(PolicyState),
    "operation_class": frozenset(OperationClass),
    "provider_outcome": frozenset(SUPPORTED_PROVIDER_OUTCOMES),
    "lifecycle_state": frozenset(LifecycleState),
    "delivery_target": frozenset(JourneyDeliveryTarget),
}


# Deliberately selected high-risk pairs.  This is the auditable denominator for
# pairwise breadth; it does not imply every pair in the Cartesian product is
# useful.  Values are strings so provider outcomes can retain their existing
# Literal vocabulary while enum-backed dimensions use the same checker.
REQUIRED_EQUIVALENCE_PAIRS = frozenset(
    {
        ("ingress", "scheduled_trigger", "delivery_target", "slack"),
        ("ingress", "extension_webhook", "delivery_target", "none"),
        ("auth_state", "auth_required", "policy_state", "approval_required"),
        ("auth_state", "wrong_scope", "policy_state", "denied"),
        ("auth_state", "wrong_scope", "provider_outcome", "unchanged"),
        (
            "operation_class",
            "non_idempotent_write",
            "provider_outcome",
            "committed_without_ack",
        ),
        ("provider_outcome", "uncertain", "lifecycle_state", "failed"),
        (
            "policy_state",
            "approval_required",
            "lifecycle_state",
            "blocked_approval",
        ),
        ("policy_state", "denied", "lifecycle_state", "denied"),
        ("delivery_target", "webui", "lifecycle_state", "completed"),
    }
)


def duplicate_case_ids(
    cases: tuple[StateMachineCoverageCase, ...],
) -> frozenset[str]:
    ids = [case.case_id for case in cases]
    return frozenset(case_id for case_id in ids if ids.count(case_id) > 1)


def dimension_gaps(
    cases: tuple[StateMachineCoverageCase, ...],
) -> dict[str, frozenset[str]]:
    """Return supported dimension values not represented by any row."""
    gaps = {}
    for dimension, required in SUPPORTED_DIMENSIONS.items():
        covered = {getattr(case, dimension) for case in cases}
        missing = {str(value) for value in required - covered}
        if missing:
            gaps[dimension] = frozenset(missing)
    return gaps


def pairwise_gaps(
    cases: tuple[StateMachineCoverageCase, ...],
) -> frozenset[tuple[str, str, str, str]]:
    """Return selected critical interactions absent from the registry."""
    missing = set()
    for pair in REQUIRED_EQUIVALENCE_PAIRS:
        left_axis, left_value, right_axis, right_value = pair
        if not any(
            str(getattr(case, left_axis)) == left_value
            and str(getattr(case, right_axis)) == right_value
            for case in cases
        ):
            missing.add(pair)
    return frozenset(missing)


def sequence_gaps(
    cases: tuple[StateMachineCoverageCase, ...],
) -> frozenset[SequenceClass]:
    covered = {sequence for case in cases for sequence in case.sequences}
    return frozenset(SequenceClass) - covered


def invariant_gaps(
    cases: tuple[StateMachineCoverageCase, ...],
) -> frozenset[StateMachineInvariant]:
    covered = {invariant for case in cases for invariant in case.invariants}
    return frozenset(StateMachineInvariant) - covered


def coverage_gap_count(
    cases: tuple[StateMachineCoverageCase, ...],
) -> int:
    """Mechanical WS9 dimension × sequence × invariant remaining-gap count."""
    return (
        sum(len(values) for values in dimension_gaps(cases).values())
        + len(pairwise_gaps(cases))
        + len(sequence_gaps(cases))
        + len(invariant_gaps(cases))
        + len(duplicate_case_ids(cases))
    )
