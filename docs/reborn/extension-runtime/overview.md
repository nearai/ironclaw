# Unified Extension Runtime — Overview

**Status:** Approved design.
**Companions:** `implementation.md` (what changes, where), `checklist.md` (acceptance).
**Baseline:** the NEA-25 unified-extension stack (`origin/nea25/08-audit-fixes`), which this branch contains.

This document is the complete mental model: the product shape, the manifest, the
adapter seams, and how the generic core uses them. It deliberately contains no
machinery beyond what the goal requires; section 7 lists what was considered and
excluded, so it does not creep back in.

## 1. Goal

Every integration (Slack, Gmail, GitHub, Telegram, …) is an ordinary installable
extension package. The generic runtime installs, activates, dispatches, and
removes extensions using only their manifest and two narrow adapter seams (plus
one recipe-driven host engine for auth). No generic crate contains a concrete
product name, protocol type, route, or behavior branch.

Acceptance is three concrete tests:

1. **Deletion test.** Remove the Slack package and `ironclaw_slack_extension`
   from the build: every generic crate still compiles and its tests pass.
2. **Addition test.** Add a new channel extension (e.g. Discord): no generic
   source file changes — a new package and a new extension crate only.
3. **Testing test.** Each capability has *one* conformance/behavior suite that
   every implementation passes, plus protocol unit tests inside each extension
   crate. There is no per-provider copy of the OAuth, ingress, or delivery test
   suites (no "Gmail OAuth tests" *and* "Slack OAuth tests" — one auth-engine
   suite, table-driven over recipes).

## 2. Product model

Unchanged from NEA-25; restated so this document stands alone.

- **Extension** — the only installable product object. One `ExtensionId`
  (`slack`, `github`, `gmail`) owns every surface in the package.
- **Capability surfaces** — an extension declares up to three kinds:
  **tool** (model-callable capability), **channel** (message inbound/outbound),
  **auth** (credential acquisition for a provider). `trigger` and `file` remain
  reserved enum variants with no runtime behavior.
- **`ProviderId`** — names an external credential authority. Several extensions
  may share one (`google` across gmail/drive/calendar/docs/sheets/slides). It is
  never a product identity.
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
provider = "slack"
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
provider = "slack"
scopes = ["chat:write"]
audience = { scheme = "https", host = "slack.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

# ---- channel surface (at most one per extension) ---------------------------

[channel]
id = "messages"
display_name = "Slack messages"
inbound = true
outbound = true

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

[channel.config]                  # operator setup; host renders the generic form
fields = [
  { handle = "slack_bot_token", label = "Bot token", secret = true },
  { handle = "slack_signing_secret", label = "Signing secret", secret = true },
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

[auth.slack]
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
  entries reuse the existing v2 injection model (`audience` + `injection`); the
  host injects secrets during restricted egress, adapters never see bytes.
- **Dynamic tools** (hosted MCP) replace `[[tools]]` with one declaration:

  ```toml
  [dynamic_tools]
  source = "mcp_tools_list"
  namespace = "notion"            # every discovered tool id must be `notion.*`
  max_tools = 256
  default_permission = "ask"
  ```

  Discovered tools are ordinary tool surfaces validated against this ceiling
  (namespace, count, bounded schema size). They cannot add credentials, egress
  hosts, or effects beyond what the manifest declares.
- **`[channel.ingress.verification]`** is a declarative recipe the *host*
  executes: `hmac_sha256` (segment list: literals, named headers, body),
  `shared_secret_header` (constant-time compare), or `none`. Signing secrets
  never reach the adapter. Two recipe kinds cover Slack and Telegram; new kinds
  are added to the host when a protocol genuinely needs one.
- **`[channel.config]`** replaces bespoke setup panels. The host renders the
  form, validates, and stores secret fields under the named handles. Vendor-side
  validation of the config happens in the adapter's `activate()` hook.
- **`[auth.*]`** is one recipe per provider the extension needs. See section 4.3.

### 3.1 Shared providers

Extensions that use the same `ProviderId` each carry the recipe (gmail, drive,
calendar all embed the `[auth.google]` recipe). At activation the host unifies
them: recipes for one provider must be **identical except `scopes` and
`display_name`**, or activation fails with a conflict. Scope ceilings union
across active extensions exactly as NEA-25 does today; a new extension needing
more scopes triggers incremental re-consent. Accounts and grants are stored per
provider and shared — connecting Google once serves gmail and drive.

This replaces any notion of installable/shared "provider packages": ~20
duplicated TOML lines per Google extension, one 5-line conflict check, zero new
mechanism.

### 3.2 The resolved contract

The manifest is compiled **once** per install/upgrade into a typed
`ResolvedExtensionManifest` (surfaces, tools, channel descriptor, auth recipes,
egress, credentials) plus a `manifest_digest` of the source bytes. That record
is persisted; discovery, lifecycle, dispatch, ingress, auth, and the frontend
consume the record. Production code never reparses raw TOML. On upgrade, the
host diffs the old and new resolved contracts: **widening** (new scopes, egress
hosts, effects, routes, credential handles) requires renewed user approval;
equal or narrower contracts do not.

## 4. The adapters

The count follows the product model: one adapter per surface kind that has
runtime behavior. Auth has none — every provider difference is data — so
extensions implement at most **two** traits.

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
the installation context and the resolved contract. The binding rule is a
direct check, not a framework:

- manifest declares `[[tools]]` or `[dynamic_tools]` → `tools` must be `Some`;
- manifest declares `[channel]` → `channel` must be `Some`;
- nothing undeclared may be bound; auth never binds (host-managed);
- violations fail activation with a typed error. No partial extension activates.

Loaders by runtime kind:

- **`first_party`** — resolves `runtime.service` in a native factory registry
  that the *binary* assembles (composition receives it as input and never links
  a concrete extension crate).
- **`wasm`** — wraps the existing WASM tool execution lane in a generic
  `WasmToolAdapter`; the entrypoint is synthesized from the manifest. (WASM
  channel adapters are out of scope until a WASM channel exists.)
- **`mcp`** — synthesizes a `ToolAdapter` whose `discover_tools` runs the
  hosted-MCP `tools/list` discovery and whose `invoke` proxies the MCP call.

Adapters implement behavior only. They never report ids, schemas, effects,
scopes, routes, credentials, or display metadata — the resolved manifest is the
sole authority, and adapters receive the declaration they implement.

### 4.1 `ToolAdapter` — one per extension

```rust
#[async_trait]
pub trait ToolAdapter: Send + Sync {
    /// Only meaningful for `[dynamic_tools]` extensions; static extensions
    /// keep the default. The host validates results against the manifest
    /// ceiling and publishes them as ordinary tool surfaces.
    async fn discover_tools(&self, ports: &ToolPorts<'_>) -> Result<Vec<DiscoveredTool>, ToolError> {
        Ok(Vec::new())
    }

    /// Invoke one declared (or validated-discovered) capability.
    async fn invoke(&self, call: ToolCall, ports: &ToolPorts<'_>) -> Result<ToolResult, ToolError>;
}
```

`ToolCall` carries the capability id, schema-validated input, invocation id,
actor/turn scope, and deadline. `ToolPorts` exposes restricted egress (with
host-side credential injection), scoped key-value state, and logging — derived
from the resolved contract, nothing wider. **Discovery is host-owned:** the
model's tool list comes from the resolved manifest (static) or from validated
discovery results (dynamic), never from asking the adapter at dispatch time.

### 4.2 `ChannelAdapter` — one per extension

```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Idempotent; runs during activation. Vendor-side wiring and config
    /// validation live here (Telegram `setWebhook`, Slack `auth.test`).
    /// Failure fails activation.
    async fn activate(&self, ctx: &ChannelContext<'_>, egress: &dyn RestrictedEgress) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Idempotent, best-effort; runs during deactivation/removal. Vendor-side
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

### 4.3 Auth — one host engine, recipes, no adapter

There is deliberately **no auth trait in the extension ABI.** The host's
`AuthEngine` implements each auth *method* once — `oauth2_code` (with PKCE) and
`api_key` — and executes manifest recipes:

- **Engine owns (once, for every provider):** start/status/callback/revoke
  routes, state/CSRF/PKCE/replay/TTL, reserved OAuth parameters, redirect
  validation, scope intersection against the recipe ceiling, token exchange
  HTTP, secret encryption and storage, identity claim extraction via the
  recipe's JSON-pointer map, grant/account records, the blocked-tool auth gate
  and resume, revocation and grant cleanup on extension removal.
- **Recipe declares (per provider, data only):** endpoints, scope parameter
  name, extra authorize params, token-exchange auth style, token-response and
  identity field paths (RFC 6901 JSON pointers, closed vocabulary), refresh
  rotation flags, revoke endpoint, client-credential handles — or, for
  `api_key`: form fields and an optional validation probe:

  ```toml
  [auth.github]
  method = "api_key"
  display_name = "GitHub personal access token"
  fields = [ { handle = "github_token", label = "Personal access token", secret = true } ]
  validation = { method = "GET", url = "https://api.github.com/user", success_status = [200], inject = { handle = "github_token", type = "header", name = "authorization", prefix = "Bearer " } }
  ```

Why no code seam: no third-party code ever executes inside an auth flow (a
whole class of attack surface — parameter override, state tampering, token
exfiltration — cannot exist); OAuth is tested once against a scripted provider
server with recipes as table rows; and a pure-manifest extension gets working
auth with zero Rust. The current five providers (Slack, Google, Notion, GitHub,
NEAR AI) are all expressible as recipes. If a future provider genuinely defeats
the descriptor, add a narrow quirk hook *then* — do not pre-build it.

### 4.4 Ownership summary

| Concern | Host (generic, once) | Extension (adapter/recipe) |
| --- | --- | --- |
| Tool discovery | manifest / validated dynamic results | `discover_tools` for MCP only |
| Tool invocation | authz, approvals, obligations, resources, audit, credential injection | `invoke` behavior |
| Ingress | route table, body/rate/deadline limits, signature recipes, replay, durable admission, ack | payload parsing → normalized outcome |
| Outbound | intent envelope, target policy, attempt persistence, retry/dedupe, drain | rendering, vendor API calls, part outcomes |
| Auth | everything (engine + recipes) | recipe data in manifest |
| Connection/setup | config form, secret storage, connect/disconnect state | `activate`/`cleanup` vendor wiring |
| Lifecycle | staging, binding checks, atomic publish, generic cleanup | idempotent hooks |
| Secrets | storage, encryption, injection | opaque handles only |

## 5. Core flows

### 5.1 Activation

1. Load the persisted resolved record (compile the manifest if new/changed;
   widening diff → approval).
2. Loader produces the entrypoint; `bind` returns adapters; the binding rule is
   checked; global conflicts (duplicate capability id, duplicate route) are
   checked against the staged next snapshot.
3. `channel.activate()` hook runs (vendor wiring). Failure aborts with nothing
   published.
4. Enabled state is persisted, then one immutable `Arc<ActiveSnapshot>` swap
   publishes the generation. Readers resolve through snapshot views; in-flight
   work keeps the `Arc` it started with.

### 5.2 Tool call

Dispatcher keeps its existing policy pipeline (authorization, approvals,
obligations, resources, events, audit) and replaces per-invocation
package/runtime-kind selection with one lookup: `resolve_tool(capability_id)` →
prebound `ToolAdapter` + its resolved declaration. Missing credentials raise
the generic auth gate (engine), then resume. `slack.send_message` stays an
explicit delegated side-effect tool; final replies never go through it.

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

With multiple installations on one route the host verifies each candidate's
secret (bounded, small constant); hints are unnecessary at current scale.

### 5.4 Outbound delivery

Every user-visible channel output — final reply, progress, gate/auth prompt,
failure/connect/busy notice, cleanup — is a semantic intent entering **one
delivery coordinator** (product workflow). It validates the target, persists
the attempt, resolves the bound channel from the active snapshot, and calls
`deliver`. The coordinator is the **sole writer of delivery state**: retries,
backoff, dedupe, drain. A crash after vendor send and before persistence
records `Unknown` — never a blind resend. No direct product send path exists
anywhere else.

### 5.5 Auth connect

UI/gate → `POST start` → engine builds the authorize URL from the recipe (host
constructs `state`, `redirect_uri`, PKCE, `client_id`, scopes) → provider →
callback on the existing `/api/reborn/product-auth/oauth/{provider}/callback`
path (`{provider}` is a route parameter resolved against active recipes — not a
code branch; registered vendor redirect URLs keep working) → engine validates
state/PKCE/replay, exchanges the code per recipe, extracts the normalized grant
and identity via pointer paths, encrypts and stores, resumes any blocked
invocation. `api_key`: generic form from recipe fields → optional validation
probe → store.

## 6. Lifecycle and standard state machines

`ExtensionHost` (new crate) is the **only** active-set writer. Operations:
install, activate, deactivate, upgrade, remove, restore-at-startup. Every
extension moves through the **same pipeline and the same states** — the only
extension-specific participation is manifest data and the two idempotent
adapter hooks. No extension may introduce a state, a cleanup path, or a
lifecycle branch of its own; that is enforced by where the enums live (generic
crates) and by the architecture gates.

### 6.1 Installation state machine (one enum, every extension)

```text
Installed ──activate──▶ Activating ──publish──▶ Active
    ▲                        │ failure                │
    └────────────────────────┘                        │ deactivate/upgrade
                                                      ▼
Removed ◀──done── Removing ◀──remove── Installed ◀── Deactivating (drain)
                     │ cleanup failure        
                     ▼
              RemovalPending ──retry──▶ Removing
```

- `Activating`, `Deactivating`, `Removing` are transient and persisted, so a
  crash mid-transition resumes deterministically at startup.
- Activation failure returns to `Installed` with a typed, redacted error on the
  record — never a half-published extension.
- `RemovalPending` is the standard "vendor cleanup failed, will retry" state.
  It is visible, retryable, and cannot report success early or resurrect the
  extension.
- The wire exposes exactly this enum; the UI renders it identically for every
  extension.

### 6.2 Removal order (fixed, host-owned)

1. Persist `Removing`; unpublish from the active snapshot (new work rejected).
2. Drain in-flight work (bounded deadline; in-flight holds its generation `Arc`).
3. `channel.cleanup()` — vendor-side unwiring, idempotent, best-effort.
4. Auth engine: best-effort remote revoke per recipe; **always** delete local
   grants/accounts scoped to this extension's providers (unless another active
   extension shares the provider — then grants survive for it).
5. Delete channel config/secrets, identity bindings, and route registrations.
6. Persist `Removed`. Failure at 3–5 → `RemovalPending` + typed reason.
7. Conversation and LLM history is **never** deleted (repo law); cleanup means
   integration state only.

The same order runs for every extension. Slack's old bespoke cleanup
(`extension remove` special cases) is deleted, not generalized.

### 6.3 Auth account state machine (one enum, every provider)

Owned entirely by the auth engine; recipes affect HTTP details only, never
states or transitions:

```text
Disconnected ──start flow──▶ Authenticating ──callback ok──▶ Connected
      ▲                            │ TTL/denied/error              │
      │◀───────────────────────────┘                               │
      │                                     refresh failure/expiry ▼
      │◀──────── Revoking ◀──disconnect/removal── Connected / Expired
```

- States: `disconnected | authenticating | connected | expired | revoking`,
  plus a typed `last_error`. `Refreshing` is internal to the engine and never
  observable as a distinct wire state.
- `api_key` uses the same machine (`authenticating` = form submitted +
  validation probe running).
- Exactly one transition consumes a callback (replay-safe); flow TTL expiry
  lands back in `disconnected` with a typed reason.
- The generic auth card renders this enum for every provider. There is no
  provider-specific or extension-specific connection state anywhere.

### 6.4 Derived connection status

"Is this extension connected?" is **derived, not stored**: installation state
(`Active`) + required `[channel.config]` fields present + auth account state
(`connected`) for required providers. The UI computes affordances (Configure /
Connect / Reconnect / Remove) from those two enums plus config completeness —
no third state machine, no per-extension logic.

### 6.5 Other lifecycle rules

- Startup restores all enabled generations from persisted records and publishes
  once; an invalid extension is skipped with a typed error and does not block
  valid ones.
- Upgrade stages the new generation, applies the widening rule (§3.2), swaps
  atomically; the old generation drains via its `Arc`.
- Deployment assumption, documented not engineered: **one serving process per
  deployment** owns the active set. Multi-replica serving needs its own ADR.

## 7. Deliberately not built

Considered and excluded. Each has a named revisit trigger; none may be
reintroduced without one.

| Excluded | Why | Revisit when |
| --- | --- | --- |
| Manifest fragments / multi-file compilation | largest manifest ≈ 700 lines; single file is reviewable | a manifest becomes genuinely unreviewable |
| Content-addressed package blob store, generation leases, GC | packages are host-bundled; the binary is the immutable store | registry distributes mutable third-party packages |
| Package signing (Ed25519, trust store, revocation) | no third-party distribution channel yet; own project | same as above |
| Serving-leader lease / fencing tokens | single serving process documented | multi-replica deployment is real (new ADR) |
| Digest-pinned shared provider packages | shared provider = identical-recipe rule (§3.1); native code shares via crate deps | third-party binary provider sharing exists |
| Per-provider auth adapters, manual-validator trait | recipes cover all five current providers; no code in auth flows | a provider defeats the descriptor (add a narrow hook) |
| Channel sub-adapter set (connection/target/action traits) | folded into `ChannelAdapter` methods + `[channel.config]` | a real action that config + hooks cannot express |
| Multiple channel surfaces per extension | no extension has two | one does (wire already carries surface keys) |
| Trigger/file runtime | reserved kinds, no implementation exists | a production trigger/file use case (fourth adapter, additive) |
| Multi-digest canonicalization, golden canonical JSON | one source digest + resolved-contract diff suffices | never, absent a concrete need |
| Machine-readable evidence ledger, sign-off roles, checker scripts | checklist + CI + architecture gates are the evidence | never |
| Second (TypeScript) Playwright harness | `tests/e2e/` already has `reborn_webui_harness.py` + fake vendor APIs | never |

## 8. Testing model

The abstraction pays for itself here:

- **One conformance suite per adapter.** `ironclaw_product_adapters` exports a
  reusable channel-adapter conformance suite (inbound outcomes, deliver report
  shape, activate/cleanup idempotency against a scripted vendor server); each
  extension crate runs it in its tests. Tool adapters get the same treatment.
- **One auth-engine suite.** State/PKCE/replay/exchange/refresh/revoke/identity
  tested once against a scripted provider server; each provider recipe is a
  table row, not a suite.
- **One fixture extension** (`acme-messenger`: invented vendor, tools + channel
  + oauth recipe) drives every generic end-to-end path in the integration
  harness: install → activate → connect → inbound → turn → outbound → remove.
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
