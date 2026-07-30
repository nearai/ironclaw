"""Fail-loud completeness gate for WS9 lifecycle equivalence evidence."""

from dataclasses import replace

from journey_types import PytestEvidence
from state_machine_coverage import (
    REQUIRED_EQUIVALENCE_PAIRS,
    STATE_MACHINE_COVERAGE_CASES,
    AuthState,
    SequenceClass,
    StateMachineInvariant,
    coverage_gap_count,
    dimension_gaps,
    duplicate_case_ids,
    invariant_gaps,
    pairwise_gaps,
    sequence_gaps,
)

from scenarios.test_journey_coverage import (
    _assert_python_evidence,
    _assert_rust_evidence,
)


def test_state_machine_registry_has_zero_coverage_gaps():
    """All 7 dimensions, 6 sequences, 5 invariants, and selected pairs map."""
    assert not duplicate_case_ids(STATE_MACHINE_COVERAGE_CASES)
    assert not dimension_gaps(STATE_MACHINE_COVERAGE_CASES)
    assert not pairwise_gaps(STATE_MACHINE_COVERAGE_CASES)
    assert not sequence_gaps(STATE_MACHINE_COVERAGE_CASES)
    assert not invariant_gaps(STATE_MACHINE_COVERAGE_CASES)
    assert coverage_gap_count(STATE_MACHINE_COVERAGE_CASES) == 0


def test_every_state_machine_claim_names_executable_evidence():
    """Typed rows are claims only when their exact test still exists and runs."""
    for case in STATE_MACHINE_COVERAGE_CASES:
        if isinstance(case.evidence, PytestEvidence):
            _assert_python_evidence(case, case.evidence)
        else:
            _assert_rust_evidence(case, case.evidence)


def test_journey_projection_derives_lifecycle_claims_from_declared_assertions():
    """Admission-only evidence cannot be credited with terminal stability."""
    webhook = next(
        case
        for case in STATE_MACHINE_COVERAGE_CASES
        if case.case_id
        == "journey_generic_extension_webhook_signed_post_becomes_a_turn"
    )
    assert webhook.lifecycle_state.value == "running"
    assert StateMachineInvariant.TERMINAL_STABILITY not in webhook.invariants

    trigger = next(
        case
        for case in STATE_MACHINE_COVERAGE_CASES
        if case.case_id
        == "journey_scheduled_trigger_slack_delivery_default_and_explicit"
    )
    assert trigger.sequences == (SequenceClass.TRIGGER, SequenceClass.RESTART)
    assert trigger.invariants == (StateMachineInvariant.AT_MOST_ONCE_EFFECT,)


def test_provider_effect_invariants_follow_operation_class():
    reads = tuple(
        case
        for case in STATE_MACHINE_COVERAGE_CASES
        if case.case_id
        in {"provider_github_get_issue", "provider_google_sheets_read_values_empty"}
    )
    assert reads
    assert all(
        StateMachineInvariant.AT_MOST_ONCE_EFFECT not in case.invariants
        for case in reads
    )

    writes = tuple(
        case
        for case in STATE_MACHINE_COVERAGE_CASES
        if case.case_id
        in {"provider_github_update_issue", "provider_github_create_issue"}
    )
    assert writes
    assert all(
        StateMachineInvariant.AT_MOST_ONCE_EFFECT in case.invariants for case in writes
    )


def test_approval_denial_uses_the_lifecycle_its_evidence_observes():
    denial = next(
        case
        for case in STATE_MACHINE_COVERAGE_CASES
        if case.case_id == "approval_denial_completes_without_effect"
    )
    assert denial.policy_state.value == "denied"
    assert denial.lifecycle_state.value == "completed"


def test_journey_projection_rejects_a_missing_supported_ingress():
    """Sabotage the generated journey projection, not a hand-built fixture."""
    sabotaged = tuple(
        case
        for case in STATE_MACHINE_COVERAGE_CASES
        if case.ingress.value != "extension_webhook"
    )
    assert dimension_gaps(sabotaged)["ingress"] == frozenset({"extension_webhook"})


def test_dimension_checker_rejects_a_missing_auth_class():
    sabotaged = tuple(
        case
        for case in STATE_MACHINE_COVERAGE_CASES
        if case.auth_state is not AuthState.WRONG_SCOPE
    )
    assert dimension_gaps(sabotaged)["auth_state"] == frozenset({"wrong_scope"})


def test_pairwise_checker_rejects_a_removed_critical_interaction():
    target = (
        "operation_class",
        "non_idempotent_write",
        "provider_outcome",
        "committed_without_ack",
    )
    assert target in REQUIRED_EQUIVALENCE_PAIRS
    sabotaged = tuple(
        case
        for case in STATE_MACHINE_COVERAGE_CASES
        if not (
            case.operation_class.value == "non_idempotent_write"
            and case.provider_outcome == "committed_without_ack"
        )
    )
    assert target in pairwise_gaps(sabotaged)


def test_sequence_checker_rejects_a_missing_generated_sequence():
    sabotaged = tuple(
        replace(
            case,
            sequences=tuple(
                sequence
                for sequence in case.sequences
                if sequence is not SequenceClass.CONCURRENT_DOUBLE_SUBMIT
            ),
        )
        for case in STATE_MACHINE_COVERAGE_CASES
    )
    assert sequence_gaps(sabotaged) == frozenset(
        {SequenceClass.CONCURRENT_DOUBLE_SUBMIT}
    )


def test_invariant_checker_rejects_removed_lease_wedge_truthfulness():
    """The retained lease_wedge proof cannot be replaced by a weaker claim."""
    sabotaged = tuple(
        replace(
            case,
            invariants=tuple(
                invariant
                for invariant in case.invariants
                if invariant is not StateMachineInvariant.TRUTHFUL_UNCERTAIN_OUTCOME
            ),
        )
        for case in STATE_MACHINE_COVERAGE_CASES
    )
    assert invariant_gaps(sabotaged) == frozenset(
        {StateMachineInvariant.TRUTHFUL_UNCERTAIN_OUTCOME}
    )


def test_duplicate_checker_rejects_a_generator_repeating_a_row():
    first = STATE_MACHINE_COVERAGE_CASES[0]
    sabotaged = (*STATE_MACHINE_COVERAGE_CASES, first)
    assert duplicate_case_ids(sabotaged) == frozenset({first.case_id})


def test_composite_gap_count_increases_for_each_sabotaged_denominator():
    """The reported remaining-gap count cannot stay zero after lost coverage."""
    without_restart = tuple(
        replace(
            case,
            sequences=tuple(
                sequence
                for sequence in case.sequences
                if sequence is not SequenceClass.RESTART
            ),
        )
        for case in STATE_MACHINE_COVERAGE_CASES
    )
    assert coverage_gap_count(without_restart) > 0
