# Standardized Messaging Framework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Host-owned standard messaging operations (closed vocabulary, canonical
enforced input/output schemas, error taxonomy) that channel extensions bind via one
`standard_op` manifest field — Slack re-badged onto its existing 8 ops, acme-messenger
implementing all 16 core ops as the conformance fixture.

**Architecture:** Design A / tools-entry binding (spec:
`docs/superpowers/specs/2026-07-27-standardized-messaging-framework-design.md`).
Standard ops project as ordinary per-extension tool surfaces; capability ids, wire
names, approvals, credentials, dispatch are untouched. New: an
`ironclaw_host_api::messaging` authority module (enum + contracts + error codes), a
`standard:` schema-ref resolver beside the builtin one, post-dispatch output
validation for standard ops, and parse-time binding validation in manifest v3.
**No self-send guard anywhere** — rejected in review-by-product; the channel-delivery
tool is a sibling project referenced in guidance wording only.

**Tech Stack:** Rust workspace (`crates/`), TOML manifests
(`reborn.extension_manifest.v3`), JSON Schema draft-07 (`jsonschema` crate 0.46),
WASM extension module (`wit_bindgen`, built by `scripts/build-wasm-extensions.sh`),
in-process integration harness (`tests/integration/`).

## Global Constraints

- No `.unwrap()`/`.expect()` in production code; `thiserror` errors; map with
  context. No `unwrap_or_default()`/`.ok()?` on fallible boundary calls without a
  `// silent-ok:` justification (`.claude/rules/error-handling.md`).
- Newtypes/enums over strings (`.claude/rules/types.md`). Raw TOML shapes are
  `#[serde(deny_unknown_fields)]`; persisted shapes are not; new persisted fields
  are `#[serde(default)]` so pre-existing rows rehydrate.
- Multi-line prompt/description text lives in `prompts/*.md` (or `schemas/*.json`)
  loaded via `include_str!`, inside the crate that owns the behavior — never inline
  Rust string constants.
- Model-correctable failures surface as model-visible **Denied/Failed** outcomes,
  never host errors that kill the run (`.claude/rules/agent-loop-capabilities.md`).
- Generic crates never name a concrete extension ("slack"/"acme" strings only in
  their packages/tests) — `cargo test -p ironclaw_architecture` gates this.
- Integration-first testing: extend the suite that owns the seam
  (`.claude/rules/testing.md`, `tests/integration/CLAUDE.md`). Scripted replies via
  `RebornScriptedReply` only.
- Zero clippy warnings, both lanes:
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` and
  `cargo clippy --all --tests --examples -- -D warnings`.
- Deep integration binaries need `RUST_MIN_STACK=16777216`.
- Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- Spec appendix A/B (`docs/superpowers/specs/2026-07-27-standardized-messaging-framework-design.md`)
  is the normative source for all 16 op schemas and the description-core
  requirements; where this plan shows only exemplars, transcribe the rest from the
  spec verbatim.

**Verified code anchors below are from 2026-07-27 against `main` @ `31b9583c2`; if a
line has drifted, grep the named symbol.**

---

### Task 1: `ironclaw_host_api::messaging` — vocabulary, contracts, error codes

**Files:**
- Create: `crates/contracts/ironclaw_host_api/src/messaging.rs`
- Create: `crates/contracts/ironclaw_host_api/schemas/messaging/<op>.input.v1.json` and
  `<op>.output.v1.json` — 32 files, one pair per core op
- Create: `crates/contracts/ironclaw_host_api/prompts/messaging/<op>.core.md` — 16 files
- Modify: `crates/contracts/ironclaw_host_api/src/lib.rs` (module decl + re-exports beside the
  existing `capability`/`channel` modules)
- Test: `#[cfg(test)]` in `messaging.rs`

**Interfaces:**
- Produces (consumed by every later task — exact names are load-bearing):

```rust
pub enum StandardMessagingOp {
    // core writes
    SendMessage, EditMessage, DeleteMessage, AddReaction, RemoveReaction, OpenDm,
    // core reads
    ListConversations, GetConversationInfo, GetConversationHistory,
    GetThreadReplies, GetMessage, SearchMessages,
    // core people
    GetUserInfo, ResolveUser, ListMembers, Whoami,
    // reserved (contract() == None)
    ForwardMessage, ScheduleMessage, ListReactions, PinMessage, UnpinMessage,
    ListPins, CreateGroup, JoinConversation, LeaveConversation, InviteMember,
    RemoveMember, SetTopic, ArchiveConversation,
}
impl StandardMessagingOp {
    pub fn op_name(&self) -> &'static str;          // "send_message", …
    pub fn is_write(&self) -> bool;                  // 6 core writes = true; reserved per spec §4 grouping
    pub fn contract(&self) -> Option<&'static StandardOpContract>; // None = reserved
    pub const ALL: &'static [StandardMessagingOp];   // every variant, core first
}
pub struct StandardOpContract {
    pub op: StandardMessagingOp,
    pub input_schema: &'static str,       // include_str! JSON
    pub output_schema: &'static str,
    pub description_core: &'static str,   // include_str! markdown
    pub is_write: bool,
}
pub const STANDARD_SCHEMA_REF_PREFIX: &str = "standard:messaging/";
pub fn resolve_standard_schema_ref(schema_ref: &str) -> Option<&'static str>;
pub enum StandardMessagingErrorCode {
    UnknownConversation, UnknownMessage, UnknownUser, NotAMember, PermissionDenied,
    CannotMessageUser, OutsideMessagingWindow, MessageTooLong, UnsupportedContent, RateLimited,
    EditNotAllowed, VendorError,
}
impl StandardMessagingErrorCode {
    pub fn as_str(&self) -> &'static str;            // "messaging.unknown_conversation", …
    pub const ALL: &'static [StandardMessagingErrorCode];
}
```

- Serde: `#[derive(Serialize, Deserialize)] #[serde(rename_all = "snake_case")]` on
  `StandardMessagingOp` (wire tokens = `op_name()`), plus `Debug, Clone, Copy,
  PartialEq, Eq, Hash`.

- [ ] **Step 1: Write the failing unit tests** in `messaging.rs` `mod tests`:

```rust
#[test]
fn op_names_round_trip_snake_case_serde() {
    for op in StandardMessagingOp::ALL {
        let token = serde_json::to_value(op).expect("serializes");
        assert_eq!(token, serde_json::Value::String(op.op_name().to_string()));
        let back: StandardMessagingOp =
            serde_json::from_value(token).expect("deserializes");
        assert_eq!(back, *op);
    }
}

#[test]
fn sixteen_core_ops_have_complete_contracts() {
    let core: Vec<_> = StandardMessagingOp::ALL
        .iter()
        .filter(|op| op.contract().is_some())
        .collect();
    assert_eq!(core.len(), 16, "exactly the 16 core ops carry contracts");
    for op in core {
        let contract = op.contract().expect("core contract");
        for (label, schema) in [
            ("input", contract.input_schema),
            ("output", contract.output_schema),
        ] {
            let parsed: serde_json::Value = serde_json::from_str(schema)
                .unwrap_or_else(|e| panic!("{} {label} schema parses: {e}", op.op_name()));
            jsonschema::validator_for(&parsed)
                .unwrap_or_else(|e| panic!("{} {label} schema compiles: {e}", op.op_name()));
        }
        assert!(!contract.description_core.trim().is_empty(), "{} core", op.op_name());
        assert_eq!(contract.is_write, op.is_write());
    }
}

#[test]
fn reserved_ops_have_no_contract() {
    assert!(StandardMessagingOp::ForwardMessage.contract().is_none());
    assert!(StandardMessagingOp::ArchiveConversation.contract().is_none());
    assert_eq!(
        StandardMessagingOp::ALL.iter().filter(|op| op.contract().is_none()).count(),
        13
    );
}

#[test]
fn standard_schema_refs_resolve() {
    let hit = resolve_standard_schema_ref("standard:messaging/send_message.input.v1")
        .expect("send input resolves");
    assert!(hit.contains("conversation"));
    assert!(resolve_standard_schema_ref("standard:messaging/nope.input.v1").is_none());
    assert!(resolve_standard_schema_ref("schemas/slack/x.json").is_none());
}

#[test]
fn error_codes_are_namespaced() {
    for code in StandardMessagingErrorCode::ALL {
        assert!(code.as_str().starts_with("messaging."), "{}", code.as_str());
    }
    assert_eq!(StandardMessagingErrorCode::ALL.len(), 12);
}

#[test]
fn write_output_schemas_require_evidence() {
    for op in [StandardMessagingOp::SendMessage, StandardMessagingOp::EditMessage] {
        let schema: serde_json::Value =
            serde_json::from_str(op.contract().unwrap().output_schema).unwrap();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "message_ref"), "{}", op.op_name());
    }
}
```

  `jsonschema` is not currently a dependency of `ironclaw_host_api` — add it as
  an optional normal dependency (`jsonschema = { version = "0.46", default-features = false,
  optional = true }`) enabled by the crate's `test-support` feature. This keeps the
  public downstream conformance helpers buildable while production validation
  continues to happen in host_runtime (Task 5).

- [ ] **Step 2: Run** — `cargo test -p ironclaw_host_api messaging` → FAIL
  (module does not exist).

- [ ] **Step 3: Write the 32 schema files.** Transcribe every op's input/output
  schema from spec Appendix A. Shared shapes are inlined into each file
  (self-contained draft-07, no cross-file `$ref`). All inputs
  `"additionalProperties": false`; all outputs `"additionalProperties": false` with
  an optional `"vendor": { "type": "object" }` property. Use the spec's canonical
  field descriptions verbatim for `conversation`, `thread`, `message_ref`,
  `user_ref`. The two normative exemplars (copy exactly):

`schemas/messaging/send_message.input.v1.json`:

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Standard messaging send_message input",
  "type": "object",
  "required": ["conversation", "text"],
  "properties": {
    "conversation": {
      "type": "string",
      "minLength": 1,
      "description": "Conversation ref for this extension, from list_conversations / open_dm / get_conversation_info or an earlier result. Never invented; never valid on another extension."
    },
    "text": {
      "type": "string",
      "minLength": 1,
      "description": "Message text (markdown baseline; rendered per channel)."
    },
    "thread": {
      "type": "string",
      "description": "Opaque thread anchor from a message or message_ref, to reply in-thread."
    }
  },
  "additionalProperties": false
}
```

`schemas/messaging/send_message.output.v1.json`:

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Standard messaging send_message output",
  "type": "object",
  "required": ["message_ref"],
  "properties": {
    "message_ref": {
      "type": "object",
      "required": ["conversation", "message_id"],
      "properties": {
        "conversation": { "type": "string" },
        "message_id": { "type": "string" }
      },
      "additionalProperties": false,
      "description": "Provider-issued evidence for the sent message; pass to edit_message / delete_message / reaction operations."
    },
    "vendor": { "type": "object" }
  },
  "additionalProperties": false
}
```

  The `message` object shape (history/thread/search/get_message outputs), inlined
  per file:

```json
{
  "type": "object",
  "required": ["message_ref", "author", "text", "is_self"],
  "properties": {
    "message_ref": { "type": "object", "required": ["conversation", "message_id"],
      "properties": { "conversation": {"type": "string"}, "message_id": {"type": "string"} },
      "additionalProperties": false },
    "author": { "type": "object", "required": ["user_ref"],
      "properties": { "user_ref": {"type": "string"}, "display_name": {"type": "string"} },
      "additionalProperties": false },
    "text": { "type": "string" },
    "timestamp": { "type": "string", "description": "RFC3339" },
    "is_self": { "type": "boolean" },
    "thread": { "type": "object", "required": ["thread"],
      "properties": { "thread": {"type": "string"}, "reply_count": {"type": "integer", "minimum": 0} },
      "additionalProperties": false },
    "edited": { "type": "boolean" },
    "vendor": { "type": "object" }
  },
  "additionalProperties": false
}
```

  `delete_message` output pins `"deleted": { "const": true }` in `required`.
  `list_conversations` `kinds` items enum: `["dm", "group_dm", "channel", "other"]`.
  `limit` fields: `{ "type": "integer", "minimum": 1 }`.

- [ ] **Step 4: Write the 16 description cores** (`prompts/messaging/<op>.core.md`).
  Each ≤120 words, extension-neutral, stating purpose + ref-provenance rule +
  evidence returned (spec Appendix B). `send_message.core.md` is normative-verbatim
  from spec Appendix B (including the final sentence: "Use this to reach other
  people and places when messaging is itself the requested task; delivering your
  answer or results to the user is the host's delivery affordance, not this tool.").
  Reads state newest-first ordering and cursor paging; people ops state their refs
  feed other ops; `whoami` states it reports the extension's connected messaging
  identity (which may be a bot).

- [ ] **Step 5: Implement `messaging.rs`** — enum with the serde derives, `ALL`,
  `op_name` (match), `is_write` (match), `contract` (match returning
  `Some(&CONTRACT_X)` for the 16 core ops, `None` for the 13 reserved), `static`
  contract instances built from `include_str!("../schemas/messaging/…")` and
  `include_str!("../prompts/messaging/…")`, `resolve_standard_schema_ref` (strip
  `STANDARD_SCHEMA_REF_PREFIX`, match `<op>.input.v1`/`<op>.output.v1` against core
  contracts), `StandardMessagingErrorCode` with `as_str`/`ALL`. Wire the module in
  `lib.rs` (`pub mod messaging;` + re-export `StandardMessagingOp` at crate root
  beside `CapabilityDescriptor`'s imports so downstream crates use
  `ironclaw_host_api::StandardMessagingOp`).

- [ ] **Step 6: Run** — `cargo test -p ironclaw_host_api messaging` → PASS; then
  `cargo clippy -p ironclaw_host_api --all-targets --all-features -- -D warnings`.

- [ ] **Step 7: Commit** — `feat(host-api): standard messaging op vocabulary and
  canonical contracts`.

---

### Task 2: Manifest v3 `standard_op` binding + parse-time validation

**Files:**
- Modify: `crates/extensions/ironclaw_extension_registry/src/v3.rs` (`RawToolV3` ~:126-158; per-tool
  loop ~:440-523 where `RawCapabilityV2` is built ~:484-514)
- Modify: `crates/extensions/ironclaw_extension_registry/src/v2.rs` (`RawCapabilityV2` ~:1757 area;
  `CapabilityDeclV2` + `from_raw` ~:1052-1090; empty-description check ~:1073-1076)
- Test: `crates/extensions/ironclaw_extension_registry/tests/manifest_v3_contract.rs` (helpers:
  `ACME_MANIFEST` `:21`, `parse_v3` `:41-43`; mirror neighboring tests)

**Interfaces:**
- Consumes: `ironclaw_host_api::StandardMessagingOp` (Task 1).
- Produces: `CapabilityDeclV2.standard_op: Option<StandardMessagingOp>`
  (`#[serde(default)]`, persisted in the resolved record) with `input_schema_ref` =
  `standard:messaging/<op>.input.v1` and `output_schema_ref` =
  `Some("standard:messaging/<op>.output.v1")` synthesized for bound entries —
  consumed by Tasks 3, 4, 7, 9.

- [ ] **Step 1: Write the failing contract tests** (in `manifest_v3_contract.rs`,
  composing with `ACME_MANIFEST.replace(...)` / inline v3 manifests exactly like
  the neighbors). One test per rule:

```rust
#[test]
fn standard_op_binding_threads_and_synthesizes_canonical_refs() {
    // Inline v3 manifest with one [[tools]] entry:
    //   standard_op = "send_message", id = "zeta.send_message",
    //   effects = ["network", "use_secret", "external_write"],
    //   default_permission = "ask", visibility = "model",
    //   description = "Zeta notes.", no *_schema_ref, one [[tools.credentials]].
    let record = parse_v3(&toml).expect("standard op binding parses");
    let cap = &record.manifest().capabilities[0];
    assert_eq!(cap.standard_op, Some(StandardMessagingOp::SendMessage));
    assert_eq!(cap.input_schema_ref, "standard:messaging/send_message.input.v1");
    assert_eq!(
        cap.output_schema_ref.as_deref(),
        Some("standard:messaging/send_message.output.v1")
    );
}

#[test]
fn standard_op_reserved_name_is_rejected() { /* standard_op = "forward_message" → Err containing "reserved" */ }

#[test]
fn standard_op_unknown_name_fails_serde() { /* standard_op = "send_msg" → Err containing "unknown variant" */ }

#[test]
fn standard_op_id_must_match_extension_and_op_name() { /* id = "zeta.send" + standard_op = "send_message" → Err containing "zeta.send_message" */ }

#[test]
fn standard_op_rejects_declared_schema_refs() { /* entry with input_schema_ref → Err containing "canonical" */ }

#[test]
fn standard_op_write_requires_external_write_effect() { /* send_message without external_write → Err containing "external_write" */ }

#[test]
fn standard_op_duplicate_binding_rejected() { /* two entries binding send_message → Err containing "once" */ }

#[test]
fn standard_op_allows_empty_description_addendum() { /* description = "" + standard_op → parses; cap.description is empty (composition is Task 3) */ }

#[test]
fn v2_manifest_declaring_standard_op_is_rejected() { /* v2 manifest + standard_op = "send_message" → Err containing "v3" */ }
```

- [ ] **Step 2: Run** — `cargo test -p ironclaw_extensions --test
  manifest_v3_contract standard_op` → FAIL (unknown field).

- [ ] **Step 3: Implement.**
  - `RawToolV3` gains `#[serde(default)] pub standard_op: Option<StandardMessagingOp>`;
    thread onto `RawCapabilityV2` (same `#[serde(default)]`) and
    `CapabilityDeclV2` in `from_raw`.
  - In the v3 per-tool loop, **before** building `RawCapabilityV2`:

```rust
let mut seen_standard_ops: std::collections::HashSet<StandardMessagingOp> = Default::default();
// per tool:
if let Some(op) = tool.standard_op {
    if op.contract().is_none() {
        return Err(ManifestV3Error::Invalid { reason: format!(
            "standard_op `{}` is reserved and not yet bindable", op.op_name()) });
    }
    let expected_id = format!("{}.{}", id.as_str(), op.op_name());
    if tool.id != expected_id {
        return Err(ManifestV3Error::Invalid { reason: format!(
            "standard op tool id must be `{expected_id}`, got `{}`", tool.id) });
    }
    if tool.input_schema_ref.is_some() || tool.output_schema_ref.is_some() {
        return Err(ManifestV3Error::Invalid { reason: format!(
            "standard op `{}` uses host-canonical schemas; remove input_schema_ref/output_schema_ref",
            tool.id) });
    }
    if op.is_write() && !tool.effects.contains(&EffectKind::ExternalWrite) {
        return Err(ManifestV3Error::Invalid { reason: format!(
            "standard op `{}` is a write operation and must declare the external_write effect",
            tool.id) });
    }
    if !seen_standard_ops.insert(op) {
        return Err(ManifestV3Error::Invalid { reason: format!(
            "standard op `{}` may be bound at most once per extension", op.op_name()) });
    }
}
```

    (`input_schema_ref` on `RawToolV3` is a required `String` on main
    (`v3.rs:126-158`) — make it `#[serde(default)] Option<String>` and add the
    inverse validation: a **non**-standard tool without `input_schema_ref` fails
    with `ManifestV3Error::Invalid { reason: "tool <id> requires input_schema_ref" }`
    so bespoke behavior is unchanged; add the matching rejection test.)
  - After validation, synthesize the refs for bound entries when building
    `RawCapabilityV2`: `input_schema_ref = format!("standard:messaging/{}.input.v1", op.op_name())`,
    `output_schema_ref = Some(format!("standard:messaging/{}.output.v1", op.op_name()))`.
  - In `CapabilityDeclV2::from_raw` (v2.rs ~:1073): skip the
    empty-description rejection when `raw.standard_op.is_some()` (the composed
    description is guaranteed non-empty by the core — Task 3). In
    `ExtensionManifestV2::from_raw`'s capability path, reject
    `standard_op.is_some()` with
    `ManifestV2Error::Invalid { reason: "standard_op requires manifest schema v3" }`
    (match the file's existing error-variant style).

- [ ] **Step 4: Run** — the new tests, then whole crates:
  `cargo test -p ironclaw_extensions` and `cargo test -p ironclaw_product` (the
  adapter-registry ingestion suite lives there). Fix exhaustive-struct-literal
  fallout: `rg -n "RawCapabilityV2 \{|CapabilityDeclV2 \{" crates/ tests/`.

- [ ] **Step 5: Commit** — `feat(extensions): standard_op binding on tool surfaces`.

---

### Task 3: Descriptor threading + description composition

**Files:**
- Modify: `crates/contracts/ironclaw_host_api/src/capability.rs` (`CapabilityDescriptor`
  ~:168-200 — add the field beside `origin_gate_matrix`)
- Modify: the decl→descriptor projections — find every site with
  `rg -n "origin_gate_matrix" crates/extensions/ironclaw_extension_registry/src/lib.rs crates/extensions/ironclaw_extension_host/src/active.rs crates/extensions/ironclaw_extension_registry/src/registry.rs crates/extensions/ironclaw_extension_registry/src/installations.rs`
  and mirror how `origin_gate_matrix` flows onto `CapabilityDescriptor`
  (known sites: `capability_descriptors_from_manifest`,
  `crates/extensions/ironclaw_extension_registry/src/lib.rs:703-743`;
  `crates/extensions/ironclaw_extension_host/src/active.rs:64-80`)
- Test: extend `crates/extensions/ironclaw_extension_registry/tests/manifest_v3_contract.rs` + a
  descriptor-level assertion wherever the existing suite pins descriptor fields

**Interfaces:**
- Consumes: `CapabilityDeclV2.standard_op` (Task 2); `StandardMessagingOp::contract`
  (Task 1).
- Produces: `CapabilityDescriptor.standard_op: Option<StandardMessagingOp>`
  (`#[serde(default)]`) and the composed model-visible description
  `"{core}\n{addendum}"` (bare `{core}` when the addendum is empty) — consumed by
  Tasks 4, 5, 9 and the visible surface.

- [ ] **Step 1: Write the failing tests** —

```rust
#[test]
fn standard_op_descriptor_carries_binding_and_composed_description() {
    // same inline manifest as Task 2 Step 1 with description = "Zeta notes."
    let record = parse_v3(&toml).expect("parses");
    let descriptors = /* the suite's existing descriptor projection helper —
        mirror how neighboring tests obtain CapabilityDescriptor values
        (capability_descriptors_from_manifest or the package validate path) */;
    let d = descriptors.iter().find(|d| d.id.as_str() == "zeta.send_message").unwrap();
    assert_eq!(d.standard_op, Some(StandardMessagingOp::SendMessage));
    let core = StandardMessagingOp::SendMessage.contract().unwrap().description_core;
    assert!(d.description.starts_with(core.trim()));
    assert!(d.description.ends_with("Zeta notes."));
}

#[test]
fn standard_op_descriptor_with_empty_addendum_is_core_only() { /* description = "" → d.description == core.trim() */ }

#[test]
fn bespoke_descriptor_description_is_untouched() { /* acme send_note description unchanged */ }
```

- [ ] **Step 2: Run** — FAIL (no field).

- [ ] **Step 3: Implement** — add
  `#[serde(default)] pub standard_op: Option<StandardMessagingOp>` to
  `CapabilityDescriptor`; at every decl→descriptor site, thread the field and
  compose the description:

```rust
let description = match decl.standard_op.and_then(|op| op.contract()) {
    Some(contract) => {
        let addendum = decl.description.trim();
        if addendum.is_empty() {
            contract.description_core.trim().to_string()
        } else {
            format!("{}\n{}", contract.description_core.trim(), addendum)
        }
    }
    None => decl.description.clone(),
};
```

  Extract this as a helper `composed_capability_description(&CapabilityDeclV2) ->
  String` in the extensions crate and call it from every site — do not duplicate
  the match. Fix exhaustive-literal fallout
  (`rg -n "CapabilityDescriptor \{" crates/ tests/`).

- [ ] **Step 4: Run** — `cargo test -p ironclaw_extensions -p ironclaw_host_api
  --no-fail-fast`, then `cargo test -p ironclaw_extension_host --no-fail-fast`
  (active-snapshot pins). Update pinned descriptor snapshots to include the new
  field where suites assert exhaustively.

- [ ] **Step 5: Commit** — `feat(extensions): standard_op descriptor attach point +
  composed descriptions`.

---

### Task 4: `standard:` schema-ref resolution in host_runtime

**Files:**
- Modify: `crates/kernel/ironclaw_host_runtime/src/surface.rs` (`surface_descriptor`
  ~:289-350 — branch beside `resolve_builtin_input_schema_ref` ~:300-315)
- Modify: `crates/kernel/ironclaw_host_runtime/src/capability_catalog.rs`
  (`publish_hot_capability_catalog` schema reads ~:95-113 — same branch)
- Test: the existing test homes for those files (find with
  `rg -ln "resolve_builtin_input_schema_ref|publish_hot_capability_catalog" crates/kernel/ironclaw_host_runtime/`)

**Interfaces:**
- Consumes: `resolve_standard_schema_ref` + `STANDARD_SCHEMA_REF_PREFIX` (Task 1);
  descriptors whose refs are `standard:messaging/…` (Tasks 2-3).
- Produces: visible-surface descriptors whose `parameters_schema` is the resolved
  canonical JSON — consumed by the loop host's existing input validation, Task 7's
  fixture, Task 9's Slack surface.

- [ ] **Step 1: Write the failing tests** in the surface test home, mirroring the
  builtin-resolution tests:

```rust
#[test]
fn standard_messaging_schema_ref_resolves_from_registry() {
    // a descriptor with input_schema_ref "standard:messaging/send_message.input.v1"
    // resolves to a schema whose properties include "conversation" and "text",
    // without touching the filesystem/package root.
}

#[test]
fn unknown_standard_ref_fails_closed() {
    // "standard:messaging/bogus.input.v1" → the same error class a missing
    // package-asset schema produces (surface build failure), not a silent skip.
}
```

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement** — in both files, before the package-asset read:

```rust
if schema_ref.starts_with(ironclaw_host_api::messaging::STANDARD_SCHEMA_REF_PREFIX) {
    return match ironclaw_host_api::messaging::resolve_standard_schema_ref(schema_ref) {
        Some(raw) => serde_json::from_str(raw).map_err(/* existing schema-parse error path */),
        None => Err(/* existing unresolved-schema error path with the ref named */),
    };
}
```

  (Match each site's real signature/error type; the builtin branch immediately
  above is the template.)

- [ ] **Step 4: Run** — `cargo test -p ironclaw_host_runtime --no-fail-fast` →
  PASS; clippy the crate both lanes.

- [ ] **Step 5: Commit** — `feat(host-runtime): resolve standard: messaging schema
  refs from the canonical registry`.

---

### Task 5: Post-dispatch output validation for standard ops

**Files:**
- Create: `crates/kernel/ironclaw_host_runtime/src/standard_op_output.rs`
- Modify: `crates/kernel/ironclaw_host_runtime/src/lib.rs` (module decl)
- Modify: `crates/kernel/ironclaw_host_runtime/src/production.rs` — every path in
  `DefaultHostRuntime` that turns a successful `CapabilityDispatchResult` into
  `RuntimeCapabilityOutcome::Completed` (`invoke_capability` ~:480-518 **and** any
  resume path — enumerate with `rg -n "RuntimeCapabilityCompleted|CapabilityDispatchResult" crates/kernel/ironclaw_host_runtime/src/production.rs`;
  apply the check at every site, per the resume-path lesson in
  `.claude/rules/review-discipline.md`)
- Test: `standard_op_output.rs` `#[cfg(test)]` + the production.rs test home

**Interfaces:**
- Consumes: `StandardMessagingOp::contract().output_schema` (Task 1);
  `CapabilityDescriptor.standard_op` (Task 3).
- Produces:

```rust
/// None = valid or not a standard op. Some(issues) = canonical-output violation;
/// issues are schema-path summaries, bounded to 3, safe for model visibility.
pub fn standard_op_output_violations(
    standard_op: StandardMessagingOp,
    output: &serde_json::Value,
) -> Option<Vec<String>>;
```

  and the runtime behavior: a violating standard-op output becomes a
  **model-visible Failed outcome** with safe summary
  `"standard op output failed validation: {issues}"` — consumed by Task 7/8/9
  tests.

- [ ] **Step 1: Write the failing unit tests** in `standard_op_output.rs`:

```rust
#[test]
fn valid_send_output_passes() {
    let out = serde_json::json!({ "message_ref": { "conversation": "C1", "message_id": "168.1" } });
    assert!(standard_op_output_violations(StandardMessagingOp::SendMessage, &out).is_none());
}

#[test]
fn vendor_key_is_admitted() {
    let out = serde_json::json!({ "message_ref": { "conversation": "C1", "message_id": "1" },
                                  "vendor": { "channel_name": "eng" } });
    assert!(standard_op_output_violations(StandardMessagingOp::SendMessage, &out).is_none());
}

#[test]
fn missing_message_ref_is_a_violation() {
    let out = serde_json::json!({ "ok": true, "ts": "168.1" });
    let issues = standard_op_output_violations(StandardMessagingOp::SendMessage, &out)
        .expect("violation");
    assert!(issues.iter().any(|i| i.contains("message_ref")));
    assert!(issues.len() <= 3);
}
```

  And in the production.rs test home (mirror the existing invoke-path tests, using
  their fake registry/dispatcher): a dispatch returning `{"ok": true}` for a
  descriptor with `standard_op: Some(SendMessage)` yields
  `RuntimeCapabilityOutcome::Failed` whose sanitized message contains
  `"standard op output failed validation"` and whose failure **kind matches the
  kind wasm `InvalidResult` dispatch errors produce** (locate with
  `rg -n "InvalidResult" crates/kernel/ironclaw_host_runtime/src/production.rs` — this
  kind is proven model-visible by the existing loop mapping); a bespoke descriptor
  (`standard_op: None`) with the same output completes untouched.

- [ ] **Step 2: Run** — `cargo test -p ironclaw_host_runtime standard_op_output`
  → FAIL.

- [ ] **Step 3: Implement** — in `standard_op_output.rs`: a
  `static VALIDATORS: LazyLock<HashMap<StandardMessagingOp, jsonschema::Validator>>`
  built from every core contract's `output_schema` (compile errors here are
  impossible-by-construction — Task 1's completeness test compiles them all; still
  handle with an `expect`-free fallback that reports the op as violating with a
  "canonical schema failed to compile" issue). `standard_op_output_violations`
  runs the validator and maps the first 3 errors to
  `format!("{} at {}", error, error.instance_path)` strings passed through the same
  schema-path sanitizer the input path uses
  (`rg -n "safe_schema_path_summary" crates/loop/ironclaw_loop_host/` — if it is not
  reachable from host_runtime, replicate its bounded formatting locally: truncate
  each issue to 200 chars, strip values, keep paths).
  In `production.rs`, at each Completed-construction site:

```rust
if let Some(op) = descriptor_standard_op {   // looked up from the registry by capability id
    if let Some(issues) = crate::standard_op_output::standard_op_output_violations(op, &result.output) {
        return /* the Failed outcome construction used by dispatch InvalidResult
                  errors, with safe summary:
                  format!("standard op output failed validation: {}", issues.join("; ")) */;
    }
}
```

- [ ] **Step 4: Run** — `cargo test -p ironclaw_host_runtime --no-fail-fast` →
  PASS; clippy both lanes on the crate.

- [ ] **Step 5: Commit** — `feat(host-runtime): enforce canonical outputs for
  standard messaging ops`.

---

### Task 6: Conformance test-support in `ironclaw_host_api`

**Files:**
- Create: `crates/contracts/ironclaw_host_api/src/test_support/messaging_conformance.rs`
  (find the existing test-support home first:
  `rg -n "test_support|test-support" crates/contracts/ironclaw_host_api/Cargo.toml crates/contracts/ironclaw_host_api/src/lib.rs`
  — put the module beside the existing tool-adapter conformance helpers and gate it
  exactly the way they are gated; if that gate is a `test-support` feature, add
  `jsonschema` as an optional dependency under it, mirroring how the existing
  test-support dependencies are declared)
- Test: self-tests in the same file

**Interfaces:**
- Consumes: Task 1's registry.
- Produces (consumed by Tasks 7, 8, 9 tests):

```rust
pub fn canonical_input_schema(op: StandardMessagingOp) -> serde_json::Value;
pub fn canonical_output_schema(op: StandardMessagingOp) -> serde_json::Value;
/// Panics with the issue list when `output` violates the op's canonical output schema.
pub fn assert_canonical_output(op: StandardMessagingOp, output: &serde_json::Value);
/// Panics when `input` is NOT accepted by the op's canonical input schema.
pub fn assert_canonical_input_accepted(op: StandardMessagingOp, input: &serde_json::Value);
/// Panics when `input` IS accepted (for additionalProperties/closed-input checks).
pub fn assert_canonical_input_rejected(op: StandardMessagingOp, input: &serde_json::Value);
/// Extracts the message_ref object from a write output (send/edit) for the
/// evidence loop: send's ref feeds edit/delete/react inputs.
pub fn message_ref_from_output(output: &serde_json::Value) -> serde_json::Value;
```

- [ ] **Step 1: Write failing self-tests** — `assert_canonical_output(SendMessage,
  valid)` passes; panics (use `std::panic::catch_unwind`) on `{"ok": true}`;
  `assert_canonical_input_rejected(SendMessage, json!({"conversation":"C","text":"t","extra":1}))`
  passes (closed input); `message_ref_from_output` round-trips into
  `assert_canonical_input_accepted(EditMessage, json!({"message_ref": ref, "text": "x"}))`.

- [ ] **Step 2: Run** — `cargo test -p ironclaw_host_api messaging_conformance`
  (add the feature flag to the command if the test-support gate requires it —
  mirror how existing test-support tests are invoked) → FAIL.

- [ ] **Step 3: Implement** — thin wrappers over `jsonschema::validator_for` on the
  Task 1 contract data; panic messages list every issue with instance paths.

- [ ] **Step 4: Run** — PASS; clippy both lanes.

- [ ] **Step 5: Commit** — `feat(host-api): messaging standard conformance
  test-support`.

---

### Task 7: acme-messenger implements all 16 core ops

**Files:**
- Modify: `tests/fixtures/extensions/acme-messenger/manifest.toml`
- Modify: `tests/integration/support/harness/profiles/extension.rs` (the acme
  first-party adapter — `extension_runtime_acme_tools_profile()` ~:607 and the
  `ToolAdapter` impl backing service `acme-messenger.extension/v1`; read the
  existing `send_note` arm first and mirror its egress/scripted-response pattern
  exactly)
- Test: conformance-driven tests beside the adapter (same file's test module or the
  harness support test home)

**Interfaces:**
- Consumes: Tasks 1-6 (binding parses; refs resolve; outputs enforced; conformance
  helpers).
- Produces: an acme extension whose active surface exposes
  `acme-messenger.<op>` for all 16 core ops **plus** bespoke
  `acme-messenger.send_note` — consumed by Task 8's integration scenarios.

- [ ] **Step 1: Extend the manifest.** Keep `[[tools]]` `acme-messenger.send_note`
  exactly as-is (bespoke coexistence proof — its input schema keeps
  `conversation_id`/`text`). Add 16 entries; the two exemplars (repeat the same
  shape for the rest — writes carry `external_write`, reads/people do not):

```toml
[[tools]]
standard_op = "send_message"
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
id = "acme-messenger.send_message"
description = "Acme notes: conversation refs look like ACME-C-…."
effects = ["network", "use_secret", "external_write"]
default_permission = "ask"
visibility = "model"

[[tools.credentials]]
handle = "acme_user_token"
vendor = "acme"
scopes = ["notes:write"]
audience = { scheme = "https", host = "api.acme.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[[tools]]
standard_op = "get_conversation_history"
origin_gate_matrix = { loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }
id = "acme-messenger.get_conversation_history"
description = ""
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"

[[tools.credentials]]
handle = "acme_user_token"
vendor = "acme"
scopes = ["notes:read"]
audience = { scheme = "https", host = "api.acme.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }
```

  Ops with `external_write`: `send_message`, `edit_message`, `delete_message`,
  `add_reaction`, `remove_reaction`, `open_dm`. Reads/people
  (`list_conversations`, `get_conversation_info`, `get_conversation_history`,
  `get_thread_replies`, `get_message`, `search_messages`, `get_user_info`,
  `resolve_user`, `list_members`, `whoami`): `effects = ["network", "use_secret"]`,
  scope `notes:read`.
  Run `cargo test -p ironclaw_extensions --test manifest_v3_contract` — the acme
  fixture parse test must stay green (extend its assertions with one standard-op
  spot check: `capabilities` now include `acme-messenger.send_message` with
  `standard_op == Some(SendMessage)`).

- [ ] **Step 2: Write the failing conformance test** for the adapter (in the
  adapter's test home), driving `ToolAdapter::invoke` directly with the harness's
  scripted egress the way existing acme adapter tests do:

```rust
#[tokio::test]
async fn acme_standard_ops_satisfy_canonical_contracts() {
    // for each of the 16 ops: build the canonical happy-path input
    // (assert_canonical_input_accepted first), invoke the adapter against the
    // scripted acme vendor responses, then assert_canonical_output(op, &output).
    // Evidence loop: send → message_ref_from_output → edit → delete.
}

#[tokio::test]
async fn acme_standard_ops_emit_canonical_error_codes() {
    // scripted vendor failure per code: e.g. vendor "conversation_missing" →
    // adapter error whose model-visible text contains
    // StandardMessagingErrorCode::UnknownConversation.as_str(). Cover all 12 codes
    // via a table of (scripted response → expected code).
}
```

- [ ] **Step 3: Run** — FAIL (ops unimplemented).

- [ ] **Step 4: Implement the adapter arms.** Extend the acme `ToolAdapter` match
  with the 16 capability ids. Invent the acme vendor API as one POST per op at
  `https://api.acme.example/<op_name>` (scripted in tests); each arm parses the
  canonical input, makes the egress call via the same pattern `send_note` uses,
  and builds the canonical output. Exemplar arm:

```rust
"acme-messenger.send_message" => {
    let conversation = input_str(&call.input, "conversation")?;
    let text = input_str(&call.input, "text")?;
    let response = self.post_acme("send_message", json!({
        "conversation": conversation, "text": text,
        "thread": call.input.get("thread"),
    }), ports).await?;
    // scripted vendor returns {"id": "AMSG-1"}; map vendor errors via
    // acme_error_to_standard_code() before this point.
    Ok(tool_result(json!({
        "message_ref": { "conversation": conversation, "message_id": response["id"] }
    })))
}
```

  Add `fn acme_error_to_standard_code(vendor_code: &str) -> StandardMessagingErrorCode`
  covering all 12 codes (`"conversation_missing"→UnknownConversation`,
  `"message_missing"→UnknownMessage`, `"user_missing"→UnknownUser`,
  `"not_member"→NotAMember`, `"forbidden"→PermissionDenied`,
  `"dm_closed"→CannotMessageUser`, `"window_closed"→OutsideMessagingWindow`,
  `"too_long"→MessageTooLong`,
  `"bad_content"→UnsupportedContent`, `"slow_down"→RateLimited`,
  `"edit_locked"→EditNotAllowed`, fallback → `VendorError`), and surface it through
  the adapter's error path so the code string reaches the model-visible summary
  (mirror how `send_note` errors surface today). Reads return canonical `message`
  objects (`is_self` computed against the scripted whoami identity;
  `timestamp` RFC3339).

- [ ] **Step 5: Run** — the two tests → PASS; then
  `cargo test -p ironclaw_extensions --test manifest_v3_contract` → PASS.

- [ ] **Step 6: Commit** — `feat(fixtures): acme-messenger implements the full
  standard messaging core`.

---

### Task 8: Integration proofs (end-to-end, gated resume, taxonomy, coexistence)

**Files:**
- Modify: `tests/integration/extension_runtime.rs` (acme is wired via
  `extension_runtime_acme_tools_profile()` —
  `tests/integration/support/harness/profiles/extension.rs:607`); follow
  `tests/integration/CLAUDE.md` scripting rules (one script entry per model call;
  gated tool-call turn = exactly 2 entries)

**Interfaces:**
- Consumes: Tasks 1-7.
- Produces: the pinned end-to-end behavior later tasks must not break.

- [ ] **Step 1: Write the failing scenarios** (extend the existing acme suite —
  do not stand up a parallel file):

```rust
#[tokio::test]
async fn standard_send_completes_with_canonical_evidence() {
    // Script: tool_call("acme-messenger.send_message",
    //   json!({"conversation": "ACME-C-1", "text": "hello"})) + post-approval model
    //   text turn (default_permission = ask ⇒ gated turn = 2 entries; approve via
    //   the harness's existing approval flow — this also exercises the RESUME path
    //   through output validation).
    // Assert at seams: egress request hit api.acme.example/send_message;
    // the persisted capability result contains message_ref (read the result the
    // way existing acme scenarios read tool outputs);
    // assert_canonical_output(SendMessage, &output) via the Task 6 helper;
    // run completes.
}

#[tokio::test]
async fn standard_op_vendor_failure_surfaces_canonical_code_and_run_continues() {
    // Scripted vendor responds "conversation_missing" → tool error class Denied/
    // Failed (mirror the suite's assert_tool_error pattern) with summary containing
    // "messaging.unknown_conversation"; next scripted model turn recovers with
    // text; run completes (not terminal).
}

// (Host-layer output-validation failure behavior is pinned at Task 5's
// production-tier test — a correct adapter never emits invalid canonical output,
// so no integration scenario fakes one here.)

#[tokio::test]
async fn bespoke_send_note_coexists_with_standard_ops() {
    // tool_call("acme-messenger.send_note", json!({"conversation_id": "ACME-C-1",
    // "text": "note"})) still dispatches and completes exactly as the existing
    // send_note scenario pins — extend that scenario's assertions rather than
    // duplicating it; additionally assert the visible surface contains BOTH
    // acme-messenger.send_note and acme-messenger.send_message.
}
```

- [ ] **Step 2: Run** — `RUST_MIN_STACK=16777216 cargo test --test
  reborn_integration_extension_runtime` (confirm exact target name from the file
  header) → new tests FAIL before wiring gaps are fixed, PASS after.

- [ ] **Step 3: Coverage floor** — follow
  `tests/integration/coverage-floor.toml` same-PR recapture instructions if the
  ratchet moved.

- [ ] **Step 4: Commit** — `test(integration): standard messaging ops end-to-end
  proofs`.

---

### Task 9: Slack re-badge (manifest + assets + WASM, atomic)

**Files:**
- Modify: `crates/extensions/packages/slack/manifest.toml`
  (all 8 `[[tools]]` entries)
- Delete: `crates/extensions/packages/slack/schemas/slack/*.json`
  for the 8 ops' inputs + the output/raw files (keep nothing the manifest no longer
  references — verify each file's references first:
  `rg -n "schemas/slack/" crates/extensions/`)
- Modify: `crates/extensions/ironclaw_extension_support/src/packages/slack.rs`
  (`assets()` embed list ~:49-75 — remove deleted schema embeds; update the
  "one schema + prompt pair per entry" invariant comment: standard-op entries have
  host-canonical schemas and package prompt docs only)
- Modify: `crates/extensions/packages/slack/wasm-src/src/types.rs`
  (canonical field names), `…/src/api.rs` (canonical outputs + error mapping),
  `…/src/lib.rs` (only if field names appear there)
- Modify: `crates/extensions/packages/slack/prompts/slack/*.md`
  (trim content now covered by canonical cores; keep vendor-specific notes)
- Rebuild: `bash scripts/build-wasm-extensions.sh` → commit the updated
  `assets/slack/wasm/slack_user_tool.wasm`
- Test: `cargo test -p ironclaw_first_party_extensions`; lifecycle projection pins
  (`rg -ln "send_message" crates/app/ironclaw_composition/tests/` — known pin in
  `extension_lifecycle.rs`); scripted slack tool inputs across tests
  (`rg -n '"channel"|thread_ts' tests/ crates/ --type rust -l | xargs rg -ln "slack"`)

**Interfaces:**
- Consumes: Tasks 1-5 (binding, resolution, output enforcement all live — this task
  must land after them, atomically, because the manifest switch and the WASM
  field-rename must move together: old WASM + new manifest fails input validation,
  new WASM + old manifest fails the old schemas).
- Produces: Slack's 8 ops on the standard; wire names and capability ids unchanged.

- [ ] **Step 1: Manifest.** Each of the 8 entries gains
  `standard_op = "<op_name>"` (`send_message`, `search_messages`,
  `list_conversations`, `get_conversation_info`, `get_conversation_history`,
  `get_thread_replies`, `get_user_info`, `whoami`), loses `input_schema_ref`, and
  its `description` shrinks to the vendor addendum. Addenda (exact text):
  - `send_message`: `"Slack notes: text is Slack mrkdwn. To notify someone, mention them as <@U…> with their real user id — a plain @name notifies nobody; never guess a user id or derive one from a channel or DM conversation id (for a DM conversation id, get_conversation_info returns the authoritative user). Raw Slack ids (U…/W…/C…/D…) are for tool calls only — never include one in a reply. Requires the chat:write user scope."`
  - `search_messages`: `"Slack notes: indexed search — for the newest message in a known conversation use get_conversation_history instead. Requires the search:read user scope. Raw Slack ids are for tool calls only — never include one in a reply."`
  - `list_conversations`: `"Slack notes: lists channels, private channels, DMs, and group DMs visible to you; is_member is the authoritative membership signal. DM entries carry the counterpart user."`
  - `get_conversation_info`: `"Slack notes: for a DM, counterpart.user_ref is the authoritative mention target."`
  - `get_conversation_history`: `"Slack notes: newest-first; thread replies are NOT included — a reply_count > 0 means fetch get_thread_replies. Limit above 999 is clamped."`
  - `get_thread_replies`: `"Slack notes: thread is the parent message's ts."`
  - `get_user_info`: `"Slack notes: includes status text/emoji, timezone, and title for presence-style questions."`
  - `whoami`: `""` (core suffices).
  Keep every entry's `origin_gate_matrix`, `effects`, `default_permission`,
  `visibility`, `prompt_doc_ref`, and `[[tools.credentials]]` unchanged.
- [ ] **Step 2: WASM canonicalization.** In `types.rs`, rename serde fields:
  `SendMessage { conversation, text, thread }`,
  `GetConversationHistory { conversation, limit, cursor }`,
  `GetThreadReplies { conversation, thread, limit, cursor }`,
  `GetConversationInfo { conversation }`, `GetUserInfo { user_ref }`,
  `SearchMessages { query, limit, cursor }`,
  `ListConversations { kinds, limit, cursor }`, `Whoami`. In `api.rs`: map
  canonical→Slack params (`conversation`→`channel`, `thread`→`thread_ts`,
  `cursor` decodes the previous `next_cursor`; search encodes page numbers into
  `next_cursor`), shape outputs to the canonical envelopes
  (`message_ref {conversation, message_id: ts}`,
  `author {user_ref, display_name}`, `is_self` from the connected user id,
  `timestamp` = RFC3339 derived from `ts` seconds, `conversation_info.kind` from
  Slack's `is_im/is_mpim/is_channel` flags → `dm/group_dm/channel/other`), and add
  `fn slack_error_to_standard_code(code: &str) -> &'static str` returning
  `StandardMessagingErrorCode`-style strings
  (`"channel_not_found"|"user_not_found"→"messaging.unknown_conversation"/"messaging.unknown_user"`,
  `"not_in_channel"→"messaging.not_a_member"`,
  `"missing_scope"|"not_allowed_token_type"→"messaging.permission_denied"`,
  `"msg_too_long"→"messaging.message_too_long"`,
  `"no_text"→"messaging.unsupported_content"`,
  `"ratelimited"→"messaging.rate_limited"`,
  `"cant_update_message"|"edit_window_closed"→"messaging.edit_not_allowed"`,
  else `"messaging.vendor_error"`), returned as the structured guest error:
  `Err(json!({"code": code, "kind": kind}).to_string())` (the WASM string-error
  channel the host already sanitizes; `kind` is one of the landed
  `StructuredWasmGuestErrorKind` wire variants (`input`, `client`,
  `operation_failed`, `network_denied`, etc.), not the prose class labels in
  the table above). A send whose Slack response
  lacks `ts` returns the error code `"messaging.vendor_error"` — never
  `message_id: ""`.
- [ ] **Step 3: Rebuild + embeds.** `bash scripts/build-wasm-extensions.sh`;
  delete the 8 input schema files + the unreferenced output/raw schema files;
  update `assets()` in `packages/slack.rs` accordingly; commit the rebuilt
  `slack_user_tool.wasm` alongside.
- [ ] **Step 4: Prompt docs.** Trim each of the 8 `prompts/slack/*.md` to
  vendor-specific content only (mention mechanics, id hygiene, scope notes);
  delete sentences now stated by the canonical cores. Keep the files (still
  referenced by `prompt_doc_ref` and embedded).
- [ ] **Step 5: Run + fix pins.**
  `cargo test -p ironclaw_first_party_extensions --no-fail-fast`;
  the lifecycle/projection suites found via
  `rg -ln "send_message" crates/app/ironclaw_composition/tests/`;
  `RUST_MIN_STACK=16777216 cargo test --test reborn_integration_extension_delivery`
  and any slack-driving integration targets
  (`rg -ln "slack.send_message\|slack__send_message" tests/integration/`); update
  every scripted slack tool-call input to canonical field names and every pinned
  description/schema assertion to the new composed values. Never weaken an
  assertion — update it to pin the new exact value.
- [ ] **Step 6: Slack conformance.** Add the conformance pass for Slack's 8 ops at
  the seam that executes real WASM (locate the existing wasm-executing test home
  with `rg -ln "slack_user_tool.wasm" crates/ tests/`); drive each op with a
  canonical input against scripted Slack responses and
  `assert_canonical_output(op, &output)`; include one vendor-error case asserting
  `messaging.not_a_member` surfaces.
- [ ] **Step 7: Commit** — `feat(slack): re-badge the 8 tools onto standard
  messaging ops`.

---

### Task 10: Golden payloads + recorded fixture fallout

**Files:**
- Locate the golden payload source: `grep -rln "golden_payload" tests/` (the
  `.snap` files under `tests/snapshots/` name their source test file in the
  `source:` header line)
- Modify: `tests/snapshots/golden_payload__*.snap` via the insta refresh flow
- Audit: `tests/fixtures/llm_traces/` for slack tool calls with old field names
  (`grep -rln '"channel"' tests/fixtures/llm_traces/`)

- [ ] **Step 1: Regenerate goldens.** Run the located golden test target; where
  snapshots differ, verify the diff is exactly the tool-definition change
  (canonical schemas + composed descriptions), then accept via the repo's
  hash-only refresh convention (see the snapshot headers/openwiki notes; never
  hand-edit `.snap` files).
- [ ] **Step 2: Recorded traces.** For each fixture invoking a slack tool with old
  field names: if the fixture merely *lists* tool definitions, re-record is not
  needed (definitions come from the live surface at replay); if it *calls* a
  renamed field, re-record with credentials if available — otherwise mark the
  recorder case ignored with a PR note per the recorded-QA convention
  (`.claude/skills/ironclaw-reborn-testing`), never hand-edit a recorded trace.
- [ ] **Step 3: Run** — the golden target + `scripts/ci/check-reborn-qa-fixtures.sh`
  → PASS.
- [ ] **Step 4: Commit** — `test(golden): refresh payload snapshots for canonical
  messaging schemas`.

---

### Task 11: Documentation

**Files:**
- Create: `docs/reborn/extension-runtime/standard-operations.md`
- Modify: `docs/reborn/extension-runtime/overview.md` (add §3.4 pointer)
- Modify: `.claude/skills/reborn-extension-surfaces/SKILL.md`
- Do NOT touch `openwiki/` (auto-generated)

- [ ] **Step 1: Write `standard-operations.md`** — the durable normative copy:
  vocabulary table (core/reserved/excluded with reasons), binding rules (the six
  §6 validations), contract principles (nouns, closed inputs, enforced outputs,
  `vendor` key, dialect posture), error-code table, conformance expectations, the
  reserved-op graduation rule, and the coexistence review guidance (an extension
  should not declare a bespoke tool semantically duplicating a standard op it
  binds — review guidance, not a machine gate). Source: spec §§4-8 + Appendix A/B — condense,
  do not fork semantics; where the spec and this doc would disagree, the spec
  loses only via a new approved revision.
- [ ] **Step 2: overview.md §3.4** — ~10 lines: standard operation families exist;
  `standard_op` on `[[tools]]`; contracts host-owned in
  `ironclaw_host_api::messaging`; pointer to `standard-operations.md`.
- [ ] **Step 3: Skill update** — in `.claude/skills/reborn-extension-surfaces/SKILL.md`
  "Adding a tool surface": add one bullet — messaging-shaped tools bind
  `standard_op = "<op>"` (closed vocabulary, host-canonical schemas; see
  `docs/reborn/extension-runtime/standard-operations.md`). Fix the three stale
  lines while present: `[channel.config]` → `[admin_configuration]` (operator
  fields; there is no `[channel.config]`); "5 `[[tools]]` entries" → 8;
  `ChannelAdapter` home → `crates/contracts/ironclaw_host_api/src/product_adapter/channel_adapter.rs`
  (with `crates/ironclaw_product_adapters` removed). Follow
  `ironclaw-reborn-skill-maintainer` discipline; verify each replacement target
  first with `rg -n "channel.config|product_adapters|5 .\[\[tools\]\]" .claude/skills/reborn-extension-surfaces/SKILL.md`.
- [ ] **Step 4: Verify** — `rg -n "channel.config" .claude/skills/reborn-extension-surfaces/` →
  nothing live; no absolute developer paths introduced (`rg -n "/Users/" docs/ .claude/`).
- [ ] **Step 5: Commit** — `docs: standard messaging operations reference + skill
  refresh`.

---

### Task 12: Ship gate

- [ ] **Step 1:** `cargo fmt`
- [ ] **Step 2:** `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  and the default lane `cargo clippy --all --tests --examples -- -D warnings`
- [ ] **Step 3:** `cargo test -p ironclaw_host_api -p ironclaw_extensions -p
  ironclaw_extension_host -p ironclaw_host_runtime -p ironclaw_first_party_extensions
  -p ironclaw_composition --no-fail-fast`
- [ ] **Step 4:** `cargo test -p ironclaw_architecture`
- [ ] **Step 5:** the touched integration targets with `RUST_MIN_STACK=16777216`
  (`reborn_integration_extension_runtime`, `…_extension_delivery`, exact names from
  file headers) and the golden target from Task 10
- [ ] **Step 6:** `bash scripts/reborn-e2e-rust.sh` and `scripts/pre-commit-safety.sh`
- [ ] **Step 7:** Final commit if anything moved; verify every commit message
  carries the co-author trailer.
