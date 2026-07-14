# Messaging Tool Framework — Acceptance Checklist

**Companions:** `messaging-framework.md` (engineering design),
`adr/0002-messaging-tool-framework.md` (decision).
**Status:** Proposed — nothing below is built yet; every box is unchecked.

Rules — kept short on purpose (same discipline as `checklist.md`):

- Check an item only when a named test or command proves it; write that name next
  to the item in the PR that makes it true.
- Persistent behavior counts only when it passes on **libSQL and PostgreSQL**.
- Behavior that gates a side effect needs a caller-level test (dispatcher,
  coordinator, handler), not only a helper unit test.
- `wait_for_status(Completed)` alone is never evidence.
- The **CRITICAL** section (SAFE) is a release gate: no messaging extension ships
  puppeting tools enabled to real users until every SAFE item is checked.
- Phase tags (M0–M4) map to `messaging-framework.md` §14.

## 1. Data model and profiles (TYPE) — M0

- [ ] TYPE-1 `schemas/messaging/types.v1.json` defines `UserRef`,
  `ConversationRef`, `MessageRef`, `Message`, `Reaction`, `AttachmentRef` with
  `additionalProperties:false`; every field matches the design §4.
- [ ] TYPE-2 `UserRef` requires `id` **and** `display_name` — a `UserRef` with a
  raw id and no resolved name fails schema validation (the normalization
  guarantee is schema-enforced, not advisory).
- [ ] TYPE-3 `ConversationRef.kind` is the closed enum `dm|group|channel`; an
  unknown kind fails validation.
- [ ] TYPE-4 `MessageRef` requires both `id` and `conversation` (ids are
  conversation-scoped, not global — prior art §2); a bare id fails validation.
- [ ] TYPE-5 One host-defined `CapabilityProfileContract` exists per standard
  tool (`ironclaw.messaging.<tool>.v1`), each with input **and** output
  `schema_ref`; all nine load and validate.
- [ ] TYPE-6 Each tool's input/output schema (§5.1–5.7) is a shipped asset and
  `$ref`s `types.v1`; the schemas compile (no unresolved `$ref`).

## 2. Manifest and expansion (MAN) — M0

- [ ] MAN-1 A `[messaging]` section parses through the single v3 entry point
  (`ExtensionManifestRecord::from_toml`), not a bespoke reader.
- [ ] MAN-2 `tools = […]` accepts only known standard tool names; an unknown name
  fails closed with a path-qualified error.
- [ ] MAN-3 A declared tool carrying a `thread` param with `supports_threads =
  false` (or absent) is rejected at parse.
- [ ] MAN-4 `[[messaging.credentials]]` is required when any declared tool has the
  `use_secret` effect; absence fails closed.
- [ ] MAN-5 The credential `vendor` must resolve to an identity source (an
  `[auth.<vendor>]` recipe **or** a declared pairing modality); an unresolvable
  vendor fails activation.
- [ ] MAN-6 The expander turns each `tools` entry into a `CapabilityDeclV2` with
  id `<ext>.<tool>`, `implements = [profile]`, framework schema refs, framework
  effects/permission, `visibility = Model`, and the declared credentials
  (design §6.4 table), landing in `ResolvedExtensionManifest.tools`.
- [ ] MAN-7 Expanded tools are indistinguishable downstream from static
  `[[tools]]`: resolved through `ToolResolver::resolve`, indexed in the active
  snapshot, listed to the model — no messaging-specific dispatch branch.
- [ ] MAN-8 An extension may declare `[messaging]` only, `[channel]` only, or
  both; the binding rule (`overview.md` §4.0) still holds for each combination.
- [ ] MAN-9 A recipe cannot **widen** a tool's framework effect ceiling; a
  narrower `default_permission`/effect set is honored, a wider one is rejected.

## 3. Conformance and output validation (VAL) — M0

- [ ] VAL-1 `evaluate_profile_conformance` runs at activation over each expanded
  decl; a tool whose schema refs mismatch its claimed profile **fails
  activation** with a typed error.
- [ ] VAL-2 The host validates `ToolResult.output` against the profile's
  `output_schema_ref` before the model sees it.
- [ ] VAL-3 A non-conforming adapter output (e.g. a raw `U0123` where a `UserRef`
  is required) surfaces as a recoverable `ToolError::Failed`, **not** a
  terminal host error and **not** delivered to the model.
- [ ] VAL-4 A missing/expired credential returns `ToolError::AuthRequired` and
  raises the generic gate (not a silent failure).

## 4. Adapter contract and normalization (ADP) — M0/M2/M3

- [ ] ADP-1 A messaging extension implements only `ToolAdapter::invoke`; no new
  trait is added to the extension ABI.
- [ ] ADP-2 The adapter routes internally by `capability_id` (one adapter per
  extension, not per tool).
- [ ] ADP-3 `read_history`/`search_messages` output has every `Message.author`
  resolved to a `UserRef` with `display_name` — verified against a scripted
  backend that returns raw ids (the adapter performs the resolution).
- [ ] ADP-4 The user-id → display-name resolution is cached in `ScopedToolState`;
  a second reference to the same user within a turn does not re-fetch.
- [ ] ADP-5 The vendor "messaging core" (rendering, splitting, target/DM
  formatting, error mapping) is shared between `invoke` and the channel adapter's
  `deliver` within the extension crate — no duplicated rendering.
- [ ] ADP-6 The adapter holds no delivery store and cannot mark anything
  delivered (reliability stays coordinator-only).

## 5. Relay/act boundary — CRITICAL safety (SAFE) — M1

- [ ] SAFE-1 **Constraint A:** a `send_message` (or edit/delete/react) whose
  resolved recipient is the owner / the owner's own conversation is **blocked**
  before the vendor call, returning a recoverable `ToolError::Failed`. Driven
  through the dispatcher, not a helper unit test.
- [ ] SAFE-2 **Constraint B:** the delivery coordinator rejects any delivery whose
  target is not an owner destination (reply-where-it-came-from or the owner's
  saved target); a third-party target fails closed.
- [ ] SAFE-3 **No duplicate:** "send me a DM with XYZ" produces exactly **one**
  delivery (the channel relay) and **zero** `send_message` calls — the canonical
  regression for the duplicate-send bug.
- [ ] SAFE-4 **No leak:** "summarize `<source channel>` and show me the bugs"
  delivers the summary to the owner and never posts to the source channel (neither
  via a tool nor via the bot).
- [ ] SAFE-5 **Labeled approval:** a legitimate outward `send_message` (to a
  non-owner) is `ask`, and the approval names the target and its visibility
  ("#eng — public") and "as you".
- [ ] SAFE-6 **Automation denial:** a proactive (routine/heartbeat) run cannot
  act-as-user to a non-owner unless the routine pre-authorized that target;
  without a live approver, `ask` does not resolve to "yes". Driven through a
  proactive run, asserting the denial.
- [ ] SAFE-7 **Connect gate, not relay:** an outward send with no connected user
  identity raises the connect/pairing gate and never falls back to a
  bot-attributed send.
- [ ] SAFE-8 **Cross-channel identical:** SAFE-1..7 pass with the same assertions
  for Slack, Telegram, WebUI, and the `acme-messenger` fixture — proof the
  guarantee is generic, not per-vendor.

## 6. Identity, credentials and connect (ID) — M1/M3

- [ ] ID-1 The messaging credential is the user-acquired identity, stored and
  injected host-side; credential bytes never reach the adapter.
- [ ] ID-2 It is distinct from the channel's bot credential; the two are never
  conflated (a Slack extension resolves `slack_user_token` for tools and
  `slack_bot_token` for delivery).
- [ ] ID-3 OAuth acquisition (Slack) rides the existing `[auth.<vendor>]` engine
  and yields the user token with the declared scopes.
- [ ] ID-4 Pairing acquisition (Telegram) rides the connect surface and yields a
  stored session; the gate/resume path resumes the blocked turn on connect.
- [ ] ID-5 Connect is modeled as a step-based flow covering OAuth and pairing
  uniformly (design §9); a new vendor's connect is data/steps, not bespoke code.
- [ ] ID-6 Multi-account resolution (`adr/0001`) applies unchanged: a messaging
  tool executes under exactly one resolved account.

## 7. Transport (XP) — M0/M3

- [ ] XP-1 HTTP vendors (Slack) drive `invoke` through the existing
  `RestrictedEgress` port with host-side credential injection and host allowlist —
  no new transport mechanism.
- [ ] XP-2 A host-side Telegram-user (MTProto/TDLib) client exists behind a narrow
  adapter-facing port; the adapter never speaks MTProto directly and never holds
  session bytes.
- [ ] XP-3 The Telegram session persists across process restarts and is encrypted
  at rest; revocation/logout is supported and observable.
- [ ] XP-4 `list_conversations`/`read_history`/`search_messages` for Telegram work
  on the first call after pairing (getDialogs bootstrap), with no background
  message-mirror requirement.
- [ ] XP-5 Telegram bounded caveats (cold reference resolve, `FLOOD_WAIT`, secret
  chats invisible) surface as recoverable `ToolError::Failed`, never terminal.

## 8. Discovery / anti-bloat (DISC) — M0

- [ ] DISC-1 Messaging tools are `Discoverable`-tier surfaces reachable through the
  existing `tool_search → capability_info → call` flow; the framework adds no
  messaging-specific discovery tool.
- [ ] DISC-2 With disclosure enabled, `<ext>.<tool>` names appear in the
  `tool_search` catalog index grouped by extension prefix; the framework does not
  fork or modify the disclosure layer.

## 9. Slack migration — parity (SLK) — M2

- [ ] SLK-1 Slack's five bespoke tools become a `[messaging]` block; the expander
  emits the **same** capability ids (`slack.send_message`,
  `slack.get_conversation_history` → `slack.read_history`, …) with no
  model-visible regression (parity test over the tool surface).
- [ ] SLK-2 `slack.read_history` returns normalized output with authors resolved
  to `UserRef` — the separate `get_user_info` round-trip is folded away (the tool
  still exists as `get_user` for explicit lookups).
- [ ] SLK-3 Slack output validates against the messaging profiles (the old
  `raw_output.v1.json` unvalidated passthrough is gone).
- [ ] SLK-4 The Slack `send_message` tool acts on the **user** token and is
  subject to SAFE-1; delivery still uses the bot token.
- [ ] SLK-5 Extends the existing Slack tool integration test; no parallel suite.

## 10. Telegram — new user-acting tools (TG) — M3

- [ ] TG-1 Telegram declares a `[messaging]` block acting as the paired user
  (not the Bot API); the channel surface is unchanged.
- [ ] TG-2 Each declared Telegram tool's `invoke` returns schema-valid normalized
  output through the host MTProto client.
- [ ] TG-3 Telegram reuses the extension crate's rendering core (shared with
  `deliver`); no duplicated formatting.
- [ ] TG-4 One end-to-end integration proof: pair → `list_conversations` →
  `read_history` → `send_message` (to a non-owner) through the production
  dispatcher.

## 11. Testing and conformance suite (TEST) — all phases

- [ ] TEST-1 A reusable messaging **conformance suite** exists (vendors as rows,
  scripted backend), asserting input-schema adherence and schema-valid normalized
  output per declared tool. Slack, Telegram, and `acme-messenger` run it.
- [ ] TEST-2 The `acme-messenger` fixture declares `[messaging]` and drives every
  generic path end-to-end (install → connect → invoke → normalized output),
  proving no generic path needs a real product.
- [ ] TEST-3 The CRITICAL cross-channel relay/act test (SAFE-8) is green in CI.
- [ ] TEST-4 Persistence (Telegram session, resolution cache) passes on both DB
  backends.

## 12. Architecture gates and genericity (GATE) — M4

- [ ] GATE-1 No messaging tool id, profile id, or vendor name appears in a generic
  crate (`ironclaw_architecture` specificity gate extended; allowlist → zero).
- [ ] GATE-2 Composition names no messaging extension; the deletion test (remove a
  messaging extension crate; generic workspace still builds and tests) passes.
- [ ] GATE-3 The retired-taxonomy and dependency-direction gates still pass.

## 13. Docs and rollout (DOC) — M4

- [ ] DOC-1 `messaging-framework.md` and this checklist reflect the shipped design;
  `adr/0002` status updated when accepted.
- [ ] DOC-2 The `reborn-extension-surfaces` skill gains a "messaging tools"
  section (declare a `[messaging]` subset; the puppeting-vs-relaying split).
- [ ] DOC-3 Each `send_message`-class tool ships a prompt doc stating it acts as
  the user and never delivers the final reply (§5.1).
- [ ] DOC-4 Open questions (§16) are tracked; D1–D3 defaults are recorded with
  their revisit triggers.
