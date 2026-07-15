# Messaging Tool Framework — Tool Contract (canonical)

**Status:** Proposed (2026-07-14).
**Companions:** `messaging-framework.md` (architecture), `adr/0002-messaging-tool-framework.md`
(decision), `messaging-framework-checklist.md` (acceptance).
**This document is canonical** for the exact tool schemas, the model-facing
descriptions, and the prompt docs. Where it and the architecture doc's illustrative
schemas differ, this wins. It **incorporates the Slack migration-parity audit**
(Appendix A) — every field/param the old Slack tools exposed is either carried here
or listed as a conscious drop.

## Conventions

- **Description** = the one-line string the model sees in its tool list (in the
  expanded surface's `description`). Written vendor-neutral; the expander may
  prefix the platform display name (e.g. "Slack — …"). An extension package MAY
  override the *wording* (not the schema) — e.g. to add vendor query-operator
  syntax — via `prompt_doc_ref`.
- **Prompt doc** = the longer `prompts/messaging/<tool>.md` guidance loaded on
  demand (via `capability_info` / disclosure). Safety-critical text lives here and
  in the description.
- **Voice:** concise, action-first, and identity-forward — every write tool states
  it acts *as you*. Modeled on the existing Slack prompt docs.
- Schemas are JSON Schema draft-07, `additionalProperties:false`, and `$ref` the
  shared types in §1 as `types.v1#/$defs/<Type>`.
- **Thread params:** the `thread` field on send_message / read_history appears in
  an extension's expanded schema **only** when it declares `supports_threads`. The
  model never sees an unusable thread param, so descriptions carry no conditional.

---

## 1. Shared types — `schemas/messaging/types.v1.json`

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
        "id":           { "type": "string", "description": "Opaque vendor user id (round-trippable; pass back to get_user / mentions)." },
        "display_name": { "type": "string", "description": "Resolved human name. Mapping rule: the vendor's best full/real name, else the chosen display name, else the handle. Always present — the adapter resolves it even at the cost of an extra lookup." },
        "username":     { "type": "string", "description": "Handle without a leading @, where the platform has one (Slack `name`, Telegram `username`)." },
        "is_bot":       { "type": "boolean", "description": "True if this user is a bot/app account." }
      }
    },
    "ConversationRef": {
      "type": "object",
      "required": ["id", "kind"],
      "additionalProperties": false,
      "properties": {
        "id":      { "type": "string", "description": "Opaque conversation id. Pass to read_history / send_message." },
        "kind":    { "type": "string", "enum": ["dm", "group", "channel"], "description": "dm = 1:1; group = multi-person private chat (Slack mpim, Telegram group); channel = named/broadcast channel (Slack public/private channel, Telegram channel)." },
        "title":   { "type": "string", "description": "Name of a group/channel; for a dm, the other participant's display name." },
        "private": { "type": "boolean", "description": "For a channel/group: true if private/invite-only, false if public. Omitted when the platform has no public/private distinction." }
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
        "emoji": { "type": "string", "description": "Unicode emoji where the platform uses one; else a :shortcode: or a vendor custom-emoji id (see messaging-framework.md §16 Q3)." },
        "count": { "type": "integer", "minimum": 0 },
        "me":    { "type": "boolean", "description": "Whether the acting (you) user reacted." }
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
        "created_at":  { "type": "string", "format": "date-time", "description": "RFC 3339 UTC." },
        "edited_at":   { "type": "string", "format": "date-time", "description": "Present if the message was edited." },
        "reply_to":    { "$ref": "#/$defs/MessageRef", "description": "The message this one replies to, when applicable." },
        "permalink":   { "type": "string", "description": "Stable URL to this message where the platform provides one (Slack permalink). Lets you cite/link a message." },
        "reactions":   { "type": "array", "items": { "$ref": "#/$defs/Reaction" } },
        "attachments": { "type": "array", "items": { "$ref": "#/$defs/AttachmentRef" } }
      }
    }
  }
}
```

Audit-driven additions vs. the architecture doc's first sketch: `ConversationRef.private`
and `Message.permalink` (both optional) recover dropped Slack fields; `UserRef`
carries an explicit `display_name` mapping rule. `email` is deliberately **not** on
`UserRef` (privacy — Appendix A gap G7).

---

## 2. `send_message` (core) — `ironclaw.messaging.send_message.v1`

**Description:**
> Send a message as you to a conversation (DM, group, or channel). Posts under
> your own account — delegated authority — so use it only for actions the user
> asked you to take toward others (e.g. "DM Sergey the plan"). Never use it to
> answer the user or report results back to them; the host delivers your reply on
> the channel after the turn. Takes a conversation id from list_conversations or
> an inbound message. You can't message yourself — the host blocks a send
> addressed to the user.

**Prompt doc** (`prompts/messaging/send_message.md`):
> Send a message as you to a conversation. The message appears to come from your
> own account — this is delegated authority, so use it only for side effects the
> user explicitly asked for inside the current job (for example, "DM Sergey this
> joke", "post the release note to #announcements").
>
> Never use this tool to deliver your final answer or report results back to the
> user. Final replies and notifications are delivered by the host on the user's
> own channel after the turn completes — just finish the turn with your answer. If
> you find yourself about to send the user a message that says what you were going
> to say anyway, stop and answer normally instead. (The host also blocks a send
> whose recipient is the user — you cannot message yourself.)
>
> Provide the target `conversation` id and `text`. To reply in a thread, pass
> `thread`; to reply to a specific message, pass `reply_to`.

**Input:**
```json
{
  "type": "object",
  "required": ["conversation", "text"],
  "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string", "description": "Target conversation id (from list_conversations or an inbound message). Some adapters also accept a channel name like \"#general\"." },
    "text":         { "type": "string", "description": "Message body as Markdown; the adapter renders to the vendor dialect and splits to the vendor length limit." },
    "thread":       { "type": "string", "description": "Reply inside this thread id." },
    "reply_to":     { "type": "string", "description": "Message id (within `conversation`) to reply to." }
  }
}
```
**Output:**
```json
{ "type": "object", "required": ["message"], "additionalProperties": false,
  "properties": { "message": { "$ref": "types.v1#/$defs/MessageRef" } } }
```
**Slack mapping:** `conversation`←`channel` (adapter resolves a `#name` to an id),
`thread`←`thread_ts`; POST `chat.postMessage`; output `message.id`←`ts`,
`message.conversation`←returned `channel`. **New:** `reply_to`. **Effects:**
`network, use_secret, external_write`; `ask`. Subject to constraint A (§12 of the
architecture doc): a send to the owner is blocked.

---

## 3. `read_history` (core) — `ironclaw.messaging.read_history.v1`

**Description:**
> Read recent messages from a conversation you can access, newest first — each
> with its author already resolved to a name (no get_user needed). Give a
> `conversation` id; bound with `limit`, and page to older messages with the
> returned `cursor`. For the latest messages of a known conversation, prefer this
> over search_messages, which is keyword-indexed and unreliable for recency.

**Prompt doc** (`prompts/messaging/read_history.md`):
> Read message history from a conversation by its id. Returns messages newest-first
> with the author resolved to a display name (no separate lookup needed). Use
> `limit` to bound the count, `before` to page toward older messages, `after` to
> bound the window forward, and `thread` to read a single thread. For "the latest
> messages of a known conversation," this is the right tool (search is keyword
> indexed and unreliable for recency).

**Input:**
```json
{
  "type": "object",
  "required": ["conversation"],
  "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string", "description": "Target conversation id (from list_conversations or an inbound message). Some adapters also accept a channel name like \"#general\"." },
    "limit":        { "type": "integer", "minimum": 1, "maximum": 200, "default": 50, "description": "Maximum messages to return." },
    "cursor":       { "type": "string", "description": "Opaque page cursor from a prior read_history response (its `cursor`); returns the next older page. An opaque token — NOT a date/timestamp." },
    "after":        { "type": "string", "description": "Opaque cursor from a prior response; bounds the window forward (messages newer than it). NOT a date — for date-bounded lookups use search_messages." },
    "thread":       { "type": "string", "description": "Restrict to this thread." }
  }
}
```
**Output:**
```json
{
  "type": "object",
  "required": ["messages", "has_more"],
  "additionalProperties": false,
  "properties": {
    "messages": { "type": "array", "items": { "$ref": "types.v1#/$defs/Message" } },
    "has_more": { "type": "boolean" },
    "cursor":   { "type": "string", "description": "Opaque token; pass back as `cursor` to page to older messages." }
  }
}
```
**Slack mapping:** `conversation`←`channel`;
`conversations.history` (`cursor`←`latest`, `after`←`oldest`); each
`Message.author` is resolved via cached `users.info`
(folds the old `get_user_info` round-trip); `Message.ref.id`←`ts`,
`created_at`←`ts`, `thread`←`thread_ts`; `has_more`←`has_more`. **Audit:** old
`limit` max 1000/default 50 → 200/50 here (raised from the sketch's 100/20);
`msg_type` dropped (system/join messages filtered out). **Effects:**
`network, use_secret`; `ask`.

---

## 4. `list_conversations` (core) — `ironclaw.messaging.list_conversations.v1`

**Description:**
> List the conversations you can access — DMs, group chats, and channels — to find
> a conversation's id before reading or sending. Filter by `kinds` or a name
> `query`.

**Prompt doc** (`prompts/messaging/list_conversations.md`):
> List conversations you can access. Use it to discover a conversation id (e.g.
> the DM with a person, or a channel by name) before read_history or send_message
> — to message a person by name, find their DM here (query their name), then
> send_message. Filter by `kinds` (dm/group/channel) or a name `query`; page with
> `cursor`.

**Input:**
```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "kinds":  { "type": "array", "items": { "type": "string", "enum": ["dm", "group", "channel"] }, "description": "Filter by kind; default all." },
    "query":  { "type": "string", "description": "Optional name filter (client- or server-side)." },
    "limit":  { "type": "integer", "minimum": 1, "maximum": 200, "default": 100 },
    "cursor": { "type": "string" }
  }
}
```
**Output:**
```json
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
**Slack mapping:** `conversations.list`; `kinds`→`types` (dm→im, group→mpim,
channel→public_channel,private_channel); `ConversationRef.id`←`id`,
`title`←`name` (for a dm, the other user's resolved name via `user`),
`kind` from the `is_im`/`is_mpim`/`is_channel` flags, `private`←`is_private`.
**Audit:** old `limit` max 1000/default 200 → 200/100; the DM partner `user` id is
folded into `title` (resolved). **Effects:** `network, use_secret`; `ask`.

---

## 5. `get_user` (core) — `ironclaw.messaging.get_user.v1`

**Description:**
> Look up a user by id → resolved profile (display name, handle, is_bot). Use it
> when you hold a bare user id — e.g. from an @mention in message text — and need
> the name behind it. (read_history and search_messages already include a resolved
> author, so you rarely need this for those.)

**Prompt doc** (`prompts/messaging/get_user.md`):
> Resolve a user id to a profile (display name, handle, whether it's a bot). Note
> that read_history/search_messages already return authors resolved, so you rarely
> need this — reach for it only when you have a bare user id (e.g. from a mention)
> and want its name.

**Input:**
```json
{ "type": "object", "required": ["user_id"], "additionalProperties": false,
  "properties": { "user_id": { "type": "string", "description": "Opaque user id (e.g. from a Message author or an @mention)." } } }
```
**Output:**
```json
{ "type": "object", "required": ["user"], "additionalProperties": false,
  "properties": { "user": { "$ref": "types.v1#/$defs/UserRef" } } }
```
**Slack mapping:** `users.info`; `UserRef.display_name`←`profile.real_name ‖
profile.display_name ‖ name`, `username`←`name`, `is_bot`←`is_bot`. **Audit:**
old `email` is **not** surfaced (privacy — gap G7); old `real_name`/`display_name`
collapse per the mapping rule. **Effects:** `network, use_secret`; `ask`.

---

## 6. `search_messages` (optional) — `ironclaw.messaging.search_messages.v1`

**Description:**
> Search messages you can access by keyword. Returns matches as messages (author
> resolved), by relevance or recency (`sort`). Prefer list_conversations +
> read_history for the latest messages of a known conversation; use search to find
> by keyword, person, or topic.

**Prompt doc** (`prompts/messaging/search_messages.md`):
> Search across messages you can see. Returns matches as full messages (author
> resolved, with a `permalink` where available). Set `sort` to `recency` for the
> newest matches first, or leave it `relevance` (default). Restrict to one
> conversation with `conversation`, or omit for a global search.
>
> Search indexes content and is unreliable for "the single most recent message I
> sent" — for that, use list_conversations then read_history (newest first).
>
> *Vendor note (Slack):* the `query` supports Slack search operators — `from:me`
> for your own messages (NOT `from:@me`), `from:@user`, `in:#channel`, `in:@user`
> (a DM), `after:2024-01-01`, `before:…`, `has:link`. Combine with keywords.

**Input:**
```json
{
  "type": "object",
  "required": ["query"],
  "additionalProperties": false,
  "properties": {
    "query":        { "type": "string", "description": "Keyword query. Vendors may support inline operators (Slack: from:me for your own messages — NOT from:@me — from:@user, in:#channel, after:2024-01-01, has:link; combine with keywords). Full list in the prompt doc." },
    "conversation": { "type": "string", "description": "Restrict to one conversation; omit for a global search." },
    "sort":         { "type": "string", "enum": ["relevance", "recency"], "default": "relevance", "description": "Order matches by relevance (default) or recency." },
    "limit":        { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
    "cursor":       { "type": "string" }
  }
}
```
**Output:** (same shape as read_history)
```json
{ "type": "object", "required": ["messages", "has_more"], "additionalProperties": false,
  "properties": {
    "messages": { "type": "array", "items": { "$ref": "types.v1#/$defs/Message" } },
    "has_more": { "type": "boolean" },
    "cursor":   { "type": "string" } } }
```
**Slack mapping:** `search.messages`; `sort`→`score`/`timestamp`; `limit`→`count`
(max 100 preserved); each match → a `Message` with `author` resolved,
`ref.conversation`←`channel.id`, `permalink`←`permalink`. **Audit:** `sort` and
`permalink` recovered (both were dropped in the sketch); `total` → `has_more`;
inline `channel_name` dropped (use `ref.conversation`). **Effects:**
`network, use_secret`; `ask`.

---

## 7. `edit_message` / `delete_message` (optional)

**`edit_message`** (`ironclaw.messaging.edit_message.v1`) — **Description:**
> Edit a message you previously sent, replacing its full text — posted as you.
> Works only on your own messages (platforms reject editing others'). Pass the
> `conversation` id, the target `message` id, and the new Markdown `text`; the new
> text replaces the old entirely. To add a new message instead, use send_message.

**`delete_message`** (`ironclaw.messaging.delete_message.v1`) — **Description:**
> Delete a message you previously sent — as you, and irreversibly. Works only on
> your own messages. Pass the `conversation` id and the `message` id (its
> MessageRef.id from read_history/search_messages). There is no undo — use it only
> when the user explicitly asked to remove something you posted.

**Prompt doc** (`prompts/messaging/edit_message.md`, `delete_message.md`): the
`message` id is a `Message.ref.id` from a prior read_history/search_messages
result; you can only edit or delete your **own** messages; a delete is permanent.
Both are `ask`, `external_write`, and act as you.

```json
// edit_message input
{ "type": "object", "required": ["conversation", "message", "text"], "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string", "description": "Conversation the message is in." },
    "message":      { "type": "string", "description": "MessageRef.id within `conversation` (from read_history/search_messages) — must be your own message." },
    "text":         { "type": "string", "description": "New body (Markdown); replaces the old text entirely." } } }
// edit_message output
{ "type": "object", "required": ["message"], "additionalProperties": false,
  "properties": { "message": { "$ref": "types.v1#/$defs/MessageRef" } } }

// delete_message input
{ "type": "object", "required": ["conversation", "message"], "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string", "description": "Conversation the message is in." },
    "message":      { "type": "string", "description": "MessageRef.id (from read_history/search_messages) of your own message to delete. Permanent." } } }
// delete_message output
{ "type": "object", "required": ["deleted"], "additionalProperties": false,
  "properties": { "deleted": { "type": "boolean" } } }
```
**Slack mapping:** `chat.update` / `chat.delete` on the user token.

---

## 8. `add_reaction` / `remove_reaction` (optional)

**`add_reaction`** (`ironclaw.messaging.add_reaction.v1`) — **Description:**
> Add an emoji reaction to a message, as you — the reaction appears under your own
> account. Pass the `conversation` id, the target `message` id, and a Unicode
> `emoji` (the adapter maps it to the platform's format). Use for lightweight
> acknowledgement; to actually reply, use send_message.

**`remove_reaction`** (`ironclaw.messaging.remove_reaction.v1`) — **Description:**
> Remove a reaction you previously added to a message, as you — only your own
> reaction, and only the emoji you name. Provide the `conversation` id, the
> `message` id, and the Unicode `emoji` to remove.

Both `ask`, `external_write`, and act as you.

```json
// add_reaction / remove_reaction input (identical)
{ "type": "object", "required": ["conversation", "message", "emoji"], "additionalProperties": false,
  "properties": {
    "conversation": { "type": "string", "description": "Conversation the message is in." },
    "message":      { "type": "string", "description": "MessageRef.id (from read_history/search_messages) of the message to react to." },
    "emoji":        { "type": "string", "description": "Unicode emoji; the adapter maps to the vendor's reaction format." } } }
// output
{ "type": "object", "required": ["ok"], "additionalProperties": false,
  "properties": { "ok": { "type": "boolean" } } }
```
**Slack mapping:** `reactions.add` / `reactions.remove`; the adapter maps a Unicode
emoji to Slack's `:shortcode:`. (Emoji normalization is an open question — §16 Q3.)

---

## Appendix A — Slack migration-parity audit

Ground truth: `assets/slack/manifest.toml`, `schemas/slack/*.json`,
`prompts/slack/*.md`, `wasm-src/src/{types,api,lib}.rs` (all read at source
2026-07-14). Every old input param and output field, mapped to the new profiles.

**Legend:** ✓ carried · **＋** new capability · **⚠** changed (note) · **✗** dropped.

**search_messages:** `query`✓ · `count`(1-100,d20)→`limit`(1-100,d20)✓ ·
`sort`(score|timestamp) **re-added**✓ · out `ts`→`ref.id`✓ · `text`✓ ·
`user`(raw)→`author`(resolved)✓＋ · `username`✓ · `channel_id`→`ref.conversation`✓ ·
`channel_name`✗(use ref) · `permalink` **re-added**✓ · `total`→`has_more`/`cursor`⚠ ·
**＋**`conversation` filter, `sort=recency`.

**list_conversations:** `types`→`kinds`✓⚠ · `limit`(1-1000,d200)→(1-200,d100)⚠ ·
out `id`✓ · `name`→`title`✓ · `is_channel/is_private/is_im/is_mpim`→`kind`+`private`✓⚠
(private recovered) · `user`(DM partner)→folded into resolved `title`⚠ · **＋**`query`.

**get_conversation_history → read_history:** `channel`→`conversation`✓ ·
`limit`(1-1000,d50)→(1-200,d50)⚠ · `latest`→`before`✓ · `oldest`→`after` **re-added**✓ ·
out `ts`→`ref.id`+`created_at`✓＋ · `text`✓ · `user`→`author`(resolved)✓＋ ·
`msg_type`✗(system messages filtered) · `thread_ts`→`ref.thread`✓ · `has_more`✓ ·
**＋**`thread` filter.

**get_user_info → get_user:** `user_id`→`user`✓ · out `id`✓ · `name`→`username`✓ ·
`real_name`→`display_name`(mapping rule)⚠ · `display_name`→`display_name`⚠ ·
`email`✗(**privacy — deliberate**) · `is_bot`✓.

**send_message:** `channel`→`conversation`(adapter accepts #name)✓⚠ · `text`✓ ·
`thread_ts`→`thread`✓ · out `ok`✗(→ToolError) · `channel`/`ts`→`message`(MessageRef)✓ ·
**＋**`reply_to`. Prompt safety text **preserved** (§2).

### Gap decisions (most concerning first)

| # | Gap | Class | Decision |
| --- | --- | --- | --- |
| G1 | send_message safety prompt | must-preserve | Preserved as the canonical send_message prompt (§2). |
| G2 | history default 20 / max 100 (was 50 / 1000) | UX regression | **Raised** to default 50, max 200. |
| G3 | list default 50 / max 100 (was 200 / 1000) | UX regression | **Raised** to default 100, max 200. |
| G4 | search `sort` | genuine loss | **Re-added** (`relevance`/`recency`). |
| G5 | search `permalink` | genuine loss | **Re-added** (`Message.permalink`). |
| G6 | history forward window (`oldest`) | genuine loss | **Re-added** as `after`. |
| G7 | `get_user` **email** | genuine loss (PII) | **Kept off** UserRef (privacy). *Override: add optional `email?`.* |
| G8 | channel public/private (`is_private`) | genuine loss | **Re-added** `ConversationRef.private`. |
| G9 | `#channel-name` targeting | behavior loss | Adapter **accepts name or id**. |
| G10 | `real_name` vs `display_name` | needs rule | `display_name = real_name ‖ display_name ‖ handle`. |
| G11 | `msg_type` / `total` / inline `channel_name` / DM `user` id | intentional | Dropped; covered by `ref`/`title`/filtering. |

## Appendix B — cross-check (what is and isn't chat-messaging)

Grepped every first-party manifest. Chat-messaging (framework scope): **Slack**
(migrates), **Telegram** (gains — currently channel-only, 0 tools). **Not**
chat-messaging, deliberately excluded:

- **Gmail** (`gmail.send_message`, `reply_to_message`, `list_messages`, …) — email:
  no chat conversation model, no reactions, threading is different. Stays bespoke
  `[[tools]]`; the framework's scope note (design §5) excludes SMS/email surfaces.
- **GitHub** (`reply_pull_request_comment`, `search_*`) — PR comments / code
  search, not chat.
- **web-access** (`search`) — web search.

No other extension exposes a chat send/read/react surface; only Slack and Telegram
declare `[messaging]`.
