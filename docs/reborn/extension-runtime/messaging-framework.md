# Messaging Tool Framework — Design Proposal

**Status:** Proposed (2026-07-14). New feature work on the finished extension
runtime, sequenced **after** the P0–P7 train.
**Companion:** `adr/0002-messaging-tool-framework.md` holds the *decision,
rationale, and rejected options*; this document is the *concrete design* — the
tools, their schemas, and how an extension opts in.
**Reading order:** `overview.md` (runtime model) → `adr/0002` (why) → this (how).

---

## 1. Summary

A **standard, vendor-neutral set of messaging tools** — send, read, list, search,
edit, delete, react — is defined **once**, as host-owned **capability profiles**
with normalized input/output schemas. An extension does **not** hand-write these
tools; it **declares the subset it supports** in a small `[messaging]` manifest
section. At install time the host **expands** that declaration into ordinary
per-extension tool surfaces (`slack.send_message`, `telegram.read_history`, …),
which flow through the existing resolver / dispatcher / disclosure / UI unchanged.
Each extension's tool adapter implements the per-vendor behavior and **normalizes
its output** to the shared schema (raw `U0123` → `{id, display_name}`).

Two invariants frame everything:

- **The messaging tools act as the *user-acquired identity*** (Slack OAuth,
  Telegram pairing), never the bot. The bot is only the **channel** (inbound +
  the assistant's replies, owned by the delivery coordinator). See §2.
- **The model sees one contract per tool, regardless of vendor.** Same schema in,
  same schema out, whether the adapter is a Slack HTTP call or a Telegram MTProto
  client.

---

## 2. The two identities (recap)

| | Channel surface (`[channel]`) | Messaging tools (`[messaging]`) |
| --- | --- | --- |
| Acts as | the **bot** | the **user** (user-acquired identity) |
| Direction | inbound events + the assistant's replies | model-initiated actions *as the user* |
| Owner | delivery coordinator (`overview.md` §5.4) | the tool dispatcher (`overview.md` §5.2) |
| Credential | bot token (`slack_bot_token`, Telegram bot token) | OAuth user token (Slack) / pairing session (Telegram) |

An extension may declare `[channel]` only (a bot entrypoint, no user-acting
tools — Telegram today), `[messaging]` only (act-as-user tools with no inbound
bot), or both (Slack).

---

## 3. Data model — the framework-owned normalized types

Shipped once as a framework asset `schemas/messaging/types.v1.json`; every tool
schema `$ref`s it. Ids are **opaque and round-trippable** (you pass them back to
act on the object) but every reference the model reads is **enriched** with
resolved human context.

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "ironclaw:messaging/types.v1",
  "$defs": {
    "UserRef": {
      "type": "object",
      "required": ["id", "display_name"],
      "additionalProperties": false,
      "properties": {
        "id":           { "type": "string", "description": "Opaque vendor user id (round-trippable)." },
        "display_name": { "type": "string", "description": "Resolved human-readable name. Always present — the adapter resolves it even if that costs an extra lookup." },
        "username":     { "type": "string", "description": "Handle without a leading @, where the platform has one." },
        "is_bot":       { "type": "boolean" }
      }
    },
    "ConversationRef": {
      "type": "object",
      "required": ["id", "kind"],
      "additionalProperties": false,
      "properties": {
        "id":    { "type": "string", "description": "Opaque conversation id." },
        "kind":  { "type": "string", "enum": ["dm", "group", "channel"], "description": "Normalized: dm = 1:1, group = multi-person private chat, channel = named/broadcast channel." },
        "title": { "type": "string", "description": "Name of a group/channel; a dm may derive it from the other participant." }
      }
    },
    "MessageRef": {
      "type": "object",
      "required": ["id", "conversation"],
      "additionalProperties": false,
      "properties": {
        "id":           { "type": "string", "description": "Message id — unique WITHIN its conversation, not globally (Slack ts, Telegram message_id)." },
        "conversation": { "type": "string", "description": "ConversationRef.id this message belongs to." },
        "thread":       { "type": "string", "description": "Thread id, when the message sits in a thread." }
      }
    },
    "Reaction": {
      "type": "object",
      "required": ["emoji", "count"],
      "additionalProperties": false,
      "properties": {
        "emoji": { "type": "string", "description": "Unicode emoji where the platform uses one; else a :shortcode: or a vendor custom-emoji id (see open questions)." },
        "count": { "type": "integer", "minimum": 0 },
        "me":    { "type": "boolean", "description": "Whether the acting user reacted." }
      }
    },
    "AttachmentRef": {
      "type": "object",
      "required": ["kind"],
      "additionalProperties": false,
      "properties": {
        "id":   { "type": "string" },
        "kind": { "type": "string", "enum": ["image", "video", "audio", "file", "other"] },
        "mime": { "type": "string" },
        "name": { "type": "string" },
        "url":  { "type": "string", "description": "Vendor URL/reference. Bytes are fetched host-side on demand, never returned inline (mirrors the channel adapter's AttachmentRef, overview §4.2)." }
      }
    },
    "Message": {
      "type": "object",
      "required": ["ref", "author", "text", "created_at"],
      "additionalProperties": false,
      "properties": {
        "ref":         { "$ref": "#/$defs/MessageRef" },
        "author":      { "$ref": "#/$defs/UserRef" },
        "text":        { "type": "string", "description": "Message text, normalized to Markdown where feasible." },
        "created_at":  { "type": "string", "format": "date-time" },
        "edited_at":   { "type": "string", "format": "date-time" },
        "reply_to":    { "$ref": "#/$defs/MessageRef", "description": "The message this one replies to, when applicable." },
        "reactions":   { "type": "array", "items": { "$ref": "#/$defs/Reaction" } },
        "attachments": { "type": "array", "items": { "$ref": "#/$defs/AttachmentRef" } }
      }
    }
  }
}
```

---

## 4. The standard tools

Each tool is a host-defined **capability profile** `ironclaw.messaging.<tool>.v1`
carrying an input and output schema. `send_message` is **core** (every messaging
platform and identity can send); the rest are **optional** and only appear when
the extension declares them.

| Tool | Profile id | Tier | `effects` | `default_permission` |
| --- | --- | --- | --- | --- |
| `send_message` | `ironclaw.messaging.send_message.v1` | **core** | `network`, `use_secret`, `external_write` | `ask` |
| `read_history` | `ironclaw.messaging.read_history.v1` | optional | `network`, `use_secret` | `ask` |
| `list_conversations` | `ironclaw.messaging.list_conversations.v1` | optional | `network`, `use_secret` | `ask` |
| `get_user` | `ironclaw.messaging.get_user.v1` | optional | `network`, `use_secret` | `ask` |
| `search_messages` | `ironclaw.messaging.search_messages.v1` | optional | `network`, `use_secret` | `ask` |
| `edit_message` | `ironclaw.messaging.edit_message.v1` | optional | `network`, `use_secret`, `external_write` | `ask` |
| `delete_message` | `ironclaw.messaging.delete_message.v1` | optional | `network`, `use_secret`, `external_write` | `ask` |
| `add_reaction` | `ironclaw.messaging.add_reaction.v1` | optional | `network`, `use_secret`, `external_write` | `ask` |
| `remove_reaction` | `ironclaw.messaging.remove_reaction.v1` | optional | `network`, `use_secret`, `external_write` | `ask` |

**Reply-in-thread is a parameter, not a tool** — the `thread` field on
`send_message`/`read_history`, accepted only when the extension declares
`supports_threads`. Schemas below reference the §3 `$defs` as
`types.v1#/$defs/<Type>`.

### 4.1 `send_message` (core)

> Model-as-user side effect. Its prompt doc **must** state that it posts as the
> user and is **never** used to deliver the assistant's final reply (the host
> delivers replies on the channel surface — `overview.md` §5.4).

```json
// input
{
  "type": "object",
  "required": ["conversation", "text"],
  "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string", "description": "Target ConversationRef.id (from list_conversations or an inbound message)." },
    "text":         { "type": "string", "description": "Message body as Markdown; the adapter renders to the vendor dialect and splits to the vendor length limit." },
    "thread":       { "type": "string", "description": "Reply inside this thread id. Accepted only when the extension declares supports_threads." },
    "reply_to":     { "type": "string", "description": "Message id (within `conversation`) to reply to." }
  }
}
// output
{
  "type": "object",
  "required": ["message"],
  "additionalProperties": false,
  "properties": { "message": { "$ref": "types.v1#/$defs/MessageRef" } }
}
```

### 4.2 `read_history`

```json
// input
{
  "type": "object",
  "required": ["conversation"],
  "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string" },
    "limit":        { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
    "before":       { "type": "string", "description": "Opaque cursor from a previous call; returns messages older than it." },
    "thread":       { "type": "string", "description": "Restrict to this thread." }
  }
}
// output
{
  "type": "object",
  "required": ["messages", "has_more"],
  "additionalProperties": false,
  "properties": {
    "messages": { "type": "array", "items": { "$ref": "types.v1#/$defs/Message" } },
    "has_more": { "type": "boolean" },
    "cursor":   { "type": "string", "description": "Pass as `before` to page older." }
  }
}
```

### 4.3 `list_conversations`

```json
// input
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "kinds":  { "type": "array", "items": { "type": "string", "enum": ["dm", "group", "channel"] }, "description": "Filter by kind; default all." },
    "query":  { "type": "string", "description": "Optional name filter." },
    "limit":  { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 },
    "cursor": { "type": "string" }
  }
}
// output
{
  "type": "object",
  "required": ["conversations", "has_more"],
  "additionalProperties": false,
  "properties": {
    "conversations": { "type": "array", "items": { "$ref": "types.v1#/$defs/ConversationRef" } },
    "has_more":      { "type": "boolean" },
    "cursor":        { "type": "string" }
  }
}
```

### 4.4 `get_user`

```json
// input
{ "type": "object", "required": ["user"], "additionalProperties": false,
  "properties": { "user": { "type": "string", "description": "Opaque user id (e.g. from a Message author)." } } }
// output
{ "type": "object", "required": ["user"], "additionalProperties": false,
  "properties": { "user": { "$ref": "types.v1#/$defs/UserRef" } } }
```

### 4.5 `search_messages`

```json
// input
{
  "type": "object",
  "required": ["query"],
  "additionalProperties": false,
  "properties": {
    "query":        { "type": "string" },
    "conversation": { "type": "string", "description": "Restrict to one conversation; omit for a global search." },
    "limit":        { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 },
    "cursor":       { "type": "string" }
  }
}
// output  (same shape as read_history)
{ "type": "object", "required": ["messages", "has_more"], "additionalProperties": false,
  "properties": {
    "messages": { "type": "array", "items": { "$ref": "types.v1#/$defs/Message" } },
    "has_more": { "type": "boolean" },
    "cursor":   { "type": "string" } } }
```

### 4.6 `edit_message` / `delete_message`

```json
// edit_message input
{ "type": "object", "required": ["conversation", "message", "text"], "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string" },
    "message":      { "type": "string", "description": "MessageRef.id within `conversation`." },
    "text":         { "type": "string", "description": "New body (Markdown)." } } }
// edit_message output
{ "type": "object", "required": ["message"], "additionalProperties": false,
  "properties": { "message": { "$ref": "types.v1#/$defs/MessageRef" } } }

// delete_message input
{ "type": "object", "required": ["conversation", "message"], "additionalProperties": false,
  "properties": { "conversation": { "type": "string" }, "message": { "type": "string" } } }
// delete_message output
{ "type": "object", "required": ["deleted"], "additionalProperties": false,
  "properties": { "deleted": { "type": "boolean" } } }
```

### 4.7 `add_reaction` / `remove_reaction`

```json
// add_reaction / remove_reaction input (identical)
{ "type": "object", "required": ["conversation", "message", "emoji"], "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string" },
    "message":      { "type": "string" },
    "emoji":        { "type": "string", "description": "Unicode emoji; the adapter maps to the vendor's reaction format." } } }
// output
{ "type": "object", "required": ["ok"], "additionalProperties": false,
  "properties": { "ok": { "type": "boolean" } } }
```

---

## 5. How an extension opts in — the `[messaging]` section

### 5.1 The section

One `[messaging]` section per extension. It names the **subset of standard tools**
and the **user-acquired credential** they run on. Everything else (ids, schemas,
descriptions, effects) comes from the framework.

```toml
[messaging]
tools = ["send_message", "read_history", "list_conversations", "get_user", "search_messages"]
default_permission = "ask"          # optional; framework default is "ask"
supports_threads = true             # optional; gates the `thread` param on send_message/read_history

[[messaging.credentials]]           # reuses the v3 [[tools.credentials]] model verbatim; the USER identity
handle = "slack_user_token"
vendor = "slack"
scopes = ["chat:write", "channels:history", "groups:history", "im:history", "mpim:history",
          "channels:read", "groups:read", "im:read", "mpim:read", "users:read", "search:read"]
audience = { scheme = "https", host = "slack.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }
```

Validation (extends `manifest_v3_contract`): every `tools` entry is a known
standard tool; a `thread`-bearing tool with `supports_threads = false` is
rejected; `[[messaging.credentials]]` is required if any declared tool has the
`use_secret` effect; the credential vendor must have a resolvable identity source
(an `[auth.<vendor>]` recipe **or** a pairing modality — §8).

### 5.2 Worked example — Slack (migrates today's five bespoke tools)

The block in §5.1 **is** the Slack opt-in. It expands to the same five capability
ids Slack ships today (`slack.send_message`, …) — so migration is parity (§11).

### 5.3 Worked example — Telegram (gains user-acting tools via pairing)

Telegram is a bot **channel** today with no tools. Once its **pairing** flow
yields a user session (§8), it adds:

```toml
[messaging]
tools = ["send_message", "read_history", "list_conversations", "get_user", "search_messages", "add_reaction"]
supports_threads = true             # forum topics / reply threads

[[messaging.credentials]]
handle = "telegram_user_session"    # the paired user session (NOT the bot token the channel uses)
vendor = "telegram"
# No `audience`/`injection`: Telegram user-acting is MTProto, not HTTP — the host
# drives a Telegram-user client and injects the session there (see §7). The
# credential is declared so the auth/pairing gate and storage work generically.
```

### 5.4 What the host does with it — expansion into ordinary tool surfaces

At install/upgrade, a **messaging expander** (in the manifest resolver, sibling
to the MCP discovery loader) turns each `tools` entry into an ordinary
`CapabilityDeclV2` (`crates/ironclaw_extensions/src/v2.rs:455`):

| `CapabilityDeclV2` field | Value |
| --- | --- |
| `id` | `<ext>.<tool>` (e.g. `slack.send_message`) |
| `implements` | `["ironclaw.messaging.<tool>.v1"]` |
| `input_schema_ref` / `output_schema_ref` | the **profile's** framework-owned schemas |
| `description` / `prompt_doc_ref` | framework canon (extension may override wording, not schema) |
| `effects` / `default_permission` | framework defaults per tool (a ceiling the recipe may narrow, not widen) |
| `visibility` | `Model` |
| `runtime_credentials` | from `[[messaging.credentials]]` |

These land in `ResolvedExtensionManifest.tools`
(`crates/ironclaw_extensions/src/resolved.rs:48`) exactly like static `[[tools]]`.
Downstream is unchanged: the active snapshot indexes them
(`ironclaw_extension_host/src/active.rs:71`), `ToolResolver::resolve`
(`ironclaw_dispatcher/src/lib.rs:48`) returns the prebound adapter, and the
extension's single `ToolAdapter` routes `<ext>.<tool>` internally — the pattern
Slack's module already uses (`assets/slack/wasm-src/src/lib.rs:134`).

### 5.5 The capability-profile contract (the reuse hook)

Each standard tool is a host-defined `CapabilityProfileContract`
(`crates/ironclaw_host_api/src/capability_profile.rs:206`) — a single operation
carrying the input/output schema refs:

```jsonc
// ironclaw.messaging.send_message.v1
{
  "id": "ironclaw.messaging.send_message.v1",
  "required_operations": [
    { "id": "send_message",
      "input_schema_ref":  "schemas/messaging/send_message.input.v1.json",
      "output_schema_ref": "schemas/messaging/send_message.output.v1.json" }
  ]
}
```

At activation the host runs the existing structural conformance evaluator
(`evaluate_profile_conformance`, `crates/ironclaw_capabilities/src/conformance.rs`)
over each expanded decl's claim: a tool claiming a messaging profile whose schema
refs don't match the host contract **fails activation** — the auth engine's
"recipe validates or activation fails" discipline, for tool profiles. (Today
`CapabilityDeclV2.implements`/`output_schema_ref` exist but are unwired — v3's
reader drops them, `v3.rs:119,362,368`; this framework is what wires them.)

---

## 6. The adapter contract (per-vendor behavior)

The extension implements exactly one method — the existing `ToolAdapter::invoke`
(`crates/ironclaw_host_api/src/tool_adapter.rs:94`); no new trait:

```rust
async fn invoke(&self, call: ToolCall, ports: &ToolPorts<'_>) -> Result<ToolResult, ToolError>;
```

For each `<ext>.<tool>` capability id the adapter must:

1. **Route** by `call.capability_id` to the vendor operation (a `match`).
2. **Read** `call.input` (already schema-validated by the host).
3. **Do the work** through its transport (§7).
4. **Normalize** the vendor response into the profile's output shape —
   **resolving ids to `UserRef`** with `display_name` filled (even at the cost of
   an extra lookup), and mapping vendor conversation/message shapes to
   `ConversationRef`/`MessageRef`/`Message`.
5. **Return** `ToolResult.output` (validated host-side against the profile's
   output schema, §10). Recoverable failures are `ToolError::Failed`
   (model-visible, run continues); a missing/expired credential is
   `ToolError::AuthRequired` → the generic gate (`tool_adapter.rs:64`).

The adapter reports **behavior only** — never ids, schemas, or effects (those are
the resolved manifest). A vendor "messaging core" of pure functions (Markdown→
dialect rendering, splitting, target/DM formatting, error mapping) is **shared
intra-crate** between this `invoke` and the channel adapter's `deliver`
(`adr/0002` §6.2); reliability (retry/persistence/dedupe) stays coordinator-only.

---

## 7. Transport — HTTP vendors vs. Telegram (MTProto)

The framework supports two adapter transports; the tool contract is identical
across both.

**HTTP vendors (Slack, Discord).** The adapter uses the existing host port
`RestrictedEgress` (`tool_adapter.rs:103`): scheme/host/method allowlist from the
resolved contract, host-side credential injection, size caps. No new mechanism —
a messaging adapter looks like today's Slack tool.

**Telegram (user-acting = MTProto).** MTProto is a **binary, non-HTTP** protocol,
so it does **not** fit `RestrictedEgress` (HTTP-only — verified). Proposal: add a
narrow **host-side Telegram-user client** port when Telegram user-acting is built
(the runtime's "add a hook when a vendor defeats the descriptor" rule, overview
§4.3). The adapter calls that port; the host owns the MTProto/TDLib client and
**holds the paired session** (the credential — bytes never reach the adapter,
same guarantee as HTTP injection).

Behavioral facts that shape it (verified Telegram-protocol behavior, not repo
code — no Telegram user-client exists yet):

- **Pull-on-demand, no message mirror required.** After pairing, a single
  `messages.getDialogs` returns recent conversations + last message + the
  entities (with `access_hash`) needed to reference them; `messages.getHistory`
  and `messages.search` then read on demand. `list_conversations` ≈ getDialogs,
  `read_history` ≈ getHistory, `search_messages` ≈ messages.search.
- **State the adapter/host keeps** = the **session** (persist across restarts;
  don't re-login) + an optional `id → access_hash` cache (rebuildable from
  getDialogs). This maps onto `ScopedToolState` (`tool_adapter.rs:164`). It is
  **not** a message-history mirror; a TDLib local DB is an optional optimization,
  not a requirement.
- **Bounded caveats** the adapter surfaces as `ToolError::Failed`: cold
  references (`resolveUsername` first for a peer never seen); rate limits
  (`FLOOD_WAIT`) on large enumerations; secret (E2E) chats are device-local and
  invisible to a server session.

---

## 8. Identity, credentials, and auth

- **The messaging credential is the user-acquired identity**, distinct from the
  bot the channel delivers on. It is declared in `[[messaging.credentials]]`.
- **Acquisition** rides the existing connect surface:
  - **OAuth vendors (Slack):** the `[auth.<vendor>]` recipe + the auth engine
    (`overview.md` §4.3) — the flow that already yields `slack_user_token`.
  - **Pairing vendors (Telegram):** the existing pairing modality — the
    `Pairing { .. }` lifecycle gate
    (`crates/ironclaw_product_workflow/src/lifecycle.rs:157`), the
    `PairingRequired` event (`crates/ironclaw_common/src/event.rs:163`), the
    Telegram pairing-code card (`ironclaw_product_adapters/src/outbound.rs:1027`).
    Whether pairing is modeled as a **third auth-engine method** (beside
    `oauth2_code`/`api_key`) or stays a separate connect modality is an open
    question (§13).
- **Gate + resume:** a tool call with a missing/expired grant returns
  `ToolError::AuthRequired`; the host raises the generic gate keyed by the tool's
  declared vendor (OAuth gate or pairing gate) and resumes the blocked turn on
  connect — unchanged from `overview.md` §5.2/§4.3.

---

## 9. Discovery (anti-bloat)

The expanded tools are ordinary **`Discoverable`-tier** surfaces, so at scale the
existing progressive-disclosure system defers them behind `tool_search` and
surfaces them by name in its catalog index (`tool_disclosure.rs`; `adr/0002`
§5). The `<ext>.<tool>` naming makes the model's "what messaging tools does Slack
have?" answerable through the generic `tool_search → capability_info → call`
flow. **No messaging-specific discovery tool is added.** (Note: that disclosure
layer is production-wired but currently opt-in/off — orthogonal to this
framework, which must not fork it.)

---

## 10. Normalization & validation

- **Adapter normalizes** vendor specifics into the profile output shape — the one
  behavior addition over today (Slack currently returns raw ids;
  `assets/slack/schemas/slack/raw_output.v1.json` is unvalidated).
- **Host validates** `ToolResult.output` against the profile's `output_schema_ref`
  before it reaches the model — making "the model knows exactly what comes out"
  an enforced invariant. A validation miss is a recoverable `ToolError::Failed`.
- **Cache** the resolution work in `ScopedToolState` (`user_id → display_name`,
  `id → access_hash`) to amortize the extra lookups.
- **Boundary:** framework owns the types + output validation + the cache
  primitive + conformance; the adapter does the fetch/resolve/fill.

---

## 11. Rollout / migration

1. **Framework foundation.** Ship the messaging `CapabilityProfileContract`s +
   schema assets; add the `[messaging]` reader + expander to
   `ironclaw_extensions`; wire `implements`/`output_schema_ref` + conformance +
   host-side output validation.
2. **Slack — parity + one enrichment.** Replace the five `[[tools]]` with a
   `[messaging]` block; the expander emits the **same capability ids** (no model
   regression); the adapter now returns normalized output (authors → `UserRef`,
   folding the `get_user_info` round-trip). Extends the existing Slack tool
   integration test.
3. **Telegram — add user-acting tools.** Build the pairing→session flow (§8) and
   the host-side MTProto client (§7); declare `[messaging]`; implement `invoke`
   reusing the crate's rendering core. Tool *definitions* are free; the `invoke`
   + client + session are the real work.
4. **Discord / future — the addition test.** A new package declares its subset;
   no generic source changes.

## 12. Testing

- **One messaging conformance suite, vendors as rows** (mirroring
  `crates/ironclaw_auth/tests/auth_engine_contract.rs`): given an adapter claiming
  a profile + a scripted vendor backend, assert each declared tool honors the
  input schema and returns **schema-valid, normalized** output (ids resolved).
  Slack, Telegram, and the `acme-messenger` fixture run it.
- **Structural profile conformance** in the resolver/activation tests.
- **Integration proof** through the production dispatcher (activate the real Slack
  package; invoke `slack.send_message`/`slack.read_history`; assert output
  validates and no Slack branch exists in dispatch).
- Repo law: test-first, integration tier for production-wired behavior, both DB
  backends where state persists.

## 13. Open questions

1. **Pairing as a credential mechanism.** Third auth-engine method vs. a separate
   `manual_token` pairing modality; what a paired Telegram tool actually injects
   (session vs. bearer). (`adr/0002` open Q2.)
2. **Host-side Telegram client** hosting: TDLib vs. a native MTProto lib; process
   model (one long-lived client per paired user); session persistence + security
   (the auth key is a full-account credential — encryption at rest, revocation,
   "active session" hygiene).
3. **Custom / vendor emoji** in `Reaction.emoji` and `add_reaction`: normalize to
   Unicode, `:shortcode:`, or a tagged union for custom/guild emoji ids?
4. **Cross-conversation "recent messages."** There is no single "my last N
   messages across all chats" primitive; it is a composition
   (`list_conversations` → `read_history` per chat). Do we expose a convenience
   profile, or leave it to the model to compose?
5. **`text` normalization fidelity** — how far to normalize vendor formatting
   (mentions, links, custom entities) into Markdown without losing round-trip
   fidelity for `edit_message`.
6. **Output-schema versioning** — how `types.v1` → `types.v2` rolls without a
   wire break for already-installed extensions.
