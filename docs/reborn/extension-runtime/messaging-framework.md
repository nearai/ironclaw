# Messaging Tool Framework — Engineering Design

**Status:** Proposed (2026-07-14). New feature work on the finished extension
runtime, sequenced **after** the P0–P7 train.
**Companions:** `adr/0002-messaging-tool-framework.md` (the *decision*, rationale,
and rejected options); `messaging-framework-tools.md` (**canonical** tool
schemas + descriptions + the Slack migration-parity audit);
`messaging-framework-checklist.md` (acceptance).
**Reading order:** `overview.md` (runtime model) → `adr/0002` (why) → this (how) →
`messaging-framework-checklist.md` (done-when).

---

## 1. Summary

A **standard, vendor-neutral set of messaging tools** — send, read, list, search,
edit, delete, react — is defined **once**, as host-owned **capability profiles**
with normalized input/output schemas. An extension does **not** hand-write these
tools; it **declares the subset it supports** in a small `[messaging]` manifest
section. At install time the host **expands** that declaration into ordinary
per-extension tool surfaces (`slack.send_message`, `slack.read_history`, …),
which flow through the existing resolver / dispatcher / disclosure / UI unchanged.
Each extension's tool adapter implements the per-vendor behavior and **normalizes
its output** to the shared schema (raw `U0123` → `{id, display_name}`).

Three invariants frame everything:

- **The messaging tools act as the *user-acquired identity*** — the Matrix
  bridge term is **puppeting** (Slack's OAuth user token), never the bot. The bot is only the **channel** (inbound + the assistant's
  replies, owned by the delivery coordinator; the bridge term is **relaying**).
  See §3.
- **The recipient decides the surface** (§3.1): anything to *you* is a channel
  relay (no tool); anything to *someone else* is a puppeting tool call. This is a
  host-enforced confidentiality guarantee, not a convention.
- **The model sees one contract per tool, regardless of vendor.** Same schema in,
  same schema out, regardless of the vendor the adapter drives.

### 1.1 Design intent — build generic, implement Slack (the addition test)

**Slack is the only consumer now, but the framework must be built fully general —
so adding the *next* vendor is a manifest declaration plus a thin adapter, not a
second integration.** This is the bar the implementing engineer is held to, not
an aspiration:

- **The addition test.** Adding a new HTTP vendor (e.g. Discord) = a `[messaging]`
  block naming its subset + one `ToolAdapter` that does `match capability_id →
  call the vendor API → normalize`, with **zero change to any generic crate**. If
  adding a vendor requires touching generic code, the abstraction leaked and the
  design failed.
- **Slack goes *through* the generic seams, never around them.** The profiles,
  expander, normalized types, output validation, dispatch pipeline, relay/act
  enforcement, discovery, and the shared `messaging_core` pattern are the generic
  infrastructure; Slack is their first *user*, not a shortcut. Everything
  Slack-specific lives in the Slack adapter; nothing Slack-shaped leaks upward.
- **What every future vendor reuses for free:** all of the above. **The only
  per-vendor work:** the `[messaging]` subset + the adapter's `invoke` (the API
  calls + the normalization mapping).
- **Proven, not asserted** — three enforcers (§15): the **`acme-messenger`
  fixture** (an invented vendor that drives every generic path, so no generic path
  can secretly depend on a real product), the **genericity gate** (no vendor id /
  Slack-ism in generic crates), and the **conformance suite** (vendors as rows).
  Build these *alongside* Slack, not after.
- **The one caveat.** "Trivial" holds for an HTTP vendor. A vendor whose user API
  is **not** HTTP (Telegram = MTProto) additionally needs a host-side transport
  client (§8) — the single non-trivial part, and a property of that vendor's
  protocol, not a gap in the framework: it still reuses every generic piece above.

---

## 2. Prior art and rationale

The design was validated against a deep prior-art pass (adversarially verified;
`adr/0002` cites the sources). The load-bearing findings:

- **The id/display split is universal.** Matrix identifies users by an immutable
  `@user:homeserver` MXID with a *separate* mutable display name, and rooms by an
  immutable `!id` distinct from alias/title — exactly `UserRef{id, display_name}`
  and `ConversationRef{id, title}`.
- **Message ids are conversation-scoped, not global.** Modern Matrix scopes event
  ids per-room; a "globally unique" claim was refuted. This validates
  `MessageRef{id, conversation}` — carry the conversation, never assume global
  uniqueness.
- **mautrix `bridgev2` is the structural twin.** Each remote conversation → a
  **Portal**, each remote user → a **Ghost**, keyed by network-scoped id newtypes
  (`PortalKey`, `UserID`, `MessageID`) with a durable Matrix↔remote message-id
  map — the same shape as `ConversationRef`/`UserRef`/`MessageRef`.
- **The bot-vs-user split is first-class, named prior art.** `bridgev2` separates
  a `Ghost` (a puppet for a remote user *not* logged in) from a `UserLogin` (the
  user's *own* authenticated session). Two independent projects (mautrix,
  mx-puppet-bridge) converge on the **puppeting** (act-as-user) vs **relaying**
  (act-as-bot) vocabulary — adopted here. Matrix's own fidelity spectrum ranks the
  pure single-bot approach as the *worst* option ("loses all metadata about
  messages and senders"), which is exactly what the puppeting tools avoid.
- **Capability gaps are handled by composition + advertisement**, never a fat
  interface: mautrix uses optional per-capability sub-interfaces plus runtime
  capability signaling. Our manifest **declared-subset** (`tools = […]`) is the
  *static* form of that advertisement — and it is strictly better for an LLM
  caller, because an unsupported tool is simply **absent** as a surface (the model
  cannot call it and get a "not supported" error). Independently corroborated by
  the harness scan (NullClaw's nullable channel VTable methods).
- **Capability is gated by identity, not just platform.** A Slack *bot* token
  cannot search or read uninvited channels; a *user* token can. A second
  ecosystem confirming that the read-rich surface belongs to the puppeting
  identity, not the bot.
- **Connect flows generalize to steps.** mautrix models every login (OAuth-ish,
  QR/device-link, token paste, shared-secret) as one **step-based** `LoginProcess`
  rather than per-vendor methods — the steer behind §9's connect design.

**Deliberate divergences from the prior art:**

- **No bot-relay fallback (§3.1).** mautrix falls back to a name-prefixed *bot
  relay* when a user hasn't authenticated. We **gate** (raise the connect prompt)
  instead: for a personal assistant, a bot-attributed outward message ("IronClaw,
  on behalf of Ben") is worse than none.
- **Per-operation tools with a *normalized* schema.** The verified agent-tool
  sample (a Slack MCP) favors per-operation tools that thinly wrap *native* APIs.
  We keep the per-operation granularity but add the bridge world's cross-platform
  **normalization** on top — a synthesis neither camp does alone.

**Honest coverage note:** the managed agent-tool platforms (Composio, Arcade,
Pipedream) and legacy stacks (libpurple, TDLib, XMPP) did **not** survive
verification, so this design leans on the Matrix/mautrix bridge world plus one
Slack MCP. Treat the "per-app vs unified" reading as directional, not settled.

---

## 3. The two identities — puppeting vs relaying

| | Channel surface (`[channel]`) — *relaying* | Messaging tools (`[messaging]`) — *puppeting* |
| --- | --- | --- |
| Acts as | the **bot** | the **user** (user-acquired identity) |
| Direction | inbound events + the assistant's replies | model-initiated actions *as the user* |
| Owner | delivery coordinator (`overview.md` §5.4) | the tool dispatcher (`overview.md` §5.2) |
| Credential | bot token (`slack_bot_token`) | OAuth user token (`slack_user_token`) |

An extension may declare `[channel]` only (a bot entrypoint, no user-acting
tools), `[messaging]` only (act-as-user tools with no inbound bot), or both
(Slack).

### 3.1 The relay/act boundary — decided by the recipient (CRITICAL)

The channel-vs-tools split is a **correctness and confidentiality guarantee**,
and the rule that decides which surface handles a message is simply **who the
recipient is:**

- **Recipient = you (the owner/requester)** → the **channel** delivers it (relay:
  the Slack bot, WebUI). Results, summaries, notifications,
  automation output, "send me…", "DM me…" — all of it. The model does **not** call
  a tool for this: it produces the answer and ends the turn, and the host delivers
  it back to where the request came from (or your saved notification target).
- **Recipient = anyone else — another person or a channel** → the **messaging
  tools** send it, **as you** ("DM Sergey", "post to #announcements").

Two **hard constraints** fall out, both host-enforced (wiring in §12):

- **A. The messaging tools never send to you.** You cannot be the recipient of an
  act-as-you message — that is always a relay, which is the channel's job. A
  `send_message` whose recipient is you / your own conversation is **blocked**.
- **B. The channel/bot is never a sender to a third party.** The bot only ever
  delivers to *you* — as a reply to your own request (where it came from) or a
  notification to your target. It never initiates a message to someone else,
  never posts to a channel on your behalf, never sends *as you*. **Outward-facing
  is always from you, via the tools.**

The concrete bugs this kills:

- **Duplicate send.** "Send me a DM with XYZ" previously became *both* a
  `send_message` self-DM (as you) *and* the channel relay — you received it twice.
  Constraint A blocks the self-send, so only the single channel relay happens.
- **Self-DM / leak.** A private "summarize this channel and show me the bugs"
  answer has recipient = you, so it is a channel relay, never a post to the source
  channel — and the bot cannot post it either (constraint B).

A *legitimate* outward send (to someone else) stays `default_permission = "ask"`,
with the approval plainly naming **what**, **to whom/where** ("#eng-bugs — a public
channel"), and **as whom** ("as you"). In an **automation** (routine/heartbeat)
there is no one to approve, so an act-as-you send is **denied unless the routine
was set up with that target ahead of time** — automations still relay results to
you via the channel freely.

**When the user identity is not connected**, an outward send **gates** (raises the
connect prompt, §9) — it does **not** fall back to a bot relay (the
deliberate divergence from mautrix, §2). Real identity or nothing.

All of this lives in the shared **coordinator + dispatch** pipeline over the
owner's identity, so **every channel behaves identically** — it is what the two
surfaces *mean*, not per-vendor logic. Pinned by a
cross-channel conformance test (§15).

*Product default (§16):* invoking IronClaw **in a shared channel** replies in that
channel (as the bot) via "reply where it came from" — still a reply to your
request, not a bot-to-third-party send. A config knob offers a stricter variant
that always DMs you privately and puts nothing in shared spaces.

---

## 4. Data model — the framework-owned normalized types

> **Canonical:** `messaging-framework-tools.md` §1 holds the authoritative
> `types.v1` (with the audit-driven `ConversationRef.private` and
> `Message.permalink` additions and the `UserRef.display_name` mapping rule). The
> block below is the illustrative core; where they differ, the tools doc wins.

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
        "id":           { "type": "string", "description": "Message id — unique WITHIN its conversation, not globally (e.g. Slack ts)." },
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

`MessageRef` is a named type (the *address* of a message: id + its conversation)
distinct from `Message` (the *content*). Reads return `Message`s; writes return a
`MessageRef`. It could be flattened to a bare `{id, conversation}` object — the
named type is a readability choice, not a requirement (§16).

---

## 5. The standard tools

> **Canonical:** each tool's exact model-facing **description**, **prompt doc**,
> and input/output schema — with the Slack migration-parity fixes (search `sort`
> + `permalink`, raised `read_history`/`list_conversations` limits,
> `read_history` `after`, preserved send safety prompt) — lives in
> `messaging-framework-tools.md` §2–8. The per-tool JSON below is the illustrative
> baseline; the tools doc is authoritative.

Each tool is a host-defined **capability profile** `ironclaw.messaging.<tool>.v1`
carrying an input and output schema. The **core** tools are the baseline of a
messaging integration — converse (`send_message`), observe (`read_history`,
`list_conversations`), and identify (`get_user`) — and every real chat platform
supports them, whether the tools act as the user (as Slack does) or, on a
bot-only platform, as a bot. The **optional** tools are richer or spottier and
appear only when the
extension declares them. **"Core" does not mean mandatory** — the manifest
declares any subset, so a read-only or send-only integration is valid; core is
the standard baseline plus a genericity signal. (Scope is chat platforms; an
SMS/email-style surface, which lacks a conversation list and reactions, would
revisit the tiers.)

| Tool | Profile id | Tier | `effects` | `default_permission` |
| --- | --- | --- | --- | --- |
| `send_message` | `ironclaw.messaging.send_message.v1` | **core** | `network`, `use_secret`, `external_write` | `ask` |
| `read_history` | `ironclaw.messaging.read_history.v1` | **core** | `network`, `use_secret` | `ask` |
| `list_conversations` | `ironclaw.messaging.list_conversations.v1` | **core** | `network`, `use_secret` | `ask` |
| `get_user` | `ironclaw.messaging.get_user.v1` | **core** | `network`, `use_secret` | `ask` |
| `search_messages` | `ironclaw.messaging.search_messages.v1` | optional — spotty (no Discord-bot search API) | `network`, `use_secret` | `ask` |
| `edit_message` | `ironclaw.messaging.edit_message.v1` | optional | `network`, `use_secret`, `external_write` | `ask` |
| `delete_message` | `ironclaw.messaging.delete_message.v1` | optional | `network`, `use_secret`, `external_write` | `ask` |
| `add_reaction` | `ironclaw.messaging.add_reaction.v1` | optional | `network`, `use_secret`, `external_write` | `ask` |
| `remove_reaction` | `ironclaw.messaging.remove_reaction.v1` | optional | `network`, `use_secret`, `external_write` | `ask` |

**Reply-in-thread is a parameter, not a tool** — the `thread` field on
`send_message`/`read_history`, accepted only when the extension declares
`supports_threads`. Schemas below reference the §4 `$defs` as
`types.v1#/$defs/<Type>`.

### 5.1 `send_message` (core)

> Model-as-user side effect (puppeting). Its prompt doc **must** state that it
> posts as the user and is **never** used to deliver the assistant's final reply
> (the host delivers replies on the channel surface — `overview.md` §5.4). It is
> `default_permission = "ask"`, and its approval plainly names the target and that
> it posts *as the user* (§3.1). Subject to constraint A: a send to the owner is
> blocked (§12).

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

### 5.2 `read_history`

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

### 5.3 `list_conversations`

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

### 5.4 `get_user`

```json
// input
{ "type": "object", "required": ["user"], "additionalProperties": false,
  "properties": { "user": { "type": "string", "description": "Opaque user id (e.g. from a Message author)." } } }
// output
{ "type": "object", "required": ["user"], "additionalProperties": false,
  "properties": { "user": { "$ref": "types.v1#/$defs/UserRef" } } }
```

### 5.5 `search_messages`

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

### 5.6 `edit_message` / `delete_message`

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

### 5.7 `add_reaction` / `remove_reaction`

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

## 6. How an extension opts in — the `[messaging]` section

### 6.1 The section

One `[messaging]` section per extension. It names the **subset of standard tools**
and the **user-acquired credential** (the puppeting identity) they run on.
Everything else (ids, schemas, descriptions, effects) comes from the framework.

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
(an `[auth.<vendor>]` recipe — §9).

### 6.2 Worked example — Slack (migrates today's five bespoke tools)

The block in §6.1 **is** the Slack opt-in. It expands to the same five capability
ids Slack ships today (`slack.send_message`, …) — so migration is parity (§14).

### 6.3 Other vendors (future)

Slack is the only consumer in scope now. A future messaging extension (e.g.
Discord) opts in the same way — a `[messaging]` block naming its subset and its
user-acquired credential — with **no change to the framework**; that reuse is the
genericity the profiles + expander buy. A vendor whose user-acting transport is
not HTTP (and so does not fit restricted egress) additionally needs its own
host-side client — out of scope until such a vendor is built (§8).

### 6.4 What the host does with it — expansion into ordinary tool surfaces

At install/upgrade, a **messaging expander** (in the manifest resolver, sibling to
the MCP discovery loader) turns each `tools` entry into an ordinary
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

### 6.5 The capability-profile contract (the reuse hook)

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

## 7. The adapter contract (per-vendor behavior)

The extension implements exactly one method — the existing `ToolAdapter::invoke`
(`crates/ironclaw_host_api/src/tool_adapter.rs:94`); no new trait:

```rust
async fn invoke(&self, call: ToolCall, ports: &ToolPorts<'_>) -> Result<ToolResult, ToolError>;
```

For each `<ext>.<tool>` capability id the adapter must:

1. **Route** by `call.capability_id` to the vendor operation (a `match`).
2. **Read** `call.input` (already schema-validated by the host).
3. **Do the work** through its transport (§8).
4. **Normalize** the vendor response into the profile's output shape — **resolving
   ids to `UserRef`** with `display_name` filled (even at the cost of an extra
   lookup), and mapping vendor conversation/message shapes to
   `ConversationRef`/`MessageRef`/`Message`.
5. **Return** `ToolResult.output` (validated host-side against the profile's
   output schema, §11). Recoverable failures are `ToolError::Failed`
   (model-visible, run continues); a missing/expired credential is
   `ToolError::AuthRequired` → the generic gate (`tool_adapter.rs:64`).

The adapter reports **behavior only** — never ids, schemas, or effects (those are
the resolved manifest). A vendor "messaging core" of pure functions (Markdown→
dialect rendering, splitting, target/DM formatting, error mapping) is **shared
intra-crate** between this `invoke` and the channel adapter's `deliver`
(`adr/0002` §6.2); reliability (retry/persistence/dedupe) stays coordinator-only.
Convergent with mautrix's optional-capability composition and NullClaw's nullable
channel methods (§2): capability lives in the declared surface, behavior in the
adapter.

### 7.1 Runtime kind — native `first_party`, not WASM (for a vendor with a channel)

Slack's tools today are a **WASM** module (`assets/slack/wasm-src/`,
`runtime.kind = "wasm"`). For the messaging framework, Slack's tools become a
**native `first_party` `ToolAdapter`** in `ironclaw_slack_extension`
(`runtime.kind = "first_party"`, `service = "slack.extension/v1"`), replacing the
WASM module. The deciding factor is the shared-core requirement, not capability:

- **Shared messaging core (§6.2, §7.4).** The tool `invoke` and the channel
  `deliver` must share vendor rendering / splitting / target-resolution / error
  mapping. WASM is a separate compilation unit and cannot call the native channel
  crate's functions, so a WASM tool would **re-implement** rendering — exactly the
  duplication this framework exists to remove. Native lets both adapters call one
  `messaging_core` module.
- **Normalization is multi-call orchestration.** Resolving author ids → names
  (§7.3) means fanning out extra calls and caching them — natural in native Rust
  over `ScopedToolState`.
- **Trust.** The WASM sandbox mainly protects against *untrusted* code; Slack is
  first-party (ships in the binary), so trading the sandbox for code-sharing is
  acceptable here.

WASM stays the right choice for a **pure-tool** extension (no channel, nothing to
share) or a third-party tool that must be sandboxed; the loader wraps it in a
generic `WasmToolAdapter` and the tool contract is byte-identical either way. So
the rule is: *a vendor that has both a channel and messaging tools implements them
natively in one crate; a tools-only or untrusted extension may stay WASM.*

### 7.2 The Slack adapter, worked

One adapter per extension, routing by capability id (sketch — illustrative, not
literal):

```rust
// crates/ironclaw_slack_extension/src/messaging.rs
pub struct SlackMessagingAdapter { cfg: SlackNonSecretConfig } // from bind(): team id, bot user id…

#[async_trait]
impl ToolAdapter for SlackMessagingAdapter {
    async fn invoke(&self, call: ToolCall, ports: &ToolPorts<'_>) -> Result<ToolResult, ToolError> {
        let egress = ports.egress.ok_or_else(|| internal("egress not granted"))?;
        match call.capability_id.as_str() {
            "slack.send_message"       => self.send_message(call.input, egress).await,
            "slack.read_history"       => self.read_history(call.input, egress, ports.state).await,
            "slack.list_conversations" => self.list_conversations(call.input, egress).await,
            "slack.get_user"           => self.get_user(call.input, egress, ports.state).await,
            "slack.search_messages"    => self.search_messages(call.input, egress, ports.state).await,
            "slack.edit_message" | "slack.delete_message"
            | "slack.add_reaction" | "slack.remove_reaction" => self.mutate(&call, egress).await,
            other => Err(internal(&format!("unrouted capability {other}"))),
        }
    }
}
```

Each per-op function is the same five-step shape (parse → build → egress → parse →
normalize). `send_message`:

```rust
async fn send_message(&self, input: Value, egress: &dyn RestrictedEgress) -> Result<ToolResult, ToolError> {
    let req: SendInput = serde_json::from_value(input)                       // host already schema-validated
        .map_err(|e| ToolError::InvalidInput { reason: e.to_string() })?;
    let channel = messaging_core::resolve_target(&req.conversation, egress).await?;  // #name → id if needed
    let text    = messaging_core::render_mrkdwn(&req.text);                          // shared with deliver()
    let body = json!({ "channel": channel, "text": text, "thread_ts": req.thread });
    let resp = egress.send(RestrictedEgressRequest {
        method: NetworkMethod::Post,
        url: "https://slack.com/api/chat.postMessage".into(),
        headers: vec![("content-type".into(), "application/json".into())],
        body: Some(body.to_string().into_bytes()),
        credential: Some(SecretHandle::new("slack_user_token")?),   // host injects the bytes
    }).await.map_err(messaging_core::map_egress_error)?;
    let parsed = messaging_core::parse_slack_ok(&resp.body)?;        // ok:false → ToolError::Failed
    Ok(ToolResult::json(json!({ "message": { "id": parsed.ts, "conversation": parsed.channel } })))
}
```

The adapter **never holds the token** — it names the `slack_user_token` handle and
the host injects it during restricted egress (`tool_adapter.rs` `RestrictedEgress`).

### 7.3 Normalization — resolving ids to names

`read_history` is where the one real behavior addition lives. Slack returns
messages with `user: "U0123"`; the profile requires `Message.author: UserRef{id,
display_name}`. So the function:

1. `conversations.history` → raw messages.
2. Collect the **distinct** author ids.
3. Resolve each to a display name: check the `ScopedToolState` cache
   (`user:<id>` → name) first; for misses, `users.info` (concurrent), then write
   the cache. Mapping rule per `messaging-framework-tools.md` §5.
4. Build `Message`s with resolved authors; validate host-side (§11).

The cache amortizes the extra calls across the turn/conversation; a resolution
miss degrades gracefully (id as `display_name`) rather than failing the call.

### 7.4 The shared vendor "messaging core"

A `messaging_core` module inside `ironclaw_slack_extension` holds the pure,
credential-agnostic vendor mechanics, called by **both** `SlackMessagingAdapter`
and `SlackChannelAdapter::deliver`:

| Function | Used by tool | Used by channel |
| --- | --- | --- |
| `render_mrkdwn(md) -> String` | send/edit | deliver |
| `parse_mrkdwn(s) -> String` (→ normalized Markdown) | read/search | inbound |
| `split(text, max) -> Vec<String>` | send | deliver |
| `resolve_target(ref, egress)` (name→id, `conversations.open` for DMs) | send | deliver target |
| `map_egress_error` / `parse_slack_ok` | all | deliver |

Today these are `pub(crate)` in the channel crate (`mrkdwn.rs`,
`preference_targets.rs`, `delivery.rs`); the migration **promotes them into
`messaging_core`** and points both adapters at it (`adr/0002` §6.2). Reliability
(retry / attempt persistence / dedupe / sole-writer) stays in the delivery
coordinator and is **not** in the core — the tool path is a one-shot dispatch.

### 7.5 Wiring — how the adapter reaches the runtime

No generic crate names Slack; the binary assembles it:

1. `ironclaw_reborn_cli` registers a **native factory** mapping
   `runtime.service = "slack.extension/v1"` → a factory that builds the Slack
   `ExtensionEntrypoint` (the only generic-side crate allowed to link the concrete
   extension crate — `implementation.md` dependency rule).
2. At activation, `ExtensionEntrypoint::bind(ctx)` returns
   `ExtensionBindings { tools: Some(Arc<SlackMessagingAdapter>),
   channel: Some(Arc<SlackChannelAdapter>) }` — both sharing `messaging_core`.
   `bind` receives only installation context + **non-secret** config; no ports.
3. The binding rule checks declared↔bound (manifest has `[messaging]` ⇒ `tools`
   must be `Some`; `[channel]` ⇒ `channel` must be `Some`).
4. The host prebinds the adapter into the active snapshot; a call resolves via
   `ToolResolver::resolve("slack.read_history")` → the prebound adapter (no
   per-invocation construction), then runs the dispatcher pipeline (§ overview
   5.2) and `invoke`.

---

## 8. Transport — HTTP via restricted egress

The Slack messaging adapter uses the existing host port `RestrictedEgress`
(`tool_adapter.rs:103`): scheme/host/method allowlist from the resolved contract,
host-side credential injection, response size caps. **No new transport
mechanism** — a messaging adapter looks like today's Slack tool, just native
(§7.1).

A future vendor whose user-acting API is **not** HTTP (and so does not fit
`RestrictedEgress`) would need its own **host-side client** behind a narrow
adapter-facing port (the runtime's "add a hook when a vendor defeats the
descriptor" rule, overview §4.3). That is **out of scope** here — Slack is HTTP —
and is noted only so the seam is understood: the tool *contract* is
transport-independent, so such a vendor later changes the adapter's transport, not
the profiles the model sees.

---

## 9. Identity, credentials, and connect

- **The messaging credential is the user-acquired (puppeting) identity**, distinct
  from the bot the channel delivers on. It is declared in `[[messaging.credentials]]`;
  for Slack it is the OAuth user token (`slack_user_token`).
- **Acquisition** rides the existing connect surface: Slack's `[auth.slack]`
  recipe + the auth engine (`overview.md` §4.3) — the flow that already yields
  `slack_user_token`, unchanged. (A future vendor with a different connect
  mechanism plugs into the same gate; generalizing the connect flow is deferred
  until such a vendor exists — §16.)
- **Gate + resume:** a tool call with a missing/expired grant returns
  `ToolError::AuthRequired`; the host raises the generic auth gate keyed by the
  tool's declared vendor and resumes the blocked turn on connect — unchanged from
  `overview.md` §5.2/§4.3. There is **no bot-relay fallback** (§3.1): unconnected →
  gate, never a bot-attributed send.

---

## 10. Discovery (anti-bloat)

The expanded tools are ordinary **`Discoverable`-tier** surfaces, so at scale the
existing progressive-disclosure system defers them behind `tool_search` and
surfaces them by name in its catalog index (`tool_disclosure.rs`; `adr/0002` §5).
The `<ext>.<tool>` naming makes the model's "what messaging tools does Slack have?"
answerable through the generic `tool_search → capability_info → call` flow. **No
messaging-specific discovery tool is added.** (Note: that disclosure layer is
production-wired but currently opt-in/off — orthogonal to this framework, which
must not fork it.)

---

## 11. Normalization & validation

- **Adapter normalizes** vendor specifics into the profile output shape — the one
  behavior addition over today (Slack currently returns raw ids;
  `assets/slack/schemas/slack/raw_output.v1.json` is unvalidated).
- **Host validates** `ToolResult.output` against the profile's `output_schema_ref`
  before it reaches the model — making "the model knows exactly what comes out" an
  enforced invariant. A validation miss is a recoverable `ToolError::Failed`.
- **Cache** the resolution work in `ScopedToolState` (`user_id → display_name`,
  `id → access_hash`) to amortize the extra lookups.
- **Boundary:** framework owns the types + output validation + the cache primitive
  + conformance; the adapter does the fetch/resolve/fill.

---

## 12. Enforcement wiring — the relay/act guarantee in code

The two hard constraints (§3.1) must be enforced host-side, not left to the model.
Where each hooks (exact call sites to be confirmed against live code — §16):

- **Constraint A (tools never send to the owner)** — a generic **policy step on
  the messaging write tools** in the dispatch pipeline (`ironclaw_dispatcher`),
  ahead of `ToolAdapter::invoke`. It resolves the tool's target and compares it to
  the owner's own conversation / reply identity carried on the turn
  (`source_binding_ref` + `reply_target_binding_ref`,
  `crates/ironclaw_turns/src/request.rs:59`; the actor identity). Match → the tool
  returns a recoverable `ToolError::Failed` ("that is a relay — end the turn and
  answer; the host delivers it"). Applies to `send_message`, `edit_message`,
  `delete_message`, `add_reaction`, `remove_reaction`.
- **Constraint B (the channel never sends to a third party)** — enforced in the
  **delivery coordinator** (`crates/ironclaw_product_workflow/src/delivery_coordinator.rs`),
  which already resolves the delivery target from `reply_context`/preference and
  is the sole delivery-state writer. Its target set is constrained to *owner-only*
  destinations (reply-where-it-came-from or the owner's saved notification
  target); a third-party target is rejected, fail-closed.
- **Automation denial** — in proactive runs (routine/heartbeat) there is no live
  approver, so a messaging write tool's `ask` cannot resolve to "yes." The
  permission/approval path must **deny** act-as-user in a non-interactive context
  unless the routine pre-authorized that specific target. (The exact place where
  `ask` resolves under a routine is the least-verified point — §16.)
- **Connect gate, not relay** — an outward send with no connected user identity
  raises the connect gate (§9), never a bot relay.

This is generic (dispatch + coordinator), not per-vendor, so every channel behaves
identically. Pinned by the cross-channel conformance test (§15).

---

## 13. Crate-by-crate change map

| Crate | Change | New / seam exists |
| --- | --- | --- |
| `ironclaw_host_api` | Messaging `CapabilityProfileContract`s + the `types.v1` normalized schemas; the `MessagingToolId` vocabulary | scaffolding exists (`capability_profile.rs`), profiles new |
| `ironclaw_extensions` | `[messaging]` reader; the **expander** (declaration → `CapabilityDeclV2`); re-wire `implements`/`output_schema_ref` (v3 currently drops them); validation | expander new; `CapabilityDeclV2` exists |
| `ironclaw_capabilities` | Wire `evaluate_profile_conformance` into activation | evaluator exists, unwired |
| `ironclaw_dispatcher` | Host-side **output-schema validation** on `ToolResult`; **constraint-A policy step** on messaging write tools | new steps in existing pipeline |
| `ironclaw_product_workflow` | **Constraint-B** owner-only target enforcement in the delivery coordinator; automation-denial for act-as-user | coordinator exists |
| `ironclaw_slack_extension` | Migrate 5 tools → `[messaging]`; **native `ToolAdapter`** replacing the WASM module (§7.1); `invoke` normalizes output; extract shared `messaging_core` from `pub(crate)` helpers (§7.4) | crate exists |
| `ironclaw_architecture` | Genericity gate: no messaging tool id / vendor name leaks into generic crates | gate pattern exists |

Composition stays assembly-only (`overview.md` §3.3 discipline); no messaging tool
id or vendor name appears in a generic crate.

---

## 14. Phased execution order

Each phase lands green (`cargo fmt`, `clippy` zero-warnings, `cargo test` +
integration where touched, `cargo test -p ironclaw_architecture`).

| Phase | Content | Depends on |
| --- | --- | --- |
| **M0** | Profiles + `types.v1` schemas + `[messaging]` reader/expander + `implements`/`output_schema_ref` wiring + conformance + host output validation. Fixture (`acme-messenger`) declares `[messaging]` and passes the conformance suite. | — |
| **M1** | **Relay/act guarantee** — constraint-A dispatch step, constraint-B coordinator enforcement, automation denial, connect-gate-not-relay. Cross-channel conformance test green (the CRITICAL safety gate). | M0 |
| **M2** | Slack migration — 5 tools → `[messaging]`, normalized output (authors → `UserRef`), parity test (same capability ids); folds the `get_user_info` round-trip. | M0, M1 |
| **M3** | Genericity gate to zero; docs; checklist fully evidenced. A future vendor (e.g. Discord) remains a pure addition test — no generic change. | M2 |

M1 is sequenced early and on the critical path deliberately: the safety guarantee
ships **before** any real puppeting tool is broadly enabled.

---

## 15. Testing

- **One messaging conformance suite, vendors as rows** (mirroring
  `crates/ironclaw_auth/tests/auth_engine_contract.rs`): given an adapter claiming
  a profile + a scripted vendor backend, assert each declared tool honors the
  input schema and returns **schema-valid, normalized** output (ids resolved).
  Slack and the `acme-messenger` fixture run it.
- **Structural profile conformance** in the resolver/activation tests: a mismatched
  schema ref or missing operation fails activation.
- **Integration proof** through the production dispatcher (activate the real Slack
  package; invoke `slack.send_message`/`slack.read_history`; assert output
  validates and no Slack branch exists in dispatch).
- **Relay/act boundary conformance (CRITICAL), cross-channel** (§3.1, §12) — the
  same assertions for every messaging extension + the `acme-messenger` fixture:
  - "summarize `<conversation>` and send me the results" — and "send me a DM with
    XYZ" ⇒ the run relays **once** via the channel and makes **no** `send_message`
    call (no duplicate, no self-send);
  - a `send_message` whose recipient is the owner / their own conversation ⇒
    **blocked** (constraint A);
  - the channel/coordinator never delivers to a third-party recipient (constraint
    B);
  - a legitimate outward `send_message` (to someone else) ⇒ `ask`, approval names
    the target + "as you";
  - a **proactive** (routine/heartbeat) run's act-as-user send ⇒ denied unless
    pre-authorized;
  - an outward send with no connected identity ⇒ connect gate, **not** a bot
    relay. LLM-trace / integration tests, identical per channel.
- Repo law: test-first, integration tier for production-wired behavior, both DB
  backends where state persists.

---

## 16. Decisions and open questions

**Decided defaults (revisit if evidence warrants):**

- **D1. `MessageRef` stays a named type** ({id, conversation}) — validated by
  prior art as conversation-scoped. May be flattened to a bare object later; no
  behavior change.
- **D2. `edit`/`delete`/`react` stay optional**, not core. All three chat
  platforms support them, but they are higher-stakes `external_write` mutations
  and the first to disappear on simpler surfaces. Promotion is a one-line move.
- **D3. Shared-channel invocation replies in-channel** (as the bot; "reply where
  it came from"), with a config knob for the strict always-DM variant.

**Open questions:**

1. **Custom / vendor emoji** in `Reaction.emoji` and `add_reaction`: normalize to
   Unicode, `:shortcode:`, or a tagged union for custom/guild emoji ids?
2. **Cross-conversation "recent messages."** There is no single "my last N
   messages across all chats" primitive; it is a composition (`list_conversations`
   → `read_history` per chat). Do we expose a convenience profile, or leave it to
   the model to compose?
3. **`text` normalization fidelity** — how far to normalize vendor formatting
   (mentions, links, custom entities) into Markdown without losing round-trip
   fidelity for `edit_message`.
4. **Output-schema versioning** — how `types.v1` → `types.v2` rolls without a wire
   break for already-installed extensions.
5. **Relay/act enforcement wiring (§12).** Confirm the exact call sites for
   constraint A (dispatch), constraint B (coordinator), and especially how `ask`
   resolves in a non-interactive routine (the automation-denial rule) — the
   least-verified point. Verify none silently regress the duplicate / self-send /
   leak cases.
6. **Coverage gap from the prior-art pass** (§2): the managed agent-tool platforms
   (Composio, Arcade, Pipedream) and legacy stacks (libpurple, TDLib, XMPP) were
   not verified. A targeted follow-up would confirm whether any do cross-platform
   normalization worth borrowing.
