# Standard Messaging Operations

**Status:** Current (standardized messaging framework).
**Authority module:** `ironclaw_host_api::messaging`
(`crates/contracts/ironclaw_host_api/src/messaging.rs`).
**Design source:** `docs/internal/superpowers/specs/2026-07-27-standardized-messaging-framework-design.md`
(§§4-8, Appendix A/B). This page condenses that spec into the durable
reference for anyone binding, calling, or reviewing a standard messaging
operation. It does not fork the spec's semantics — where landed code diverges
from the spec's original wording, this page states the **landed behavior**
and calls the divergence out explicitly (see §5.1). A disagreement between
this page and the spec is resolved by a new approved spec revision, not by
silent drift here.

## 1. Why

Every channel extension used to ship bespoke model-callable messaging tools:
its own send/read shapes, its own recipient field names, its own error
strings. The model had to learn N vendor shapes for one concept ("send a
message to a conversation"), and nothing declared vendor capability gaps or
enforced an evidence contract (a send returning proof it happened). The
standard closes this with one host-owned vocabulary of messaging operations
that extensions **bind**, not reimplement: extensions declare which
operations their tools implement and supply vendor mechanics; canonical
shapes, wording, and error codes live once in the host.

## 2. The vocabulary

One closed enum, `ironclaw_host_api::messaging::StandardMessagingOp`,
snake_case wire tokens (`op_name()`). An unknown name is rejected at
manifest-parse time (§3); the enum itself has no other constructor.

### Core operations (16) — full contract, `contract()` returns `Some`

| Group | Operations |
|---|---|
| Writes (6) | `send_message`, `edit_message`, `delete_message`, `add_reaction`, `remove_reaction`, `open_dm` |
| Reads (6) | `list_conversations`, `get_conversation_info`, `get_conversation_history`, `get_thread_replies`, `get_message`, `search_messages` |
| People (4) | `get_user_info`, `resolve_user`, `list_members`, `whoami` |

Slack binds 8 of the 16 (`search_messages`, `list_conversations`,
`get_conversation_info`, `get_conversation_history`, `get_thread_replies`,
`get_user_info`, `whoami`, `send_message`) — a re-badge of its pre-existing
tools, zero new vendor mechanics. `acme-messenger`
(`tests/fixtures/extensions/acme-messenger/`) is the conformance fixture and
binds all 16.

### Reserved (13) — name claimed, `contract()` returns `None`

`forward_message`, `schedule_message`, `list_reactions`, `pin_message`,
`unpin_message`, `list_pins`, `create_group`, `join_conversation`,
`leave_conversation`, `invite_member`, `remove_member`, `set_topic`,
`archive_conversation`. Binding one fails manifest parse with
`"standard_op '<name>' is reserved and not yet bindable"` — the name is
claimed in the closed enum so it can never collide with a future op, but no
contract exists yet (§6).

### Excluded from the model-callable standard (3, with reasons)

| Op | Why excluded | Revisit when |
|---|---|---|
| `send_typing` | Act-as-user model op would fabricate a human presence signal | Product decision |
| `mark_read` | Same — assistant-activity indicators are a channel-host/delivery concern under the bot identity, not a model tool | Product decision |
| `set_presence` | Impersonation-adjacent; no use case | Product decision |

`get_presence` is not excluded — it folds into `get_user_info`'s output
`presence` field (`"active" \| "away" \| "unknown"`) rather than existing as
its own op.

## 3. Binding: the manifest `standard_op` field

`RawToolV3` and the shared `RawCapabilityV2`/`CapabilityDeclV2` carry
`#[serde(default)] standard_op: Option<StandardMessagingOp>`
(`crates/extensions/ironclaw_extension_registry/src/v3.rs`, `.../src/v2.rs`). A bound tool's
`input_schema_ref`/`output_schema_ref` are **synthesized by the host at
parse time**, never author-declared:

```text
input_schema_ref  = "standard:messaging/<op_name>.input.v1"
output_schema_ref = "standard:messaging/<op_name>.output.v1"
```

`ironclaw_host_api::messaging::resolve_standard_schema_ref` resolves these
refs from the compiled-in registry at descriptor build — the same precedent
builtin schema refs already use.

The `.v1` suffix is a literal, not a placeholder. Published schema files are
immutable once shipped: a schema change ships as a new version file alongside
the old one (`.v2`), never an in-place edit, and the registry serves every
published version forever — an installed manifest whose synthesized refs still
name `.v1` keeps resolving to `.v1` indefinitely. A binding moves to a new
schema version only when its own `standard:` refs change, which rides the
same manifest-digest-changing rebind path any other manifest change does;
nothing re-resolves an existing binding to a newer schema version silently.

### The six binding validations (spec §6), fail-closed at manifest parse

Enforced in `parse_v3`'s per-tool loop
(`crates/extensions/ironclaw_extension_registry/src/v3.rs`); each violation is a
`ManifestV3Error::Invalid` (install-time, never a runtime surprise):

1. **Reserved op** → `"standard_op '<name>' is reserved and not yet bindable"`.
2. **Tool id must equal `<extension_id>.<op_name>` exactly** →
   `"standard op tool id must be '<expected_id>', got '<id>'"`.
3. **Schema refs must be absent** on a bound tool (host supplies the
   canonical ones) →
   `"standard op '<id>' uses host-canonical schemas; remove input_schema_ref/output_schema_ref"`.
4. **Effects floor** — a write op (`is_write()`) must declare
   `external_write`; reads are not forced to →
   `"standard op '<id>' is a write operation and must declare the external_write effect"`.
5. **At most one binding per op per extension** →
   `"standard op '<op>' may be bound at most once per extension"`.
6. **v2 manifests declaring `standard_op` fail parse** — the field threads
   through the shared raw types, but the v2 reader rejects it explicitly:
   `"standard_op requires manifest schema v3"`
   (`crates/extensions/ironclaw_extension_registry/src/v2.rs`). `standard_op` is additive,
   v3-only vocabulary; it does not require a v3 schema *version* bump beyond
   the field's own presence.

A convergent seventh check (shared by both manifest versions, in `v2.rs`)
rejects a **bespoke** tool that hand-writes a `standard:` schema ref — that
namespace is reserved to real `standard_op` bindings, so a tool cannot wear a
canonical schema while skipping every validation above.

### MCP extensions cannot bind

A `standard_op` on a manifest that also declares `[mcp]` fails parse:
`"tool '<id>' declares standard_op on an [mcp] manifest; static tools inherit
the server connection template and cannot bind a standard op"`. Discovered
MCP tools have no `[[tools]]` entries to carry the field in the first place
(spec §11); this is revisited only if an MCP messaging vendor appears.

### Install-time asset validation exemption

Bundled-package asset validation (`validate_bundled_package_assets` /
`is_standard_op_schema_ref`,
`crates/extensions/ironclaw_extension_host/src/available_extensions.rs:775-778`) checks
that every manifest-declared schema/prompt ref ships a matching package
asset file, and exempts `standard:`-prefixed refs from that check by prefix.
This is not itself a resolvability guarantee — it only means the asset
validator stays out of the way of a ref shape it was never meant to check.
Parse-time gating (the six rules above) already guarantees that only a
resolvable `standard:messaging/<op>.v1` ref can be synthesized in the first
place; the fail-closed backstop for a ref that somehow still doesn't
resolve is schema resolution erroring at descriptor/surface build in
`ironclaw_host_runtime`, not this validator.

## 4. Contract principles

### 4.1 Nouns (defined once, reused by every op schema)

- **`conversation: string`** — opaque per-extension conversation ref, from
  `list_conversations` / `open_dm` / `get_conversation_info` / an earlier
  result. Never invented; never valid across extensions.
- **`message_ref: { conversation, message_id }`** — identity of one message.
  Evidence out = address in: every write returns one; `edit_message`,
  `delete_message`, and the reaction ops take one.
- **`emoji: string`** — vendor emoji token (name or unicode). Required on
  `add_reaction`; optional on `remove_reaction`, where an absent `emoji`
  means "remove the connected account's own reaction(s)" — some vendors
  cannot name the emoji being removed. `remove_reaction`'s output echoes
  `emoji` only when known.
- **`thread: string`** — opaque thread anchor (Slack: parent `ts`; Discord:
  thread id), adapter-interpreted. Distinct from `reply_to`: `thread` is the
  thread/topic container a message posts *into*; `reply_to: message_ref`
  (`send_message` input only) is the specific message being quoted or
  replied to. Where a vendor has only one such mechanism (Slack: both
  coincide with `thread_ts`), the adapter maps both onto it. `send_message`'s
  output optionally echoes both `thread` and `reply_to` so a silent drop is
  checkable.
- **`user_ref: string`** — opaque vendor user id, from people ops only; never
  derived from a conversation id.
- **`message`** (read-output object) — `message_ref`, `author { user_ref,
  display_name? }`, `text`, `timestamp?` (RFC3339), **`is_self: bool`
  (required)**, `thread? { thread, reply_count? }`, `edited?`, `vendor?`.
  `is_self` is never omitted and never fabricated `true`: the landed
  convention (Slack, the acme fixture) is to set it `false` when
  self-authorship can't be determined, never to guess or drop the field.
- **`conversation_info`** — `conversation`, **`kind`** (`"dm" | "group_dm" |
  "channel" | "other"` — no public/private split), `display_name?`,
  `is_member?`, `counterpart?`, `vendor?`.
- **Pagination** — list-shaped ops take an optional `cursor` and return an
  optional `next_cursor` (opaque; adapters encode vendor paging).

### 4.2 Closed inputs

Every canonical input schema is `additionalProperties: false`. No
vendor-specific parameters ride a standard op's input — a vendor knob belongs
in a bespoke tool instead. Enforced for free by the existing pre-dispatch
loop-host schema validation; nothing new was built for this half.

### 4.3 Enforced outputs

Every core op has a full canonical output schema, also
`additionalProperties: false` except one optional `vendor: object`
passthrough key (§4.4). Unlike inputs, output enforcement is **new** host
code: after a capability bound to a standard op dispatches successfully, the
host validates its output against that op's canonical schema
(`ironclaw_host_runtime::standard_op_output::standard_op_output_violations`,
compiled `jsonschema::Validator`s cached process-wide) before the outcome is
allowed to become `Completed`. A violation becomes a model-visible `Failed`
outcome instead — `RuntimeFailureKind::InvalidOutput`, the same kind WASM
`InvalidResult` dispatch errors already produce — with a bounded summary
(`"standard op output failed validation: …"`, at most 3 issues, each
stripped of instance values and truncated to 200 characters) so the model
can retry, adjust its call, or report the broken extension rather than a run
silently completing with a shape nothing downstream validated. This makes
the evidence rule structural: a `send_message` that cannot produce a
`message_ref` *is* a failure, never a silent pass-through. Bespoke
capabilities (`standard_op: None`) are never touched by this check. The
enforcement sits at one choke point
(`completed_or_output_violation_outcome` in
`crates/kernel/ironclaw_host_runtime/src/production.rs`) shared by every path that
can complete a capability — invoke, resume, and auth-resume — so it cannot be
skipped on one entry path while covered on another.

### 4.4 The `vendor` key

Every canonical output carries one optional `vendor: object` passthrough for
genuinely useful vendor extras that don't warrant a canonical field.
Exemplar: Slack's `get_user_info` carries `vendor.status_expiration` (the
Slack-specific status-expiry timestamp — not a concept every messaging
vendor has, so it is not canonical, but real enough to pass through rather
than drop). Outputs are otherwise closed; this is the one deliberate escape
hatch.

### 4.5 Dialect posture

`text` is the message body, markdown baseline. v1 keeps rendering fidelity
vendor-side: the canonical description says text renders per channel, and
the vendor addendum (§4.6) notes dialect specifics (e.g. Slack `mrkdwn`). A
canonical-markdown render obligation for tool-lane sends is a named,
not-yet-built follow-up — adding a converter to the Slack WASM module would
violate the zero-vendor-mechanics scope of the initial rollout.

### 4.6 Descriptions: canonical core + vendor addendum

The model-visible description of a standard-op tool is
`<canonical core>\n<vendor addendum>`, composed at descriptor build. The core
is host-owned static text (`include_str!`-compiled from
`crates/contracts/ironclaw_host_api/prompts/messaging/<op>.core.md`), extension-neutral
phrasing ("this extension"); the addendum is the manifest's own `description`
field on the bound tool, and may be empty. `send_message`'s core additionally
carries the sibling-fence sentence: reaching other people and places is this
tool's job; delivering the assistant's own answer to the user is the host's
delivery affordance, not this tool.

## 5. Error taxonomy

Closed, host-owned vocabulary
(`ironclaw_host_api::messaging::StandardMessagingErrorCode`, `as_str()` =
`"messaging.*"`). Adapters map their own vendor's errors to these codes once;
anything unmapped falls to the catch-all.

| Code | Meaning | Class |
|---|---|---|
| `messaging.unknown_conversation` | conversation ref doesn't resolve on this extension | invalid input |
| `messaging.unknown_message` | message_ref doesn't resolve | invalid input |
| `messaging.unknown_user` | user_ref doesn't resolve | invalid input |
| `messaging.not_a_member` | caller's identity isn't in the conversation | denied (vendor) |
| `messaging.permission_denied` | vendor-side authz (scope/role) | denied (vendor) |
| `messaging.cannot_message_user` | DMs closed / blocked | denied (vendor) |
| `messaging.outside_messaging_window` | recipient reachable but free-form messaging is closed right now (e.g. a session-window policy); a template/re-engagement message or waiting may still succeed | denied (vendor) |
| `messaging.message_too_long` | over vendor limit | invalid input |
| `messaging.unsupported_content` | content the vendor can't render | invalid input |
| `messaging.rate_limited` | vendor rate limit | retryable |
| `messaging.edit_not_allowed` | not own message / edit window over | denied (vendor) |
| `messaging.vendor_error` | anything else; the closed vocabulary's catch-all | backend |

Credential problems are **not** part of this taxonomy — a revoked or missing
token keeps riding the existing `AuthRequired` → re-auth gate path unchanged.

### 5.1 Transport reality (landed divergence from the spec's original wording)

The spec's Appendix framing suggested an unmapped vendor error's "raw code
passes through in detail." Landed behavior is narrower, because of the shape
of the channel these codes actually ride:

- **WASM guests** (Slack, and any future WASM channel extension) return a
  structured error over `StructuredWasmGuestError { code, kind }`
  (`crates/kernel/ironclaw_host_runtime/src/services/wasm_execution.rs`) —
  **exactly two fields**, sanitized to a `[A-Za-z0-9_.-]{,64}` code and one
  of seven `kind` values (`AuthRequired | Input | OutputTooLarge | Executor |
  NetworkDenied | Client | OperationFailed`, matching
  `StructuredWasmGuestErrorKind` — not the spec's illustrative class-column
  labels verbatim). There is no separate "detail" slot alongside `code`. The
  Slack WASM module (`slack_error_to_standard_code` in
  `assets/slack/wasm-src/src/api.rs`) therefore puts the **canonical**
  `messaging.*` string into `code` and never the raw Slack error — the raw
  vendor code is consumed by the mapping function but does not cross the
  guest→host boundary on this channel at all.
- **First-party adapters** use `ToolError::Failed`'s `safe_summary` /
  `model_visible_cause` (`ironclaw_host_api::tool_adapter`), which has more
  room in principle (`model_visible_cause` is a free-form string, not a fixed
  two-field struct). The landed `acme-messenger` conformance fixture still
  puts only the canonical code in `safe_summary`
  (`"acme vendor rejected the request: <canonical code>"`), for parity with
  the WASM transport's behavior — not because the shape forces it.

Net: **the canonical `messaging.*` code is what reaches the model-visible
summary on every landed implementation; a raw vendor code is never threaded
to the model.** Server-side visibility into the original vendor error, where
it exists, comes from the host's own egress/response logging of the
underlying HTTP call — not from this taxonomy channel.

## 6. Conformance expectations

Test-support helpers live at
`ironclaw_host_api::test_support::messaging_conformance`, gated behind the
`test-support` Cargo feature (never compiled into a shipped binary — see
`.claude/rules/cargo-features.md` bar #4). They give an extension's own tests
direct assertions against the Task 1 registry instead of hand-rolling
`jsonschema::validator_for` calls:

- `assert_canonical_input_accepted` / `assert_canonical_input_rejected` — an
  input is (or is not) accepted by `op`'s canonical input schema.
- `assert_canonical_output` — an output satisfies `op`'s canonical output
  schema.
- `message_ref_from_output` — extracts a write's `message_ref` for the
  evidence loop (feeding `edit_message`/`delete_message`/reaction inputs from
  a prior `send_message` result).

An extension binding standard ops runs its own protocol tests through these
helpers. The framework itself is covered at every tier:

- **`ironclaw_extension_registry` contract tests**
  (`crates/extensions/ironclaw_extension_registry/tests/manifest_v3_contract.rs`): one rejection
  test per binding validation in §3 (reserved, id mismatch, schema-ref
  present, effects floor, duplicate, v2-declares, MCP-incompatible,
  bespoke-declares-standard-namespace), plus threading and descriptor
  composition tests.
- **`ironclaw_host_api` unit tests** (`crates/contracts/ironclaw_host_api/src/messaging.rs`):
  wire-token round-trip, registry completeness (exactly 16 core ops carry
  parseable, compiling input+output schemas and a non-empty description
  core), reserved-op count (13), schema-ref resolution, and the 12-code
  error vocabulary.
- **`ironclaw_host_runtime`** (`crates/kernel/ironclaw_host_runtime/src/standard_op_output.rs`):
  output-validator unit tests (valid output passes; a missing `message_ref`
  fails; the `vendor` key is admitted; a reserved op has nothing to enforce).
- **Integration** (`tests/integration/extension_runtime.rs`): the full-stack
  proofs — acme's standard ops group dispatching through the real pipeline,
  a scripted vendor failure surfacing as a model-visible
  `messaging.unknown_conversation` with the run continuing, and the
  bespoke-coexistence scenario (§8) asserting `send_note` still dispatches
  alongside the standard ops on the same extension.
- `cargo test -p ironclaw_architecture_tests` before and after any change here —
  no new gate was added; the existing specificity gate already keeps the
  canonical contracts vendor-blind (no extension names in generic crates).

## 7. Reserved-op graduation

Landing an implementor for a reserved op is **additive**: the op gains a
`StandardOpContract` (schemas + description core) and conformance rows, and
`contract()` starts returning `Some` for it. This requires **no manifest
schema version change and no version bump** — the enum variant and its wire
token already exist; only the contract data is new. Existing manifests that
don't bind the newly-graduated op are unaffected.

## 8. Coexistence guidance

Bespoke `[[tools]]` entries remain legal indefinitely — the standard is not
a closed list of everything a messaging extension can expose. Vendor
specific capabilities that will never standardize include Slack ephemeral
messages and Block Kit interactivity, WhatsApp templates, Telegram
polls/stickers, Discord slash-command surfaces, and voice/video.

The one rule is **review guidance, not a machine gate**: an extension should
not declare a bespoke tool that semantically duplicates a standard op it
already binds. `acme-messenger` is the pinned proof that legitimate
coexistence looks different from duplication — it binds all 16 core ops
*and* keeps `acme-messenger.send_note` as a distinct bespoke tool
(`tests/fixtures/extensions/acme-messenger/manifest.toml`), because
`send_note` is not a messaging send in the standard's sense. There is no
architecture test enforcing non-duplication; catch it in code review the way
any other API-surface redundancy gets caught.
