# Unified Extension Runtime — Implementation

**Companions:** `overview.md` (model — read it first), `checklist.md` (acceptance).
**Baseline:** this branch. It already contains the pending unified-extension-taxonomy PR stack (eight PRs, merge chain ending in #5850) that must land on main before this work starts.

This document says what changes and where. It follows the repo's testing law:
every workstream starts with failing tests at the tier that can observe the
behavior, and persistent behavior is proven on libSQL **and** PostgreSQL.

## 1. Landing strategy

1. **Merge the pending taxonomy PR stack first** (the eight-PR chain ending
   in #5850, already contained in this branch). It is reviewed and this design
   builds on it. Do not stack this work further on an unmerged base — main is
   actively churning the same Slack files.
2. Implement in phases P0–P7 (section 13), each an independently green,
   reviewable PR into main. A phase may not leave two indefinite runtime paths:
   when a production caller cuts over, the old path is deleted in that phase or
   explicitly listed in the migration shims (section 11).
3. `checklist.md` is updated in the same PR that makes items true, with the
   test/command that proves each item.
4. Composition stays assembly-only. New behavior lives in owning crates.

## 2. Current state (verified against this branch)

Already generic on this branch: one manifest per extension parsed through
`ExtensionManifestV2::parse`; surfaces projected by `capability_surfaces()`
(`crates/ironclaw_extensions/src/v2.rs`); surface kinds in
`crates/ironclaw_host_api/src/surface.rs`; channel surfaces on the extensions
wire with directions and connection affordance
(`crates/ironclaw_product/src/reborn_services/{types,extensions}.rs`);
a narrow channel protocol adapter trait
(`crates/ironclaw_product_adapters/src/adapter.rs`) implemented by
`crates/ironclaw_slack_extension`; retired-taxonomy architecture gate.

Not generic yet — the work:

| Problem | Where |
| --- | --- |
| ~30k lines of Slack production code inside composition | `crates/ironclaw_reborn_composition/src/slack/**` (host graph, serve, installation resolution, delivery, egress, targets, DM open, connection, setup, routes, personal OAuth/binding, state roots) |
| Dispatcher selects package + runtime kind per invocation | `crates/ironclaw_dispatcher/src/lib.rs` |
| Adapter trait duplicates manifest metadata (surface kind, capabilities, auth requirement, egress getters) | `crates/ironclaw_product_adapters/src/adapter.rs` |
| Adapter registry projection is not the production path | `crates/ironclaw_product_adapter_registry/src/lib.rs` |
| OAuth branches on Slack; providers multiplexed by string | `crates/ironclaw_reborn_composition/src/product_auth/serve/oauth.rs`, `.../credentials/product_auth_providers.rs`, `.../oauth/google_oauth.rs`, `src/slack/slack_personal_oauth.rs` |
| Auth surfaces implicit (derived from tool credentials); provider specs are code constants | `crates/ironclaw_extensions/src/v2.rs`, composition `product_auth/**` |
| Installed records persist raw TOML and reproject from it | `crates/ironclaw_extensions/src/installations.rs` |
| Hosted MCP mutates capabilities from live `tools/list` | `crates/ironclaw_extensions/src/hosted_mcp_discovery.rs` |
| Lifecycle emits Slack-specific connection copy; workflow has Slack cleanup literals | `crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs`, `crates/ironclaw_product/src/reborn_services/extensions.rs` |
| Slack-only frontend components and branches | `crates/ironclaw_webui/frontend/src/pages/extensions/components/{slack-setup-panel,slack-channel-picker,channels-tab,configure-modal}.tsx`, `lib/slack-{setup,channels}-api.ts`, `pages/chat/components/auth-oauth-card.tsx`, `lib/channel-connection-events.ts` |
| Concrete channel formatting in LLM prompt construction | `crates/ironclaw_llm/src/reasoning.rs` |
| Concrete channel variants in trace contributions | `crates/ironclaw_reborn_traces/src/contribution.rs` |
| Slack CLI command, cargo feature, config types | `crates/ironclaw_reborn_cli/src/commands/serve_slack.rs`, `slack-v2-host-beta` feature, `crates/ironclaw_reborn_config` |
| Telegram adapter exists but is test-only | `crates/ironclaw_telegram_extension` |

## 3. Target crate and module map

**New crates (3):**

| Crate | Owns |
| --- | --- |
| `ironclaw_extension_host` | `ExtensionEntrypoint`, `ExtensionBindings`, binding check, loaders (native/wasm/mcp), immutable active snapshot + resolver views, internal publication/removal/upgrade, generic ingress router module, restricted-egress implementation |
| `ironclaw_slack_extension` | All Slack protocol behavior: tool adapters (wrapping the existing WASM artifact initially), channel adapter (parse, render, deliver, targets, internal provisioning/cleanup), fixtures. Absorbs `ironclaw_slack_extension` and everything Slack in composition |
| `ironclaw_telegram_extension` | Telegram channel adapter (updates parsing, Bot API rendering, `setWebhook`/`deleteWebhook` hooks). Absorbs `ironclaw_telegram_extension` |

**Changed crates:**

| Crate | Change |
| --- | --- |
| `ironclaw_host_api` | Add `ToolAdapter`, `ToolCall`/`ToolResult`/`ToolPorts`, `RestrictedEgress` trait, auth recipe types, ingress verification recipe types. Base vocabulary — no implementations |
| `ironclaw_extensions` | Manifest v3 (inline `[channel]`, `[auth.*]`, `[mcp]`), v2 normalization, `ResolvedExtensionManifest` + persisted resolved record + `manifest_digest`, widening diff |
| `ironclaw_product_adapters` | `ChannelAdapter` (replaces `ProductAdapter`; metadata getters deleted), normalized inbound/outbound DTOs, exported conformance suite |
| `ironclaw_auth` | `AuthEngine` (oauth2_code + api_key), auth account state machine, recipe execution, flow/grant stores kept |
| `ironclaw_dispatcher` | Resolve prebound `ToolAdapter` via injected resolver; delete per-invocation package/runtime-kind selection |
| `ironclaw_product` | Generic delivery coordinator (all outbound intents); delete Slack cleanup literals |
| `ironclaw_reborn_composition` | Assembly only: construct stores/ports/host/engine, mount routers, inject resolvers. `src/slack/**` deleted by P6; consumes the first-party package inventory as opaque bundles — no catalog, no extension names (P7) |
| `ironclaw_first_party_extensions` | The package inventory: one module per package (`src/packages/<id>.rs`) owning that package's embeds, asset descriptors, digest, and bespoke copy, beside `assets/<id>/`; exports opaque bundles consumed by composition and the CLI |
| `ironclaw_webui_v2` | Generic surface/config/connect UI from wire data; Slack components deleted |
| `ironclaw` (`crates/ironclaw_reborn_cli`) | Assembles the native factory registry (the only generic-side crate allowed to link concrete extension crates); `serve_slack.rs` and Slack feature deleted |
| `ironclaw_llm` | `CommunicationPresentationPolicy` input replaces concrete channel formatting |
| `ironclaw_reborn_traces` | Generic extension/surface origin ids replace concrete variants |
| `ironclaw_architecture` | New specificity scanner + dependency gates (section 12) |
| `ironclaw_reborn_migration` | One-time migrations (section 11) |

**Deleted crates (2):** `ironclaw_slack_extension`, `ironclaw_telegram_extension` (folded into their extension crates).

**Dependency rule (gated):** generic crates never depend on concrete extension
crates. Only the canonical `ironclaw` CLI package (assembly of the native factory registry)
and tests may. Concrete crates depend on `ironclaw_host_api` /
`ironclaw_product_adapters` contract types only — never on composition, the
engine, or the router.

## 4. Workstream A — Manifest v3, recipes, resolved record

**Changes** (`crates/ironclaw_extensions`, `crates/ironclaw_host_api`):

- Add v3 schema: top-level `[[tools]]`, `[mcp]`, `[channel]`
  (ingress + verification + egress + presentation + `conversation_model`),
  `[auth.<vendor>]` recipes, and optional `[admin_configuration]`. The latter
  declares a reusable deployment-owned form keyed by `group_id`; it is not a
  channel sub-section or an installation-owned record. Exact shapes:
  `overview.md` §3.
- **Vendor rename:** rename `RuntimeCredentialAccountProviderId` → `VendorId`
  in `ironclaw_host_api` (temporary deprecation alias until callers migrate,
  deleted by P7). The v3 manifest field is `vendor`; v2's `provider` maps to it
  in normalization. Stored identifier strings (`google`, `slack`, …) are
  unchanged — no data migration. Related identifiers
  (`ProviderIdentityActorResolver`, …) pick up the rename whenever their files
  are touched.
- Recipe types live in `ironclaw_host_api` (both the manifest parser and the
  auth engine/ingress verifier consume them without a dependency cycle).
- v2 manifests keep parsing through the existing reader and normalize into the
  same resolved model (auth surfaces synthesized from tool credentials, as
  today). v3 requires explicit `[auth.*]` for every referenced vendor.
- Validation: JSON pointers RFC 6901, depth ≤ 8, no wildcards; all recipe
  endpoints `https`; `extra_authorize_params` may not contain reserved keys
  (`state`, `redirect_uri`, `code_challenge*`, `client_id`, `response_type`,
  the scope param); `[channel].route_suffix` one URL-safe segment;
  `[channel].conversation_model` required (`continuous` | `isolated`); egress
  hosts non-wildcard; **exactly one of `[runtime]` or `[mcp]`** declares the
  implementation; `[mcp]` requires `server` + `namespace` + `max_tools`, is
  mutually exclusive with `[[tools]]` and `[channel]`, and carries the
  server-connection credential (discovered tools cannot declare credentials or
  egress; the server host is the egress allowlist); unknown fields fail closed
  (`deny_unknown_fields`).
- Resolved record: persist `{ manifest_source, manifest_digest, resolved }` in
  the installation store; all production projection reads the record. Keep the
  raw source only for diagnostics and recompilation. Startup backfills legacy
  raw-TOML records by compiling once through the v2 reader (idempotent).
- Widening diff: compare old/new resolved contracts on upgrade — new scopes,
  egress hosts, effects, credential handles, or an ingress route change → the
  diff *classifies* the change. A widening consent gate is deliberately **not
  built** (overview §7): host-bundled contracts change only via reviewed binary
  releases, so boot-time adoption of the new record is the accepted path.
  `diff_resolved_contracts` ships as the data-model seed for a future
  registry/third-party-distribution trigger.

**Tests first:** `manifest_v3_contract.rs` in `ironclaw_extensions/tests` —
v3 Slack-shaped fixture resolves to the same surfaces as its v2 equivalent
(including `provider` → `vendor` mapping); recipe validation failures
(reserved param, bad pointer, http endpoint, wildcard egress); missing
`conversation_model` fails; `[mcp]` alongside `[runtime]`, `[[tools]]`, or
`[channel]` fails, as does `[mcp]` missing `server`/`namespace`/`max_tools`;
v2 normalization parity (v2 mcp-runtime manifests resolve to the same model
as `[mcp]`); record round-trip and
restart-without-source on both DBs; widening diff cases (equal / narrow /
widen). Extend `manifest_v2_contract.rs` rather than duplicating where a case
already exists.

## 5. Workstream B — Adapters, entrypoint, loaders, `ExtensionHost`

**Changes:**

- `ironclaw_host_api`: `ToolAdapter` (as in `overview.md` §4.1).
- `ironclaw_product_adapters`: `ChannelAdapter` (§4.2) with normalized DTOs
  (`NormalizedInboundMessage`, `InboundOutcome`, `OutboundEnvelope`,
  `DeliveryReport`, `TargetQuery`/`TargetCandidate`). Delete the metadata
  getters from the old `ProductAdapter`; a thin compatibility wrapper may keep
  current callers compiling until P4/P5 cut them over, then it is deleted.
- New `ironclaw_extension_host`:
  - `entrypoint.rs` — `ExtensionEntrypoint`, `ExtensionBindings`, `BindContext`
    (installation identity + `Arc<ResolvedExtensionManifest>`; no authority
    ports). Binding check: declared↔bound exactly (overview §4.0).
  - `loaders/{native,wasm,mcp}.rs` — native resolves `runtime.service` in the
    injected factory registry; wasm wraps the existing WASM execution lane
    (`ironclaw_host_runtime`) as a `WasmToolAdapter`; the **mcp loader owns
    discovery**: selected by the presence of `[mcp]`, it connects to the
    declared server, runs `tools/list` through the existing hosted-MCP client
    with the connection credential injected, validates results against the
    declared ceiling (namespace/count/schema-size/effects), publishes the
    discovered tool surfaces atomically, and synthesizes a `ToolAdapter` that
    proxies invocations. `ToolAdapter` itself has one method (`invoke`);
    discovery is never part of the extension ABI.
  - `active.rs` — immutable `ActiveSnapshot` (`Arc` swap), resolver views.
    Resolver ports are defined in consumer crates and implemented here:
    `ToolResolver` (dispatcher), `ChannelResolver` (workflow/router),
    `AuthRecipeResolver` (auth engine — returns data, not adapters). Duplicate
    capability id or ingress route across published extensions → publication
    conflict (no scope-based disambiguation in this version).
  - `lifecycle.rs` — the membership/readiness pipeline exactly as
    `overview.md` §6.1–6.2. Product state is only `uninstalled |
    setup_needed | active`: absence/presence of caller membership plus derived
    tenant-admin and caller-auth readiness. Internal host checkpoints and
    publish-or-nothing failures are not public states or actions. The host owns
    the fixed removal order with typed-quarantine retry and the active snapshot;
    a single async mutex serializes internal publication operations (single
    serving process assumption).
    `BindContext`/`ChannelContext` carry runtime identity, the resolved
    declaration, and non-secret tenant configuration values (secrets only
    behind injection); adapter hooks run under bounded deadlines. Saving one
    `[admin_configuration]` group refreshes every tenant runtime consumer of
    that group; it never creates per-installation configuration.
  - `egress.rs` — `RestrictedEgress` implementation: scheme/host/method
    allowlist from the resolved contract, credential injection by handle
    (adapter-supplied `Authorization` rejected where injection is declared),
    response size caps, redirect denial across hosts, private-IP/DNS-rebind
    denial (reuse existing network policy), deadlines.
- Composition/CLI: CLI assembles `Vec<NativeExtensionFactory>` (Slack,
  Telegram) and passes it into composition; composition constructs
  `ExtensionHost` with stores + loaders and injects resolver handles into
  dispatcher/workflow/engine/router. The existing
  `extension_host/extension_lifecycle.rs` facade delegates to the new host and
  shrinks to wiring; `slack_host_beta.rs` manual graph construction is deleted
  in P6 after its callers cut over.

**Tests first:** `binding_contract.rs` (missing/extra/undeclared binding,
declared-but-`None`, auth-never-binds); `lifecycle_contract.rs` (caller
membership isolation, derived three-state projection, internal publication
failure remains `setup_needed` with a typed redacted error, upgrade drains old
`Arc`); the facade-owned removal
order (unpublish → drain → vendor cleanup → auth cleanup → config/identity
delete) with typed-quarantine retry on failure, observed via scripted
adapter + engine at the composition tier; loader tests (unknown
service, wasm lane invoke, mcp discovery ceiling violations); snapshot tests
(no mixed generation under concurrent publish/resolve; readers never observe
partial state). Integration tier: `tests/integration/extension_runtime.rs` —
acme fixture admin-configure → install → setup/connect → active → resolve →
remove on both DBs, with a second user proving membership isolation.

## 6. Workstream C — Tool dispatch cutover

**Changes** (`crates/ironclaw_dispatcher`, `crates/ironclaw_host_runtime`):

- Dispatcher resolves `ToolResolver::resolve(capability_id)` → prebound adapter
  + resolved declaration. Authorization, approvals, obligations, resource
  reservation, events, and audit behavior are unchanged and proven unchanged.
- Host built-in capabilities stay in the host's built-in registry and resolve
  through the same lookup, running the identical pipeline; an extension
  capability id colliding with a built-in is a publication conflict (tested).
- Existing WASM/MCP/script execution code becomes adapter implementations or
  helpers behind the loaders; credential injection and host-port enforcement
  stay host-side (ports built from the resolved declaration per call).
- Delete the per-invocation registry/package/runtime-kind lookup and any
  now-unused parallel registry (`ProductAdapterRuntimeEntry` raw projection in
  `ironclaw_product_adapter_registry` goes here or in P4, whichever cuts its
  last caller).
- Missing-credential path raises the generic auth gate keyed by the tool's
  declared vendor; resume after the engine completes.

**Tests first:** dispatcher contract tests updated to drive resolution through
a scripted resolver (unknown capability fails before adapter work; policy
pipeline still runs; adapter cannot reach undeclared egress/credential).
Integration: reconcile the real Slack package to readiness and invoke all five capability
ids through the production dispatcher with recorded egress — asserts no Slack
branch anywhere in dispatch (`tests/integration/extension_runtime.rs`).

## 7. Workstream D — Auth engine cutover

**Changes** (`crates/ironclaw_auth`, composition `product_auth/**`):

- `AuthEngine` implements two methods over recipes: `oauth2_code`, `api_key`.
  Responsibilities split exactly as `overview.md` §4.3. Flow state, PKCE,
  replay, TTL, encryption, and grant/account storage reuse the existing
  durable product-auth stores (rows gain the standard state column if absent).
- **Auth account state machine** (`overview.md` §6.3) is defined here and is
  the only connection-state representation: `disconnected | authenticating |
  connected | expired` + typed `last_error`. Disconnect and removal delete the
  account synchronously, so there is no transient `revoking` wire state.
  Exactly one transition consumes a callback; TTL expiry → `disconnected` with
  reason.
- Routes: keep the mounted paths (`.../product-auth/start`, `status`,
  `revoke`, and `/api/reborn/product-auth/oauth/{provider}/callback`) so
  vendor-registered redirect URLs keep working; the `{provider}` path
  parameter carries the vendor id and is resolved via `AuthRecipeResolver` —
  never a match arm.
- Refresh is on-demand at credential-injection time with single-flight per
  account. A recipe may additionally declare an idle keepalive threshold
  (`refresh.keepalive_idle_seconds` — a vendor lifetime constraint for vendors
  that expire refresh tokens after a fixed idle window); the engine executes
  it once as a generic, vendor-blind background sweep (leader-locked per
  deployment tick, due at half the declared lifetime, soonest-death-first
  under the per-tick cap). There is no per-vendor refresher code.
- Shared vendors: unify recipes during internal publication (identical except
  `scopes`/`display_name`, else conflict); scope union and incremental
  re-consent keep today's behavior; grants are vendor-scoped and survive
  removal of one consumer while another active extension shares the vendor.
- Recipe reference (fields beyond `overview.md` §3): `scope_param` (default
  `scope`), `scope_join` (default space), `exchange_auth = "post_body"|"basic"`,
  `token_response.{access_token,refresh_token,expires_in,scope}` pointers with
  `missing = "fallback_to_requested"` option, `identity` from token response or
  follow-up `endpoint`, `refresh.rotates_refresh_token`, `revoke.{endpoint,
  token_param}`, `client_credentials.{client_id_handle,client_secret_handle}`
  (deployment-level secrets via the existing secret store). Vendor response
  bodies are size-capped and redacted from errors/logs.
- Write the recipes: Slack and Google (`oauth2_code`), Notion (`oauth2_code`;
  if the hosted-MCP path requires dynamic client registration, implement
  `oauth2_dcr` once in the engine per RFC 7591 — it is generic MCP behavior,
  not Notion behavior), GitHub and NEAR AI (`api_key` with probe).
- **Delete:** `MultiplexAuthProviderClient`/`compose_provider_clients` string
  map, `google_provider_spec`/`slack_personal_provider_spec` code constants,
  Slack branches in `serve/oauth.rs`, `slack_personal_oauth.rs`,
  `TokenResponseShape` concrete variants. The blocked-turn gate driver
  (`OAuthGateFlowDriver`) survives — it is already generic — re-pointed at the
  engine.

**Tests first:** one `auth_engine_contract.rs` suite against a scripted
vendor HTTP server, table-driven over recipes: authorize-URL construction
(reserved params host-built; recipe cannot override), state/PKCE/replay/TTL,
exchange with `post_body` and `basic`, pointer extraction incl.
`fallback_to_requested`, refresh rotation both flags, revoke idempotency,
identity via response and via endpoint, api_key probe pass/fail, state-machine
transitions incl. exactly-once callback consumption, cross-flow callback
rejection, both DBs. Integration: real Slack package — blocked tool → gate →
scripted callback → grant stored → tool resumes (extends the existing
oauth-connect integration test rather than adding a parallel one).

## 8. Workstream E — Channel ingress cutover

**Changes** (`ironclaw_extension_host::ingress`, composition serve wiring):

- Generic router mounted once by composition into the existing webhook server:
  routes `/webhooks/extensions/{extension_id}/{route_suffix}` from the active
  snapshot's channel descriptors (publication rejects collisions with fixed
  host routes and other extensions; route table updates on snapshot swap, no
  Axum rebuild).
- Order per request: match → method/body-limit/rate/deadline enforcement →
  verification recipe execution (host verifier: `hmac_sha256` segment
  evaluation, constant-time compare, timestamp/replay window;
  `shared_secret_header` constant-time; candidate installations tried within a
  small fixed bound) → `adapter.inbound(VerifiedInbound)` (pure, panic-isolated,
  bounded input; signing secrets never in scope) → outcome:
  - `Messages` → durable dedupe + admission commit in one transaction (dedupe
    key: `(installation, event_id)`), **then** 2xx; persistence failure →
    retryable 5xx. Then existing workflow: identity and conversation binding,
    turn submission.
  - `Respond` → bounded immediate response (post-verification), no enqueue.
  - `Ignore` → 2xx after the same durable no-op commit.
- `reply_context` from the message is stored host-side with the conversation
  source binding and handed back in `OutboundEnvelope`.
- Inbound attachments are `AttachmentRef`s (vendor URL/id + mime hint) so
  `inbound` stays pure; the host-side fetch through restricted channel egress
  is specified but implemented only when a consumer needs bytes.
- Conversation binding consumes the channel's declared `conversation_model`:
  `continuous` channels bind one IronClaw conversation per external
  conversation ref (today's Slack/Telegram behavior, now declared instead of
  assumed); the host WebUI's internal channel shares the same enum
  (`isolated`), so the workflow carries no per-channel conversation logic.
- **Slack:** `SlackChannelAdapter::inbound` absorbs envelope parsing, URL
  verification challenge, ignored-event rules, and normalization from
  `slack_serve.rs` / `slack_serve/installation.rs` / the v2 adapter. Signature
  execution moves to the generic verifier via the manifest recipe. Delete the
  Slack route mount and installation resolver from composition.
- **Telegram:** `TelegramChannelAdapter::inbound` parses updates; manifest
  declares `shared_secret_header` verification; its internal `activate()` hook calls
  `setWebhook` (with the secret token), `cleanup()` calls `deleteWebhook`.
  Activate the real package through the same router — the second production
  proof.

**Tests first:** router contract tests with the acme fixture driving the real
mounted route: match/method/body/rate/deadline before adapter work; bad/stale/
replayed/missing signatures; constant-time verifier unit tests with exact
byte-recipe fixtures; ack-only-after-durable-commit (crash before/after
enqueue, concurrent duplicate, restart replay — both DBs); challenge response;
panic isolation. Slack/Telegram protocol parsing is unit-tested in their
crates with recorded payload fixtures; each gets one integration proof:
signed vendor POST → verified → normalized → turn admitted.

## 9. Workstream F — Outbound delivery cutover

**Changes** (`crates/ironclaw_product`, extension crates):

- One `DeliveryCoordinator` (evolve `outbound_delivery.rs` and the generic
  halves of `slack_delivery.rs`): intents `FinalReply | Progress | GatePrompt |
  AuthPrompt | FailureNotice | ConnectRequired | Working | Cleanup |
  TriggeredDelivery` — every current Slack observer/notice call site maps to
  exactly one intent, none bypasses.
- Coordinator: resolve target (source-route reply default, preference targets,
  fail-closed on unauthorized/unavailable), persist attempt
  (`Prepared`→`Sending`→terminal) **before** vendor egress, call
  `channel.deliver(envelope, egress)`, persist the structured report, own
  retry/backoff/dedupe/single-flight/shutdown-drain. Crash after possible
  vendor success → `Unknown`, never blind resend. Sole delivery-state writer —
  adapters get no store. Production construction rejects a no-op sink.
- `CommunicationPresentationPolicy` derived from `[channel.presentation]`
  flows into prompt construction; delete the concrete
  Discord/WhatsApp/Telegram/Slack branches in
  `crates/ironclaw_llm/src/reasoning.rs`. Replace concrete trace contribution
  variants in `crates/ironclaw_reborn_traces/src/contribution.rs` with
  extension/surface origin ids.
- **Slack:** `SlackChannelAdapter::deliver` absorbs Block Kit/plain rendering,
  splitting, `chat.postMessage`/update/delete, DM provisioning
  (`conversations.open`, from `slack_dm_open.rs`), target formats
  (`slack_outbound_targets.rs`), vendor error mapping; `list_targets` absorbs
  channel listing. `slack_egress.rs` request construction moves into the
  adapter over generic `RestrictedEgress`.
- **Telegram:** `deliver` renders Bot API messages; same coordinator, no
  branches.

**Tests first:** coordinator contract tests with a scripted channel adapter
(every intent enters; attempt persisted before egress observed; retry/backoff/
dedupe; partial multipart rule — once any part sends, a later retryable part
failure is terminal unless the adapter supplied a vendor idempotency key;
crash→`Unknown`; drain; both DBs). Egress security tests: undeclared host,
adapter-supplied auth header, redirect/private-IP escape, oversized response —
rejected before network. Slack/Telegram rendering is fixture-unit-tested in
their crates; one outbound integration proof each through the real coordinator.

## 10. Workstream G — Extraction completion: Slack, Telegram, frontend, CLI

**Slack disposition (everything under `composition/src/slack/` ends deleted):**

| Current file(s) | Disposition |
| --- | --- |
| `slack_host_beta.rs`, `slack_host_beta/runtime_setup.rs` | Delete — generic internal publication + factory registry replace manual construction |
| `slack_serve.rs`, `slack_serve/installation.rs` | P8/E: parsing → Slack crate; verification/routing → generic router; delete |
| `slack_delivery.rs` | P9/F: scheduling/persistence → coordinator; rendering/sending → Slack crate; delete |
| `slack_egress.rs` | Generic `RestrictedEgress`; request building → Slack crate; delete |
| `slack_outbound_targets.rs`, `slack_dm_open.rs` | Slack crate (`deliver`/`list_targets`); delete |
| `slack_channel_connection.rs`, `slack_setup.rs`, `slack_channel_routes*` | manifest `[admin_configuration]` + generic admin/connect endpoints + internal provisioning validation; allowed-channel lists become tenant configuration; delete |
| `slack_personal_oauth.rs`, `slack_personal_binding*.rs` | Recipe + engine (D); generic parts of binding flow stay generic; delete |
| `slack_host_state.rs` | Generic scoped state (tenant/extension/surface-keyed); one-time key migration (H); delete |
| `mod.rs` | Delete last |

Also delete/replace: Slack cleanup constants in
`product_workflow/reborn_services/extensions.rs` (standard removal pipeline
covers it); Slack connection copy in `extension_lifecycle.rs` (manifest
display data); manual Slack scopes/onboarding in `available_extensions.rs`
(manifest-driven); Slack trust/effect special-casing in `factory.rs`; Slack
config types in `ironclaw_reborn_config`; `serve_slack.rs` and the
`slack-v2-host-beta` cargo feature.

**Frontend** (`crates/ironclaw_webui/frontend`):

- Wire additions (backend `reborn_services/types.rs`): full surface key per
  surface; three-state public lifecycle projection (§6.1); auth account state enum (§6.3),
  exposed as a per-vendor **accounts list** (`account_id`, `label`, state,
  `is_default`) plus each surface's `resolved_account_id` — length ≤ 1 until
  the post-P7 multi-account follow-up
  (`adr/0001-multiple-accounts-per-vendor.md`), list-first so the golden
  fixture never breaks; tenant `[admin_configuration]` group descriptors;
  presentation/display data. Freeze one golden wire fixture with an arbitrary
  channel + a multi-surface extension.
- Add generic components: `surface-card`, `connect-card` (renders caller auth
  state), optional `target-picker`, and a separate admin configuration view
  that renders each manifest group schema, masks secrets, and never echoes
  stored values. The channels tab keys by surface and renders the same user
  components for every extension; it cannot mutate deployment configuration.
  Affordances derive from membership/readiness plus caller auth (§6.4).
- Delete: `slack-setup-panel.tsx`, `slack-channel-picker.tsx`,
  `slack-setup-api.ts`, `slack-channels-api.ts` (+ their tests), and Slack
  branches in `channels-tab.tsx`, `configure-modal.tsx`,
  `auth-oauth-card.tsx`, `channel-connection-events.ts`, automation delivery
  copy. Frontend tests use the acme fixture; a source-scan test asserts no
  concrete package-id condition remains.
- Browser coverage reuses the existing Python harness
  (`tests/e2e/reborn_webui_harness.py` + `fake_slack_api.py`): one scenario —
  configure, connect, message in, reply out, remove. No new harness.

**CLI:** `extension` command drives install/remove membership through
`ExtensionHost` (same pipeline as the UI); readiness reconciliation is
automatic and `serve` no longer knows channels.

## 11. Workstream H — One-time migration and compatibility

Small, versioned, idempotent, in `ironclaw_reborn_migration`, tested on both
DBs with old-wire fixtures. Reuse storage instead of migrating wherever
possible.

1. **OAuth grants/accounts:** storage reused as-is (vendor id strings unchanged) —
   no data migration; rows gain the standard state column with a backfill
   mapping (`connected` for live grants).
2. **Legacy raw-TOML installed records:** compiled once through the v2 reader
   at startup → resolved record persisted (idempotent; A).
3. **Slack setup slots** (bot token, signing secret, app client credentials) →
   tenant-scoped `[admin_configuration]` handles, including OAuth client
   credentials consumed by the auth recipe.
4. **Slack state roots** (`slack_host_state.rs`: identities, allowed channels,
   subject routes, DM targets, outbound preferences) → generic scoped state
   keys / tenant configuration. After migration no `slack`-named state root is
   read outside migration code.
5. **Installation lifecycle records** → caller membership backfill; product
   state is thereafter derived as `uninstalled | setup_needed | active`.
6. **Webhook URL:** `/webhooks/slack/events` forwarded to
   `/webhooks/extensions/slack/events` for the cutover release and is now
   deleted. OAuth callback paths remain on the generic product-auth route;
   operators must configure the canonical Events URL.
7. **First-party manifests:** rewrite all 11 packages v2 → v3 in one PR, with
   a projection-equality test (derived surfaces, capability ids, scopes,
   credentials identical before/after) — plus the new explicit `[auth.*]` and
   `[channel]` sections. Exception: the two hosted-MCP packages
   (`notion-mcp`, `nearai-mcp`) intentionally change shape — their placeholder
   static capabilities become one `[mcp]` declaration; their parity assertion
   is the declared ceiling plus the post-discovery tool set instead of static
   equality. The v2 reader remains for third-party/local packages.

Each migration: dry-run flag, idempotent second run, malformed-record skip
with log. Nothing here needs telemetry windows — the installed base is beta;
the alias and the v2 reader are the only compatibility surfaces, and each
carries a removal note in `checklist.md`.

## 12. Workstream I — Testing and architecture gates

- **Conformance suites** (the payoff Ben asked for):
  - `ironclaw_product_adapters::conformance` — a public test-support module:
    given any `ChannelAdapter` + a scripted vendor server + fixture payloads,
    asserts the contract (inbound outcomes well-formed and bounded; deliver
    honors the envelope and reports per-part outcomes; internal
    provisioning/cleanup hooks are idempotent; unsupported methods error
    cleanly). Slack, Telegram, and acme
    all run it.
  - Tool adapter conformance in `ironclaw_host_api` test-support (invoke
    respects deadline/ports; dynamic discovery respects ceilings).
  - The auth engine suite (D) is the auth conformance — vendors are rows.
- **Fixture extension** `tests/fixtures/extensions/acme-messenger/` (invented
  vendor): manifest with 1 tool + channel (hmac recipe) + oauth recipe + config
  fields; a tiny native factory registered only in tests. Drives every generic
  integration path end-to-end (install → configure → connect → inbound → turn
  → outbound → remove) — proof that no generic path needs a real product.
  (The upgrade-with-widening approval leg is removed — see overview §7; the
  contract diff ships as data-model code without a consent gate.)
- **Architecture gates** (`crates/ironclaw_architecture/tests/`):
  - keep `reborn_retired_taxonomy.rs`;
  - add `reborn_extension_specificity.rs`: scans generic `crates/**/src`,
    WebUI TS, and production TOML for concrete extension ids, vendor ids,
    and vendor API hosts **derived from the bundled package inventory** (a future
    `discord` package is caught without editing the scanner). Path-scoped
    allowlist with enforced shrinkage: an entry whose file no longer matches
    fails; a new violating path fails; the list is empty at P7.
  - add dependency gate: no generic crate depends on a concrete extension
    crate (`cargo metadata`); only the canonical `ironclaw` CLI package and tests may.
  - `scripts/ci/check-generic-without-concrete.sh`: asserts via `cargo tree`
    that each generic crate's graph contains no concrete extension crate, then
    `cargo test -p` each generic crate — the deletion test, automated in CI.
- Repo law throughout: test-first; integration tier for production-wired
  behavior (through the harness, asserting at a seam); both DBs for
  persistence; no `wait_for_status(Completed)`-only assertions; extend
  existing tests before adding parallel ones.

## 13. Execution order

Phases are PRs into main (after the taxonomy stack merges). Each lands green: `cargo fmt`,
`cargo clippy --all --benches --tests --examples --all-features` (zero
warnings), `cargo test` (+ integration features where touched),
`cargo test -p ironclaw_architecture`, frontend `vitest` when touched.

| Phase | Content | Depends on |
| --- | --- | --- |
| **P0** | Architecture gates (scanner + dependency rule, allowlist enumerating today's violations) + acme fixture assets | — |
| **P1** | Workstream A: manifest v3 + recipes + resolved record + v2 normalization + first-party manifest rewrite (H.7) | P0 |
| **P2** | Workstream B + C: adapters, entrypoint, loaders, `ExtensionHost`, state machine, tool dispatch cutover (Slack tools prove it) | P1 |
| **P3** | Workstream D: auth engine + recipes + state machine; delete provider multiplexing (grants storage reused) | P2 (parallel with P4) |
| **P4** | Workstream E: generic router + verifier; Slack + Telegram inbound through adapters | P2 (parallel with P3) |
| **P5** | Workstream F: delivery coordinator; Slack + Telegram outbound; presentation policy; traces | P4 |
| **P6** | Workstream G: config/connect UI, frontend replacement, CLI/config cleanup, delete `composition/src/slack/**` + old adapter crates; H.3–H.6 migrations land here with the cutovers they enable | P3, P5 |
| **P7** | Allowlist → zero; `check-generic-without-concrete.sh` in CI; docs (`docs/reborn/contracts/*`, `reborn-extension-surfaces` skill, `FEATURE_PARITY.md`, `CHANGELOG.md`); checklist fully evidenced | P6 |

Slack code keeps working at every phase boundary: a cutover phase moves the
production caller and deletes the old path in the same PR (or lists it as one
of the two sanctioned compatibility surfaces in section 11).

## 14. Out of scope

See `overview.md` §7 for the full exclusion table (fragments, package
blob store/leases, signing, fencing, shared vendor packages, per-vendor auth
adapters, trigger/file runtime, evidence tooling, second e2e harness) and the
named triggers for revisiting each. Reintroducing any of them requires a new
ADR that cites its trigger.

One exclusion's trigger has already fired: **multiple accounts per vendor**
(accepted 2026-07-13,
`docs/reborn/extension-runtime/adr/0001-multiple-accounts-per-vendor.md`).
It is implemented as a dedicated PR after P7 — within P0–P7 only the
list-shaped wire exposure lands (Workstream G; checklist UI-1 / AUTH-9
evidence must name the list shape).
