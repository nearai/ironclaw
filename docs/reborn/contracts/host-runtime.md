# Reborn Host Runtime Contract

`ironclaw_host_runtime` is the composition-facing host boundary above Reborn capability, process, network, secret, audit, and resource substrates. Upper turn/loop services depend on the `HostRuntime` trait and receive structured outcomes instead of concrete substrate handles.

## Obligation composition

`DefaultHostRuntime` may be configured with a `CapabilityObligationHandler` through `with_obligation_handler(...)`. It forwards the handler into `CapabilityHost` for capability invocations.

Production/service-graph construction should prefer `BuiltinObligationServices` plus `DefaultHostRuntime::with_builtin_obligation_services(...)`. `BuiltinObligationServices` requires an audit sink, secret store, and resource governor at construction time, creates the network-policy and runtime-secret handoff stores, and keeps those mutable stores owned by host-runtime composition. Runtime adapters/HTTP egress that need the exact state staged by the handler must be built through host-runtime helper methods such as `BuiltinObligationServices::host_http_egress(...)` or `HostRuntimeServices::try_with_host_http_egress(...)`, rather than by receiving raw store handles. The remaining raw handoff-store constructors and `with_*_store` seams are low-level host-runtime/test harness escape hatches for contract coverage and bespoke host-owned composition; upper Reborn crates must not use them. `HostRuntimeServices` can also adapt durable event/audit logs with `with_durable_event_log(...)` and `with_durable_audit_log(...)`; the latter is the production path for built-in obligation audit records that must be replayable through the Reborn event substrate. Standalone Reborn composition should obtain those durable logs from `ironclaw_reborn_event_store::build_reborn_event_stores(...)`, which rejects in-memory stores in production and keeps storage drivers outside `ironclaw_events`.

`BuiltinObligationHandler` is the default host-owned implementation for current V1 obligations. It is deliberately fail-closed: obligations that require backing services fail unless the corresponding store/sink/governor is configured. The convenience `with_builtin_obligation_handler()` installs an explicit empty/dev handler and keeps those obligations fail-closed until a fully configured services value is supplied.

Supported built-in behavior:

- `AuditBefore`: emits metadata-only `AuditStage::Before` records.
- `AuditAfter`: emits metadata-only `AuditStage::After` records after dispatch output is available.
- `ApplyNetworkPolicy`: validates policy metadata and stages a scoped policy in `NetworkObligationPolicyStore` for runtime handoff.
- `InjectSecretOnce`: verifies the secret exists, leases and consumes it exactly once, then stages material in `RuntimeSecretInjectionStore` for one runtime take.
- `UseScopedMounts`: accepts only mount views that are subsets of the execution context mount view and returns the narrowed view to the capability host.
- `ReserveResources`: reserves the exact requested reservation id through a configured `ResourceGovernor` and returns the reservation for dispatch/process handoff.
- `EnforceResourceCeiling`: for immediate invoke/resume, decomposes supported ceiling dimensions into host-owned estimate and result checks. `max_usd` and input/output token ceilings require matching host estimates before dispatch and are re-checked against measured `ResourceUsage` after dispatch. `max_output_bytes` is enforced after redaction before publication and reports the same output-limit failure category as `EnforceOutputLimit`. Wall-clock ceilings and sandbox CPU-time, memory, disk, network-egress, and process-count ceilings fail closed until a concrete runtime/sandbox adapter handoff exists.
- `RedactOutput`: sanitizes dispatch output string values and object keys before publication, failing closed if redacted keys collide.
- `EnforceOutputLimit`: fails before publication if serialized output exceeds the limit.

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

## Visible capability surface

`HostRuntime::visible_capabilities(...)` is a visibility producer only. `DefaultHostRuntime` computes a deterministic, versioned surface from the extension registry, the caller `ExecutionContext`, caller/host-supplied `provider_trust`, the trust-aware authorizer, and the caller/profile supplied `CapabilitySurfacePolicy`. It does not evaluate the runtime host trust policy while computing visibility. Missing grants, missing provider trust, denied trust ceilings, authorizer denials, disallowed runtimes, and disallowed effects omit capabilities from the surface. Authorizer `RequireApproval` decisions may be represented as `VisibleCapabilityAccess::RequiresApproval`, but that status does not issue a grant or bypass approval storage.

Surface versioning returns `sha256:<lower-hex>` over canonical JSON that includes the configured base version, `SurfaceKind`, visibility policy, visibility-affecting context fields, grants, relevant provider trust ceilings, visible capability ids/providers/runtimes/effects/schemas, selected resource estimates, and visible access status. Policy allow-lists and visible capability payloads are canonicalized before hashing, so semantically equivalent allow-list ordering or registry insertion ordering does not churn the version; returned capabilities still preserve filtered registry order for deterministic rendering. The hash is stable across process runs so upper loop services can checkpoint the visible tool surface. Direct invocation remains authoritative: a capability omitted from the visible surface must still fail closed through `CapabilityHost` authorization if a caller tries to invoke it directly.

## FirstParty runtime adapter

`HostRuntimeServices` can register host-owned first-party capability handlers through `FirstPartyCapabilityRegistry`. When a declared capability uses `RuntimeKind::FirstParty`, dispatch still follows the normal path:

```text
HostRuntime::invoke_capability
  -> CapabilityHost authorization / approvals / obligations
  -> RuntimeDispatcher
  -> FirstPartyRuntimeAdapter
  -> registered host-owned handler
```

First-party handlers are composition-owned and keyed by `CapabilityId`; user-installed manifests must not self-assign first-party/system authority. A missing handler fails closed as `UndeclaredCapability` before handler side effects. The adapter owns resource reservation/reconciliation, emits the same dispatcher runtime events as other runtime lanes, and surfaces only stable redacted dispatch categories. `RuntimeKind::System` remains deferred for a stricter host-only adapter.

## Runtime HTTP egress

Runtime HTTP remains host-mediated through `RuntimeHttpEgress` and `HostHttpEgressService`. Runtime requests carry the full `ResourceScope` and `CapabilityId` so `HostHttpEgressService` can borrow the matching staged policy from `NetworkObligationPolicyStore` immediately before outbound transport, while obligation completion/abort or process lifecycle cleanup owns final discard. The production constructor is fail-closed until a policy store is attached; request-embedded policy fallback is only available through an explicitly named test/legacy constructor. A missing scoped/capability policy fails before transport, and staged credential material is consumed before credential, request, network, or response failure can make it reusable. Runtime code must not perform ad-hoc DNS/private-IP checks or direct HTTP clients; `ironclaw_network` owns network policy enforcement and `ironclaw_secrets` owns secret lease/consume semantics.

MCP HTTP/SSE follows the same rule through `ironclaw_mcp::McpHostHttpClient`: the host supplies an `McpRuntimeHttpAdapter<RuntimeHttpEgress>` and an egress planner for scoped network policy, credential injection handles, response body limits, and timeouts. Generic or direct-network MCP clients keep `uses_host_mediated_http_egress() == false`, so `McpRuntime` rejects HTTP/SSE manifests before any outbound attempt.

Credential injection plans identify their material source. `RuntimeCredentialSource::SecretStoreLease` keeps the compatibility path for host-derived credentials that have not already been consumed by an obligation handler. `RuntimeCredentialSource::StagedObligation { capability_id }` is the `InjectSecretOnce` handoff path: `HostHttpEgressService` must be configured with the same `RuntimeSecretInjectionStore` as the obligation handler and must call `take(scope, capability_id, handle)` before runtime/network use. Missing required staged material fails before outbound transport, and successful or failed transport attempts cannot reuse the staged value because `take(...)` removes it first. If one approved request plan injects the same source+handle into multiple targets, the egress service consumes the staged or leased material once and reuses it only within that request.

For WASM host-mediated HTTP imports, `WasmRuntimeHttpAdapter` carries the invoking capability id into `WasmRuntimeCredentialProvider`. Host composition can use `WasmStagedRuntimeCredentials` rules to emit exact-request or request-wide `StagedObligation` injection plans; the WASM guest still supplies only method/url/headers/body and never chooses credential handles or targets.

## Credential account resolution

Host API manifest contracts can project credential requirements into `HostApiCredentialRequirement` records keyed by extension id, host API id, manifest section path, capability id, credential account id, and secret handle. `CredentialAccountResolver` consumes those projected records plus a scoped `CredentialAccountStore` and request method/URL, then proves:

- the projected requirement belongs to the invoking extension, host API section, and capability
- the scoped credential account exists when required
- the account is active and owned by the same extension
- the account destination policy matches the request method/URL
- the projected secret handle is actually bound to that credential account

Successful resolution returns secret staging requirements, including each handle's required/optional semantics, plus exact-method-and-URL `WasmStagedRuntimeCredential` rules. The method/URL narrowing is deliberate: even if a projected requirement is broader, the resolved runtime credential rule must not silently apply to a different WASM HTTP request in the same invocation.

This resolver is a planning/projection boundary only. It does not read raw secret material, issue OAuth repairs, mutate credential-account state, or grant authority without the upstream capability/obligation workflow. Final `InjectCredentialOnce` obligation wiring remains the product path that should connect credential account resolution, secret staging, blocked-auth repair, and runtime adapter construction.

Script execution keeps Docker containers ambient-network-disabled by default (`docker run --network none`). If scripts later gain a brokered HTTP SDK, sidecar, helper process, or host API, every request must flow through `ironclaw_scripts::ScriptRuntimeHttpAdapter<RuntimeHttpEgress>`. The host supplies the `ResourceScope`, `CapabilityId`, `NetworkPolicy`, credential injection plan, response body limit, and timeout; script/runtime input must not invent secret handles, raw credential headers/query parameters, DNS checks, private-IP checks, or direct HTTP clients inside `ironclaw_scripts`.
