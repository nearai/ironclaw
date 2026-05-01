# Reborn Host Runtime Contract

`ironclaw_host_runtime` is the composition-facing host boundary above Reborn capability, process, network, secret, audit, and resource substrates. Upper turn/loop services depend on the `HostRuntime` trait and receive structured outcomes instead of concrete substrate handles.

## Obligation composition

`DefaultHostRuntime` may be configured with a `CapabilityObligationHandler` through `with_obligation_handler(...)`. It forwards the handler into `CapabilityHost` for capability invocations.

`BuiltinObligationHandler` is the default host-owned implementation for current V1 obligations. It is deliberately fail-closed: obligations that require backing services fail unless the corresponding store/sink/governor is configured.

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
- Direct `satisfy(...)` releases any prepared resource reservation without discarding successfully staged network/secret handoffs that the caller still needs to pass to runtime adapters.
- Inline dispatch completion discards any unconsumed staged network/secret handoffs so successful calls do not leave reusable ambient state behind.
- Staged secrets must never be logged or exposed through debug output.
- Handler errors must use stable categories and avoid raw provider/backend details.

## Runtime HTTP egress

Runtime HTTP remains host-mediated through `RuntimeHttpEgress` and `HostHttpEgressService`. Runtime code must not perform ad-hoc DNS/private-IP checks or direct HTTP clients; `ironclaw_network` owns network policy enforcement and `ironclaw_secrets` owns secret lease/consume semantics.
