# Unified Extension Runtime — Overview

**Status:** Approved design.
**Companions:** `implementation.md` (what changes, where), `checklist.md` (acceptance).
**Baseline:** the unified extension taxonomy this branch already contains (extension as the only installable product object).

This document is the complete mental model: the product shape, the manifest, the
adapter seams, and how the generic core uses them. It deliberately contains no
machinery beyond what the goal requires; section 7 lists what was considered and
excluded, so it does not creep back in.

## 1. Goal

Every integration (Slack, Gmail, GitHub, Telegram, …) is an ordinary installable
extension package. The generic runtime manages caller membership, derives
readiness, dispatches, and removes extensions using only their manifest and two
narrow adapter seams (plus one recipe-driven host engine for auth). There is no
separate user-facing activation operation. No generic crate contains a concrete
product name, protocol type, route, or behavior branch.

Acceptance is three concrete tests:

1. **Deletion test.** Remove the Slack package and `ironclaw_slack_extension`
   from the build: every generic crate still compiles and its tests pass.
2. **Addition test.** Add a new channel extension (e.g. Discord): no generic
   source file changes — a new package and a new extension crate only.
3. **Testing test.** Each capability has *one* conformance/behavior suite that
   every implementation passes, plus protocol unit tests inside each extension
   crate. There is no per-vendor copy of the OAuth, ingress, or delivery test
   suites (no "Gmail OAuth tests" *and* "Slack OAuth tests" — one auth-engine
   suite, table-driven over recipes).

## 2. Product model

One rename aside, this is the taxonomy the codebase already establishes,
restated so this document stands alone.

- **Extension** — the only installable product object. One `ExtensionId`
  (`slack`, `github`, `gmail`) owns every surface in the package.
- **Capability surfaces** — an extension declares up to three kinds:
  **tool** (model-callable capability), **channel** (message inbound/outbound),
  **auth** (credential acquisition from a vendor). `trigger` and `file` remain
  reserved enum variants with no runtime behavior.
- **`VendorId`** — names the external service that issues credentials and
  accounts (`google`, `slack`, `github`). Several extensions may share one
  (`google` across gmail/drive/calendar/docs/sheets/slides). It is never a
  product identity. *(Renamed from `ProviderId`/`RuntimeCredentialAccountProviderId`:
  "provider" is already taken three ways in this codebase — `LlmProvider`,
  `EmbeddingProvider`, the `capability_provider` host API — while "vendor"
  matches the existing vendor-API/vendor-payload vocabulary. Stored id strings
  are unchanged.)*
- **Runtime kind** (`first_party` native, `wasm`, `mcp`) — how implementations
  load. Never taxonomy: it cannot affect what surfaces exist or how they render.
- **Simplification, this version:** at most **one channel surface per
  extension**. Every real and planned extension has exactly zero or one. The
  wire format keeps full surface keys, so lifting this later is additive.

The retired vocabulary (`slack_bot`, `slack_personal`, channel-as-product,
extension `kind` strings) stays pinned at zero by
`crates/ironclaw_architecture/tests/reborn_retired_taxonomy.rs`.

## 3. The manifest

One `manifest.toml` per package. No fragments, no imports, no includes — the
largest real manifest today is under 700 lines, and a single reviewable file is
a feature. Schema `reborn.extension_manifest.v3` is v2 plus explicit `[channel]`
and `[auth.*]` sections; the v2 reader continues to parse old manifests and
normalizes them into the same resolved model.

**The package is the unit of self-containment.** Everything an extension
needs ships inside its package directory — the manifest, input schemas,
prompt docs, WASM modules, and any bespoke display or onboarding copy that
cannot be derived from the manifest. Host code consumes a package as one
opaque, cleanly built bundle (id, display name, manifest source, assets);
nothing outside the package enumerates or re-describes its contents, and
generic crates never name one. There is no hand-maintained catalog: the
bundled inventory (`ironclaw_first_party_extensions`) holds exactly one
small module per package (`src/packages/<id>.rs`) beside its
`assets/<id>/` directory, and a collector concatenates the per-module
bundles. Adding an integration is a new assets directory plus its module;
removing one deletes both; no other file changes anywhere.

The unified Slack manifest, complete except three tools elided:

```toml
schema_version = "reborn.extension_manifest.v3"
id = "slack"
name = "Slack"
version = "0.2.0"
description = "Slack tools, account authorization, and messaging channel"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "slack.extension/v1"   # looked up in the native factory registry; data, not code

[admin_configuration]            # tenant/deployment setup, independent of user installs
group_id = "extension.slack"
display_name = "Slack deployment configuration"
description = "Deployment credentials and routing configuration for Slack."
fields = [
  { handle = "slack_bot_token", label = "Bot token", secret = true, required = true },
  { handle = "slack_signing_secret", label = "Signing secret", secret = true, required = true },
  { handle = "slack_oauth_client_id", label = "OAuth client ID", secret = false, required = true },
  { handle = "slack_oauth_client_secret", label = "OAuth client secret", secret = true, required = true },
]

# ---- tool surfaces ---------------------------------------------------------

[[tools]]
id = "slack.search_messages"
description = "Search all Slack messages visible to the connected user."
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/slack/search_messages.input.v1.json"
prompt_doc_ref = "prompts/slack/search_messages.md"

[[tools.credentials]]
handle = "slack_user_token"
vendor = "slack"
scopes = ["search:read"]
audience = { scheme = "https", host = "slack.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

# … slack.list_conversations, slack.get_conversation_history,
#   slack.get_user_info declared the same way …

[[tools]]
id = "slack.send_message"
description = "Send a Slack message as the connected user. Side effect inside a job; never used to deliver the final answer."
effects = ["network", "use_secret", "external_write"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/slack/send_message.input.v1.json"
prompt_doc_ref = "prompts/slack/send_message.md"

[[tools.credentials]]
handle = "slack_user_token"
vendor = "slack"
scopes = ["chat:write"]
audience = { scheme = "https", host = "slack.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

# ---- channel surface (at most one per extension) ---------------------------

[channel]
id = "messages"
display_name = "Slack messages"
inbound = true
outbound = true
conversation_model = "continuous"   # see notes below

[channel.ingress]
route_suffix = "events"           # served at /webhooks/extensions/slack/events
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "hmac_sha256"
secret_handle = "slack_signing_secret"
signature_header = "X-Slack-Signature"
signature_prefix = "v0="
signature_encoding = "hex"
timestamp_header = "X-Slack-Request-Timestamp"
max_age_seconds = 300
signed_payload = [
  { literal = "v0:" },
  { header = "X-Slack-Request-Timestamp" },
  { literal = ":" },
  { body = true },
]

[[channel.egress]]
scheme = "https"
host = "slack.com"
methods = ["post"]
credential_handle = "slack_bot_token"

[channel.presentation]
supports_markdown = true
supports_threads = true
max_message_chars = 40000

# ---- auth surface: recipe data, zero extension code ------------------------

[auth.slack]                      # section key = the vendor id
method = "oauth2_code"
display_name = "Slack account"
authorization_endpoint = "https://slack.com/oauth/v2/authorize"
token_endpoint = "https://slack.com/api/oauth.v2.access"
scope_param = "user_scope"        # Slack reserves `scope=` for bot tokens
pkce = "s256"
scopes = ["search:read", "channels:history", "channels:read", "users:read", "chat:write"]
client_credentials = { client_id_handle = "slack_oauth_client_id", client_secret_handle = "slack_oauth_client_secret" }

[auth.slack.token_response]
access_token = "/authed_user/access_token"
scope = { path = "/authed_user/scope", missing = "fallback_to_requested" }

[auth.slack.identity]
account_id = "/authed_user/id"
team_id = "/team/id"
```

Notes on the sections:

- **Tools** are declared one per `[[tools]]` entry; the manifest is the tool
  list the model sees. Schemas and prompt docs are package assets. Credential
  entries reuse the existing v2 injection model (`audience` + `injection`;
  v2's `provider` field is `vendor` in v3); the host injects secrets during
  restricted egress, adapters never see bytes.
- **MCP extensions** declare one `[mcp]` section (the proxied server) instead
  of `[runtime]` + `[[tools]]` — see §3.1.
- **`[channel.ingress.verification]`** is a declarative recipe the *host*
  executes: `hmac_sha256` (segment list: literals, named headers, body),
  `shared_secret_header` (constant-time compare), or `none`. Signing secrets
  never reach the adapter. Two recipe kinds cover Slack and Telegram; new kinds
  are added to the host when a protocol genuinely needs one.
- **`conversation_model`** (required on `[channel]`) classifies how external
  conversations map to IronClaw conversations:
  - `continuous` — the protocol supplies conversation identity; each external
    conversation (a Slack DM/channel, a Telegram chat) is one ongoing IronClaw
    conversation, bound per external conversation ref. Slack and Telegram are
    continuous.
  - `isolated` — the client explicitly creates and switches isolated
    conversations (the host WebUI's model).
  Conversation binding and presentation policy consume the declared value, and
  the host's own WebUI channel uses the same enum internally — the workflow
  reasons about every channel one way, with no per-channel special cases.
- **`[admin_configuration]`** declares deployment-owned setup independently of
  channel or tool surfaces. The admin UI groups equal `group_id` descriptors,
  renders the form generically, and stores values once per tenant; it does not
  install the extension for the operator or any other user. Secret values are
  write-only and only their presence is projected. Extensions that share a
  deployment credential set (for example the Google family) use the same group
  id and exactly matching descriptors.
- **`[auth.*]`** is one recipe per vendor the extension needs. See section 4.3.

### 3.1 MCP extensions

Hosted MCP servers (`notion-mcp`, `nearai-mcp`) are the one case where the tool
list is not known at authoring time: the server's `tools/list` is the source of
truth. An MCP extension *is* a proxied server, so the manifest says exactly
that — one `[mcp]` section instead of `[runtime]` + `[[tools]]`, and no
generic "dynamic tools" abstraction:

```toml
schema_version = "reborn.extension_manifest.v3"
id = "notion"
name = "Notion"
version = "0.2.0"
description = "Notion workspace tools via the hosted Notion MCP server"
trust = "first_party_requested"

[mcp]
server = "https://mcp.notion.com/mcp"
namespace = "notion"              # discovered tools publish as `notion.<tool>`
max_tools = 256
default_permission = "ask"
effects = ["network", "use_secret"]   # ceiling for every discovered tool

[[mcp.credentials]]               # the server connection's credential —
handle = "notion_account"         # discovered tools cannot declare their own
vendor = "notion"
scopes = ["read_content"]
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[auth.notion]
method = "oauth2_code"
# … recipe …
```

Rules that keep the boundary easy to reason about:

- An extension declares its implementation as **exactly one of** `[runtime]`
  (a native service or WASM module implementing declared `[[tools]]` and/or
  `[channel]`) **or** `[mcp]` (a proxied server whose tools are discovered).
  The presence of `[mcp]` selects the MCP loader; internally it resolves to
  the same runtime-kind descriptor, so "runtime is a loading detail, never
  taxonomy" holds unchanged.
- Discovery is **host-owned**: the MCP loader calls `tools/list` while
  reconciling an eligible member to readiness (and on explicit refresh),
  validates every result against the declared
  ceiling — namespace, count, bounded schema size, effects — and publishes the
  set atomically as ordinary tool surfaces. A refresh replaces the whole set
  or changes nothing.
- Discovered tools cannot carry credentials or egress of their own: the
  `[mcp]` connection credential is injected on every server call, and egress
  is restricted to the server's host. Nothing a server returns can widen
  authority.
- **Past readiness reconciliation there is no "MCP" anywhere in the dispatch
  path.** A
  discovered tool is an ordinary tool surface: same dispatcher pipeline, same
  policy and approvals, same UI. The distinction exists only in the manifest
  and the MCP loader.

### 3.2 Shared vendors

Extensions that use the same `VendorId` each carry the recipe (gmail, drive,
calendar all embed the `[auth.google]` recipe). During internal publication the
host unifies them: recipes for one vendor must be **identical except `scopes`
and `display_name`**, or publication fails with a conflict. Scope ceilings union
across extensions available to the caller exactly as the system does today; a
new extension needing more scopes triggers incremental re-consent. Accounts and
grants are stored per user and vendor and shared — connecting Google once serves
that user's gmail and drive memberships, never another user's.

This replaces any notion of installable/shared "vendor implementation
packages": ~20 duplicated TOML lines per Google extension, one 5-line conflict
check, zero new mechanism.

### 3.3 The resolved contract

The manifest is compiled **once** per install/upgrade into a typed
`ResolvedExtensionManifest` (surfaces, tools, channel descriptor, auth recipes,
egress, credentials) plus a `manifest_digest` of the source bytes. That record
is persisted; discovery, lifecycle, dispatch, ingress, auth, and the frontend
consume the record. Production code never reparses raw TOML. On upgrade, the
resolved-contract diff classifies the change — equal / narrowed / **widening**
(new scopes, egress hosts, effects, routes, credential handles). A widening
*consent gate* is deliberately **not built** in this train (§7): packages are
host-bundled, so a contract changes only via a reviewed binary release, and
boot-time adoption of the new bundled record is the accepted path. The
classifier (`diff_resolved_contracts`) ships as data-model code — the seed for
a future registry/third-party-distribution trigger.

## 4. The adapters

The count follows the product model: one adapter per surface kind that has
extension-specific runtime behavior. Auth has none — every vendor difference is
data — so extensions implement at most **two** traits.

### 4.0 Entrypoint and binding

Each runtime loader produces one entrypoint per extension:

```rust
pub trait ExtensionEntrypoint: Send + Sync {
    fn bind(&self, ctx: BindContext) -> Result<ExtensionBindings, BindError>;
}

pub struct ExtensionBindings {
    pub tools: Option<Arc<dyn ToolAdapter>>,
    pub channel: Option<Arc<dyn ChannelAdapter>>,
}
```

`bind` is side-effect-free and receives no network/secret/store ports — only
the runtime context, the resolved contract, and resolved **non-secret tenant
configuration** values (adapters are parameterized at bind time; secrets exist
only behind host injection). The binding rule is a direct check, not a
framework:

- manifest declares `[[tools]]` or `[mcp]` → `tools` must be `Some`;
- manifest declares `[channel]` → `channel` must be `Some`;
- nothing undeclared may be bound; auth never binds (host-managed);
- internal publication requires an operational surface: a tool, channel, or hook. The
  host-internal `[mcp]` connection template is discovery authority, not a
  callable tool, so an empty discovered catalog cannot publish by itself;
  channel-only and hook-only contracts remain valid;
- violations fail publication with a typed error. No partial surface publishes.

Loaders by runtime kind:

- **`first_party`** — resolves `runtime.service` in a native factory registry
  that the *binary* assembles (composition receives it as input and never links
  a concrete extension crate).
- **`wasm`** — wraps the existing WASM tool execution lane in a generic
  `WasmToolAdapter`; the entrypoint is synthesized from the manifest. (WASM
  channel adapters are out of scope until a WASM channel exists.)
- **`mcp`** — selected by the presence of `[mcp]`; the loader connects to the
  declared server, runs ceiling-validated discovery (§3.1), and synthesizes a
  `ToolAdapter` that proxies invocations with the connection credential
  injected. The extension ships no code.

Adapters implement behavior only. They never report ids, schemas, effects,
scopes, routes, credentials, or display metadata — the resolved manifest is the
sole authority, and adapters receive the declaration they implement.

### 4.1 `ToolAdapter` — one per extension, one method

```rust
#[async_trait]
pub trait ToolAdapter: Send + Sync {
    /// Invoke one declared (or MCP-discovered) capability.
    async fn invoke(&self, call: ToolCall, ports: &ToolPorts<'_>) -> Result<ToolResult, ToolError>;
}
```

One method is not an impoverished interface — it is everything that remains
once the historical adapter responsibilities are hoisted to where they belong:

- *what tools exist, their ids, schemas, descriptions, prompt docs, effects,
  permissions, credentials* — **manifest data**, read from the resolved
  contract; the adapter is never asked, so it cannot lie or drift;
- *listing tools to the model, input validation, authorization, approvals,
  obligations, resource reservation, credential injection, events, audit* —
  the **dispatcher pipeline**, implemented once;
- *how the implementation is obtained* — the **loaders**, implemented once per
  runtime kind.

What remains extension-specific is exactly "given validated input for
capability X, do the work" — `invoke`.

There is **one adapter instance per extension, not per tool**. `ToolCall`
carries the `capability_id` and the adapter routes internally: the Slack
adapter is a `match` over its five capability ids calling five functions.
(Concrete dispatch inside the concrete crate is fine — the ban is on concrete
names in *generic* code.) This is also why binding is a single `Option` field
rather than a per-capability binding map.

Who implements `invoke`, by runtime kind:

| Runtime | `invoke` implementation | Extension author ships |
| --- | --- | --- |
| `first_party` | the extension crate: match id → build request → `ports.egress` → parse | Rust in the extension crate |
| `wasm` | the loader's generic `WasmToolAdapter`: call the module export | a WASM module, zero host Rust |
| `[mcp]` | the loader's generic `McpToolAdapter`: `tools/call` on the connected server | nothing — manifest only |

`ToolCall` carries the capability id, schema-validated input, invocation id,
actor/turn scope, and deadline. `ToolPorts` exposes restricted egress (with
host-side credential injection), scoped key-value state, and logging — derived
from the resolved contract, nothing wider. A mid-flight authorization failure
(a revoked token, say) returns as a typed `ToolError` category that the host
maps to the generic re-auth gate. **Discovery is never the adapter's job:**

| | Static `[[tools]]` | `[mcp]` |
| --- | --- | --- |
| Source of truth | the manifest | the server's `tools/list` |
| When discovered | manifest compile/resolve | readiness reconciliation + explicit refresh (loader-run, ceiling-validated, atomic) |
| Published as | tool surfaces in the active snapshot | tool surfaces in the active snapshot |
| Downstream difference | none | none |

### 4.2 `ChannelAdapter` — one per extension

```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Idempotent; runs during internal publication. Vendor-side wiring and
    /// configuration validation live here (Telegram `setWebhook`, Slack
    /// `auth.test`). Failure aborts publication.
    async fn activate(&self, ctx: &ChannelContext<'_>, egress: &dyn RestrictedEgress) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Idempotent, best-effort; runs during unpublication/removal. Vendor-side
    /// unwiring (Telegram `deleteWebhook`). Failure is recorded and retryable;
    /// it does not block removal forever.
    async fn cleanup(&self, ctx: &ChannelContext<'_>, egress: &dyn RestrictedEgress) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Parse one host-verified inbound request into a normalized outcome.
    /// Pure protocol work: no I/O, no secrets, bounded input.
    fn inbound(&self, request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError>;

    /// Render and send one normalized outbound envelope through restricted
    /// egress. Owns vendor formatting, splitting, target syntax, DM
    /// provisioning, and safe error mapping. Returns structured outcomes;
    /// never touches the delivery store.
    async fn deliver(&self, envelope: OutboundEnvelope, egress: &dyn RestrictedEgress) -> Result<DeliveryReport, ChannelError>;

    /// Optional: list/search delivery targets for pickers.
    async fn list_targets(&self, query: TargetQuery, egress: &dyn RestrictedEgress) -> Result<Vec<TargetCandidate>, ChannelError> {
        Err(ChannelError::Unsupported)
    }
}
```

```rust
pub enum InboundOutcome {
    /// Normalized message(s) for the workflow (actor, conversation, event id,
    /// text, attachments, optional opaque `reply_context` ≤ 4 KiB that the
    /// host stores server-side and hands back at delivery time).
    Messages(Vec<NormalizedInboundMessage>),
    /// Bounded immediate response (e.g. Slack URL-verification challenge).
    Respond(ImmediateResponse),
    /// Authenticated no-op (ignored event types).
    Ignore,
}
```

Attachments are **references, not bytes**: an `AttachmentRef` carries the
vendor URL/id and a mime hint, which keeps `inbound` pure. When something
actually needs the bytes, the *host* fetches them through restricted egress
with the channel credential — secrets stay host-side. Outbound attachments are
envelope parts; `deliver` owns the upload. (The ref type is defined now; the
fetch path is built only when a consumer needs it.)

### 4.3 Auth — one host engine, recipes, no adapter

There is deliberately **no auth trait in the extension ABI.** The host's
`AuthEngine` implements each auth *method* once — `oauth2_code` (with PKCE) and
`api_key` — and executes manifest recipes:

- **Engine owns (once, for every vendor):** start/status/callback/revoke
  routes, state/CSRF/PKCE/replay/TTL, reserved OAuth parameters, redirect
  validation, scope intersection against the recipe ceiling, token exchange
  HTTP, secret encryption and storage, identity claim extraction via the
  recipe's JSON-pointer map, grant/account records, the blocked-tool auth gate
  and resume, revocation and grant cleanup on extension removal. Refresh is
  on-demand at injection; additionally, a recipe may declare an idle keepalive
  threshold (a vendor lifetime constraint — some vendors expire refresh tokens
  after a fixed idle window), executed once by the engine as a generic,
  vendor-blind background sweep.
- **Recipe declares (per vendor, data only):** endpoints, scope parameter
  name, extra authorize params, token-exchange auth style, token-response and
  identity field paths (RFC 6901 JSON pointers, closed vocabulary), refresh
  rotation flags and optional idle-keepalive threshold
  (`refresh.keepalive_idle_seconds`), revoke endpoint, client-credential
  handles — or, for `api_key`: form fields and an optional validation probe:

  ```toml
  [auth.github]
  method = "api_key"
  display_name = "GitHub personal access token"
  fields = [ { handle = "github_token", label = "Personal access token", secret = true } ]
  validation = { method = "GET", url = "https://api.github.com/user", success_status = [200], inject = { handle = "github_token", type = "header", name = "authorization", prefix = "Bearer " } }
  ```

Why no auth adapter trait: vendors differ in **parameters**, never in **flow
behavior**. Every OAuth2 authorization-code flow is the same state machine with
different endpoints, parameter names, and response field paths; every personal
access token is store → validate → inject. Recipes carry the parameters; the
engine implements each behavior once. This also means no third-party code ever
executes inside an auth flow (a whole class of attack surface — parameter
override, state tampering, token exfiltration — cannot exist), auth is tested
once against a scripted vendor server with recipes as table rows, and a
pure-manifest extension gets working auth with zero Rust. All five current
vendors (Slack, Google, Notion, GitHub, NEAR AI) are expressible as recipes.
If a future vendor genuinely defeats the descriptor, add a narrow quirk hook
*then* — do not pre-build it.

### 4.4 Ownership summary

| Concern | Host (generic, once) | Extension (adapter/recipe) |
| --- | --- | --- |
| Tool discovery | manifest (static) / MCP loader (discovered) | — |
| Tool invocation | authz, approvals, obligations, resources, audit, credential injection | `invoke` behavior |
| Ingress | route table, body/rate/deadline limits, signature recipes, replay, durable admission, ack | payload parsing → normalized outcome |
| Outbound | intent envelope, target policy, attempt persistence, retry/dedupe, drain | rendering, vendor API calls, part outcomes |
| Auth | everything (engine + recipes) | recipe data in manifest |
| Admin setup | manifest form, tenant-scoped storage, completeness | opaque handles only |
| User readiness | membership, personal auth/pairing, derived public state | idempotent internal publish/cleanup hooks |
| Lifecycle | membership changes, binding checks, atomic publication, generic cleanup | idempotent hooks |
| Secrets | storage, encryption, injection | opaque handles only |

## 5. Core flows

Four host pipelines wrap the extension surfaces. Each pipeline is implemented
once and owns semantics, security, and reliability; the extension contributes
one narrow call — or nothing:

| Flow | Host pipeline (once) | Extension contribution |
| --- | --- | --- |
| Tool call | dispatcher (§5.2) | `ToolAdapter::invoke` |
| Inbound message | ingress router (§5.3) | `ChannelAdapter::inbound` |
| Outbound message | delivery coordinator (§5.4) | `ChannelAdapter::deliver` |
| Auth | auth engine (§5.5) | recipe data only |

### 5.1 Install and readiness reconciliation

1. Install adds the authenticated caller to the extension's membership set. It
   does not create a tenant-wide user installation and it does not expose a
   second Activate action.
2. Load the persisted resolved record (compile the manifest if new/changed),
   then derive readiness from three independent inputs: tenant-scoped required
   `[admin_configuration]` values, this caller's membership, and this caller's
   required auth/pairing state.
3. Missing tenant setup or personal auth/pairing projects `setup_needed` and
   returns the appropriate generic admin/configure/connect affordance. No
   callable surface is published for that caller.
4. Once those requirements are satisfied, the loader produces the entrypoint;
   `bind` returns adapters; the binding rule and global conflicts are checked.
   MCP loaders run bounded discovery here (§3.1), and channel provisioning hooks
   run as an internal host step.
5. One immutable `Arc<ActiveSnapshot>` swap publishes the generation. The
   caller now projects `active`; in-flight work keeps the `Arc` it started with.

The internal host may still call this last publication step "activation" in
implementation APIs. That is not a fourth public state, a user action, or a
durable per-user activation toggle.

### 5.2 Tool call

The dispatcher keeps its existing policy pipeline and swaps only the lookup.
End to end:

1. **List** — the agent loop reads tool surfaces (id, description, schema,
   prompt doc) from the active snapshot: resolved manifest data; the adapter
   is not consulted.
2. **Resolve** — the model calls `slack.search_messages`;
   `ToolResolver::resolve(capability_id)` returns the prebound adapter, its
   resolved declaration, and the generation. An unknown id fails here, before
   any work.
3. **Policy** — authorization, permission mode, approvals, obligations,
   resource reservation: host, driven by the declaration's effects.
4. **Validate** — input checked against the manifest's input schema.
5. **Credentials** — the declaration names its vendor credential; a missing
   grant raises the generic auth gate (§4.3), then resumes. Present grants are
   injected by restricted egress at request time; the adapter never holds
   bytes.
6. **Invoke** — `adapter.invoke(call, ports)` does the work.
7. **Record** — result, events, audit: host; back to the model.

Host **built-in capabilities** (memory, workspace, …) are not extensions:
they live in the host's built-in registry, resolve through the same lookup,
and run the identical pipeline. An extension capability id that collides with
a built-in fails internal publication.

`slack.send_message` stays an explicit delegated side-effect tool; final
replies never go through it.

### 5.3 Inbound message

```mermaid
sequenceDiagram
    participant V as Vendor
    participant R as Generic ingress router
    participant A as ChannelAdapter
    participant W as Product workflow

    V->>R: POST /webhooks/extensions/{ext}/{suffix}
    R->>R: match route, enforce method/body/rate/deadline
    R->>R: execute verification recipe (constant-time, replay window)
    R->>A: inbound(verified bounded request)
    A-->>R: Messages | Respond | Ignore
    R->>W: durable dedupe + admission commit
    W-->>R: receipt
    R-->>V: 2xx (only after durable commit; else retryable 5xx)
    W->>W: identity/conversation binding, turn submission
```

Conversation binding honors the channel's declared `conversation_model`
(continuous channels bind per external conversation ref). With multiple
installations on one route the host verifies each candidate's secret (bounded,
small constant); hints are unnecessary at current scale.

### 5.4 Outbound delivery — the coordinator

Sending a message decomposes into two halves, and the split is the design:

- **Semantics and reliability** — which target, is it allowed, was it already
  sent, persist the attempt, retry with backoff, crash recovery, drain on
  shutdown. Identical for every channel → the **delivery coordinator**, one
  host component in product workflow.
- **Vendor mechanics** — Block Kit vs plain text, message splitting, which API
  method, threading syntax, DM provisioning, vendor error mapping. Different
  per channel → the adapter's **`deliver()`**.

Every user-visible channel output is a semantic intent, not an API call:
`FinalReply`, `Progress`, `GatePrompt`, `AuthPrompt`, `FailureNotice`,
`ConnectRequired`, `Working`, `Cleanup` (e.g. delete the working message),
`TriggeredDelivery` (routines/heartbeat). Emitters never know what channel the
user is on. One delivery, end to end:

1. An intent is emitted ("FinalReply for run X").
2. The coordinator resolves the target: reply where the message came from
   (via the stored `reply_context`) or a stored preference target for
   proactive sends. Unauthorized or unavailable targets fail closed.
3. It persists a delivery attempt (`Prepared` → `Sending`) **before** any
   network call.
4. It resolves the bound channel adapter from the active snapshot
   (generation-pinned; an in-flight delivery survives an upgrade).
5. `adapter.deliver(envelope, egress)` renders and sends; the host injects
   credentials.
6. The adapter returns a structured per-part report (sent + vendor message
   ref / retryable / permanent). It has no store access and cannot mark
   anything delivered.
7. The coordinator records the outcome, schedules retries, dedupes, and
   drains on shutdown.

The **sole-writer rule** is what makes the crash story tractable: if the
process dies after the vendor accepted a message but before the result was
recorded, the attempt is found in `Sending` and becomes `Unknown` — never
blindly resent (that is how users get duplicate messages) unless the vendor
supports an idempotency key that makes a resend provably safe. This works only
because exactly one component owns delivery truth, which is why "no direct
product send path" is an architecture-gated rule.

The coordinator is **not folded into `ChannelAdapter`** for the same reason
the dispatcher is not folded into `ToolAdapter` and the ingress router is not
folded into `inbound()`: folding it in would hand every channel its own copy
of retry/persistence/crash semantics, give adapters store access (a buggy or
malicious adapter could mark failures delivered), and something above the
adapter must resolve the target before an adapter can even be chosen. From an
extension author's perspective the coordinator is invisible plumbing:
envelope in, report out.

Boundary notes: `slack.send_message` (the tool) is the *model* acting as the
user — a job side effect through the tool pipeline — never how the
assistant's replies are delivered. `web_app` is the explicit no-external-egress
run target: the answer remains in canonical run/thread state for the WebUI to
render. External targets pass through the coordinator and vendor adapter.

This is a promotion, not an invention: the lower layer already exists
(`ironclaw_outbound`: target policy, preferences, attempt types, stores; plus
`outbound_delivery.rs` in product workflow). The coordinator unifies those
pieces and absorbs the generic halves of today's Slack-fused
`slack_delivery.rs` — completing the decomposition that file's own header
already tracks (#4818).

### 5.5 Auth connect

UI/gate → `POST start` → engine builds the authorize URL from the recipe (host
constructs `state`, `redirect_uri`, PKCE, `client_id`, scopes) → vendor →
callback on the existing `/api/reborn/product-auth/oauth/{provider}/callback`
path (the path parameter is the vendor id, resolved against active recipes —
not a code branch; registered vendor redirect URLs keep working) → engine
validates state/PKCE/replay, exchanges the code per recipe, extracts the
normalized grant and identity via pointer paths, encrypts and stores, resumes
any blocked invocation. `api_key`: generic form from recipe fields → optional
validation probe → store.

## 6. Lifecycle and standard state machines

The product model separates lifecycle authority from readiness prerequisites:

- caller membership is the only installation-lifecycle authority;
- tenant-scoped `[admin_configuration]` and caller-scoped personal auth/pairing
  are readiness prerequisites, not lifecycle owners; and
- the public state derived from those facts.

An admin saving deployment configuration never installs the extension for
themself or anyone else. Each user independently joins or leaves membership,
and personal grants, pairings, and bindings remain isolated by user. Generic
host internals own publication and cleanup; no extension may introduce a
state, cleanup path, or lifecycle branch of its own.

### 6.1 Public extension state (one projection, every extension)

The wire/UI state is exactly:

```text
not a member                         -> uninstalled
member + missing tenant setup,
         personal auth, or pairing  -> setup_needed
member + every requirement ready    -> active
```

- Install means "join membership." The result is immediately `active` when the
  manifest declares no unmet setup, or `setup_needed` otherwise.
- There is no public `installed`, `configured`, `disabled`, `failed`,
  `unsupported`, `activating`, or `deactivating` state and no Activate/Disable
  action. A user who wants to stop using an extension removes their membership.
- Internal loader, discovery, provisioning, conflict, and publication failures
  remain redacted diagnostics attached to `setup_needed`; they never create a
  fourth product state.
- Internal host checkpoint enums and `activate`/`cleanup` hook names are
  implementation vocabulary only. They must collapse onto this projection at
  every product boundary.

### 6.2 Removal (host-owned operation)

Removal is a single host-owned operation, not a persisted state sequence. The
fixed order is:

1. Remove the caller from membership and reject new work for that caller; drain
   their in-flight work under a bounded deadline.
2. Cancel that caller's pending auth flows and delete only their grants,
   accounts, identity bindings, and routes that no remaining membership
   requires. Another user and another extension using the same vendor are
   unaffected.
3. If members remain, keep the shared runtime publication and vendor wiring.
4. Only when the last member leaves, unpublish and drain the shared runtime,
   run the idempotent `channel.cleanup()` vendor-unwiring hook, then drop the
   shared runtime row.
5. Tenant admin configuration remains until an admin replaces or removes it;
   user removal never deletes deployment credentials.

Failure in 2–4 fails the operation loud with a typed, redacted quarantine
reason; the caller retries removal. There is no dormant "removal-pending" state
and no early success. Conversation and LLM history is **never** deleted (repo
law); cleanup means integration state only.

The same order runs for every extension. Slack's old bespoke cleanup
(`extension remove` special cases) is deleted, not generalized.

### 6.3 Auth account state machine (one enum, every vendor)

Owned entirely by the auth engine; recipes affect HTTP details only, never
states or transitions:

```text
Disconnected ──start flow──▶ Authenticating ──callback ok──▶ Connected
      ▲                            │ TTL/denied/error              │
      │◀───────────────────────────┘                              │
      │                                    refresh failure/expiry  ▼
      │                                                         Expired
      │◀───────── disconnect / removal (delete account) ───────────┘
```

- States: `disconnected | authenticating | connected | expired`, plus a typed
  `last_error` (`refresh_failed | grant_revoked | flow_expired | vendor_denied |
  exchange_failed | validation_probe_failed | credential_missing`). `Refreshing`
  is internal to the engine and never observable as a distinct wire state.
  Disconnect and removal delete the account synchronously, so there is no
  transient "revoking" wire state.
- `api_key` uses the same machine (`authenticating` = form submitted +
  validation probe running).
- Exactly one transition consumes a callback (replay-safe); flow TTL expiry
  lands back in `disconnected` with a typed reason.
- The generic auth card renders this enum for every vendor. There is no
  vendor-specific or extension-specific connection state anywhere.

### 6.4 Derived connection status

"Is this extension ready for this user?" is **derived, not stored**: caller
membership + required tenant `[admin_configuration]` fields present + that
caller's required auth account/pairing state + successful internal publication.
The admin UI computes deployment completeness from manifest groups. The user UI
computes Connect/Reconnect/Remove affordances from the derived state and recipe
or pairing descriptor — no third state machine and no per-extension logic.

The wire models a vendor's auth as a **list of accounts** (each carrying the
§6.3 state and a default marker) plus each surface's resolved account, even
while the system enforces one account per vendor — so the accepted
multi-account follow-up (`adr/0001-multiple-accounts-per-vendor.md`) extends
behavior without a wire break.

### 6.5 Other lifecycle rules

- Startup rebuilds publication from durable memberships, tenant admin
  configuration, and caller-scoped auth/pairing facts. It does not restore a
  separate per-user activation toggle; an invalid extension is skipped with a
  typed error and does not block valid ones.
- Upgrade — boot-time adoption of a changed host-bundled contract — swaps the
  new generation atomically; the old generation drains via its `Arc`. A
  widening consent gate is deliberately not built (§3.3, §7).
- Saving `[admin_configuration]` refreshes every runtime consumer of that
  tenant-scoped group through the generic reconciliation path. Users whose
  memberships become ready project `active`; failures remain `setup_needed`
  with a redacted diagnostic. There is no per-installation configuration copy
  and no separate `Reconfiguring` state.
- Deployment assumption, documented not engineered: **one serving process per
  deployment** owns the active set. Multi-replica serving needs its own ADR.

## 7. Deliberately not built

Considered and excluded. Each has a named revisit trigger; none may be
reintroduced without one.

| Excluded | Why | Revisit when |
| --- | --- | --- |
| Manifest fragments / multi-file compilation | largest manifest ≈ 700 lines; single file is reviewable | a manifest becomes genuinely unreviewable |
| Content-addressed package blob store, generation leases, GC | packages are host-bundled; the binary is the immutable store | registry distributes mutable third-party packages |
| Upgrade widening approval (parked generation, consent gate) | packages are host-bundled — upgrades happen only via reviewed binary releases + boot adoption; the diff classifier exists as data-model code | third-party/registry package distribution |
| Package signing (Ed25519, trust store, revocation) | no third-party distribution channel yet; own project | same as above |
| Serving-leader lease / fencing tokens | single serving process documented | multi-replica deployment is real (new ADR) |
| Digest-pinned shared vendor implementation packages | shared vendor = identical-recipe rule (§3.2); native code shares via crate deps | third-party binary vendor-implementation sharing exists |
| Per-vendor auth adapters, manual-validator trait | recipes cover all five current vendors; no code in auth flows | a vendor defeats the descriptor (add a narrow hook) |
| Generic "dynamic tools" abstraction | MCP is the only dynamic source; one `[mcp]` section, owned by the MCP loader | a second, non-MCP discovery source is real |
| Channel sub-adapter set (connection/target/action traits) | folded into `ChannelAdapter` methods + `[admin_configuration]` | a real action that config + hooks cannot express |
| Multiple channel surfaces per extension | no extension has two | one does (wire already carries surface keys) |
| Per-installation admin setup | manifest `[admin_configuration]` is stored once per tenant and shared by every member | a deployment genuinely needs multiple administrator-selected instances (new ADR) |
| Multiple accounts per vendor per user | one account per vendor per user matches current behavior | **triggered 2026-07-13** (work + personal Google accounts; two Notion accounts) — accepted as a dedicated post-P7 PR, `adr/0001-multiple-accounts-per-vendor.md`; the train ships only the list-shaped wire (§6.4) |
| Trigger/file runtime | reserved kinds, no implementation exists | a production trigger/file use case (fourth adapter, additive) |
| Multi-digest canonicalization, golden canonical JSON | one source digest + resolved-contract diff suffices | never, absent a concrete need |
| Machine-readable evidence ledger, sign-off roles, checker scripts | checklist + CI + architecture gates are the evidence | never |
| Second (TypeScript) Playwright harness | `tests/e2e/` already has `reborn_webui_harness.py` + fake vendor APIs | never |

## 8. Testing model

The abstraction pays for itself here:

- **One conformance suite per adapter.** `ironclaw_product_adapters` exports a
  reusable channel-adapter conformance suite (inbound outcomes, deliver report
  shape, internal publish/cleanup idempotency against a scripted vendor server); each
  extension crate runs it in its tests. Tool adapters get the same treatment.
- **One auth-engine suite.** State/PKCE/replay/exchange/refresh/revoke/identity
  tested once against a scripted vendor server; each vendor recipe is a table
  row, not a suite.
- **One fixture extension** (`acme-messenger`: invented vendor, tools + channel
  + oauth recipe) drives every generic end-to-end path in the integration
  harness: admin configure → install → `setup_needed` → connect → `active` →
  inbound → turn → outbound → remove.
  It is the proof that no generic path needs a real product.
- **Real extensions** keep protocol unit tests (parse/render fixtures) inside
  their crates plus **one** end-to-end integration proof each (Slack, Telegram).
- **Architecture gates:** the retired-taxonomy gate, a concrete-name scanner
  over generic crates (path-scoped allowlist that must shrink to zero), a
  dependency-direction gate (only the binary and tests may link concrete
  extension crates), and a CI job that builds/tests the generic workspace with
  the concrete crates removed — the deletion test, automated.

Repo testing law applies throughout: test-first, integration-tier coverage for
production-wired behavior, both database backends for persistent stores.
