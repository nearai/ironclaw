#!/usr/bin/env bash
set -euo pipefail

# Run the deterministic Rust-side Reborn E2E gate.
# Usage:
#   scripts/reborn-e2e-rust.sh              # all groups
#   scripts/reborn-e2e-rust.sh architecture-boundaries # dependency and protocol boundaries
#   scripts/reborn-e2e-rust.sh architecture-runtime    # host runtime and capability spine
#   scripts/reborn-e2e-rust.sh runtimes     # dispatcher/runtime/process lanes
#   scripts/reborn-e2e-rust.sh substrates   # event/network/secret substrates
#
# Extra cargo test args can be passed through CARGO_TEST_ARGS, for example:
#   CARGO_TEST_ARGS='-- --nocapture' scripts/reborn-e2e-rust.sh architecture

group="${1:-all}"
extra_args=${CARGO_TEST_ARGS:-"-- --nocapture"}

run_test() {
  local package="$1"
  local test_name="$2"
  echo "::group::cargo test -p ${package} --test ${test_name}"
  # shellcheck disable=SC2086 # extra_args intentionally expands into cargo's trailing args.
  cargo test -p "${package}" --test "${test_name}" ${extra_args}
  echo "::endgroup::"
}

run_test_exact() {
  local package="$1"
  local test_target="$2"
  local test_name="$3"
  local listed
  listed=$(cargo test -p "${package}" --test "${test_target}" "${test_name}" -- --exact --list)
  if ! grep -Fqx "${test_name}: test" <<<"${listed}"; then
    echo "error: exact test selector matched zero tests: ${package}/${test_target} ${test_name}" >&2
    return 1
  fi
  echo "::group::cargo test -p ${package} --test ${test_target} ${test_name} ${extra_args} --exact"
  # shellcheck disable=SC2086 # extra_args intentionally expands into cargo's trailing args.
  cargo test -p "${package}" --test "${test_target}" "${test_name}" ${extra_args} --exact
  echo "::endgroup::"
}

run_lib_test() {
  local package="$1"
  local test_filter="$2"
  echo "::group::cargo test -p ${package} --lib ${test_filter}"
  # shellcheck disable=SC2086 # extra_args intentionally expands into cargo's trailing args.
  cargo test -p "${package}" --lib "${test_filter}" ${extra_args}
  echo "::endgroup::"
}

run_lib_test_exact() {
  local package="$1"
  local test_name="$2"
  local listed
  listed=$(cargo test -p "${package}" --lib "${test_name}" -- --exact --list)
  if ! grep -Fqx "${test_name}: test" <<<"${listed}"; then
    echo "error: exact library test selector matched zero tests: ${package} ${test_name}" >&2
    return 1
  fi
  echo "::group::cargo test -p ${package} --lib ${test_name} ${extra_args} --exact"
  # shellcheck disable=SC2086 # extra_args intentionally expands into cargo's trailing args.
  cargo test -p "${package}" --lib "${test_name}" ${extra_args} --exact
  echo "::endgroup::"
}

run_architecture_boundaries() {
  run_test ironclaw_architecture_tests reborn_dependency_boundaries
  # Pins docs/internal/reborn/contracts/turns-agent-loop.md: terminal model
  # provider authentication and transcript persistence failures remain durable,
  # actionable, redacted, and never issue duplicate model/tool side effects.
  run_test_exact ironclaw_integration_tests reborn_integration_cancel \
    mid_turn_auth_provider_error_reaches_failed_with_credentials_category
  run_test_exact ironclaw_integration_tests reborn_integration_model_recovery \
    transcript_write_failure_stops_without_another_model_or_tool_side_effect
  run_test_exact ironclaw_integration_tests reborn_integration_model_recovery \
    tool_result_transcript_failure_stops_without_duplicate_model_or_tool_side_effect
  # Keep protocol/recovery selectors with the targets already compiled by this
  # lane instead of rebuilding them in the host-runtime lane.
  run_test_exact ironclaw_loop_host llm_gateway \
    gateway_maps_deterministic_provider_response_errors_to_invalid_output
  run_test_exact ironclaw_integration_tests reborn_integration_model_recovery \
    deterministic_provider_response_errors_use_bounded_invalid_output_recovery
  # Pins the retired-taxonomy Telegram identifiers and prevents v1 pairing
  # routes from re-entering the Reborn context.
  run_test ironclaw_architecture_tests telegram_extension_gates
  # Pins docs/internal/reborn/contracts/host-api.md: every recoverable verdict carries
  # an inline model diagnostic, and legacy omissions upgrade explicitly.
  run_lib_test_exact ironclaw_host_api resolution::tests::recoverable_failure_carries_its_model_visible_diagnostic
  run_lib_test_exact ironclaw_loop_contracts host::capability::tests::legacy_capability_failure_without_detail_rehydrates_explicit_fallback
  # Pins docs/internal/reborn/contracts/loop-exit.md: retired diagnostic_ref string/null
  # payloads remain readable but the retired field is never written again.
  run_lib_test_exact ironclaw_turns loop_exit::tests::loop_failed_accepts_retired_diagnostic_ref_but_does_not_serialize_it
  # Pins docs/internal/reborn/contracts/loop-exit.md and turn-runner.md: a rejected
  # checkpoint remains terminal after projection into the process journal and
  # cannot create a retry process.
  run_lib_test_exact ironclaw_turns \
    process_projection::runtime::tests::retry_rejects_checkpoint_rejection_without_creating_a_process
}

run_architecture_runtime() {
  run_test ironclaw_host_runtime host_runtime_contract
  run_test ironclaw_host_runtime host_runtime_services_contract
  # Pins docs/internal/reborn/contracts/host-runtime.md: scoped JSON files expose
  # bounded collection selection and numeric aggregation through the real turn.
  run_test_exact ironclaw_integration_tests reborn_integration_tool_call \
    json_runs_bounded_collection_operations
  run_test ironclaw_host_runtime reborn_e2e_gate
  run_test ironclaw_host_runtime reborn_invoke_vertical_slice
  run_test ironclaw_host_runtime runtime_http_egress_contract
  # Pins docs/internal/reborn/contracts/host-runtime.md: an HTTP 4xx/5xx response is a
  # model-visible failed capability outcome, not transport-level success.
  run_test_exact ironclaw_host_runtime first_party_builtin_tools \
    builtin_http_surfaces_http_error_status_as_failed_outcome
  run_test_exact ironclaw_host_runtime first_party_builtin_tools \
    builtin_http_keeps_redirect_responses_model_visible
  run_test_exact ironclaw_host_runtime first_party_builtin_tools \
    builtin_http_surfaces_server_error_status_as_failed_outcome
  run_test_exact ironclaw_host_runtime first_party_builtin_tools \
    builtin_http_save_surfaces_http_error_status_as_failed_outcome
  run_test_exact ironclaw_host_runtime first_party_builtin_tools \
    builtin_http_classifies_status_range_boundaries
  # Pins docs/internal/reborn/contracts/host-runtime.md: the failure diagnostic is
  # trimmed to the model-visible diagnostic budget and stays valid JSON.
  run_test_exact ironclaw_host_runtime first_party_builtin_tools \
    builtin_http_error_diagnostic_respects_model_diagnostic_budget
  run_test ironclaw_host_runtime builtin_obligation_handler_contract
  run_test ironclaw_host_runtime obligation_services_composition_contract
  run_test ironclaw_host_runtime production_trust_contract
  run_test ironclaw_capabilities capability_boundary_contract
  run_test ironclaw_capabilities capability_host_contract
  run_test ironclaw_capabilities capability_host_dispatcher_integration
  run_test ironclaw_capabilities capability_host_process_integration
  run_test ironclaw_capabilities capability_host_invocation_state_contract
  run_test ironclaw_capabilities capability_host_spawn_contract
  run_test ironclaw_capabilities capability_obligation_handler_contract
}

run_architecture() {
  run_architecture_boundaries
  run_architecture_runtime
}

run_runtimes() {
  # These two suites pin `RuntimeDispatcher` and live with it in
  # `ironclaw_capabilities`.
  run_test ironclaw_capabilities runtime_dispatch_contract
  run_test ironclaw_capabilities runtime_dispatch_event_contract
  # main's runtime_dispatcher_integration / vertical_slice_contract test the
  # retired RuntimeAdapter<F, G> architecture; the ToolResolver/BoundCapabilityAdapter
  # pipeline is pinned by the two dispatch contract suites above.
  run_test ironclaw_wasm wasm_dispatch_integration
  run_test ironclaw_wasm wasm_http_adapter_contract
  run_test ironclaw_wasm wit_tool_runtime_contract
  run_test ironclaw_sandbox script_dispatch_integration
  run_test ironclaw_sandbox script_http_adapter_contract
  run_test ironclaw_sandbox script_runner_contract
  run_test ironclaw_sandbox docker_security
  run_test ironclaw_mcp mcp_adapter_contract
  run_test ironclaw_mcp mcp_dispatch_integration
  # Pins docs/internal/reborn/contracts/trust-boundary-hardening.md through the whole
  # turn: the scrubbed, bounded MCP cause reaches the next model request.
  run_test_exact ironclaw_integration_tests reborn_integration_mcp mcp_tool_call_error_cause_is_scrubbed_and_bounded_in_next_model_request
  run_test ironclaw_processes process_host_contract
  run_test ironclaw_processes process_journal_store_contract
  run_test ironclaw_processes legacy_migration_backend_contract
  run_test ironclaw_processes process_services_contract
}

run_substrates() {
  run_test ironclaw_event_log durable_log_contract
  # Pins docs/internal/reborn/contracts/events.md: runtime snapshot/replay projections
  # preserve nested dispatcher failures without synthesizing child run rows.
  run_test ironclaw_event_projections nested_dispatch_projection_contract
  run_test ironclaw_filesystem catalog_contract
  run_test ironclaw_filesystem filesystem_contract
  run_test ironclaw_network boundary_contract
  run_test ironclaw_network network_http_egress_contract
  run_test ironclaw_network network_policy_contract
  run_test ironclaw_secrets boundary_contract
  run_test ironclaw_secrets secret_store_contract
  run_test ironclaw_resources resource_governor_contract
  run_test ironclaw_approvals approval_store_contract
  run_test ironclaw_approvals approval_resolution_contract
  run_test ironclaw_approvals boundary_contract
  run_test ironclaw_authorization boundary_contract
  run_test ironclaw_authorization capability_access_contract
  run_test ironclaw_authorization capability_lease_contract
}

case "${group}" in
  architecture)
    run_architecture
    ;;
  architecture-boundaries)
    run_architecture_boundaries
    ;;
  architecture-runtime)
    run_architecture_runtime
    ;;
  runtimes)
    run_runtimes
    ;;
  substrates)
    run_substrates
    ;;
  all)
    run_architecture
    run_runtimes
    run_substrates
    ;;
  *)
    echo "unknown Reborn E2E group: ${group}" >&2
    echo "expected one of: architecture, architecture-boundaries, architecture-runtime, runtimes, substrates, all" >&2
    exit 2
    ;;
esac
