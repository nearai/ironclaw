# PR-3: Slack-Native /ironclaw Dispatcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Typing `/ironclaw status` (or `model …`, or nothing) in Slack invokes the command pipeline natively — no leading-space trick — with help text rendered as `/ironclaw <cmd>` on Slack, and the app-registration snippet documented for all deployments.

**Architecture:** The Slack extension's inbound path branches on `Content-Type`: JSON keeps today's Events-API flow; `application/x-www-form-urlencoded` parses the slash form (`serde_urlencoded`, already in the workspace lock — zero new compiled crates), answering `ssl_check` with an immediate empty 200 and mapping `/ironclaw <text>` to a normalized message whose text is `/<text>` — trigger DERIVED via DM detection (`DirectChat` for DMs, `BotCommand` otherwise, which the direct-conversation admission rejects). Everything downstream is the existing pipeline. `ChannelPresentation` gains `command_prefix: Option<String>`; the observer's help renders `/ironclaw model` on Slack; the WebUI and admission internals stay bare. Spec: `docs/internal/superpowers/specs/2026-07-29-product-command-train-design.md` PR-3 section (as corrected in d80bb29c9). Branch: `pr3-slack-native-dispatcher` (stacks on `pr2-webui-command-palette`; PR base = that branch until PR-2 merges).

**Tech Stack:** Rust 2024 (`ironclaw_slack_extension`, `ironclaw_host_api`, `ironclaw_assistant`, `ironclaw_extension_host`), serde_urlencoded 0.7, Mintlify docs.

## Global Constraints

- No `.unwrap()` / `.expect()` in production code. The slack extension stays transport-light (no network clients).
- One new direct dependency only: `serde_urlencoded = "0.7"` in `ironclaw_slack_extension` (already resolved workspace-wide via axum — state this in the manifest comment).
- The slash form struct is liberal: every field `Option<String>` EXCEPT the four the mapping cannot proceed without (`channel_id`, `user_id`, `command`, `trigger_id`); NO `deny_unknown_fields`.
- Trigger derivation is mandatory: `DirectChat` only for genuine DMs (reuse `is_dm_channel` semantics — slash forms have no `channel_type`, so the `D`-prefix fallback carries it; also treat `channel_name == "directmessage"` as DM); otherwise `ProductTriggerReason::BotCommand`. Never hardcode.
- The existing ingress proptest gate (`payload.rs` `ingress_properties`, "never panics over arbitrary bytes") must stay green UNMODIFIED — the form branch may not introduce any panic path.
- Prefix rendering: ONLY the observer's user-visible help text gets the prefix. `command_admission.rs`'s internal rejection reason stays bare (never user-visible — proven in the transport map). WebUI rendering stays bare `/name`.
- Red-green per behavior change; suites with `--no-fail-fast`; never pipe test output through `head`/`tail`; `cargo fmt -- --check` clean per touched crate BEFORE committing.
- Commit messages end with: `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`

## File Structure

- `crates/ironclaw_slack_extension/src/payload.rs` — form parse + dispatcher mapping; `src/channel.rs` — SslCheck arm + test helper; `Cargo.toml` — dep.
- `crates/ironclaw_host_api/src/channel.rs` — `ChannelPresentation.command_prefix` + validation.
- `crates/ironclaw_assistant/src/commands.rs` — prefix-aware help; `run_delivery/observer.rs` — prefix threading.
- `crates/ironclaw_extension_host/src/channel_host.rs` — assembly read (~1030); `channel_host/e2e_tests.rs` — journeys.
- `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml` — `command_prefix`.
- `docs/internal/reborn/setup-slack-for-reborn-binary.md`, `docs/channels/slack.mdx` — registration.

---

### Task 1: Slash-form transport + dispatcher mapping in the Slack extension

**Files:**
- Modify: `crates/ironclaw_slack_extension/Cargo.toml`, `src/payload.rs`, `src/channel.rs`
- Test: `src/payload.rs` + `src/channel.rs` `#[cfg(test)]` modules (extend existing; the proptest module is a regression gate, don't touch it)

**Interfaces:**
- Consumes: `VerifiedInbound { body, headers, installation_id, .. }` (headers include Content-Type — verified plumbed); `SlackInboundEvent { UrlVerification, Ignore, Message }` (`payload.rs:118-123`); `NormalizedInboundMessage` fields actor/conversation/event_id/text/trigger; helpers `build_actor_ref`, `build_conversation_ref`, `is_dm_channel` (`payload.rs:571-661`); `InboundOutcome::Respond(ImmediateResponse)` precedent (`channel.rs:105-111`).
- Produces: `SlackInboundEvent::SslCheck` variant; `normalize_slack_event(body, headers, installation_id)` signature change (gains headers — update the existing callers/tests) OR a sibling entry `normalize_slack_inbound(request: &VerifiedInbound)` — pick whichever keeps `payload.rs` pure and say why; event ids shaped `slack-{installation}-slash-{trigger_id}`.

- [ ] **Step 1: Failing unit tests** (in `channel.rs` tests via an `inbound_with_headers(body, headers)` sibling of the existing `inbound()` helper, and in `payload.rs` tests for the pure mapping):
  - DM slash: form body `command=%2Fironclaw&text=status&channel_id=D123&channel_name=directmessage&user_id=U123&team_id=T1&trigger_id=111.222.abc` with header `("content-type","application/x-www-form-urlencoded")` → one normalized message: `text == "/status"`, `trigger == ProductTriggerReason::DirectChat`, `actor.id() == "U123"`, `event_id == "slack-install_alpha-slash-111.222.abc"`.
  - Dispatcher args: `text=model+set-provider+openai` → `"/model set-provider openai"`. Defensive strip: `text=%2Fstatus` → `"/status"` (not `"//status"`).
  - Bare and help: empty `text` → `"/help"`; `text=help` → `"/help"`.
  - Non-DM: `channel_id=C777&channel_name=general` → `trigger == ProductTriggerReason::BotCommand`.
  - Foreign command name (an app-config mistake registers a second command at this URL): `command=%2Fsomethingelse&text=hi` → normalized text is the raw invocation `"{command} {text}"` = `"/somethingelse hi"`; the generic classifier/admission then rejects it as undeclared with help. Pin exactly that.
  - `ssl_check=1` form → `InboundOutcome::Respond` with status 200 and EMPTY body.
  - Malformed form (missing `user_id`) → typed `SlackPayloadParseError` (no panic).
  - JSON body with JSON content-type → existing behavior byte-identical (rerun two existing cases through the new header-aware helper).
- [ ] **Step 2: Red.** `cargo test -p ironclaw_slack_extension --no-fail-fast` — new tests fail to compile/fail.
- [ ] **Step 3: Implement.**
  - `Cargo.toml`: `serde_urlencoded = "0.7" # form-encoded Slack slash payloads; already resolved workspace-wide (axum), zero new crates`.
  - `payload.rs`: `#[derive(Deserialize)] struct SlackSlashCommandForm { channel_id: String, user_id: String, command: String, trigger_id: String, text: Option<String>, channel_name: Option<String>, team_id: Option<String>, response_url: Option<String>, ssl_check: Option<String>, token: Option<String> }` (liberal; no deny_unknown_fields). Content-type detection: case-insensitive header lookup; form branch when it contains `application/x-www-form-urlencoded`. `ssl_check` present (any value) → `SlackInboundEvent::SslCheck` (check BEFORE requiring the four mandatory fields — Slack's ssl_check probe carries only `ssl_check` + `token`, so parse a minimal probe struct first or make all fields Option and validate after the ssl_check test). Dispatcher mapping: trim `text`; empty or `help` (case-insensitive) → `"/help"`; else strip leading `/` if present, then `format!("/{trimmed}")`. If `command != "/ironclaw"`, use `format!("{command} {text}")` trimmed as the message text verbatim. Trigger: DM iff `channel_name.as_deref() == Some("directmessage")` or `is_dm_channel(&channel_id, None)`; DM → DirectChat else BotCommand. Event id: `slack-{installation}-slash-{trigger_id}` via the `build_event_id`-adjacent pattern. Conversation: `build_conversation_ref(team_id, channel_id, None, None)`.
  - `channel.rs`: `SlackInboundEvent::SslCheck => Ok(InboundOutcome::Respond(ImmediateResponse { status: 200, content_type: None, body: Vec::new() }))`.
- [ ] **Step 4: Green + gates.** Full crate suite (proptest module must pass unmodified); `cargo fmt -p ironclaw_slack_extension -- --check`; `cargo clippy -p ironclaw_slack_extension --all-targets --all-features -- -D warnings`.
- [ ] **Step 5: Commit** `feat(slack): accept native slash-command payloads through the events ingress`.

---

### Task 2: Command display prefix (manifest → observer help)

**Files:**
- Modify: `crates/ironclaw_host_api/src/channel.rs` (~396-405 + validate), `crates/ironclaw_assistant/src/commands.rs` (`declared_command_help_text`), `crates/ironclaw_assistant/src/run_delivery/observer.rs` (`with_enabled_commands` ~198), `crates/ironclaw_extension_host/src/channel_host.rs` (`build_observer` ~1030), `crates/ironclaw_first_party_extensions/assets/slack/manifest.toml` (`[channel.presentation]` ~227)
- Test: host_api channel tests, `crates/ironclaw_assistant/tests/run_delivery_contract.rs`, extension_host `available_extensions` manifest pins

**Interfaces:**
- Produces: `ChannelPresentation { …, pub command_prefix: Option<String> }` (serde default + skip-if-none; validation: non-empty, starts with `/`, ≤ 32 bytes, no control chars); `declared_command_help_text_with_prefix<I, S>(commands: I, prefix: Option<&str>) -> String` (existing `declared_command_help_text` delegates with `None`; prefixed rendering replaces the leading `/` formatting: `Some("/ironclaw ")` + `model` → `/ironclaw model`); `RunDeliveryObserver::with_enabled_commands` gains the prefix (new signature `with_enabled_commands<I, S>(self, commands: I, prefix: Option<&str>)` — update ALL call sites/tests).

- [ ] **Step 1: Failing tests.** host_api: presentation deserializes `command_prefix = "/ironclaw "`; validation rejects empty / non-slash-leading / control chars; absent field stays None (existing manifests unaffected). run_delivery_contract: observer built with `(["model","status"], Some("/ironclaw "))` delivers help exactly `"Available commands:\n/ironclaw model\n/ironclaw status"`; with `None` exactly the current text (existing pin stays). Manifest pin: bundled slack presentation carries the prefix; telegram carries none.
- [ ] **Step 2: Red.** Product + host_api + extension_host focused suites.
- [ ] **Step 3: Implement** per the interfaces; assembly reads `source.resolved().channel.as_ref().and_then(|c| c.presentation.command_prefix.as_deref())` beside the existing `enabled_commands` extraction and threads it. Slack manifest: `command_prefix = "/ironclaw "` in `[channel.presentation]`. Admission's internal `declared_command_help_text(&self.allowed_commands)` call stays bare (add a one-line comment: internal reason, never user-rendered).
- [ ] **Step 4: Green + suites** for the four touched crates; fmt/clippy per crate.
- [ ] **Step 5: Commit** `feat(channels): render channel help with the manifest command prefix`.

---

### Task 3: Channel-host e2e journeys (signed form bodies)

**Files:**
- Modify: `crates/ironclaw_extension_host/src/channel_host/e2e_tests.rs`

**Interfaces:**
- Consumes: harness `post_event_with_signature` (~216-266) + `slack_signature(timestamp, body)` (~199-213); `SLACK_EVENTS_PATH`; `HarnessOptions.actor_role`; the recording egress + `command_executions`.
- Produces: `Harness::post_slash_command(form_body: &str)` — identical to `post_event_with_signature` but sets `content-type: application/x-www-form-urlencoded` explicitly (axum sets none by default — verified) and signs the raw form bytes with the SAME recipe.

- [ ] **Step 1: Failing tests** (form bodies urlencoded; paired DM uses the harness's bound `U123`/`D123` identities):
  - `slash_dispatcher_dm_status_executes_and_delivers_result`: `command=/ironclaw&text=status` in the DM → exactly one `product.status.command` invoke as the bound user, rendered Status feedback delivered, zero turns.
  - `slash_dispatcher_bare_returns_prefixed_help`: empty text → delivered notice exactly `"Available commands:\n/ironclaw model\n/ironclaw status"` (proves Task 2 end-to-end), zero invokes.
  - `slash_dispatcher_outside_dm_is_rejected_direct_only`: `channel_id=C777&channel_name=general` → direct-conversation denial copy delivered (or, if the bot cannot post there in the harness, assert zero invokes + zero turns + the denial notice attempt recorded — match what the recording egress captures), zero invokes.
  - `slash_form_with_forged_signature_is_rejected`: valid form body, bad signature → 401/403 per the existing forged-HMAC pin, nothing admitted.
  - `ssl_check_form_gets_empty_200_without_admission`: `ssl_check=1&token=x` signed → 200 empty body, zero admissions/invokes.
- [ ] **Step 2: Red** (`cargo test -p ironclaw_extension_host --no-fail-fast -- channel_host`).
- [ ] **Step 3:** implement the helper; adjust only if the red reveals harness gaps (report, don't fudge).
- [ ] **Step 4: Green** full extension_host suite.
- [ ] **Step 5: Commit** `test(slack): pin the native slash dispatcher end to end`.

---

### Task 4: Registration docs (+ fix the stale mdx paths while touching)

**Files:**
- Modify: `docs/internal/reborn/setup-slack-for-reborn-binary.md`, `docs/channels/slack.mdx`

- [ ] **Step 1:** `setup-slack-for-reborn-binary.md`: new "Slash Command" subsection beside "Event Subscriptions" (register ONE command `/ironclaw`, description `Run IronClaw commands`, usage hint `status | model <name> | help`, Request URL = the SAME events URL — one URL serves both surfaces); add `slash_commands:` to the YAML manifest sketch after `event_subscriptions`; verification-checklist line (`/ironclaw status` replies in the bot DM); troubleshooting entry (slash outside the DM is rejected by design; `dispatch_failed` = missing registration/URL); references link `https://docs.slack.dev/interactivity/implementing-slash-commands/`.
- [ ] **Step 2:** `slack.mdx`: `features.slash_commands` array in the JSON manifest; a note in the URLs table that the events URL now serves Events API + slash commands; a step note about registering `/ironclaw`; FIX the pre-existing stale paths in this file while touching it (`/webhooks/slack/events` → `/webhooks/extensions/slack/events`; `oauth/slack_personal/callback` → `oauth/slack/callback`) and say so in the commit body.
- [ ] **Step 3: Commit** `docs(slack): register the /ironclaw slash command and fix stale ingress paths`.

---

### Task 5: Gauntlet + PR

- [ ] `cargo fmt`; three clippy lanes (default / `--all-features` / `--workspace --all-targets --all-features`), all `-D warnings`.
- [ ] `cargo test -p ironclaw_slack_extension -p ironclaw_host_api -p ironclaw_assistant -p ironclaw_extension_host -p ironclaw_architecture_tests --no-fail-fast`; `scripts/pre-commit-safety.sh`; `RUST_MIN_STACK=67108864 bash scripts/reborn-e2e-rust.sh`.
- [ ] Spec sync: re-read the PR-3 section against the diff; fix drift in the spec.
- [ ] Controller pushes and opens the PR (base `pr2-webui-command-palette` until PR-2 merges; retarget to `main` after). Body notes: the 20s-deadline nuance, slash-retry citation, the four Slack apps' one-time registration steps, follow-ups (`response_url` delivery for out-of-DM rejections; Telegram `setMyCommands`).

## Self-review checklist
- Trigger derivation pinned in unit + e2e (non-DM rejection real). Prefix renders ONLY via observer. Proptest gate untouched and green. ssl_check before mandatory-field validation. No deny_unknown_fields on the form. Foreign-command passthrough pinned.
