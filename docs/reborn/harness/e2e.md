# Reborn E2E Harness

This document is the branch-local map for the dedicated Reborn E2E gate. Reborn is not missing tests: `main` already contains extensive crate-level contract and integration coverage. The gap this branch closes is a single, named E2E workflow that runs the Reborn architecture spine together and keeps a small product-surface smoke check beside it.

## What already exists

| Area | Existing coverage |
| --- | --- |
| Architecture boundaries | `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` |
| Host runtime facade and outcomes | `crates/ironclaw_host_runtime/tests/host_runtime_contract.rs` |
| Host runtime production composition | `crates/ironclaw_host_runtime/tests/host_runtime_services_contract.rs` |
| Dedicated Reborn E2E spine | `crates/ironclaw_host_runtime/tests/reborn_e2e_gate.rs` |
| Capability host invoke/resume/spawn | `crates/ironclaw_capabilities/tests/capability_host_*` |
| Dispatcher adapter selection | `crates/ironclaw_dispatcher/tests/vertical_slice_contract.rs` |
| WASM runtime lane | `crates/ironclaw_wasm/tests/wasm_dispatch_integration.rs` and `wasm_http_adapter_contract.rs` |
| Script runtime lane | `crates/ironclaw_scripts/tests/script_dispatch_integration.rs` and `script_http_adapter_contract.rs` |
| MCP runtime lane | `crates/ironclaw_mcp/tests/mcp_dispatch_integration.rs` and `mcp_adapter_contract.rs` |
| Process lifecycle | `crates/ironclaw_processes/tests/process_dispatch_integration.rs` and process service/store contracts |
| Network policy and host HTTP egress | `crates/ironclaw_network/tests/*` plus host-runtime HTTP egress tests |
| Secret storage/leases | `crates/ironclaw_secrets/tests/secret_store_contract.rs` plus host-runtime staged-secret tests |
| Events/audit replay | `crates/ironclaw_events/tests/durable_log_contract.rs` and host-runtime durable-event tests |
| Gateway product smoke | Existing Playwright scenarios under `tests/e2e/scenarios/`, especially `test_v2_*` |

## Dedicated Reborn E2E goal

The dedicated gate should answer one question:

```text
Can the Reborn architecture path still execute happy, blocked, denied, failed,
background, network, secret, event, and product-smoke paths after a change?
```

It intentionally reuses existing deterministic contract/integration tests instead of duplicating them in a second test framework.

## Happy path spine

The Reborn E2E happy path is covered by a dedicated `reborn_e2e_gate.rs` spine test plus the broader host-runtime and runtime-lane tests:

```text
Extension manifests
-> ExtensionRegistry
-> HostRuntimeServices
-> DefaultHostRuntime / HostRuntime facade
-> CapabilityHost authorization and run-state lifecycle
-> RuntimeDispatcher adapter selection
-> WASM / Script / MCP runtime adapters
-> resource reservation and reconciliation
-> durable runtime events
-> structured outcome returned to caller
```

Required assertions across the suite:

- visible capability surfaces include expected descriptors and stable surface version;
- health reports missing runtime backends fail-closed and configured backends ready;
- authorized invocations reach the selected runtime adapter;
- resource reservations are reconciled or released;
- run-state reaches the expected terminal or blocked state;
- durable events are replayable and metadata-only;
- runtime output is structured JSON and redacted where obligations require it.

## Other paths

The dedicated gate includes new Reborn E2E gate tests plus existing tests for these non-happy paths:

### Authorization and approval

- denied authorization fails before dispatch;
- approval-required invocation blocks with a persisted approval request id;
- approved resume consumes the exact lease once;
- changed input, wrong scope/user, expired lease, or missing stores fail before dispatch;
- unsupported obligations fail closed.

### Runtime availability and adapter failures

- missing runtime backend reports a stable missing-runtime failure;
- runtime lane errors map to stable failure categories;
- dispatcher/runtime adapter boundaries remain dependency-clean.

### Resource, process, and cancellation

- reservations are released on failure and reconciled on success;
- spawned background processes publish started/completed/failed/killed transitions;
- cancellation reaches the process graph;
- late completion after kill does not publish a misleading success;
- process handoffs for the same scoped capability fail closed while active.

### Network and secrets

- runtime HTTP egress is host-mediated;
- missing staged network policy fails before transport;
- staged secret material is consumed once;
- runtime-supplied manual credentials are rejected;
- raw secrets, credential-shaped values, and private host paths are not exposed in runtime-visible output, errors, events, or audit records.

### Event and observability

- durable event cursors replay runtime events;
- stale/gap cursor behavior is deterministic;
- event/audit records carry correlation metadata without raw payload leaks.

### Product smoke

A small Playwright smoke scenario starts an isolated `ENGINE_V2=true` gateway with the mock LLM and verifies:

- authenticated web shell loads;
- text-only chat completes and persists;
- tool-capable prompt completes through the gateway history path;
- no duplicate assistant response is emitted for a single user turn.

This smoke test does not replace the full browser E2E workflow. It proves the branch remains product-bootable while the Rust Reborn gate proves architecture behavior.

## Local commands

Run the deterministic Rust Reborn gate:

```bash
# Full gate
scripts/reborn-e2e-rust.sh

# Or run one CI matrix group at a time
scripts/reborn-e2e-rust.sh architecture
scripts/reborn-e2e-rust.sh runtimes
scripts/reborn-e2e-rust.sh substrates
```

The script expands to the dedicated `reborn_e2e_gate.rs` tests plus the current Reborn boundary, host-runtime, capability-host, dispatcher, WASM, Script, MCP, process, event, filesystem, network, secret, resource, run-state, approval, and authorization contract tests. Use the script as the source of truth for local/CI parity rather than copying individual `cargo test` commands.

Run the gateway smoke test:

```bash
cargo build --no-default-features --features libsql
cd tests/e2e
pip install -e .
playwright install --with-deps chromium  # on Linux CI; local macOS can omit --with-deps
pytest scenarios/test_reborn_gateway_smoke.py -v --timeout=120
```

## CI ownership

`reborn-e2e.yml` is intentionally separate from `e2e.yml`:

- Reborn changes can run a focused architecture gate without destabilizing the main browser E2E matrix.
- The workflow is advisory by default and intentionally does not run on `merge_group`; add a merge-queue trigger only after the gate proves stable and is deliberately promoted to branch protection.
- The Rust jobs should stay deterministic and avoid live providers.
- The gateway job should remain a smoke test, not a second full browser matrix.
