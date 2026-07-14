# ADR 0002 — Messaging tool framework (vendor-neutral tool profiles)

**Status:** Proposed (2026-07-14). New feature work on the finished generic
runtime — a dedicated effort **after the P0–P7 train**, in the same shape as
`adr/0001-multiple-accounts-per-vendor.md` (accepted post-train follow-up). It
is not part of the closing train and must not be stacked onto it.

**Concrete design:** `../messaging-framework.md` — the engineering design (standard
tools, full input/output schemas, the `[messaging]` opt-in, transport, the
enforcement wiring, the crate map, and phasing). **Acceptance:**
`../messaging-framework-checklist.md`. This ADR holds the decision, rationale, and
rejected options; those companions hold the how and the done-when.

**Trigger citation (required by `implementation.md` §14):** `overview.md` §7
excluded a **"Generic 'dynamic tools' abstraction"** with the reason *"MCP is
the only dynamic source; one `[mcp]` section, owned by the MCP loader"* and the
revisit trigger *"a second, non-MCP discovery source is real."* This ADR is
that revisit. The messaging framework is a **second, non-MCP tool source** — a
manifest-declared, host-expanded set of standard messaging tools. It is
narrower than the machinery the fence feared: it is **static** (the tool set is
known at authoring time from the manifest, not discovered at runtime like MCP's
`tools/list`), so it reintroduces no runtime-discovery abstraction. And it is
**not a parallel mechanism**: it instantiates the already-present but dormant
**capability-profile** scaffolding (`ironclaw_host_api/src/capability_profile.rs`,
`ironclaw_capabilities/src/conformance.rs`) whose own module docs say it exists
so *"extensions may later claim that their provider-prefixed capabilities
implement these operations."* This is that "later."

---

## Context — the problem this solves

Every messaging integration hand-rolls its own near-identical read/write tools.
Slack today ships five bespoke tools as a WASM module
(`crates/ironclaw_first_party_extensions/assets/slack/manifest.toml`:
`slack.search_messages`, `slack.list_conversations`,
`slack.get_conversation_history`, `slack.get_user_info`, `slack.send_message`).
Duplicating that shape per vendor gives the model N×M near-identical tools and
duplicates the hard part — the schemas, descriptions, and normalization — in
every extension. Two distinct costs hide here, and they need different fixes:

1. **Authoring duplication.** Each vendor re-writes the same send/read/react
   tool definitions. → fixed by a shared, host-owned tool contract (this ADR).
2. **Model-facing bloat.** The model sees every duplicate flat in its tool
   list. → already fixed by the runtime's progressive-disclosure system
   (`crates/ironclaw_reborn/src/tool_disclosure.rs`); the framework must reuse
   it, not reinvent it.

A third defect is specific and concrete: **outputs are not normalized.** Slack's
declared output schema is
`assets/slack/schemas/slack/raw_output.v1.json` = `additionalProperties: true`
("Slack API response serialized by the WASM tool"), and the tool returns raw
Slack shapes — `HistoryMessage.user` / `SearchMatch.user` are raw `U…` ids,
never resolved to names (`assets/slack/wasm-src/src/api.rs:182,103`). The
separate `get_user_info` tool exists precisely so the model can resolve those
ids by hand. The framework folds that round-trip away by making `read_history`
return an author already resolved to `{id, display_name}`.

**The two-identity model — the shape every messaging integration takes.** An
integration is not one identity but two, and the framework's tools attach to the
*second*:

- a **bot / channel identity** — the *entrypoint*: it receives inbound messages
  and delivers the assistant's replies (the delivery coordinator), on a **bot**
  credential (Slack bot token, Telegram bot token);
- a **user-acquired identity** — a connect flow that lets the system *act as the
  user*: Slack's **OAuth** (yielding `slack_user_token`), Telegram's **pairing**
  (a code the user gets from the bot and pastes — the existing `manual_token`
  pairing modality: the `Pairing { .. }` lifecycle gate,
  `crates/ironclaw_product_workflow/src/lifecycle.rs:157`; the `PairingRequired`
  event, `crates/ironclaw_common/src/event.rs:163`; the Telegram pairing-code
  card, `crates/ironclaw_product_adapters/src/outbound.rs:1027,1741`).

**Messaging tools always act as the user-acquired identity, never the bot.**
Slack already proves it: `slack.send_message` posts on the *user* token while
the channel delivers replies on the *bot* token (`assets/slack/manifest.toml`;
adapter bot handle `channel.rs:33`). So **Telegram is a full peer to Slack**:
its bot is only the channel entrypoint, and once paired its tools act as the
*user* with a rich surface (read and search their own chats) — **not** the
capability-limited Bot API. Conversely, a platform that offers no sanctioned
user-acting identity is legitimately **channel-only** — it declares `[channel]`
and no messaging tools. (An earlier draft of this ADR mis-modeled Telegram as a
bot-limited, tool-poor channel; that was wrong — the bot is only the entrypoint,
exactly as the Slack bot is.)

This is the same move the runtime already made for **auth**: a generic host
engine executes manifest-declared **recipe data** with zero per-vendor code
(`overview.md` §4.3). The messaging analogue is subtler and is stated up front
so the rest of the ADR is read correctly:

> **Auth had zero per-vendor code because vendors differ only in *parameters*,
> never in *flow behavior*.** Messaging tools differ in *both*: the tool set,
> schemas, and I/O contract are identical across vendors (so they become
> host-owned recipe/profile data), but the *behavior* — one vendor's REST call
> plus id→name resolution vs. another's entirely different API and transport — is
> genuinely vendor-specific. So the framework templates the tool **definitions**
> (like auth), and **reuses the existing `ToolAdapter::invoke` seam** for the
> per-vendor behavior. It does **not** add an auth-style "zero-code" engine for
> tools; there is no generic way to *do* the send.

---

## Decision (summary)

Framing (see §Context): every messaging integration has **two identities** — a
bot *channel* (the entrypoint and the assistant's replies) and a
**user-acquired** identity via a connect flow (Slack OAuth, Telegram pairing).
**The messaging tools act as the user-acquired identity, never the bot**; the
channel is a separate surface. On that footing:

1. **A standard, host-defined messaging tool set**, expressed as
   **capability-profile contracts** (`CapabilityProfileContract`,
   `ironclaw_host_api/src/capability_profile.rs:206`) — one per standard tool,
   each with a normalized input/output schema. The **core** four —
   `send_message`, `read_history`, `list_conversations`, `get_user` (converse +
   observe + identify) — are supported by every real chat platform; the rest are
   **optional** and platform-declared. "Core" is a baseline/genericity signal,
   not a mandate — the manifest declares any subset.
2. **The manifest declares the subset** via a compact `[messaging]` recipe
   (sibling to `[channel]`/`[auth.*]`) listing which profiles the extension
   implements. A resolve-time **expander** turns each into an ordinary
   `CapabilityDeclV2` tool surface (`crates/ironclaw_extensions/src/v2.rs:455`),
   so downstream (resolver, dispatcher, disclosure, UI) sees nothing new — the
   MCP precedent, applied statically ("past activation there is no MCP anywhere
   in the dispatch path," `overview.md` §3.1).
3. **The I/O contract is normalized and vendor-neutral** — `UserRef`,
   `ConversationRef`, `MessageRef` — owned by the profile schemas, not the
   extension.
4. **The adapter normalizes vendor specifics** to satisfy the contract; the
   framework *enforces* it with host-side output-schema validation (re-enabling
   `output_schema_ref`, which v3 dropped) and a scoped cache for the extra
   lookups.
5. **Anti-bloat reuses the existing tool-disclosure system** (`tool_search` →
   `capability_info` → direct call); the framework adds **no** messaging-specific
   discovery tool.
6. **The messaging tools and the delivery coordinator share a per-vendor
   "messaging core"** (rendering, splitting, target/DM formatting, error
   mapping) *inside the extension crate*, credential-parameterized — but not the
   coordinator's sole-writer reliability layer.
7. **A hard relay/act boundary, decided by the recipient (host-enforced,
   uniform):** to *you* → the channel relays (no tool); to *anyone else* → the
   tools send *as you*, `ask`-gated. The tools never send to you (blocking the
   duplicate self-send) and the bot never sends to a third party; automations
   deny act-as-you without pre-authorization (§6.4).

---

## 1. The standard messaging tool set — core vs. optional

Derived from the real Slack tools (acting as the OAuth **user**) and Telegram's
user surface (acting as the **paired user**), then pressure-tested against a
**bot-only** platform (Discord, where user automation is against ToS). The
pressure test is the whole point: the supported subset varies, which is *why*
the manifest must declare it. The vendor column shows which *identity* performs
the operation (§Context, "the two-identity model").

| Standard tool | Purpose (abstract) | Slack (OAuth user) | Telegram (paired user) | Discord (bot) | Tier |
| --- | --- | --- | --- | --- | --- |
| `send_message` | Post a message to a conversation | ✅ | ✅ | ✅ | **core** |
| `read_history` | Read recent messages of a conversation | ✅ | ✅ (own chats) | ✅ (perms) | **core** |
| `list_conversations` | Enumerate conversations the identity sees | ✅ | ✅ (dialogs) | ✅ | **core** |
| `get_user` | Resolve a user reference to profile info | ✅ | ✅ | ✅ | **core** |
| `search_messages` | Full-text search across messages | ✅ | ✅ | ❌ *no bot search API* | optional |
| `edit_message` | Edit a previously sent message | ✅ | ✅ | ✅ (own) | optional |
| `delete_message` | Delete a message | ✅ | ✅ | ✅ | optional |
| `add_reaction` | React to a message | ✅ | ✅ | ✅ | optional |

**The core is the baseline of a messaging integration — send, read, list,
identify — and all four hold across every real chat platform** (Slack-user,
Telegram-paired, *and* a Discord bot). What varies, and therefore stays optional
and manifest-declared, is: **search** (genuinely spotty — Slack ✅, Telegram ✅,
Discord-bot ❌, no bot search API); **mutations** (`edit_message`/`delete_message`)
and **reactions**, which are widely supported on chat platforms but are
higher-stakes writes (`external_write`) and the first to disappear on simpler
surfaces; plus **threads** (a gated param). Two things also drive the subset
independently of the tool itself: the **identity** (a user-acquired identity gets
the full surface; a bot-only platform like Discord is narrower), and the
extension's own choice (a deliberately read-only or send-only integration).
**"Core" is a baseline plus a genericity signal, not a mandate** — the manifest
declares any subset. So a Slack extension and a *paired* Telegram extension both
declare the full read-rich set; a Discord extension declares everything its bot
supports (all but `search_messages`).

**Reply-in-thread is a parameter, not a tool.** Slack already expresses it as
`thread_ts` on `send_message`, Telegram as `message_thread_id`, Discord via a
thread channel id. The framework models it as an optional `thread` field on
`send_message` (and on `read_history`), **gated by a `supports_threads` flag**
that extends the existing `[channel.presentation]` descriptor (Slack
`supports_threads=true`, Telegram `supports_threads=true`,
`assets/*/manifest.toml`). A dedicated `reply_in_thread` tool is deliberately
not minted.

---

## 2. The manifest recipe

### 2.1 Reuse the dormant capability-profile scaffolding

The runtime already contains the exact abstraction this needs, unused:

- `CapabilityProfileContract { id, required_operations }` and
  `CapabilityProfileOperationContract { id, input_schema_ref, output_schema_ref }`
  (`ironclaw_host_api/src/capability_profile.rs:169,206`) — a **host-defined,
  portable contract** with both input **and** output schemas per operation.
- `CapabilityDeclV2.implements: Vec<CapabilityProfileId>`
  (`crates/ironclaw_extensions/src/v2.rs:458`) — a tool declaring which
  profiles it implements.
- `crates/ironclaw_capabilities/src/conformance.rs` — a structural conformance
  evaluator (`ProfileIdMismatch`, `MissingRequiredOperation`,
  `InputSchemaRefMismatch`, `OutputSchemaRefMismatch`), described in its own
  doc as *"zero-behavior prep."*

Verified state of this scaffolding (it is genuinely dormant, not partly wired):
the v3 manifest tool reader `RawToolV3` (`crates/ironclaw_extensions/src/v3.rs:119`,
`deny_unknown_fields`) carries **neither** `implements` nor `output_schema_ref`;
the v3→resolved projection hard-codes `implements: Vec::new()` and
`output_schema_ref: None` (`v3.rs:362,368`); no first-party manifest declares
`implements`; and no crate outside `ironclaw_capabilities` consumes
`CapabilityProfileContract`/conformance. So the framework is the first concrete
profile family, and it is the change that wires the scaffolding from prep into
enforcement.

### 2.2 The `[messaging]` section (recommended)

The framework ships, beside the auth recipes in `ironclaw_host_api`, a set of
host-defined messaging `CapabilityProfileContract`s — `ironclaw.messaging.send_message.v1`,
`…read_history.v1`, etc. — each pointing at framework-owned normalized schemas.
The manifest then declares only the *subset*, compactly:

```toml
# ---- messaging tool surface: recipe data, framework-owned schemas ----------
[messaging]                          # at most one per extension (pairs with the single channel)
profiles = [                         # THE DECLARED SUBSET (which standard tools this extension exposes)
  "send_message",
  "read_history",
  "add_reaction",
]
default_permission = "ask"
supports_threads = true              # capability flag; gates the `thread` param on send_message/read_history

[[messaging.credentials]]            # reuses the existing v3 [[tools.credentials]] model verbatim
handle = "slack_user_token"          # the *user-acquired* identity (OAuth user token here; a pairing vendor names its pairing credential) — never the bot
vendor = "slack"
scopes = ["chat:write", "channels:history", "users:read", ...]
audience = { scheme = "https", host = "slack.com" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }
```

At resolve time a **messaging expander** (sibling to the MCP loader's
discovery and to the auth recipe's resolution) turns each declared profile into
an ordinary `CapabilityDeclV2`:

- `id = "<ext>.<profile>"` (e.g. `slack.send_message`) — **the current ids are
  preserved**, so migration is parity (§7);
- `implements = ["ironclaw.messaging.send_message.v1"]`;
- `input_schema_ref` / `output_schema_ref` = the **profile's** schemas
  (framework-owned, normalized);
- `description` / `prompt_doc_ref` = framework canon (an extension may override
  the wording — e.g. Slack's "acts as the user, never delivers the final
  answer" note — but not the schema);
- `effects` / `default_permission` = framework defaults per profile, treated as
  a ceiling the recipe may narrow but not widen (§6.4);
- `runtime_credentials` from `[[messaging.credentials]]`.

The expanded decls land in `ResolvedExtensionManifest.tools`
(`crates/ironclaw_extensions/src/resolved.rs:48`) exactly like static
`[[tools]]`. From there, **nothing downstream is new**: the active snapshot
indexes them (`crates/ironclaw_extension_host/src/active.rs:71`), the dispatcher
resolves them (`ToolResolver::resolve`, `crates/ironclaw_dispatcher/src/lib.rs:48`),
and the extension's single `ToolAdapter` routes the `<ext>.<profile>` capability
ids internally — which Slack's WASM module already does
(`assets/slack/wasm-src/src/lib.rs:134`, `action_from_context`).

### 2.3 Interaction with `[[tools]]`, `[channel]`, and the resolved contract

- **`[[tools]]`** stays for bespoke, non-standard tools. `[messaging]` is
  additive; an extension may declare both. The binding rule (`overview.md` §4.0)
  is unchanged — a manifest with `[messaging]` or `[[tools]]` still requires
  `bindings.tools = Some`.
- **`[channel]`** stays independent — it is the *other* identity (§Context).
  The messaging tools act as the **user-acquired** identity; the channel surface
  (the assistant's replies, delivery coordinator) acts as the **bot**. They are
  the two distinct surfaces `overview.md` §5.4 already separates. `[messaging]`
  **reuses** the channel's `[channel.presentation]` flags for capability gating
  but declares its **own** credential and egress — the user identity, which the
  bot channel never shares (Slack: `slack_user_token` via OAuth vs. the channel's
  `slack_bot_token`, `channel.rs:33`; Telegram: the pairing credential vs. the
  bot token). An extension may declare `[messaging]` with **no** `[channel]`
  (act-as-user actions with no inbound bot) **or** `[channel]` with **no**
  `[messaging]` (a bot entrypoint on a platform with no sanctioned user-acting
  identity — Telegram's state today).
- **The resolved contract** gains nothing structurally: profiles ride the
  existing `implements`/`output_schema_ref` fields; the widening diff
  (`diff_resolved_contracts`, `overview.md` §3.3) already classifies new
  effects/scopes/credentials on the expanded decls.

### 2.4 Alternative considered — pure `[[tools]]` + `implements`, no expander

Re-expose `implements` + `output_schema_ref` in the v3 `[[tools]]` reader (which
v3 dropped) and have extensions write one `[[tools]]` entry per messaging tool,
each declaring `implements = ["ironclaw.messaging.send_message.v1"]`. This is
the lowest-mechanism option — pure reuse of `implements` + profiles + conformance,
**no new section and no expander**. It was rejected as the *primary* only
because it keeps N per-tool entries of authoring boilerplate, which is exactly
the duplication the owner wants gone; the compact `[messaging]` recipe matches
the auth-recipe philosophy (terse data → host expansion) better. **The
profiles/conformance/output-validation core is identical either way** — the
expander is a thin convenience, and if it proves contentious against the fence,
this alternative is the fallback with no loss of the normalization payoff.

---

## 3. Normalized I/O contracts

The framework owns four reference types (the profile schemas); no extension
defines them. Ids stay vendor-opaque so they round-trip, but every reference the
model reads is **enriched** with resolved human context — the owner's
"`{id, display_name}`, not a raw `U012ABC`."

```
UserRef         { id: string, display_name: string, is_bot?: bool }
ConversationRef { id: string, kind: "channel" | "dm" | "group", name?: string }
MessageRef      { id: string, conversation: ConversationRef, thread?: ThreadRef }
ThreadRef       { id: string }
Message         { ref: MessageRef, author: UserRef, text: string,
                  posted_at: string /* RFC 3339 */, thread?: ThreadRef, reactions?: Reaction[] }
```

**Example — `send_message` input** (`ironclaw.messaging.send_message.v1`,
input schema):

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "messaging.send_message input",
  "type": "object",
  "required": ["conversation", "text"],
  "properties": {
    "conversation": { "type": "string", "description": "ConversationRef id (from list_conversations / an inbound message)." },
    "text":         { "type": "string", "description": "Message body in normalized Markdown; the adapter renders to the vendor dialect." },
    "thread":       { "type": "string", "description": "Optional ThreadRef id to reply in a thread. Present only when the extension declares supports_threads." },
    "reply_to":     { "type": "string", "description": "Optional MessageRef id to reply to." }
  },
  "additionalProperties": false
}
```
Output: `{ "message": MessageRef }`.

**Example — `read_history` output** (`ironclaw.messaging.read_history.v1`,
output schema) — note `author` is a fully resolved `UserRef`, not a raw id:

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "messaging.read_history output",
  "type": "object",
  "required": ["messages", "has_more"],
  "properties": {
    "messages": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["ref", "author", "text", "posted_at"],
        "properties": {
          "ref":       { "$ref": "#/$defs/MessageRef" },
          "author":    { "$ref": "#/$defs/UserRef" },
          "text":      { "type": "string" },
          "posted_at": { "type": "string", "format": "date-time" },
          "thread":    { "$ref": "#/$defs/ThreadRef" }
        }
      }
    },
    "has_more": { "type": "boolean" },
    "cursor":   { "type": "string", "description": "Opaque pagination cursor; pass back as `before`." }
  },
  "additionalProperties": false
}
```

The vendor's native paging (Slack `latest`/`oldest` timestamps, Telegram
offsets) collapses into one opaque `cursor` — the model never sees vendor paging
grammar.

---

## 4. Normalization responsibility

**The adapter normalizes; the framework enforces and assists.**

- **Adapter (per vendor):** in `invoke`, map the vendor response into the
  profile's output shape — including resolving raw ids to `UserRef`/`ConversationRef`,
  *even at the cost of extra API calls* (Slack: call `users.info` to fill
  `display_name` the way today's separate `get_user_info` does,
  `assets/slack/wasm-src/src/api.rs:198`). This is the one genuine behavior
  addition over today.
- **Framework (host, once):**
  1. **Output-schema validation.** Re-enable `output_schema_ref` for profiled
     tools and validate the adapter's `ToolResult.output`
     (`crates/ironclaw_host_api/src/tool_adapter.rs:52`) against the profile's
     output schema before it reaches the model. Today output is unvalidated
     (`raw_output.v1.json`); this makes "the model knows exactly what comes out"
     an enforced invariant, not a convention. A validation failure is a
     recoverable `ToolError::Failed` (model-visible, run continues per
     `.claude/rules/agent-loop-capabilities.md`), never a host-internal `Err`.
  2. **A resolution cache.** The adapter already has `ScopedToolState`
     (`tool_adapter.rs:164`, per-installation KV) to memoize `user_id →
     display_name`, so the extra lookups amortize across a turn/conversation.
  3. **Conformance at activation.** Run the existing
     `evaluate_profile_conformance` (`conformance.rs`) when the expander builds
     the decls, so a claimed profile whose schema refs don't match the
     host contract fails activation with a typed error — the auth engine's
     "recipe validates or activation fails" discipline, for tool profiles.

**Boundary:** framework = types + output validation + cache primitive +
conformance; adapter = fetch/resolve/fill. The framework provides no generic
*resolver* (there is no vendor-neutral way to look up a user); resolution is
irreducibly adapter work, which is why messaging keeps a `ToolAdapter` where
auth needed none.

---

## 5. Discovery / anti-bloat — the central question

**Goal:** the model targets the right extension + capability without seeing N×M
duplicate tools. The decisive fact is that the runtime **already** solves this
generically, and it is off by default.

**What exists** (`crates/ironclaw_reborn/src/tool_disclosure.rs` +
`tool_disclosure_port.rs`, wired at `runtime.rs:688`): the full authorized
catalog is not all advertised. `ToolTier::{Core, Discoverable}`
(`tool_disclosure.rs:66`) marks builtins as core and **extension tools as
Discoverable**; `select_active_set` (`:521`) advertises everything only while
`total_schema_tokens ≤ 12_000` **and** `len ≤ 32` (`DisclosureCaps` default,
`:338`), else it advertises core + promoted and **defers** the rest behind a
single `tool_search` bridge. `tool_search`'s description doubles as an always-on
**index of discoverable tool names** (`catalog_index_tool_search_description`,
`:476`) — the model *sees* `slack.send_message`, `telegram.send_message` by name
grouped by extension prefix, and loads a schema on demand
(`tool_search` → `capability_info` → direct call; `capability_info` promotes the
tool for the rest of the scope). It reports live token reduction. It is
**production-wired but defaults OFF** (`ToolDisclosureMode::from_env`, gate
`is_bridged()`; `tool_disclosure_mode_defaults_off_with_bridged_opt_in`,
`runtime.rs:917`).

### Options

| Option | What | Verdict |
| --- | --- | --- |
| **A. Meta discovery tool + parameterized standard tools** (the owner's initial sketch) | a `messaging.list_tools(extension)` returns the subset; tools take an `extension` arg | **Rejected.** Duplicates the existing `tool_search`/`capability_info` machinery — a parallel discovery mechanism the §7 fence forbids. Forces the model to learn a messaging-specific two-step protocol distinct from how it discovers every other tool. |
| **B. Shared definition surfaced per-extension** (`<ext>.<tool>`, one shared schema) | expand the recipe into ordinary per-extension surfaces; dedup the *definition* via the profile | **Recommended half.** Reuses every existing seam; per-vendor schema fidelity preserved (Slack's thread param ≠ Telegram's). |
| **C. Single generic tool with an `extension` param** | one `send_message(extension, conversation, text)` for all vendors | **Rejected.** Fights the dispatcher: `ToolResolver::resolve(capability_id)` returns a *prebound* adapter (`dispatcher/src/lib.rs:48`) — a generic tool can't resolve to a bound adapter without new indirection. Loses per-vendor schema/description; can't gate optional capabilities per vendor in one schema. |
| **D. Progressive disclosure** (reuse the existing system) | messaging tools are ordinary deferred tools; discover via `tool_search` → `capability_info` → call | **Recommended half.** Exactly "query which tools exist, then call" — generically, already built. |

### Recommendation — **B + D**

Expand the recipe into ordinary per-extension deferred tool surfaces (B) and
rely on the existing tool-disclosure system for model-facing bloat (D). The
`<ext>.<tool>` naming makes the `tool_search` index group messaging tools by
extension prefix — which *is* "query which messaging tools an extension has,"
with no messaging-specific meta-tool. This invents no mechanism (honoring the
§7 fence), mirrors the MCP precedent ("ordinary tool surfaces… no MCP in the
dispatch path"), and gives the model the identical discover-then-call flow it
uses for everything else.

**Honest caveats.**
- At small N (a handful of messaging extensions) the catalog is under the
  32-tool / 12k-token cap, so the tools show flat and un-deferred. That is
  fine — few tools, no bloat. The anti-bloat benefit materializes exactly when
  N×M grows past the cap, which is when it matters.
- The disclosure system defaults **off** today. Turning it on is an orthogonal,
  already-planned lever; the framework must **not** couple to it or fork it. If
  it stays off, messaging tools are simply advertised flat — correct, just not
  yet compact.
- The framework's *only* discovery contribution is consistent naming (and,
  optionally, per-profile tags to sharpen `tool_search` ranking for messaging
  intents). An explicit `messaging.describe(extension)` meta-tool is a
  **non-goal** unless evaluation shows the generic index underperforms for the
  grouping case — and it would then have to justify itself against the fence.

---

## 6. Architecture integration

### 6.1 A third tool *source*, expanded to ordinary surfaces

Tool sources today are static `[[tools]]` and MCP-discovered
(`overview.md` §3.1). The framework is a third — **framework-templated +
manifest-selected** — but, like MCP, its novelty is confined to a manifest
section (`[messaging]`) and a resolve-time expander. Precision borrowed from
the MCP reality: as with an MCP tool that still dispatches through a
`RuntimeKind::Mcp` lane at call time, a messaging tool still dispatches through
its extension's `wasm`/`first_party` `ToolAdapter` — "no new source in the
dispatch path" holds for **listing/discovery/resolution identity**, not for a
claim that no per-runtime code runs at call time.

Nothing changes in the dispatcher (`ToolResolver::resolve` →
`ResolvedCapability`, `dispatcher/src/lib.rs:48,202`), the `ToolAdapter` ABI
(one method, `tool_adapter.rs:94`), the active snapshot
(`active.rs:29,112`), the disclosure system, or the settings UI — all consume
ordinary `CapabilityDeclV2` surfaces.

### 6.2 Coordinator overlap — a shared per-vendor "messaging core"

Sending overlaps the delivery coordinator, and the split is real in the code:

- `ChannelAdapter::deliver` (`crates/ironclaw_product_adapters/src/channel_adapter.rs:56`)
  "Owns vendor formatting, splitting, target syntax, DM provisioning, and safe
  error mapping. Never touches the delivery store." Slack's `deliver`
  (`crates/ironclaw_slack_extension/src/channel.rs:84`) renders mrkdwn
  (`render_slack_mrkdwn`), splits (`slack_text_chunks`), posts `chat.postMessage`,
  and maps errors; DM provisioning (`conversations.open`) lives in `list_targets`
  (`channel.rs:155`).
- The Slack `send_message` **tool** today (`assets/slack/wasm-src/src/api.rs:220`)
  does a bare `chat.postMessage` with **none** of that rendering/splitting/DM
  logic, and on the **user** token (vs. the channel's **bot** token).
- The `DeliveryCoordinator`
  (`crates/ironclaw_product_workflow/src/delivery_coordinator.rs`) owns the
  reliability layer: it is the **sole delivery-state writer**, persists
  `Prepared→Sending` before egress (`:577`), dedupes in-flight (`:255`),
  recovers stray `Sending→Unknown` (`:309`), and refuses a no-op sink (`:264`).
  Adapters get **no** store (verified: no store handle in either extension crate).

**Decision.** Extract the pure, credential-agnostic **vendor mechanics**
(Markdown→dialect rendering, message splitting to `max_message_chars`,
target/DM-ref formatting, vendor error mapping, request construction) into a
per-vendor **messaging core** *inside the extension crate*, called by **both**
`ChannelAdapter::deliver` **and** the new messaging `ToolAdapter::invoke`,
parameterized by credential and egress. The *reliably* shared part is the pure
presentation logic (rendering, splitting); transport/request-construction is
shared only when the bot channel and the user identity hit the **same** API
(Slack — one Web API, different token) and legitimately diverges when they do not
(a Telegram *bot* channel on the Bot API vs. a *paired-user* tool acting through
the user surface). This kills the duplication the owner named — one level below
the tools — without forcing a false unification where the transports differ. Do **not** share the coordinator's
sole-writer reliability layer (a tool call is a one-shot dispatch with its own
audit/obligation pipeline; it must never gain store access). The generic side
owns the abstract profiles + expander; the extension crate owns the vendor core.
This is already the crate boundary `implementation.md` §3 draws
(`ironclaw_slack_extension` owns "tool adapters… channel adapter"), so the
sharing is intra-crate. The concrete blocker is packaging, not architecture: the
reusable Slack helpers are `pub(crate)` (`mrkdwn.rs`) and Telegram's `render.rs`
is currently unwired — both get promoted/reconciled into the messaging core.

### 6.3 Effects, credentials, permissions per standard tool

Framework defaults (a ceiling; the recipe may narrow, never widen — enforced by
the resolved-contract diff):

| Profile | `effects` | `default_permission` |
| --- | --- | --- |
| `send_message`, `edit_message`, `delete_message`, `add_reaction` | `["network", "use_secret", "external_write"]` | `ask` |
| `read_history`, `list_conversations`, `get_user`, `search_messages` | `["network", "use_secret"]` | `ask` |

Credentials use the existing `[[tools.credentials]]` model (vendor + audience +
injection). **The messaging tool's credential is always the user-acquired
identity** — Slack's OAuth `slack_user_token`, Telegram's pairing credential —
distinct from the bot credential the channel delivers on. Acquisition rides the
existing connect surface: an OAuth vendor uses the `[auth.<vendor>]` recipe + the
auth engine (`overview.md` §4.3); a pairing vendor uses the `manual_token`
pairing modality (the `Pairing` gate / `PairingRequired` event). A
missing/expired grant raises the generic gate keyed by the tool's declared vendor
and resumes — `ToolError::AuthRequired` (`tool_adapter.rs:64`) — routed to the
OAuth gate or the pairing gate as the vendor requires.

### 6.4 The relay/act boundary — decided by the recipient (critical)

The channel-vs-tools split is a confidentiality guarantee, and it must not depend
on the model classifying correctly. **Which surface handles a message is decided
by the recipient:** to *you* (the owner) → the channel relays it (bot / WebUI);
to *anyone else* → the messaging tools send it, as you. Two host-enforced hard
constraints follow:

- **A. The messaging tools never send to you.** A `send_message` whose recipient
  is the owner / their own conversation is blocked — that is a relay, the
  channel's job. This is what kills the observed **duplicate send**: "send me a DM
  with XYZ" otherwise becomes both a self-send *and* the channel relay.
- **B. The channel/bot is never a sender to a third party.** The bot delivers
  only to the owner — a reply to their own request (where it came from) or a
  notification to their target — never initiating to someone else, never posting
  as the user. Outward-facing is always from you.

A legitimate outward send (to someone else) stays `ask` with a target-naming
approval; automations deny act-as-user without pre-authorization. This is generic
(coordinator + dispatch), not per-vendor, and pinned by a cross-channel
conformance test (`messaging-framework.md` §12). **Open (must verify + wire):**
constraints A and B and the automation denial are not confirmed present today;
and whether an in-shared-channel invocation replies in-channel or always DMs the
owner is a product call (`messaging-framework.md` §13).

---

## 7. Migration

- **Slack — parity, one enrichment.** Replace the five bespoke `[[tools]]` with
  `[messaging] profiles = ["send_message","read_history","list_conversations",
  "get_user","search_messages"]`. The expander emits the **same capability ids**
  (`slack.send_message`, …), so the model surface is byte-for-byte unchanged —
  no regression. The Slack `ToolAdapter` (still the WASM artifact initially, per
  `implementation.md` §3) now returns **normalized** output: `read_history`
  authors become `UserRef`s with `display_name` resolved (folding the
  `get_user_info` round-trip), and outputs validate against the profile schemas
  instead of `raw_output.v1.json`. `search_messages` is Slack-specific but is a
  first-class optional profile, so it migrates cleanly. This extends the
  existing Slack tool integration test rather than adding a parallel one.
- **Telegram — gains user-acting tools via pairing.** Telegram ships **no tools**
  today (`assets/telegram/manifest.toml`: "No tools, no WASM module") — it is a
  bot *channel* only. Once its pairing flow yields a user-acquired identity (the
  connect analogue of Slack's OAuth — the existing `manual_token`/pairing-code
  modality), it declares the **same read-rich subset** as Slack —
  `[messaging] profiles = ["send_message","read_history","list_conversations",
  "get_user","search_messages", …]` — whose tools act as the *paired user*, not
  the Bot API. The new `ToolAdapter::invoke` reuses the extension crate's pure
  rendering via the shared messaging core (§6.2), though its transport is the
  user surface, not the bot channel's Bot API. The tool *definitions/schemas*
  come free (framework-owned); the per-vendor `invoke` and the pairing credential
  are the real work. This is the tool-side "addition test."
- **Discord — the bot-acting contrast + addition test.** Discord does not
  sanction a user-acting identity (user automation violates ToS), so a Discord
  messaging extension's tools act as the **bot** and declare only what a bot
  supports (no `search_messages`). A new package + extension crate implements
  `invoke`; **no generic source changes** (`overview.md` §1 addition test, now
  covering tools). It also shows the framework standardizes the tool *contract*
  while the *identity* (user vs. bot) is the extension's credential decision.

---

## 8. Testing model

Mirror the auth engine's payoff (`overview.md` §8, `implementation.md` §12):

- **One messaging conformance suite, vendors as rows.** Like
  `crates/ironclaw_auth/tests/auth_engine_contract.rs` ("vendors are rows…
  no per-vendor suite anywhere else") driven by a scripted vendor server: given
  a `ToolAdapter` claiming a messaging profile, assert each declared profile's
  `invoke` honors the input schema and returns **schema-valid, normalized**
  output (ids resolved to `UserRef`), against recorded fixtures. Slack,
  Telegram, and the `acme-messenger` fixture all run it.
- **Structural profile conformance** (`evaluate_profile_conformance`) runs in
  the resolver/activation tests: a claimed profile with mismatched schema refs
  or a missing required operation fails activation.
- **Integration proof** through the production dispatcher: activate the real
  Slack package, invoke `slack.send_message` / `slack.read_history`, assert the
  output validates against the profile and no Slack branch exists in dispatch —
  extending the existing `tests/integration/extension_runtime.rs`.
- Repo law throughout: test-first, integration tier for production-wired
  behavior asserting at a seam, both DB backends where state persists.

---

## Verified starting points (2026-07-14)

Confirmed against `origin/nea25/runtime-rollup` @ `a312a81b` (codebase graph MCP
unavailable → grep/read fallback, per the `ironclaw-reborn-orientation` skill):

- **Capability-profile scaffolding exists and is dormant.**
  `CapabilityProfileContract`/`CapabilityProfileOperationContract`
  (`crates/ironclaw_host_api/src/capability_profile.rs:169,206`, doc "extensions
  may later claim…"); `CapabilityDeclV2.implements` + `output_schema_ref`
  (`crates/ironclaw_extensions/src/v2.rs:458,467`); conformance evaluator
  (`crates/ironclaw_capabilities/src/conformance.rs`, "zero-behavior prep").
  **Unwired:** v3 tool reader lacks both fields
  (`crates/ironclaw_extensions/src/v3.rs:119`); projection sets them empty
  (`v3.rs:362,368`); no manifest declares `implements`; no consumer outside
  `ironclaw_capabilities`.
- **The auth-recipe template.** Recipe data in
  `crates/ironclaw_host_api/src/recipe.rs:191,267`; `AuthEngine` dispatches by
  *method* with zero vendor branching
  (`crates/ironclaw_auth/src/engine/mod.rs:253,292-315`); resolver returns data
  (`AuthRecipeResolver`, `crates/ironclaw_extension_host/src/recipes.rs:120`);
  recipe stored on `ResolvedAuthSurface`
  (`crates/ironclaw_extensions/src/resolved.rs:87`); table-driven test
  (`crates/ironclaw_auth/tests/auth_engine_contract.rs`).
- **Tool seam.** `ToolAdapter::invoke`
  (`crates/ironclaw_host_api/src/tool_adapter.rs:94`); `ToolResult.output`
  (`:52`); `ScopedToolState` cache (`:164`); `ToolError::AuthRequired` → gate
  (`:64`).
- **Resolve/dispatch/list unchanged for a new surface.** `CapabilityDeclV2`
  (`v2.rs:455`) → `ResolvedExtensionManifest.tools`
  (`crates/ironclaw_extensions/src/resolved.rs:48`) → `ActiveSnapshot` /
  `ResolvedToolBinding` (`crates/ironclaw_extension_host/src/active.rs:29,41,112`)
  → `ToolResolver::resolve` (`crates/ironclaw_dispatcher/src/lib.rs:48,202`) →
  model list via `tool_definitions()` /
  `ProviderToolDefinition`
  (`crates/ironclaw_turns/src/run_profile/host.rs:2091,1483`);
  `CapabilityVisibility::Model` gate
  (`crates/ironclaw_host_runtime/src/surface.rs:212`).
- **Progressive disclosure = the anti-bloat mechanism, default off.**
  `crates/ironclaw_reborn/src/tool_disclosure.rs:66,338,476,521`;
  `tool_disclosure_port.rs:53,196`; wired `runtime.rs:688`; default-off test
  `runtime.rs:917`.
- **MCP publishes ordinary surfaces.**
  `crates/ironclaw_extensions/src/hosted_mcp_discovery.rs:34`
  (`CapabilityVisibility::Model`, `RuntimeKind::Mcp`); `tools/list` only at
  activation.
- **Normalization gap.** Slack raw output
  (`assets/slack/schemas/slack/raw_output.v1.json`; `wasm-src/src/api.rs:103,182`);
  capability-id dispatch (`wasm-src/src/lib.rs:134`).
- **Coordinator overlap.** `ChannelAdapter::deliver` doc
  (`crates/ironclaw_product_adapters/src/channel_adapter.rs:56`); Slack
  `channel.rs:84,155`; `mrkdwn.rs` (pure but `pub(crate)`);
  `DeliveryCoordinator` sole-writer
  (`crates/ironclaw_product_workflow/src/delivery_coordinator.rs:250,264,309,577`).
- **Two-identity model + pairing.** Slack tools act as the *user*
  (`slack_user_token`) while the channel delivers as the *bot* (`slack_bot_token`,
  `channel.rs:33`). Pairing is a first-class connect modality: the `Pairing { .. }`
  lifecycle gate (`crates/ironclaw_product_workflow/src/lifecycle.rs:157`), the
  `PairingRequired` event (`crates/ironclaw_common/src/event.rs:163`), and the
  Telegram pairing-code card on the `manual_token` paste modality
  (`crates/ironclaw_product_adapters/src/outbound.rs:1027,1741`); `telegram_user`
  is already a real actor kind
  (`crates/ironclaw_telegram_extension/src/payload.rs:28`). Telegram is a
  channel-only extension today (`implementation.md` §2) with no user credential
  yet — the pairing→user-acting flow that its messaging tools would use is the
  planned addition (design intent, per the owner; the exact credential mechanism
  is open question 2).

---

## Scope fence (what this ADR does NOT build)

- **No new dynamic-*discovery* abstraction.** The tool set is static
  (manifest-declared). MCP remains the only runtime-discovery source; this
  reuses `implements`/profiles + the existing disclosure system.
- **No messaging-specific discovery/query tool** (see §5 Option A). Reuse
  `tool_search`. Revisit only if evaluation shows the generic index
  underperforms for the per-extension grouping case.
- **No sharing of delivery reliability** (retry/persistence/dedupe/sole-writer)
  with tools. The shared core is vendor mechanics only (§6.2).
- **No auth-style "zero-code" tool engine.** Behavior is genuinely per-vendor;
  the framework templates definitions, not sends.
- **No per-call identity/account selection** ("send from my work account") —
  inherits the same v1 non-goal as `adr/0001` §"Open questions".

---

## Open questions (for the accepting/implementing pass)

1. **Section shape.** `[messaging]` recipe + expander (recommended) vs. pure
   `[[tools]] implements=` (re-expose v3 fields, no expander)? The latter is
   lower-mechanism but leaves per-tool boilerplate (§2.4).
2. **Pairing as a credential mechanism.** The identity rule is settled — the
   tools act as the user-acquired identity, the channel as the bot (§Context,
   §6.3). What's open is *how a pairing-acquired credential is modeled*: does
   pairing become a first-class **auth-engine recipe method** (a third alongside
   `oauth2_code`/`api_key`, `recipe.rs:195`), or stay the separate `manual_token`
   pairing modality it is today — and what does a paired Telegram tool actually
   inject (a user session vs. a bearer token), given its transport differs from
   the bot channel's Bot API (§6.2)?
3. **Threads as a param vs. a tool**, and whether one `thread` field can
   faithfully model Slack `thread_ts`, Telegram `message_thread_id` (forum
   topics), and Discord thread channels without leaking vendor semantics.
4. **Output-validation strictness.** Reject non-conforming adapter output (fail
   the tool call) vs. coerce/annotate? And how are the framework ref types
   versioned (`…​.v1` → `…​.v2`) without a wire break?
5. **Reaction identity.** `add_reaction` emoji across Unicode, vendor
   shortcodes, and custom/guild emoji ids — one normalized field or a tagged
   union?
6. **Conformance depth.** The existing evaluator is structural (schema-ref
   equality). Do we need behavioral conformance (a scripted vendor server per
   profile, §8) as a gate, or is structural + the integration proof enough?
7. **Interaction with multi-account (`adr/0001`).** A messaging tool executes
   under a resolved account; confirm the `[messaging]` credential resolves
   through the same `resolve_account(user, extension, vendor)` path with no new
   plumbing.
8. **`get_user` vs. inline resolution.** If `read_history` always resolves
   authors, is a standalone `get_user` profile still worth exposing, or does it
   survive only for explicit lookups?
