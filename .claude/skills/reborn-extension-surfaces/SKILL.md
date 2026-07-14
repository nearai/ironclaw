---
name: reborn-extension-surfaces
description: Use when adding or changing a Reborn integration — a new extension, a channel surface, model-callable tools, or a shared auth provider — or when deciding whether something is "a channel", "an extension", or "a tool". Maps the unified extension model (NEA-25) to the exact manifest sections, crates, seams, and tests.
---

# Reborn Extension Surfaces

The top-level product object is always an **extension**. A channel is not a
sibling product type: it is one **capability surface** an extension's manifest
declares, exactly like tools and auth. Runtime (`wasm` / `mcp` /
`first_party`) is implementation only and never taxonomy. The retired
vocabulary (connectable channels, `slack_bot`, `slack_personal`, extension
`kind` strings) is pinned at zero by
`crates/ironclaw_architecture/tests/reborn_retired_taxonomy.rs` — if your
change trips that gate, you are re-introducing a deleted model, not extending
the current one.

## The model in one diagram

```text
extension (one manifest, one installed identity, e.g. `slack`)
  [[host_api]] ironclaw.capability_provider/v1  → tool surfaces
  [[host_api]] ironclaw.product_adapter/v1      → channel surface
  runtime_credentials product_auth_account      → auth surface (per provider)
  [runtime] wasm|mcp|script|first_party         → how the adapter loads (only)
```

- `ExtensionId` (`slack`, `github`, `gmail`) — product/runtime identity.
- `ProviderId` (`slack`, `github`, `google`) — credential authority; several
  extensions may share one (gmail + google-drive + … share `google`).
- `CapabilitySurfaceKind` (`ironclaw_host_api/src/surface.rs`) — `tool`,
  `channel`, `auth`.
- Surfaces are **derived**: `ExtensionManifestV2::capability_surfaces()`
  (`crates/ironclaw_extensions/src/v2.rs`) projects them from the manifest —
  never store a parallel taxonomy.

Every manifest declares at least one `[[host_api]]` contract and parses
through exactly one entry point:
`ExtensionManifestV2::parse(input, source, catalog, contracts)` with
`ironclaw_host_runtime::default_host_api_contract_registry()`. Top-level
`[[capabilities]]` is rejected for every source.

## Adding a tool surface

1. Declare capabilities under `[[capability_provider.tools.capabilities]]`
   with `[[host_api]] id = "ironclaw.capability_provider/v1"`. Copy a real
   manifest: `crates/ironclaw_first_party_extensions/assets/github/manifest.toml`.
2. Prompt docs and schemas are manifest assets (`prompts/…`, `schemas/…`),
   registered in the crate's `*_assets()` fn in
   `crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs`.
3. Model-visible tool wording is product surface: if a tool acts *as the
   user* (delegated authority), its description and prompt doc must say so —
   and must say the tool is for side effects inside a job, never for
   delivering the final answer (the host delivers final replies on outbound
   channel surfaces). Exemplar: `assets/slack/prompts/slack/send_message.md`.

## Adding a channel surface

1. Add a `[product_adapter.inbound]` section referenced by
   `[[host_api]] id = "ironclaw.product_adapter/v1"` — the unified Slack
   manifest (`assets/slack/manifest.toml`) is the worked example: ingress
   auth (`request_signature`), capability flags, required credential
   handles, egress allowlist, and the host-ingress route descriptor.
2. Direction is typed from the section's flags: `inbound_messages` → inbound
   (external messages arrive here), `external_final_reply_push` → outbound
   (the host delivers final replies/notifications here). These project to
   `channel { inbound, outbound }` on the extensions wire — the agent never
   gets an "outbound delivery" tool; final delivery is runtime-owned.
3. Actor→user resolution is **data, not code**: parameterize
   `ProviderIdentityActorResolver`
   (`crates/ironclaw_reborn_composition/src/provider_identity.rs`) with your
   provider id, adapter id, and actor kind — see
   `slack_provider_identity_actor_resolver` in
   `slack_host_beta/runtime_setup.rs`. Do not write a per-channel resolver;
   the retired-taxonomy gate hunts the old pattern.
4. Binding semantics (unbound actor → fail closed + connect nudge, canonical
   refs only past the boundary) are owned by the conversation-binding
   contract: `docs/reborn/contracts/conversation-binding.md`.
5. Connect affordance: Train A has one composition-owned inbound connection:
   the host-bundled unified `slack` package paired with the `slack` product-auth
   provider. Extension-wide auth surfaces do not encode channel ownership, so
   every other inbound/auth combination fails closed until a typed association
   exists. Install checks before side effects, while restore and activation
   re-check persisted rows. Supported summaries carry the OAuth
   `channel_connection` built by
   `channel_connection_requirement()`; the WebUI renders only that typed
   connection and never infers pairing from onboarding text. Re-verify with
   `rg -n "ensure_supported_channel_connection|channel_connection_requirement" crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`.
6. Operator provisioning that activates the channel goes through
   `activate_for_channel_setup` (the `ChannelSetupActivationCredentialGate`):
   per-caller OAuth accounts never gate operator activation — callers
   auth-gate at tool-call time.

## Adding / sharing an auth provider

1. On each capability needing user authority, declare
   `runtime_credentials = [{ source = { type = "product_auth_account",
   provider = "<provider>", setup = { kind = "oauth", scopes = [...] } }, … }]`.
   One auth surface per distinct provider is derived automatically, with
   OAuth scopes unioned (see `capability_surfaces_from_parts` in
   `crates/ironclaw_extensions/src/v2.rs`).
2. Share the provider id across extensions when the credential authority is
   the same (`google` across gmail/drive/calendar/docs/sheets/slides). The
   provider id is not the extension id — tests pin this in
   `crates/ironclaw_extensions/tests/manifest_v2_contract.rs`.
3. Renaming any persisted identity (provider id, extension id) requires a
   one-time forward data migration, never a runtime alias. Exemplars:
   `normalize_retired_slack_identity`
   (`extension_host/extension_installation_store.rs`) and
   `migrate_retired_slack_personal_provider`
   (`product_auth/durable/mod.rs`), both with idempotency pins.
   Before the Train A credential migration, stop every pre-Train-A writer;
   rolling old/new overlap can create a retired row after enumeration. Rollback
   after cutover means restore the pre-cutover data backup or roll forward.
   Do not infer safety from a deployment overlap setting alone.

## Persistence cutover rules

- CAS-capable installation-state mutations always reread and update the
  winning snapshot, and fresh install commits manifest plus installation in
  one transition.
- The explicitly opted-in non-CAS local-development profile uses a bounded
  per-store compatibility worker for normal lifecycle writes. It is not a
  hosted multi-writer guarantee and is never selected by hosted profiles.

## Testing surfaces

- Manifest projection: extend
  `crates/ironclaw_extensions/tests/manifest_v2_contract.rs` (tool/auth) and
  `crates/ironclaw_product_adapter_registry/tests/manifest_ingestion.rs`
  (channel via the real contract).
- Bundled-package surface pins: `bundled_slack_package_declares_product_adapter_channel_surface`
  in `available_extensions.rs` asserts kinds + directions over the real asset.
- Wire: `list_extensions_projects_channel_surface_with_directions_and_connection`
  in `crates/ironclaw_product_workflow/tests/reborn_services_contract.rs`.
- Unsupported inbound connection policy:
  `extension_install_rejects_inbound_channel_without_supported_connection_owner`,
  `extension_install_rejects_inbound_channel_with_unrelated_oauth_surface`,
  and the persisted restore/activation `connection_owner` tests in
  `extension_lifecycle.rs` prove every lifecycle entrance fails closed;
  regenerate locations with `rg -n "connection_owner|unrelated_oauth_surface"
  crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`.
- Frontend: surface helpers live in
  `crates/ironclaw_webui_v2/frontend/src/pages/extensions/lib/extensions-schema.ts`
  (`hasChannelSurface` etc.); run `pnpm --dir crates/ironclaw_webui_v2/frontend test`.
- Always finish with `cargo test -p ironclaw_architecture` — the boundary
  suites plus the retired-taxonomy gate are the machine reviewers.

## Sibling skills

`reborn-feature` (wiring a feature through the layers) ·
`ironclaw-reborn-architecture-review` (boundaries) ·
`ironclaw-reborn-testing` (tiers).
