# ironclaw_host_runtime

The kernel's mediated-execution service: the boundary between an authorized
witness and an actual runtime lane. It completes the obligations the membrane
prepared (audit before/after, network-policy staging, one-shot secret staging,
mount restriction, resource ceilings, output redaction and limits), invokes
**only** the lane the sealed witness names through the closed executor, and
turns raw lane output into redacted evidence. It also composes the membrane
from the kernel's other services — the single `CapabilityHost` construction
site. One crate, because obligation completion → lane execution → evidence
sanitization is one atomic, security-critical fold; splitting it would scatter
that fold across boundaries without adding isolation.

- **Family / layer:** `kernel/` / `kernel` · **Package:**
  `ironclaw_host_runtime` · **Manifest:**
  `crates/kernel/ironclaw_host_runtime/Cargo.toml`
- **Use this when:** wiring the production host runtime, adding an obligation,
  touching mediated egress/secret staging, registering a first-party tool's
  *host half*, or adapting a lane.
- **Don't use this when:** you want authorization/approval policy (→ the
  stage crates via the membrane), a tool's *executor* half (→
  `ironclaw_extension_support`, which may not name this crate), sandbox
  container mechanics (→ `crates/lanes/ironclaw_sandbox`), or product
  workflow.

## Public surface

- `DefaultHostRuntime` (`src/production.rs:113`) + the `HostRuntime` port;
  `HostRuntimeServices`, `ProductionWiring*` readiness evidence
  (`src/services/`).
- Obligations (`src/obligations/`), split along three chartered owners —
  `handler` (what applies and what each does), `staged_handoffs`
  (`RuntimeSecretInjectionStore`, `NetworkObligationPolicyStore`,
  `RuntimeCredentialAccountResolver`), `process_store` (post-start cleanup +
  reservation reconciliation) — with `mod.rs` holding only the
  `BuiltinObligationServices` assembly seam.
- Mediated HTTP egress (`src/egress/`): validation → credential resolution →
  staged policy lookup → staged secret injection → transport → response
  sanitization, as separate pipeline steps.
- The closed lane executor: `RuntimeLaneExecutor` is `pub(super)`
  (`src/services/runtime_adapters.rs:253`) — deliberately not public; lanes
  are reached only through the witness-bound dispatch it drives.
- Capability surface policy (`src/surface.rs`), hot catalog
  (`src/capability_catalog.rs`), first-party registry + builtin tool host
  halves (`src/first_party*`), and the process boundary
  (`src/process_port.rs`). A manifest may expose an already-authorized
  `RuntimeCredentialRequirement` to managed shell execution through a
  placeholder environment variable. `StagedCredentialProcessPort` consumes
  that exact one-shot handle and emits provider-neutral sandbox bindings. The
  sandbox lane owns shell execution and proxy substitution.
  Memory-context builders and extension-contract *discovery* also live here
  (the `RootFilesystem` binding; default catalogs live with their vocabulary
  owners — see `AGENTS.md`).

## Depends on / consumed by

- **Normal deps (measured, 32):** the rest of the kernel (all eight siblings —
  every edge pinned in `reborn_same_layer_edge_inventory.rs`), the substrates
  it mediates (`ironclaw_filesystem`, `ironclaw_secrets`,
  `ironclaw_network`, `ironclaw_safety`), the lanes it adapts
  (`ironclaw_wasm`, `ironclaw_mcp`, `ironclaw_sandbox`),
  `ironclaw_event_log`/`ironclaw_event_store` (audit), and domain/record
  crates (`ironclaw_memory`, `ironclaw_memory_native`, `ironclaw_outbound`,
  `ironclaw_triggers`, `ironclaw_attachments`, `ironclaw_extractors`,
  `ironclaw_trace_commons`, …). Two designed edges that surprise readers:
  `ironclaw_extension_support` (first-party tool executors the kernel is
  designed to call — PROPOSAL §8.2's 2026-08-04 amendment) and
  `ironclaw_trace_commons` (the shared `is_loopback_host` predicate, D-R).
- **Direct DB drivers (`libsql`, `deadpool-postgres`):** named shrink-only
  residue — a standing §11.2.6 narrowing target, frozen by
  `reborn_persistence_driver_boundary.rs`, not a charter.
- **Normal consumers (5):** `ironclaw_composition`, `ironclaw_extension_host`,
  `ironclaw_extension_manager`, `ironclaw_loop_host`, `ironclaw_turn_runner`.
  Nothing below the loop tier names this crate.

## Invariants

- **The egress credential chokepoint (D-R, 2026-08-05):** a credential
  attaches to an outbound request only over HTTPS **or a literal loopback
  host** — decided by the same
  `ironclaw_trace_commons::onboarding::invite::is_loopback_host` predicate
  the Trace Commons validators trust (`src/egress/credential.rs:398-412`),
  applied to **every** injection target shape (`Header`, `QueryParam`,
  `PathPlaceholder`, `BodyJsonPointer`). Literal only: no DNS resolution, so
  a hostname that merely resolves to loopback does not qualify. Both
  perimeter sides are test-frozen, driven through the configured egress port:
  `host_http_egress_refuses_to_attach_a_credential_over_plaintext_http`
  (`src/services/tests.rs:511`, sabotage-verified) and
  `host_http_egress_attaches_a_credential_over_literal_loopback_http`
  (`src/services/tests.rs:611`). Run:
  `cargo test -p ironclaw_host_runtime --lib host_http_egress`. Widening the
  exception (a hostname class, DNS) must change that shared predicate and
  both tests — it cannot drift silently.
- Raw secret material stays inside the narrow lease/injection path: staged
  secrets are one-shot, runtime-supplied manual credentials are rejected, raw
  and percent-decoded URLs are scanned, leased values are redacted from
  runtime-visible errors/responses, sensitive response headers stripped.
- Credentialed process execution does not search for secrets. The manifest
  declares a placeholder environment variable on each exact header-target
  requirement. Authorization selects and stages matching requirements.
  `StagedCredentialProcessPort` validates the placeholder and exact HTTPS
  header targets, atomically consumes all requested handles, and passes raw
  material only to `SandboxCommandTransport`. The sandbox proxy loads one
  invocation-scoped credential bundle and substitutes each value only for its
  approved destination. No host layer branches on a provider.
- No verified tenant sandbox ⇒ the process/shell capability is hidden by the
  visibility filter (`src/surface.rs`) and refused by the planner
  (`ironclaw_runtime_policy`, `ProcessBackendKind::None`) — never silently
  downgraded to a host process (`.claude/rules/safety-and-sandbox.md`; the
  #6170 rule).
- Accounting: `network_egress_bytes` counts outbound request bytes only.
- Production egress is constructed with staged `NetworkObligationPolicyStore`
  and `RuntimeSecretInjectionStore` handoffs; request-carried policy and
  direct lease sources are legacy/test compatibility paths.

## Tests

```bash
cargo test -p ironclaw_host_runtime
cargo test -p ironclaw_host_runtime --lib host_http_egress   # the D-R pair, 7 tests
cargo test -p ironclaw_architecture_tests                    # after dependency/API changes
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules, the obligations/egress module
  charters, first-party host-half vs executor-half seam, guardrails.
- [`../AGENTS.md`](../AGENTS.md) — the kernel family.
- `docs/internal/reborn/contracts/host-runtime.md`; PROPOSAL §6.5.9 and §12.13 D-R;
  `docs/internal/reborn/target-architecture/ws12-security-audit.md` (seam 4).
