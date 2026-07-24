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
change trips that gate, you are re-introducing a deleted model.

**Schema is `reborn.extension_manifest.v3`** — v2 plus explicit `[channel]`
and `[auth.*]` sections. The design is law: `docs/reborn/extension-runtime/`
(`overview.md` §3 the manifest, §4 the adapters, §5 the flows, §6 lifecycle).
The worked example for *every* section below is the live Slack manifest —
read it first: `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml`.

## The model in one diagram

```text
extension  (one manifest.toml, one installed identity, e.g. `slack`)
  [[tools]]              → tool surfaces (model-callable), each with [[tools.credentials]]
  [channel]             → channel surface (AT MOST ONE per extension; inbound/outbound)
  [auth.<vendor>]       → auth surface: one recipe per vendor (oauth2_code | api_key)
  [mcp]                 → hosted-MCP extension: discovered tools INSTEAD of [runtime]+[[tools]]
  [runtime] first_party|wasm|mcp   → how the adapter LOADS (implementation only, never taxonomy)
```

- `ExtensionId` (`slack`, `github`, `gmail`) — product/installed identity.
- `VendorId` (`slack`, `github`, `google`) — credential authority; several
  extensions may share one (gmail + google-drive + calendar + … share
  `google`). The manifest field is `vendor = "..."`. It is **not** the
  extension id. (Renamed from `ProviderId`/`RuntimeCredentialAccountProviderId`
  in this train — overview §2; stored id strings are unchanged.)
- `CapabilitySurfaceKind` (`crates/ironclaw_host_api/src/surface.rs`) — `tool`,
  `channel`, `auth` (+ reserved `trigger`, `file`).
- Surfaces are **derived** from the resolved manifest — never store a parallel
  taxonomy. The manifest compiles **once** per install into a typed
  `ResolvedExtensionManifest` (+ `manifest_digest`); all production projection
  reads the resolved record, never re-parsed TOML (overview §3.3).

Adapters implement **behavior only** (overview §4): they never report ids,
schemas, effects, scopes, routes, or credentials — the resolved manifest is the
sole authority. Trait homes:
`ToolAdapter` — `crates/ironclaw_host_api/src/tool_adapter.rs`;
`ChannelAdapter` — `crates/ironclaw_product_adapters/src/channel_adapter.rs`;
`ExtensionEntrypoint`/`ExtensionBindings` —
`crates/ironclaw_extension_host/src/entrypoint.rs`. Auth has **no** adapter
trait — it is one host engine driving manifest recipes (overview §4.3).

## Where a bundled package lives

Every first-party integration is a self-contained package:
`crates/ironclaw_first_party_extensions/assets/<id>/` (manifest + schemas +
prompts + any WASM) beside one module
`crates/ironclaw_first_party_extensions/src/packages/<id>.rs` (embeds via
`include_str!`/`include_bytes!`, onboarding copy, trust effects). A collector
concatenates them; add a line to `PACKAGES` in `.../src/packages/mod.rs`.
Composition and the CLI consume these as **opaque bundles** and never name a
package (overview §3). Do NOT register assets in composition — the old
`available_extensions.rs::*_assets()` home is being dissolved.
Re-verify the module list: `grep -n 'ID,' crates/ironclaw_first_party_extensions/src/packages/mod.rs`.

## Adding a tool surface

1. Declare each capability as a `[[tools]]` entry (id, description, effects,
   default_permission, visibility, `input_schema_ref`, optional
   `prompt_doc_ref`) with a `[[tools.credentials]]` block naming its `vendor`,
   `audience`, and `injection`. Copy the shape from
   `assets/slack/manifest.toml` (5 `[[tools]]` entries) or
   `assets/github/manifest.toml`.
2. Schemas and prompt docs are **package assets** (`schemas/…`, `prompts/…`)
   embedded by the package module — not composition.
3. Model-visible tool wording is product surface: if a tool acts *as the user*
   (delegated authority), its description and prompt doc must say so — and must
   say the tool is for side effects inside a job, never for delivering the final
   answer (the host delivers final replies on the outbound channel surface —
   overview §5.4). Exemplar: `assets/slack/prompts/slack/send_message.md`.

## Adding a channel surface

1. Add a `[channel]` section (**at most one per extension**) with `id`,
   `display_name`, `inbound`/`outbound` bools, and **required**
   `conversation_model` (`continuous` | `isolated`, overview §3). Then its
   subsections, all worked in `assets/slack/manifest.toml`:
   `[channel.ingress]` (route_suffix, method, body limit),
   `[channel.ingress.verification]` (declarative recipe the *host* executes —
   `hmac_sha256` segment list or `shared_secret_header`; signing secrets never
   reach the adapter), `[channel.config]` (operator setup fields; host renders
   the generic form), `[[channel.egress]]` (host allowlist + credential handle),
   and `[channel.presentation]`.
2. Direction is the `inbound`/`outbound` bools, which project to
   `channel { inbound, outbound }` on the extensions wire — the agent never gets
   an "outbound delivery" tool; final delivery is the runtime-owned delivery
   coordinator (overview §5.4).
3. Behavior lives in the extension's `ChannelAdapter` (`inbound` parse →
   normalized outcome; `deliver` render+send; idempotent `activate`/`cleanup`
   vendor wiring) — see the trait doc for the method contract. The binary
   supplies the adapter to composition through the
   `RebornHostBindings::with_channel_extension_bindings` seam
   (`crates/ironclaw_reborn_composition/src/input.rs`, `ChannelExtensionBinding`);
   composition iterates it by `extension_id` and never names a concrete crate.
4. Conversation/actor binding is **data, not per-channel code**: the
   `conversation_model` value + the identity resolver drive it. Contract:
   `docs/reborn/contracts/conversation-binding.md`. The actor→user resolver is
   `ProviderIdentityActorResolver`
   (`crates/ironclaw_reborn_composition/src/provider_identity.rs`),
   parameterized by (vendor, adapter id, actor kind) — not a per-channel
   resolver (the retired-taxonomy gate hunts the old pattern).
5. Connect affordance is **derived** (overview §6.4): installation state +
   `[channel.config]` completeness + the auth account state. The WebUI channels
   tab renders every channel surface with the same generic components — there is
   no channel registry to update (frontend helpers:
   `crates/ironclaw_webui/frontend/src/pages/extensions/lib/extensions-schema.ts`,
   `hasChannelSurface`). Editing `[channel.config]` while `Active` runs an
   automatic deactivate → reactivate cycle; there is no separate reconfigure
   state or channel-setup activation gate.

## Adding / sharing an auth provider

1. Add one `[auth.<vendor>]` recipe per vendor the extension needs — the
   section key **is** the vendor id. `method = "oauth2_code"` (endpoints,
   `scope_param`, PKCE, `client_credentials` handles, `[auth.<vendor>].token_response`
   + `[auth.<vendor>].identity` JSON-pointer maps) or `method = "api_key"` (form
   `fields` + optional `validation` probe). Worked example: `[auth.slack]` in
   `assets/slack/manifest.toml`; the full recipe vocabulary is overview §4.3 +
   implementation.md §7. There is **no auth adapter trait and no extension code
   in an auth flow** — the host engine (`crates/ironclaw_auth`) runs each method
   once over the recipe data.
2. Share a `vendor` across extensions when the credential authority is the same
   (`google` across gmail/drive/calendar/docs/sheets/slides). Recipes for one
   vendor must be identical except `scopes`/`display_name`, or activation fails
   with a conflict; scopes union across active extensions (overview §3.2).
3. Renaming any persisted identity (vendor id, extension id) requires a one-time
   forward data migration, never a runtime alias. Exemplars:
   `migrate_retired_slack_bot_identity`
   (`extension_host/extension_installation_store.rs`) and
   `migrate_retired_slack_personal_provider`
   (`product_auth/durable/mod.rs`), both with idempotency pins. These files are
   sanctioned to name the retired vocabulary (both the retired-taxonomy and
   specificity gates carve migration code out).

## Hosted-MCP extensions

An extension whose tools are discovered from a server declares one `[mcp]`
section (server, namespace, max_tools, effects, `[[mcp.credentials]]`) **instead
of** `[runtime]` + `[[tools]]` + `[channel]`, plus its `[auth.<vendor>]` recipe.
The MCP loader owns discovery; past activation a discovered tool is an ordinary
tool surface (overview §3.1). Worked example:
`assets/notion-mcp/manifest.toml`.

## Testing surfaces

- Manifest projection (v3): `crates/ironclaw_extensions/tests/manifest_v3_contract.rs`;
  channel ingestion through the real contract:
  `crates/ironclaw_product_adapter_registry/tests/manifest_ingestion.rs`. Extend
  these rather than adding parallel suites.
- Adapter behavior: the exported conformance suites — channel in
  `ironclaw_product_adapters`, tool in `ironclaw_host_api` test-support, auth in
  the `ironclaw_auth` engine suite — run by every extension crate + the `acme`
  fixture (`tests/fixtures/extensions/acme-messenger/`). Real extensions add one
  end-to-end integration proof each (`tests/integration/`).
- Frontend: `pnpm --dir crates/ironclaw_webui/frontend test`.
- Always finish with `cargo test -p ironclaw_architecture` — the specificity,
  dependency-direction, and retired-taxonomy gates are the machine reviewers.
  Generic code naming your extension trips
  `reborn_extension_specificity.rs`: put the name in the package/CLI, not
  generic crates.

## Sibling skills

`reborn-feature` (wiring a feature through the layers) ·
`ironclaw-reborn-architecture-review` (boundaries) ·
`ironclaw-reborn-testing` (tiers).
