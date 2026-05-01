# Reborn Host Runtime Contract

`ironclaw_host_runtime` is the composition-facing host boundary above Reborn capability, process, network, secret, audit, and resource substrates. Upper turn/loop services depend on the `HostRuntime` trait and receive structured outcomes instead of concrete substrate handles.

## Obligation composition

`DefaultHostRuntime` may be configured with a `CapabilityObligationHandler` through `with_obligation_handler(...)`. It forwards the handler into `CapabilityHost` for capability invocations.

Production/service-graph construction should prefer `BuiltinObligationServices` plus `DefaultHostRuntime::with_builtin_obligation_services(...)`. `BuiltinObligationServices` requires an audit sink, secret store, and resource governor at construction time, creates the network-policy and runtime-secret handoff stores, and exposes cloned store handles for runtime adapters/HTTP egress to consume the exact state staged by the handler. `HostRuntimeServices` can also adapt durable event/audit logs with `with_durable_event_log(...)` and `with_durable_audit_log(...)`; the latter is the production path for built-in obligation audit records that must be replayable through the Reborn event substrate.

`BuiltinObligationHandler` is the default host-owned implementation for current V1 obligations. It is deliberately fail-closed: obligations that require backing services fail unless the corresponding store/sink/governor is configured. The convenience `with_builtin_obligation_handler()` installs an explicit empty/dev handler and keeps those obligations fail-closed until a fully configured services value is supplied.

Supported built-in behavior:

- `AuditBefore`: emits metadata-only `AuditStage::Before` records.
- `AuditAfter`: emits metadata-only `AuditStage::After` records after dispatch output is available.
- `ApplyNetworkPolicy`: validates policy metadata and stages a scoped policy in `NetworkObligationPolicyStore` for runtime handoff.
- `InjectSecretOnce`: verifies the secret exists, leases and consumes it exactly once, then stages material in `RuntimeSecretInjectionStore` for one runtime take.
- `UseScopedMounts`: accepts only mount views that are subsets of the execution context mount view and returns the narrowed view to the capability host.
- `ReserveResources`: reserves the exact requested reservation id through a configured `ResourceGovernor` and returns the reservation for dispatch/process handoff.
- `RedactOutput`: sanitizes dispatch output string values and object keys before publication, failing closed if redacted keys collide.
- `EnforceOutputLimit`: fails before publication if serialized output exceeds the limit.

`EnforceResourceCeiling` is intentionally fail-closed in this slice until an explicit runtime/sandbox ceiling handoff exists. `EnforceOutputLimit` covers the output-byte part of a resource ceiling today; other ceiling fields must not be silently accepted.

## Isolation rules

- `NetworkObligationPolicyStore` keys policies by full `ResourceScope` plus capability id and consumes entries with `take(...)`.
- `RuntimeSecretInjectionStore` keys material by full `ResourceScope`, capability id, and secret handle and consumes entries with `take(...)`.
- Staged secret entries have a default five-minute TTL; insertion, `take(...)`, and `prune_expired(...)` drop expired material so abandoned handoffs stop being usable even if runtime setup never reaches egress.
- Direct `satisfy(...)` releases any prepared resource reservation without discarding successfully staged network/secret handoffs that the caller still needs to pass to runtime adapters.
- Inline dispatch completion discards any unconsumed staged network/secret handoffs so successful calls do not leave reusable ambient state behind.
- Background process lifecycle cleanup currently enforces a single active process handoff per scoped capability (`ResourceScope` plus capability id). Starting a second process handoff for the same scoped capability before the first reaches terminal cleanup fails closed; process-owned handoff ids are a follow-up design.
- Terminal process cleanup failures are surfaced through the process-store transition or background failure handler and logged with process id/stage context; they must not be silently swallowed.
- Staged secrets must never be logged or exposed through debug output.
- Handler errors must use stable categories and avoid raw provider/backend details.

## Runtime HTTP egress

Runtime HTTP remains host-mediated through `RuntimeHttpEgress` and `HostHttpEgressService`. Runtime requests carry the full `ResourceScope` and `CapabilityId` so `HostHttpEgressService` can consume the matching one-shot policy from `NetworkObligationPolicyStore` immediately before outbound transport. The production constructor is fail-closed until a policy store is attached; request-embedded policy fallback is only available through an explicitly named test/legacy constructor. A missing scoped/capability policy fails before transport and any taken policy is not reusable after credential, request, network, or response failure. Runtime code must not perform ad-hoc DNS/private-IP checks or direct HTTP clients; `ironclaw_network` owns network policy enforcement and `ironclaw_secrets` owns secret lease/consume semantics.

MCP HTTP/SSE follows the same rule through `ironclaw_mcp::McpHostHttpClient`: the host supplies an `McpRuntimeHttpAdapter<RuntimeHttpEgress>` and an egress planner for scoped network policy, credential injection handles, response body limits, and timeouts. Generic or direct-network MCP clients keep `uses_host_mediated_http_egress() == false`, so `McpRuntime` rejects HTTP/SSE manifests before any outbound attempt.

Credential injection plans identify their material source. `RuntimeCredentialSource::SecretStoreLease` keeps the compatibility path for host-derived credentials that have not already been consumed by an obligation handler. `RuntimeCredentialSource::StagedObligation { capability_id }` is the `InjectSecretOnce` handoff path: `HostHttpEgressService` must be configured with the same `RuntimeSecretInjectionStore` as the obligation handler and must call `take(scope, capability_id, handle)` before runtime/network use. Missing required staged material fails before outbound transport, and successful or failed transport attempts cannot reuse the staged value because `take(...)` removes it first. If one approved request plan injects the same source+handle into multiple targets, the egress service consumes the staged or leased material once and reuses it only within that request.

For WASM host-mediated HTTP imports, `WasmRuntimeHttpAdapter` carries the invoking capability id into `WasmRuntimeCredentialProvider`. Host composition can use `WasmStagedRuntimeCredentials` rules to emit exact-url or request-wide `StagedObligation` injection plans; the WASM guest still supplies only method/url/headers/body and never chooses credential handles or targets.

Script execution keeps Docker containers ambient-network-disabled by default (`docker run --network none`). If scripts later gain a brokered HTTP SDK, sidecar, helper process, or host API, every request must flow through `ironclaw_scripts::ScriptRuntimeHttpAdapter<RuntimeHttpEgress>`. The host supplies the `ResourceScope`, `CapabilityId`, `NetworkPolicy`, credential injection plan, response body limit, and timeout; script/runtime input must not invent secret handles, raw credential headers/query parameters, DNS checks, private-IP checks, or direct HTTP clients inside `ironclaw_scripts`.
