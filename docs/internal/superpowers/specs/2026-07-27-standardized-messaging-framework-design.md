# Standardized Messaging Framework — Design

- **Date:** 2026-07-27
- **Status:** Approved design (brainstorm complete; implementation plan next)
- **Problem owner:** Ben Kurrek
- **Code references:** verified against `main` @ `31b9583c2` on 2026-07-27; re-verify
  at implementation time.
- **Sibling project:** the model-callable **channel delivery tool** (bot identity,
  multi-destination; per the 2026-07-27 product decision with Firat). That project and
  this one are mechanically disjoint and reference each other in guidance wording
  only. This design contains **no self-send guard and no recipient policy** — the
  earlier `pushy-today` hard-deny approach was rejected in review-by-product and is
  not resurrected here.

## 1. Problem

Every channel extension ships bespoke model-callable messaging tools. Slack declares
8 tools with hand-written schemas and multi-hundred-word descriptions; a future
Discord, WhatsApp, or Telegram would each invent their own send/read shapes, their
own recipient field names, their own error strings. Consequences:

- The model must learn N vendor shapes for one concept ("send a message to a
  conversation"), degrading tool choice and inflating prompt surface.
- Platform rules (description guidance, display hints, audit categories, any future
  policy) attach per-extension-tool instead of once per operation.
- There is no capability negotiation: nothing declares "this vendor cannot edit
  messages", so gaps surface as runtime vendor errors instead of absent tools.
- Evidence discipline is per-extension convention: today Slack's send returns provider
  evidence only because its WASM chooses to (with silent `ts: ""` degradation —
  `assets/slack/wasm-src/src/api.rs:722-727`); nothing enforces it.

## 2. Decisions

1. **Design A, tools-entry binding.** Standard operations project as ordinary
   per-extension tool surfaces (`slack.send_message` → wire `slack__send_message`).
   One new manifest field on `[[tools]]` — `standard_op = "<op_name>"` — binds an
   entry to a host-defined operation. Rejected alternatives: a channel-section
   capability list (wrong identity lane: `[channel]` is the bot/delivery surface
   while messaging ops ride the user-identity tool lane; cannot carry per-op
   credentials/effects/gates; excludes tools-only messaging extensions) and generic
   singleton ops (`messaging__send_message` with an extension argument — the
   authorization kernel resolves trust, lane, gates, approvals, credential
   obligations, and egress policy from the capability id before dispatch; a per-call
   extension argument requires re-entrant dispatch or kernel surgery and blurs
   per-extension approval semantics).
2. **The contract is host-owned data.** Canonical input/output JSON schemas,
   description cores, and the error-code vocabulary live in one host module
   (`ironclaw_host_api::messaging`) and are resolved at runtime like builtin schemas.
   Extensions declare the binding and implement vendor mechanics; they cannot define
   or override the operation's shape.
3. **Canonical inputs AND outputs, enforced.** Inputs are validated pre-dispatch by
   the existing loop-host schema enforcement (free). Outputs are validated
   post-dispatch by new host code, for standard ops only; an output that fails
   canonical validation is a model-visible tool failure, never a silent pass-through.
   This makes the evidence rule structural: a send that cannot produce a
   `message_ref` *is* a failure.
4. **Full vocabulary now, bindable core, reserved names.** 16 core operations ship
   with contracts; 13 further names are reserved in the closed enum (binding them is
   an install-time error until a contract ships). `send_typing`, `mark_read`, and
   `set_presence` are excluded from the model-callable standard (presence-signal
   fabrication; channel-host concerns); `get_presence` folds into `get_user_info`
   fields.
5. **Slack re-badges its existing 8 ops only.** Zero new vendor mechanics, zero OAuth
   scope changes in this PR. acme-messenger implements all 16 core ops as the
   conformance fixture and keeps one bespoke tool to prove coexistence.
6. **Acting identity is per-extension, not per-standard.** Slack's ops act as the
   user (per-user OAuth token); a Telegram implementation would act as the bot
   (Telegram has no delegated user API). The standard's semantics are "the
   extension's connected messaging identity"; `whoami` reports it.
7. **Versioning: additive within manifest schema v3.** One optional field; v3's
   `deny_unknown_fields` gives fail-closed behavior on older binaries, and bundled
   manifests ship inside the binary. `standard_op` is v3-only: a v2 manifest
   declaring it fails parse with an explicit error.
8. **Naming:** field `standard_op`, bare op-name values (`"send_message"`). The host
   owns the closed vocabulary and keeps op names globally unique across any future
   standard families (email, calendar), so bare names cannot collide.

## 3. Current state (verified anchors)

- **Manifest v3 raw shapes:** `crates/extensions/ironclaw_extension_registry/src/v3.rs` — `RawToolV3`
  (`:126-158`, `deny_unknown_fields`), per-tool loop `:440-523` building
  `RawCapabilityV2` → `CapabilityDeclV2::from_raw`. Closed-enum precedent: invalid
  enum values fail install-time as `ManifestV3Error::Parse` (`:231-233`).
- **Resolved record:** `ResolvedExtensionManifest`
  (`crates/extensions/ironclaw_extension_registry/src/resolved.rs:34-71`) persisted via
  `WireManifestRecord` (`installations.rs:1484-1508`); production never reparses TOML;
  rehydration via `ExtensionManifestRecord::from_resolved` (`installations.rs:216-230`).
- **Descriptors:** `capability_descriptors_from_manifest`
  (`crates/extensions/ironclaw_extension_registry/src/lib.rs:703-743`); schema refs become
  `{"$ref": …}` and are dereferenced at
  `crates/kernel/ironclaw_host_runtime/src/surface.rs:289-350`, where **builtin refs already
  resolve from compiled-in constants** (`resolve_builtin_input_schema_ref`,
  `:300-315`) — the precedent this design extends.
- **Input validation exists; output validation does not.** Inputs: loop host
  validates against the resolved schema pre-dispatch
  (`crates/loop/ironclaw_loop_host/src/capability_port.rs:2611-2636`,
  `capability_port/provider_input.rs`). Outputs: `output_schema_ref` is parsed and
  threaded but **has no consumer**; nothing validates tool output anywhere.
- **Wire naming:** dotted capability id → `__` wire name
  (`capability_port.rs:3099-3115`); Slack's 8 ids are already exactly
  `slack.<op_name>` for 8 of the 16 core op names, so re-badging changes zero ids.
- **Structured guest errors exist:** WASM guests may return `{code, kind}`
  (`crates/kernel/ironclaw_host_runtime/src/services/wasm_execution.rs:333-349`), code
  sanitized to `[A-Za-z0-9_.-]{,64}` and surfaced on the model-visible cause channel
  — the transport for the standard error taxonomy.
- **Slack WASM dispatches on capability id already:** single `execute(params,
  context)` entry; `context.capability_id` maps to an internal action
  (`assets/slack/wasm-src/src/lib.rs:171-188`) — the ABI needs no change.
- **Origin gates, effects, permissions** are existing per-tool machinery
  (`crates/contracts/ironclaw_host_api/src/capability.rs`;
  `crates/app/ironclaw_composition/src/profile_approval_authorization.rs:279-414`)
  and are untouched by this design.

## 4. The vocabulary

One closed enum, `ironclaw_host_api::messaging::StandardMessagingOp`, snake_case wire
tokens, unknown names rejected at install time.

**Core — writes (6):** `send_message`, `edit_message`, `delete_message`,
`add_reaction`, `remove_reaction`, `open_dm`.

**Core — reads (6):** `list_conversations`, `get_conversation_info`,
`get_conversation_history`, `get_thread_replies`, `get_message`, `search_messages`.

**Core — people (4):** `get_user_info`, `resolve_user`, `list_members`, `whoami`.

**Reserved (13, names claimed, binding rejected with "reserved, not yet bindable"):**
`forward_message`, `schedule_message`, `list_reactions`, `pin_message`,
`unpin_message`, `list_pins`, `create_group`, `join_conversation`,
`leave_conversation`, `invite_member`, `remove_member`, `set_topic`,
`archive_conversation`.

**Excluded from the model-callable standard** (with reasons, revisit only with a
product decision): `send_typing` and `mark_read` (as act-as-user model ops they
fabricate human presence signals; assistant-activity indicators are a channel-host /
delivery concern under the bot identity), `set_presence` (impersonation-adjacent, no
use case). `get_presence` is folded into `get_user_info` output fields.

Slack v1 binds: `send_message`, `search_messages`, `list_conversations`,
`get_conversation_info`, `get_conversation_history`, `get_thread_replies`,
`get_user_info`, `whoami`.

Vendor-specific capabilities that never standardize (stay bespoke `[[tools]]`):
Slack ephemeral messages and Block Kit interactivity, WhatsApp templates, Telegram
polls/stickers, Discord slash-command surfaces, voice/video.

## 5. Canonical contracts

### 5.1 Nouns (defined once, reused by every op schema)

- **`conversation: string`** — opaque per-extension conversation ref. Obtained from
  `list_conversations` / `open_dm` / `get_conversation_info` / earlier results; never
  invented; never valid across extensions.
- **`message_ref: { conversation: string, message_id: string }`** — identity of one
  message. **Evidence out = address in:** every write returns one; `edit_message`,
  `delete_message`, and reactions take one.
- **`thread: string`** — opaque thread anchor (Slack: parent `ts`; Discord: thread
  id), adapter-interpreted.
- **`user_ref: string`** — opaque vendor user id, from people ops only; never derived
  from a conversation id.
- **`message` (read-output object):**
  `{ message_ref, author: { user_ref, display_name? }, text, timestamp? (RFC3339),
  is_self: bool, thread?: { thread, reply_count? }, edited?: bool, vendor?: object }`.
- **Pagination:** list-shaped ops take optional `cursor` and return optional
  `next_cursor` (opaque; adapters encode vendor paging — Slack search encodes page
  numbers).

### 5.2 Contract rules

1. **Inputs strictly closed** (`additionalProperties: false`), no vendor-specific
   parameters on standard ops (vendor knobs belong in bespoke tools). Enforced by the
   existing pre-dispatch loop-host validation.
2. **Outputs canonical and enforced.** Every core op has a full canonical output
   schema; the host validates standard-op outputs post-dispatch (new; §7.4). Outputs
   are closed except one optional **`vendor: object`** passthrough key for genuinely
   useful vendor extras.
3. **Text dialect:** `text` is the message body, markdown baseline. v1 keeps
   rendering fidelity vendor-side: the canonical core says text renders per channel;
   the vendor addendum notes dialect specifics (Slack mrkdwn). A canonical-markdown
   render obligation for tool-lane sends is a named follow-up (adding a converter to
   the Slack WASM module would violate the zero-vendor-mechanics scope of this PR).
4. **At-most-once per invocation.** No idempotency keys in v1 (no vendor support to
   build on; retries are explicit model decisions). Named follow-up if a vendor
   offers idempotent sends.
5. **Timestamps** are RFC3339 strings derived by the adapter; vendor-native ids stay
   raw inside `message_id`.

### 5.3 Descriptions: canonical core + vendor addendum

The model-visible description of a standard-op tool is composed at descriptor build:
`<canonical core>\n<vendor addendum>` (addendum = the manifest `description`, may be
empty). Cores are host-owned static text, extension-neutral phrasing ("this
extension"). Every core must state: the operation's purpose; the ref-provenance rule
(refs come from this extension's own discovery ops and never cross extensions); what
evidence it returns. The `send_message` core additionally carries the sibling fence
sentence (verbatim core in Appendix B). Cores live beside the schemas in the host
module; because composition happens at descriptor build (per boot), core wording
improvements take effect on binary upgrade without manifest digest changes.

`prompt_doc_ref` remains extension-owned and untouched (it is not injected into the
model surface on main; the hot-capability catalog that would serve it has no
production consumer).

## 6. Manifest binding and parse-time validation

`RawToolV3` and the shared `RawCapabilityV2`/`CapabilityDeclV2` gain
`#[serde(default)] standard_op: Option<StandardMessagingOp>`. Rules enforced in
`parse_v3`'s per-tool loop, all install-time fail-closed `ManifestV3Error`s:

1. Reserved op → `"standard_op '<name>' is reserved and not yet bindable"`.
2. Tool id must equal `<extension_id>.<op_name>` exactly.
3. `input_schema_ref` / `output_schema_ref` must be absent (host supplies canonical).
4. Effects floor: write-family ops (`is_write()`) must declare `external_write`;
   reads are not forced to.
5. At most one binding per op per extension.
6. v2 manifests declaring `standard_op` fail parse with
   `"standard_op requires manifest schema v3"` (the field threads through shared
   raw types; the v2 path rejects it explicitly).

Per-extension declarations keep today's meaning and stay required/owned by the entry:
credentials + scopes, effects, `default_permission`, `visibility`,
`origin_gate_matrix`. `description` becomes the vendor addendum (empty = none).

At parse, schema refs are synthesized as stable pointers:

```text
input_schema_ref  = "standard:messaging/<op_name>.input.v1"
output_schema_ref = "standard:messaging/<op_name>.output.v1"
```

## 7. Projection and runtime resolution

### 7.1 Authority home

`ironclaw_host_api::messaging` (new module in the existing vocabulary crate; a
dedicated crate would not earn its keep):

```rust
pub enum StandardMessagingOp { /* §4, snake_case serde */ }

pub struct StandardOpContract {
    pub op: StandardMessagingOp,
    pub input_schema: &'static str,    // include_str! JSON (schemas/messaging/…)
    pub output_schema: &'static str,
    pub description_core: &'static str,
    pub is_write: bool,
}

impl StandardMessagingOp {
    pub fn op_name(&self) -> &'static str;
    pub fn is_write(&self) -> bool;
    /// None = reserved (name claimed, not yet bindable).
    pub fn contract(&self) -> Option<&'static StandardOpContract>;
}

pub const STANDARD_SCHEMA_REF_PREFIX: &str = "standard:messaging/";
pub fn resolve_standard_schema_ref(schema_ref: &str) -> Option<&'static str>;

pub enum StandardMessagingErrorCode { /* §8, as_str() = "messaging.…" */ }
```

### 7.2 Record and descriptor threading

`CapabilityDeclV2` and `CapabilityDescriptor` gain
`#[serde(default)] standard_op: Option<StandardMessagingOp>` (rehydration-safe for
previously persisted resolved records). The resolved record stores the binding, the
synthesized `standard:` refs, and the raw addendum. The descriptor is the durable
platform attach-point: audit, display hints, the conformance suite, and any future
policy key off `descriptor.standard_op` — never off tool-name strings.

### 7.3 Resolution

`surface_descriptor` (`crates/kernel/ironclaw_host_runtime/src/surface.rs:289-350`) and the
hot-catalog publisher gain one branch beside the builtin resolver: `standard:` refs
resolve from the compiled-in registry. Description composition (core + addendum)
happens at descriptor build. Everything downstream — visible-surface assembly, wire
naming, input validation, authorize, lanes, credentials, egress — is untouched.

### 7.4 Output enforcement (new)

After a successful dispatch, when `descriptor.standard_op` is set, the host validates
the tool output against the canonical output schema (compiled validators cached
process-wide, one per op). Failure produces a **model-visible tool failure** with a
safe summary naming the top schema issues ("standard op output failed validation:
…"), never a terminal host error — the model can retry or report. Scoped strictly to
standard ops; bespoke tools keep today's behavior. Home: the host-runtime invoke
path, beside the existing outcome translation (`DefaultHostRuntime::invoke_capability`
in `crates/kernel/ironclaw_host_runtime/src/production.rs`); exact failure-kind mapping is
pinned at plan time with a regression test asserting model-visibility.

## 8. Error taxonomy

Closed code vocabulary, host-owned. Codes ride the existing channels — structured
WASM guest errors (`{code, kind}`) and `ToolError::Failed` safe summaries for
first-party — so no new plumbing. The standard defines the codes, their meaning, and
their failure class:

**Amended 2026-07-29 post-audit** (pre-merge amendment wave W6): added
`messaging.outside_messaging_window` (row marked †, 11 → 12 codes). Also
landed-behavior note: an unmapped vendor error's sanitized code does NOT ride
this taxonomy in detail on any shipped implementation — see
`standard-operations.md` §5.1 for the transport reality this table's original
"vendor code passes through in detail" wording oversimplified.

| Code | Meaning | Class |
|---|---|---|
| `messaging.unknown_conversation` | conversation ref doesn't resolve on this extension | invalid input |
| `messaging.unknown_message` | message_ref doesn't resolve | invalid input |
| `messaging.unknown_user` | user_ref doesn't resolve | invalid input |
| `messaging.not_a_member` | caller's identity isn't in the conversation | denied (vendor) |
| `messaging.permission_denied` | vendor-side authz (scope/role) | denied (vendor) |
| `messaging.cannot_message_user` | DMs closed / blocked | denied (vendor) |
| `messaging.outside_messaging_window` † | recipient reachable but free-form messaging is closed right now (e.g. a session-window policy); a template/re-engagement message or waiting may still succeed | denied (vendor) |
| `messaging.message_too_long` | over vendor limit | invalid input |
| `messaging.unsupported_content` | content the vendor can't render | invalid input |
| `messaging.rate_limited` | vendor rate limit | retryable |
| `messaging.edit_not_allowed` | not own message / edit window over | denied (vendor) |
| `messaging.vendor_error` | anything else; the closed vocabulary's catch-all | backend |

Rules: adapters map vendor errors to canonical codes once (Slack:
`channel_not_found` → `unknown_conversation`, `not_in_channel` → `not_a_member`,
`ratelimited` → `rate_limited`, …); unmapped vendor errors fall to
`messaging.vendor_error`. Credential
problems are **not** taxonomy — revoked/missing tokens keep riding the existing
`AuthRequired` → re-auth gate path unchanged.

## 9. Package changes

### 9.1 Slack (re-badge, zero vendor mechanics)

- `manifest.toml`: the 8 entries gain `standard_op`, drop `input_schema_ref`,
  descriptions shrink to vendor addenda (mention encoding `<@U…>`, mrkdwn dialect,
  raw-ids-never-in-replies, scope notes). Credentials, scopes, effects, origin
  gates, `[channel]`, `[auth.slack]`, `[admin_configuration]`: untouched.
- Package schema assets for the 8 ops (input + the unreferenced output JSONs) are
  deleted; canonical replaces them.
- WASM module: serde field renames (`channel`→`conversation`, `thread_ts`→`thread`),
  output shaping to canonical envelopes (`message_ref`, `author`, `is_self`,
  `next_cursor`, RFC3339 timestamps), one vendor→canonical error-code mapping table.
  Slack API calls byte-identical; module rebuilt and committed.
- Prompt docs: content overlapping the canonical cores is trimmed to vendor-specific
  notes (plan-time detail; not model-visible today either way).

### 9.2 acme-messenger (conformance vehicle)

- Implements **all 16 core ops** in its first-party adapter against the scripted
  vendor server (`api.acme.example`), including scripted failure modes for every
  canonical error code.
- **Keeps `acme-messenger.send_note` as a bespoke tool** beside the standard ops —
  the pinned proof that standard and bespoke tools coexist on one extension.

## 10. Testing

Integration-first per repo law; extend owning suites, don't proliferate.

1. **`ironclaw_extensions` contract tests** (extend `manifest_v3_contract.rs`):
   `standard_op` threading into resolved record + descriptor; one rejection test per
   §6 rule (reserved, id mismatch, schema-ref present, effects floor, duplicate,
   v2-declares); rehydration of pre-existing resolved records without the field;
   v2/v3 equivalence untouched.
2. **`ironclaw_host_api` unit:** enum wire tokens; registry completeness (every core
   op has parseable input+output schema + non-empty description core; write flags
   consistent); error-code vocabulary pinned; every reserved op returns
   `contract() == None`.
3. **`ironclaw_host_runtime`:** `standard:` ref resolution (hit; unknown ref fails
   closed); output validation (valid passes; missing `message_ref` → model-visible
   failure; `vendor` key admitted; bespoke tools bypass).
4. **Conformance suite** (exported test-support in `ironclaw_host_api`,
   parameterized over an extension's declared ops): canonical inputs accepted per
   op; outputs validate against canonical schemas; canonical codes emitted for the
   scripted failure cases; the evidence loop (send's `message_ref` accepted by
   edit/delete/react where declared). acme runs the full 16; Slack runs its 8
   through real WASM execution.
5. **Integration (`tests/integration/`):** scripted turns through the real stack —
   acme standard send end-to-end (canonical output asserted at the result seam,
   egress asserted); error-taxonomy scenario (scripted vendor failure →
   model-visible `messaging.unknown_conversation`, run completes, model recovers);
   bespoke-coexistence scenario (`send_note` still dispatches); Slack
   lifecycle/projection pins updated for the new descriptor field.
6. **Known fallout, handled deliberately:** golden payload snapshots pin the tool
   definitions sent to the model — canonical schemas change them (hash-only refresh
   protocol); recorded QA fixtures carrying old field names are audited and
   re-recorded or retired, never hand-edited. Coverage-floor recapture if the
   ratchet moves.
7. `cargo test -p ironclaw_architecture` before and after — no new gate; the
   specificity gate already keeps canonical contracts vendor-blind (no extension
   names in generic crates).

No WebUI changes → no frontend/e2e gate triggered.

## 11. Migration and coexistence

- **Bespoke tools stay legal indefinitely** (vendor-specific capabilities, §4).
  Review guidance, not a machine gate: an extension should not declare a bespoke
  tool that semantically duplicates a standard op it binds.
- **Nothing to sunset:** the `recipient_argument` interim never reached `main`
  (`pushy-today` was never PR'd); no cleanup exists.
- **Reserved-op graduation is additive:** an implementor lands → the op gains
  contract data + conformance rows; no manifest schema change, no version bump.
- **Future extensions** (Discord/WhatsApp/Telegram) declare their supported subset;
  vocabulary gaps are visible as absent tools, not runtime errors.
- **MCP extensions cannot bind standard ops in v1** (discovered tools have no
  `[[tools]]` entries); revisit when an MCP messaging vendor appears.
- **Sibling fence, wording only:** the `send_message` core carries one sentence —
  reaching other people and places is this tool's job; delivering answers/results
  *to the user* is the host's delivery affordance — phrased to survive whatever the
  delivery-tool project ships. No mechanical coupling in either direction.

## 12. Documentation

- New normative page `docs/reborn/extension-runtime/standard-operations.md`
  (vocabulary, binding rules, contract principles, error taxonomy) + a short §3.4
  pointer in `overview.md`. `openwiki/` untouched (auto-generated).
- `.claude/skills/reborn-extension-surfaces`: new subsection on binding standard
  ops; while editing, fix the three known-stale lines (`[channel.config]` →
  `[admin_configuration]`; Slack has 8 tools; `ChannelAdapter` lives at
  `crates/contracts/ironclaw_host_api/src/product_adapter/channel_adapter.rs`). The sibling
  project flagged the same lines; whoever lands second takes a trivial doc rebase.
- This spec's Appendix A/B pin the canonical contracts; the durable normative copy
  lands in `docs/reborn/extension-runtime/` with the implementation.

## 13. Deliberately not built

| Excluded | Why | Revisit when |
|---|---|---|
| Self-send guard / recipient policy | rejected in review-by-product 2026-07-27; bot-send default is the sibling delivery-tool project's domain | never here; posture questions belong to the sibling project |
| Generic router tool (`messaging__send_message` w/ extension arg) | kernel keys everything on capability id; disclosure layer owns tool-count pressure | real tool-count pain at many extensions — add a B1-style router *on top of* the standard |
| Standard-op bindings for `[mcp]` extensions | discovered tools have no `[[tools]]` entries | an MCP messaging vendor appears |
| Canonical-markdown render obligation | Slack WASM converter would break zero-vendor-mechanics scope | first vendor whose dialect diverges harmfully |
| Idempotency keys on writes | no vendor support to build on | a vendor offers idempotent sends |
| New Slack ops / scopes (edit, delete, reactions, open_dm, resolve_user) | keeps this the framework PR | immediate fast-follow after merge |
| Typing / read-receipt / presence-set ops | presence-signal fabrication; channel-host concerns | explicit product decision |
| WebUI standard-op badges | wire projection unchanged; no consumer need | a UI consumer wants op-level display |

## Appendix A — canonical schemas (16 core ops)

Shared shapes are written once here; each generated schema file
(`crates/contracts/ironclaw_host_api/schemas/messaging/<op>.{input,output}.v1.json`,
draft-07) inlines them and is self-contained. All inputs
`additionalProperties: false`. All outputs `additionalProperties: false` plus an
optional `vendor: object`. `*` = required.

**Shared:**

```jsonc
message_ref = { "conversation"*: string, "message_id"*: string }
author      = { "user_ref"*: string, "display_name"?: string }
message     = { "message_ref"*: message_ref, "author"*: author, "text"*: string,
                "timestamp"?: string (RFC3339), "is_self"*: boolean,
                "thread"?: { "thread"*: string, "reply_count"?: integer },
                "edited"?: boolean, "vendor"?: object }
user_match  = { "user_ref"*: string, "display_name"?: string }
conversation_info = { "conversation"*: string,
                "kind"*: "dm" | "group_dm" | "channel" | "other",
                "display_name"?: string, "is_member"?: boolean,
                "counterpart"?: author, "vendor"?: object }
```

**Writes:** (rows below marked † amended 2026-07-29 post-audit — pre-merge
amendment wave W3/W4/W5; see `amendment-wave-brief.md`)

| Op | Input | Output |
|---|---|---|
| `send_message` † | `{ conversation*, text*, thread?, reply_to? }` | `{ message_ref*, thread?, reply_to?, vendor? }` |
| `edit_message` | `{ message_ref*, text* }` | `{ message_ref*, vendor? }` |
| `delete_message` | `{ message_ref* }` | `{ deleted*: true, message_ref*, vendor? }` |
| `add_reaction` | `{ message_ref*, emoji* }` | `{ message_ref*, emoji*, vendor? }` |
| `remove_reaction` † | `{ message_ref*, emoji? }` | `{ message_ref*, emoji?, vendor? }` |
| `open_dm` | `{ user_ref* }` | `{ conversation*, vendor? }` |

`emoji` is the vendor's emoji token (name or unicode; addendum documents the
dialect); required on `add_reaction`, optional on `remove_reaction` † (absent =
remove the connected account's own reaction(s) — some vendors cannot name the
emoji on removal), echoed on output only when known. `deleted` is the literal
`true` (const) — a delete that didn't happen is an error, not `deleted: false`.
`reply_to` † (`message_ref` shape) is the specific message being quoted or
replied to, distinct from `thread` (the thread/topic container posted into) —
where a vendor has only one such mechanism the adapter maps both onto it;
`send_message`'s output echoes both when supplied and honored, so a silent
drop is checkable.

**Reads:**

| Op | Input | Output |
|---|---|---|
| `list_conversations` | `{ kinds?: [kind], limit?, cursor? }` | `{ conversations*: [conversation_info], next_cursor?, vendor? }` |
| `get_conversation_info` | `{ conversation* }` | `conversation_info` (top-level) |
| `get_conversation_history` | `{ conversation*, limit?, cursor? }` | `{ messages*: [message], next_cursor?, vendor? }` |
| `get_thread_replies` | `{ conversation*, thread*, limit?, cursor? }` | `{ messages*: [message], next_cursor?, vendor? }` |
| `get_message` | `{ message_ref* }` | `{ message*: message, vendor? }` |
| `search_messages` | `{ query*, sort?: "relevance"\|"timestamp", limit?, cursor? }` | `{ matches*: [message], next_cursor?, total?: integer, vendor? }` |

`limit` is `integer >= 1`; adapters clamp to vendor maxima. History ordering is
newest-first; thread replies return oldest-first (chronological) — both pinned
in the core descriptions. (**Amended 2026-07-29 post-audit**, W9/W12: this
paragraph previously read "History ordering is newest-first" without
distinguishing `get_thread_replies`, which is oldest-first — the core itself
carried the same error until W9/W12 fixed it.)

Search defaults to vendor relevance ranking. `sort: "timestamp"` requests
newest-first results for latest/most-recent questions; adapters map that
portable semantic onto their vendor's ordering controls.

**People:**

| Op | Input | Output |
|---|---|---|
| `get_user_info` | `{ user_ref* }` | `{ user_ref*, display_name?, real_name?, status_text?, status_emoji?, timezone?, title?, is_bot?, presence?: "active"\|"away"\|"unknown", vendor? }` |
| `resolve_user` | `{ query*, limit?, cursor? }` | `{ matches*: [user_match], next_cursor?, vendor? }` |
| `list_members` | `{ conversation*, limit?, cursor? }` | `{ members*: [user_match], next_cursor?, vendor? }` |
| `whoami` | `{}` | `{ user_ref*, display_name?, vendor? }` |

Canonical field descriptions (written into every generated schema): `conversation` —
"Conversation ref for this extension, from list_conversations / open_dm /
get_conversation_info or an earlier result. Never invented; never valid on another
extension." `thread` — "Opaque thread anchor from a message or message_ref — the
thread/topic container to post into. Distinct from reply_to, which quotes one
specific message rather than posting into a container." `message_ref` — "The exact
object returned by a prior send or read on this extension." `user_ref` — "Opaque
user id from this extension's people operations; never derived from a conversation
id." `reply_to` † (send_message input; amended 2026-07-29 post-audit, W4) — "The
exact message_ref of the specific message being quoted or replied to — distinct
from thread, which is the thread/topic container to post into. Vendors map this as
fits: Slack coincides with thread_ts, Telegram uses reply_parameters, WhatsApp uses
context, Signal uses a quote."

## Appendix B — description cores

Requirements for every core (host-owned static text, extension-neutral): state the
operation's purpose; the ref-provenance rule; the evidence returned. Reads state
ordering/paging; people ops state that refs feed other ops. Cores are plain text,
≤120 words each, finalized as `include_str!` assets in the implementation PR and
pinned by the registry-completeness test.

**`send_message` core (normative, verbatim):**

> **Amended 2026-07-29 post-audit** (pre-merge amendment wave W7, W11): the
> ref-provenance sentence was reworded binding-subset-agnostic (no longer
> names specific sibling ops, so it stays true for extensions that bind a
> narrower subset) and the thread sentence now also covers `reply_to`
> (W4's new optional input). The delivery-fence sentence (last, below) is
> UNCHANGED — this amendment does not touch it.

> Send a message to a conversation on this extension, as its connected account.
> `conversation` is an opaque conversation ref belonging to this extension — from
> this extension's discovery operations, where available, or a conversation ref
> the channel context provides, or an earlier message_ref; never invented, never
> reused across extensions. Optional `thread` posts into that thread/topic
> container; optional `reply_to` quotes one specific message — where only one such
> mechanism exists, the adapter maps both onto it. Returns a `message_ref` — use it
> with edit_message, delete_message, and reaction operations where available. Use
> this to reach other people and places when messaging is itself the requested
> task; delivering your answer or results to the user is the host's delivery
> affordance, not this tool.
