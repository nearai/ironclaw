# ironclaw_host_api

The dependency-free authority vocabulary of the whole system. It names what
crosses every privileged boundary — identities, scopes, paths, mounts,
capability/action/decision/approval shapes, the sealed `Authorized` witness and
the `CapabilityDispatcher` port it is handed to, sanitized resolution and
failure vocabulary, ingress/egress descriptors, runtime and trust vocabulary,
and the complete canonical turn vocabulary — and executes none of it. It is a
separate crate because ~53 workspace manifests depend on it, and its
zero-internal-dependency posture is what makes that safe: any dependency added
here becomes a dependency of the entire system.

- **Family / layer:** `crates/contracts/` / `contracts` · **Package:** `ironclaw_host_api` · **Manifest:** `crates/contracts/ironclaw_host_api/Cargo.toml`
- **Use this when:** you need to *name* authority, identity, a turn, a
  capability effect, or a host decision from anywhere in the workspace.
- **Don't use this when:** you need extension surfaces (→
  `ironclaw_extension_contracts`), the product membrane or wire DTOs (→
  `ironclaw_product_contracts`), loop ports (→ `ironclaw_loop_contracts`), or
  anything that executes, persists, or logs (→ the owning crate above this
  tier).

## Public surface

Module-qualified only — there is no prelude, and the sole crate-root item is
the `Timestamp` alias. The clusters (see `src/lib.rs` for the full list of ~40
modules):

- Identity and authority primitives: `ids`, `scope` (`ExecutionContext`),
  `path`, `mount`, `error`.
- Requested-effect and decision vocabulary: `capability`,
  `capability_profile`, `action`, `decision`, `approval`, `resource`, `audit`.
- The sealed witness and its port: `authorized` (`Authorized`,
  `AuthorizationGrant`), `dispatch` (`CapabilityDispatcher`), `invocation`,
  `lane` (closed `RuntimeLane`).
- Sanitized results: `resolution`, `result_meta`, `gate_record`, `failure`,
  `safe_summary`, `model_result_preview`, `host_remediation`,
  `credential_redaction`.
- Host transport vocabulary: `http` (`RuntimeHttpEgress` port), `ingress`
  (route/policy/listener descriptors), `host_port` (catalog +
  `default_host_port_catalog`), `outbound`, `messaging` (the standardized
  messaging vocabulary — declarations only; validation lives in
  `ironclaw_host_runtime`).
- Runtime and trust: `runtime` (`RuntimeKind`, `TrustClass` with serde-sealed
  privileged variants), `runtime_policy`, `trust`, `process`.
- **`turn` — the complete canonical turn vocabulary.** A crate that only
  *names* turns (ids, refs, `TurnStatus`, `EventCursor`, origin adapters)
  depends on this crate, never on `ironclaw_turns`.
- Protocol-auth evidence: `product_adapter::auth` (`ProtocolAuthEvidence`, the
  bearer/session mint family, and the two witness grants), plus
  `product_adapter_error` and `user_identity` (store traits still here — a
  recorded WS6 debt, PROPOSAL §6.1.1).

## Depends on / consumed by

- **Internal deps: none.** Asserted against every other workspace crate by
  `reborn_crate_dependency_boundaries_hold`.
- **Consumed by** 53 workspace manifests (reproduce:
  `grep -rl '^ironclaw_host_api = ' --include=Cargo.toml crates tests Cargo.toml | wc -l`)
  — effectively every family.
- One feature: `test-support` (dev-only seam — the shared `TestDispatcher`
  double and messaging conformance helpers; never enabled by a shipped
  artifact).

## Invariants

- **Zero internal dependencies, forever** — gate above.
- **No frameworks/drivers** —
  `reborn_contracts_crates_hold_no_framework_dependencies`.
- **Evidence minting is sealed.** `Authorized`, bearer/session
  `ProtocolAuthEvidence`, and the grant types cannot be constructed without a
  witness grant; only one crate may implement each grant-producing trait
  (`reborn_sealed_evidence_mint_ratchet.rs`). See `AGENTS.md` before touching
  anything in `authorized` or `product_adapter::auth`.
- **Size ceiling** — `reborn_contracts_crates_carry_a_checked_size_ceiling`.
- **No wildcard re-exports** — module-qualified imports only (rule stated in
  `src/lib.rs`).

## Tests

```bash
cargo test -p ironclaw_host_api                 # crate suite
cargo test -p ironclaw_architecture_tests       # boundary + seal + ceiling gates
```

## See also

- Working rules and traps: [`AGENTS.md`](./AGENTS.md) (canonical crate
  guidance; `CLAUDE.md` points here).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.1.1;
  `docs/internal/reborn/target-architecture/families/contracts.md`.
- Frozen contract docs: `docs/internal/reborn/contracts/host-api.md`,
  `kernel-boundary.md`, `capability-access.md`.
