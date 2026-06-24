# Manifest-Driven Channels — Sequencing Plan

**Status:** in progress · **Started:** 2026-06-20
**Goal:** Make channel/extension ingress, egress, secrets, and capabilities
**manifest-defined** instead of provider-specific Rust, so adding a channel
(Discord, WeCom, …) is a manifest authoring task — not an edit to `serve`,
`factory`, the host-ingress registry, and N composition modules.

This plan grew out of the review of [#5100] (project Telegram ingress from
extension state), which correctly puts Telegram on the same manifest-projected
path as Slack ([#5093]) but stops at declaring a *static contract*: the manifest
still only carries selectors, and provider-specific Rust turns them into runtime
behavior. The moves below convert those selectors into data, one keystone at a
time, with a hard rule: **each move must delete Rust, not just rearrange it.**

## Design decisions (locked)

1. **Policy is manifest data, not a Rust-keyed selector.** The closed
   `IngressPolicyProfile` enum (magic-string → Rust-constant policy) is replaced
   by an `IngressPolicy` projected directly from the manifest section. (Move 1.)
2. **One inbound-transport contract with a shared `IngressPolicy` envelope** plus
   a transport discriminator (`webhook | websocket | polling`) — NOT sibling
   `host_ingress` / `websocket` contracts that each re-declare
   auth/scope/audit/effect-path/limits. The security envelope is declared once,
   transport-agnostic. (Move 2.)
3. **Cross-contract credential coherence is a single post-projection invariant.**
   Every credential handle referenced by any contract must resolve to exactly one
   declared credential; a canonical `CredentialHandle` carries identity across the
   per-domain newtypes. (Move 3.)
4. **Each move's success criterion is a named Rust deletion.** A move that adds a
   parallel mechanism instead of replacing one has failed its bar.

## Move sequence

```
Move 0 (this doc) ─► Move 1 (IngressPolicy, KEYSTONE) ─► Move 2 (transport union)
                                                     └─► Move 3 (credential coherence)
                            Move 2 + Move 3 ──────────► Move 4 (serve.rs collapse)
                                                     └─► Move 5+ (setup, connectable, …)
```

### Move 1 — IngressPolicy projectable; delete the profile enum ✅ (this PR)

Replace `policy_profile = "..."` (a selector into the closed
`IngressPolicyProfile` enum) with an inline, typed `[host_ingress.*.policy]`
declaration projected straight onto the existing `IngressPolicy` type. Route
descriptor is built from the manifest's own `route_id`/`method`/`path`.

- **Deleted:** `IngressPolicyProfile` + all impls, `slack_events_policy`,
  `telegram_updates_policy`, the two `*_route_descriptor` fns, all `SLACK_EVENTS_*`
  / `TELEGRAM_UPDATES_*` constants, the `policy_profile` field, and the
  `ProfileRouteMismatch` / route-identity validation.
- **Auth honesty:** added `IngressAuthScheme::SharedSecretHeader`; Telegram now
  declares shared-secret-header auth, Slack stays webhook-signature. The lossy
  "everything is `WebhookSignature`" coercion is partially removed.
- **Fail-closed:** unknown enum value, zero limit, and missing field are rejected
  with typed errors (tests added). Registry file shrank 968 → 807 lines.
- **Both** Slack and Telegram manifests migrated; projected policy is
  behavior-equal to the deleted Rust functions.

### Move 2 — Unify the transport model behind one contract ✅

**Base:** stacks on Move 1. **Bar:** the webhook-specific policy fields collapse
into the shared envelope; the residual string→scheme mapping is deleted.

Completed:
- Added a tagged transport shape under `[host_ingress.*.transport]`. Slack and
  Telegram webhook ingress now express `route_id`, `method`, `path`, `ack`, and
  `drain` through `kind = "webhook"` while sharing the same manifest-projected
  `IngressPolicy` security envelope.
- Kept websocket/polling runtime support out of this move. Unsupported transport
  kinds fail closed at parse time, so the contract can grow only when there is a
  real second transport consumer.
- Deleted the auth-scheme string fallback path. `host_ingress.*.auth.verifier`
  is now the typed `IngressAuthScheme`; unknown values fail at parse time, and
  mismatched policy/binding pairs reject at declaration validation time. Slack's
  signature verifier and Telegram's shared-secret-header verifier are pinned by
  caller-level registry tests.
- **Deletion bar met:** the residual `declared_auth_scheme()` string match and
  webhook-only policy duplication are gone.

Out of scope: `serve.rs` mount wiring (Move 4), websocket runtime,
connectable/egress modules.

### Move 3 — Cross-contract credential coherence ✅ (separate PR)

Add a canonical `CredentialHandle` (`ironclaw_host_api::ids`) and a
post-projection invariant in `HostApiContractRegistry::project_manifest`: every
referenced credential handle must resolve to a declared credential. Closes the
"same credential spelled in two sections, drifts" class (bug #2574 family).

- Extended `HostApiManifestProjection` with `declared_credentials` +
  `referenced_credentials` (with `host_api`/section provenance).
- `ManifestV2Error::DanglingCredentialHandle { handle, host_api, section }`.
- Wired product-adapter (declared = `required_credentials`, referenced = egress
  handles) and capability-provider (referenced = `SecretHandle` runtime creds).
- **Integration seam (done at Move 1 ↔ Move 3 merge):** the host-ingress contract
  reports each `host_ingress.*.auth.credential_handles` entry as a
  `ReferencedCredential` (≈3 lines in
  `HostIngressHostApiContract::project_section_with_context`). Then tighten the
  empty-declared-set rule to require a declaration for every referenced handle.

### Move 4 — Collapse the `serve.rs` per-channel cfg sprawl ✅ / residual setup work

Replace the `#[cfg(feature = "telegram-v2-host-beta")]` + nested
`is_generic()/is_generic_shadow()` mount block (cloned from Slack) with a single
loop that iterates enabled extensions and mounts whatever transports they project.
**Bar:** "add a channel" touches zero lines in `serve.rs`.

Current state:
- `serve.rs` now builds one `HostIngressServePlanInput`, calls one
  `build_host_ingress_serve_plan`, and applies the resulting generic public-route
  mounts/connectable-channel metadata to WebUI serving.
- `HostIngressServePlan` reads enabled extension installations, projects
  `ironclaw.host_ingress/v1` declarations, and serves route mounts according to
  per-route projection policy.
- Channel-specific Slack/Telegram code still exists only at the runtime adapter
  boundary and for legacy host-beta config import. That residual import path is
  deliberately deferred to `ironclaw.extension_setup/v1` below; removing it here
  would turn this move into a product setup rewrite instead of an ingress
  contract deletion.

### Move 5+ — Remaining contracts (one per PR, each deleting Rust)

- `ironclaw.extension_setup/v1` — deletes the provider-specific
  `import_*_host_beta_config_as_extension_installation` path.
- `ironclaw.connectable_channel/v1` — deletes the hardcoded
  `*_inbound_proof_code_connectable_channel()` composition. References credentials
  by handle; never re-declares them.
- `channel_runtime` only if it deletes real branching (skip if it merely relocates
  `capabilities.flags`).

## Verification gate (every move)

`cargo fmt` · `cargo clippy -D warnings` on touched crates ·
`cargo test` on touched crates · `cargo check --workspace --all-features`.
Agent sandboxes cannot reach the network, so the workspace check is run by the
human/host reviewer before merge.

[#5072]: https://github.com/nearai/ironclaw/pull/5072
[#5093]: https://github.com/nearai/ironclaw/pull/5093
[#5100]: https://github.com/nearai/ironclaw/pull/5100
