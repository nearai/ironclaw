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
`crates/app/ironclaw_architecture_tests/tests/reborn_retired_taxonomy.rs` — if your
change trips that gate, you are re-introducing a deleted model.

**Schema is `reborn.extension_manifest.v3`** — v2 plus explicit `[channel]`
and `[auth.*]` sections. The design is law: `docs/reborn/extension-runtime/`
(`overview.md` §3 the manifest, §4 the adapters, §5 the flows, §6 lifecycle).
The worked example for *every* section below is the live Slack manifest —
read it first: `crates/extensions/packages/slack/manifest.toml`.

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
- `CapabilitySurfaceKind` (`crates/contracts/ironclaw_extension_contracts/src/surface.rs`) — `tool`,
  `channel`, `auth` (+ reserved `trigger`, `file`).
- Surfaces are **derived** from the resolved manifest — never store a parallel
  taxonomy. The manifest compiles **once** per install into a typed
  `ResolvedExtensionManifest` (+ `manifest_digest`); all production projection
  reads the resolved record, never re-parsed TOML (overview §3.3).

Adapters implement **behavior only** (overview §4): they never report ids,
schemas, effects, scopes, routes, or credentials — the resolved manifest is the
sole authority. Trait homes:
`ToolAdapter` — `crates/contracts/ironclaw_extension_contracts/src/tool_adapter.rs`;
`ChannelAdapter` — `crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs`;
`ExtensionEntrypoint`/`ExtensionBindings` —
`crates/extensions/ironclaw_extension_host/src/entrypoint.rs`. Auth has **no** adapter
trait — it is one host engine driving manifest recipes (overview §4.3).

## Where a bundled package lives

Every first-party integration is a self-contained package:
`crates/extensions/packages/<id>/` (manifest + schemas +
prompts + any WASM) beside one module
`crates/extensions/ironclaw_extension_support/src/packages/<id>.rs` (embeds via
`include_str!`/`include_bytes!`, onboarding copy, trust effects). A collector
concatenates them; add a line to `PACKAGES` in `.../src/packages/mod.rs`.
Composition and the CLI consume these as **opaque bundles** and never name a
package (overview §3). Do NOT register assets in composition — the old
`available_extensions.rs::*_assets()` home (now `crates/extensions/ironclaw_extension_host/src/available_extensions.rs`) is being dissolved.
Re-verify the module list: `grep -n 'ID,' crates/extensions/ironclaw_extension_support/src/packages/mod.rs`.

## Adding a tool surface

1. Declare each capability as a `[[tools]]` entry (id, description, effects,
   default_permission, visibility, `input_schema_ref`, optional
   `prompt_doc_ref`) with a `[[tools.credentials]]` block naming its `vendor`,
   `audience`, and `injection`. Copy the shape from
   `crates/extensions/packages/slack/manifest.toml` (8 `[[tools]]` entries — count with
   `grep -c '^\[\[tools\]\]' <manifest>`) or
   `crates/extensions/packages/github/manifest.toml`.
2. Schemas and prompt docs are **package assets** (`schemas/…`, `prompts/…`)
   embedded by the package module — not composition.
3. Model-visible tool wording is product surface: if a tool acts *as the user*
   (delegated authority), its description and prompt doc must say so — and must
   say the tool is for side effects inside a job, never for delivering the final
   answer (the host delivers final replies on the outbound channel surface —
   overview §5.4). Exemplar: `crates/extensions/packages/slack/prompts/slack/send_message.md`.
4. A **messaging-shaped** tool (send/read/react over a conversation) binds
   `standard_op = "<op_name>"` instead of declaring its own schemas — a
   closed, host-owned vocabulary with host-canonical input/output schemas.
   See `docs/reborn/extension-runtime/standard-operations.md` for the full
   vocabulary and binding rules.

## Adding a channel surface

1. Add a `[channel]` section (**at most one per extension**) with `id`,
   `display_name`, `inbound`/`outbound` bools, and **required**
   `conversation_model` (`continuous` | `isolated`, overview §3). Then its
   subsections, all worked in `crates/extensions/packages/slack/manifest.toml`:
   `[channel.ingress]` (route_suffix, method, body limit),
   `[channel.ingress.verification]` (declarative recipe the *host* executes —
   `hmac_sha256` segment list or `shared_secret_header`; signing secrets never
   reach the adapter), `[channel.connection]` (connection strategy),
   `[[channel.egress]]` (host allowlist + credential handle), and
   `[channel.presentation]`. Operator/deployment setup fields are a separate
   top-level `[admin_configuration]` section (there is no `[channel.config]`);
   the host renders the generic form from it.
2. Direction is the `inbound`/`outbound` bools, which project to
   `channel { inbound, outbound }` on the extensions wire — the agent never gets
   an "outbound delivery" tool; final delivery is the runtime-owned delivery
   coordinator (overview §5.4).
3. Behavior lives in the extension's `ChannelAdapter` (`inbound` parse →
   normalized outcome; `deliver` render+send; idempotent `activate`/`cleanup`
   vendor wiring) — see the trait doc for the method contract. The binary
   supplies the adapter to composition through the
   `RebornHostBindings::with_channel_extension_bindings` seam
   (`crates/app/ironclaw_composition/src/input.rs`, `ChannelExtensionBinding`);
   composition iterates it by `extension_id` and never names a concrete crate.
4. Conversation/actor binding is **data, not per-channel code**: the
   `conversation_model` value + the identity resolver drive it. Contract:
   `docs/reborn/contracts/conversation-binding.md`. The actor→user resolver is
   `ProviderIdentityActorResolver`
   (`crates/extensions/ironclaw_extension_host/src/provider_identity.rs`, re-exported by
   composition),
   parameterized by (vendor, adapter id, actor kind) — not a per-channel
   resolver (the retired-taxonomy gate hunts the old pattern).
5. Connect affordance is **derived** (overview §6.4): installation state +
   `[admin_configuration]` completeness + the auth account state. The WebUI channels
   tab renders every channel surface with the same generic components — there is
   no channel registry to update (frontend helpers:
   `crates/product/ironclaw_webui/frontend/src/pages/extensions/lib/extensions-schema.ts`,
   `hasChannelSurface`). Editing `[channel.connection]` while `Active` runs an
   automatic deactivate → reactivate cycle; there is no separate reconfigure
   state or channel-setup activation gate.

## Adding / sharing an auth provider

1. Add one `[auth.<vendor>]` recipe per vendor the extension needs — the
   section key **is** the vendor id. `method = "oauth2_code"` (endpoints,
   `scope_param`, PKCE, `client_credentials` handles, `[auth.<vendor>].token_response`
   + `[auth.<vendor>].identity` JSON-pointer maps) or `method = "api_key"` (form
   `fields` + optional `validation` probe). Worked example: `[auth.slack]` in
   `crates/extensions/packages/slack/manifest.toml`; the full recipe vocabulary is overview §4.3 +
   implementation.md §7. There is **no auth adapter trait and no extension code
   in an auth flow** — the host engine (`crates/domains/ironclaw_auth`) runs each method
   once over the recipe data.
2. Share a `vendor` across extensions when the credential authority is the same
   (`google` across gmail/drive/calendar/docs/sheets/slides). Recipes for one
   vendor must be identical except `scopes`/`display_name`, or activation fails
   with a conflict; scopes union across active extensions (overview §3.2).
3. Renaming any persisted identity (vendor id, extension id) requires a one-time
   forward data migration, never a runtime alias. There is currently **no
   in-tree migration exemplar**: the retired-identity boot migration this skill
   used to cite was deleted 2026-08-04 by owner ruling, and its identifiers
   (`RETIRED_SLACK_USER_EXTENSION_ID`, `remove_retired_internal_installation`)
   are now on the banned list in
   `crates/app/ironclaw_architecture_tests/tests/reborn_retired_taxonomy.rs` — do not
   reintroduce them. The behavioral pin is
   `restore_special_cases_no_extension_id_and_leaves_every_uncatalogued_row_intact`
   (`crates/extensions/ironclaw_extension_host/tests/lifecycle_restore_contract.rs`):
   boot restore must leave uncatalogued rows intact, never delete or re-key
   them.

## Hosted-MCP extensions

An extension whose tools are discovered from a server declares one `[mcp]`
section (server, namespace, max_tools, effects, `[[mcp.credentials]]`) **instead
of** `[runtime]` (and normally instead of `[[tools]]` + `[channel]`), plus its
`[auth.<vendor>]` recipe. A hosted-MCP package MAY additionally pin static
`[[tools]]` entries beside `[mcp]` — guaranteed model-visible from first boot
and on the bundled-manifest fallback; a successful `tools/list` discovery
replaces the static set with the server's live catalog (worked example:
`nearai.web_search` in `crates/extensions/packages/nearai-mcp/manifest.toml`).
The MCP loader owns discovery; past activation a discovered tool is an ordinary
tool surface (overview §3.1). Worked example:
`crates/extensions/packages/notion-mcp/manifest.toml`.

## Testing surfaces

- Manifest projection (v3): `crates/extensions/ironclaw_extension_registry/tests/manifest_v3_contract.rs`;
  channel ingestion through the real contract:
  `crates/extensions/ironclaw_extension_registry/tests/product_adapter_manifest_ingestion.rs`
  (drives `parse_product_adapter_manifest_record` in
  `crates/extensions/ironclaw_extension_registry/src/host_api/product_adapter.rs`). Extend
  these rather than adding parallel suites.
- Adapter behavior: the exported conformance suites — channel in
  `crates/contracts/ironclaw_extension_contracts/src/test_support/conformance.rs`
  (`run_channel_adapter_conformance`, consumed by
  `crates/extensions/packages/{slack,telegram}/tests/channel_conformance.rs`),
  standard-messaging schema conformance in
  `crates/contracts/ironclaw_host_api/src/test_support/messaging_conformance.rs`, and auth
  in `crates/domains/ironclaw_auth/src/test_support/conformance.rs`. Run by every
  applicable extension crate + the `acme`
  fixture (`tests/fixtures/extensions/acme-messenger/`). Real extensions add one
  end-to-end integration proof each (`tests/integration/`).
- Frontend: `pnpm --dir crates/product/ironclaw_webui/frontend test`.
- Always finish with `cargo test -p ironclaw_architecture_tests` — the specificity,
  dependency-direction, and retired-taxonomy gates are the machine reviewers.
  Generic code naming your extension trips
  `reborn_extension_specificity.rs`: put the name in the package/CLI, not
  generic crates.

## Sibling skills

`reborn-feature` (wiring a feature through the layers) ·
`ironclaw-reborn-architecture-review` (boundaries) ·
`ironclaw-reborn-testing` (tiers).
