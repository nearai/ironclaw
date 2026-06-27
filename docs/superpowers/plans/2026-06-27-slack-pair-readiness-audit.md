# Slack Pair Readiness Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the Slack `/pair` recovery and Reborn Slack account pairing flow is production-ready end to end, with every previously red or blocked item either fixed and tested or proven to require an external resource that is explicitly documented.

**Architecture:** Slack ingress has two separate paths: slash commands enter through `/webhooks/slack/commands`, while Event Subscriptions and DMs enter through `/webhooks/slack/events`. IronClaw operator Slack setup is separate from a human Slack account pairing; readiness requires both the operator setup contract and the user pairing contract to be verified without leaking bot tokens, signing secrets, bearer tokens, pairing codes that remain live, or personal Slack identifiers beyond non-secret app/team/channel IDs needed for audit.

**Tech Stack:** Rust `ironclaw_reborn_composition`, Rust `ironclaw_reborn_cli`, WebUI v2 static JavaScript, Slack classic app admin UI, Slack Web, local ngrok tunnel, local `ironclaw-reborn serve`.

## Global Constraints

- Do not print, commit, or paste Slack bot tokens, signing secrets, Railway variable dumps, WebUI bearer tokens, or raw secret env files.
- Do not revert unrelated dirty worktree changes.
- Do not declare readiness until every success criterion in this file is checked or moved to the External Blockers section with evidence.
- For production-code changes, use TDD: write the failing test, run it red, implement the minimal fix, run it green.
- For bugs and red tests, use systematic debugging: reproduce, inspect error text, identify root cause, compare working examples, then fix.
- The final report must include exact test commands, results, files changed, and remaining risk.

---

## Current State Snapshot

- Slack app `A0BECPGRKK2` in team `T0AQMKHM7LK` has `/pair` registered with request URL ending in `/webhooks/slack/commands`.
- Slack Event Subscriptions must remain pointed at `https://prideful-nuzzle-payroll.ngrok-free.dev/webhooks/slack/events` and include bot events `message.im` and `app_mention`.
- Local public tunnel used in the prior audit was `https://prideful-nuzzle-payroll.ngrok-free.dev`.
- Local WebUI used in the prior audit was `http://127.0.0.1:8745/v2/` with the token supplied out-of-band.
- Local serve log from the prior audit was `/tmp/serve-pair-test.log`.
- Local serve process from the prior audit was PID `54883`, listening on `127.0.0.1:8745`.
- Prior targeted checks passed: frontend hook test, slash already-linked Rust test, frontend build, CLI build, `cargo fmt --all --check`, composition clippy, product workflow clippy.
- Prior full command `cargo test -q -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --lib` failed with `1384 passed; 3 failed`.
- Consistently red in isolation: `projection::tests::live_progress_stream::skill_learned_bubble_delivers_when_sse_resumes_from_advanced_durable_cursor`.
- Full-suite-only failures from the prior run passed individually: `runtime::tests::runtime_nearai_mcp_bootstraps_from_nearai_session_token` and `runtime::tests::multi_tool_call_response_survives_surface_change_mid_register`.
- Product ambiguity from prior live audit: a blank unconnected chat did not show the Slack pairing panel until the user triggered extension activation from chat.
- Prior live blockers were resolved or covered by later evidence below: the real Slack actor was force-unpaired in the local scratch runtime and re-paired through the real Slack DM + WebUI code flow; deterministic TTL, stale-code, locale, and detailed paired-turn tests cover the remaining cases.
- Product-manager chat-first requirements added during audit:
  - Extensions install/activate and chat-implicit Slack activation must both tell the user to go to Slack, DM the IronClaw Reborn bot, and paste the pairing code into the WebUI pairing panel.
  - The pairing code must be redeemed locally in the UI and must never be sent to the model.
  - After successful pairing, the chat flow must continue the original request.
  - Stale/expired codes entered in chat must keep the pairing panel open, show the `/pair` recovery guidance, and must not continue the chat or leak the stale code to the model.
  - Multiple chats can each surface their own Slack pairing panel; completing one chat's pairing must not silently clear or resume another chat's panel.

## Definition Of Done

- [x] Event Subscriptions Request URL is exactly `https://prideful-nuzzle-payroll.ngrok-free.dev/webhooks/slack/events`, shows Verified, and persists after page reload.
- [x] Event Subscriptions are enabled, changes are saved, and subscribed bot events include `message.im` and `app_mention` while preserving any pre-existing events.
- [x] `/pair` is registered in Slack admin, saved, and re-opened in the edit modal with the expected command, request URL, and description.
- [x] Slack app OAuth scopes include `commands`, and live signed `/pair` invocation proves the installed app is using the command route.
- [x] Local IronClaw Slack setup says `configured: true` and token validation through Slack `auth.test` returns the expected team and bot user without printing secrets.
- [x] Signed manual challenge to `/webhooks/slack/events` returns the challenge body.
- [x] Missing or invalid Slack signatures for `/webhooks/slack/events` and `/webhooks/slack/commands` return `401` without leaking details.
- [x] `GET /webhooks/slack/events` and `GET /webhooks/slack/commands` return `405`.
- [x] Unpaired Slack DM pairing-code instruction path is live-tested by force-unpairing the real local scratch actor, sending a real Slack DM, receiving the bot pairing-code response, redeeming the code through WebUI, and restoring paired Slack DM behavior.
- [x] WebUI Configure modal and chat pairing panels redeem pairing codes through local setup/pairing APIs and never send codes to the model.
- [x] A paired real Slack DM produces a visible assistant reply and a server-side WebUI timeline audit trail showing accepted Slack input, busy rejection for a concurrent message, and finalized assistant output.
- [x] `/pair` for an already-linked real user returns exact ephemeral text `You're already connected.`.
- [x] `/pair` for an unlinked user mints a fresh code, invalidates the previous `/pair` code for that same Slack user, rejects the stale code, and accepts the latest code in automated coverage.
- [x] The pairing panel dismissal persists across browser reloads and has automated regression coverage.
- [x] Blank unconnected chat behavior now shows the Slack pairing panel immediately when Slack is installed but the user is unpaired, with automated coverage.
- [x] Chat-first Slack pairing copy leads with "message the IronClaw Reborn app in Slack for a pairing code" and treats `/pair` as stale/expired-code recovery, not the primary happy path.
- [x] Stale/expired code entry from the in-chat panel shows the invalid/expired `/pair` recovery copy, keeps the panel open, does not continue the chat, and never sends the stale code to the model.
- [x] Successful in-chat Slack pairing resumes the original/left-over chat flow without sending the code to the model.
- [x] Multiple chat threads that need Slack connection remain isolated: pairing one thread resumes only that thread and does not clear another thread's pending pairing panel.
- [x] Explicit Extensions install/Activate/Configure path shows the same DM-bot pairing instructions and direct-code redemption semantics.
- [x] Normal DM pairing-code requests reuse an active unexpired code for the same actor, while `/pair` force-mints a fresh recovery code and invalidates the previous `/pair` code; both semantics have automated coverage.
- [x] Pairing-code TTL expiration is tested deterministically without waiting manually.
- [x] Locale/i18n behavior was audited and all Slack pairing fallback strings were updated across the checked locale files; no missing-key risk was found in the changed path.
- [x] `cargo fmt --all --check` passes.
- [x] `cargo clippy -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --tests` passes.
- [x] `cargo clippy -p ironclaw_product_workflow --all-features --tests` passes.
- [x] `cargo test -q -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --lib` passes with zero failures.
- [x] `node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChat-send.test.mjs` passes.
- [x] `cd crates/ironclaw_webui_v2_static/frontend && npm run build` passes and generated static output is updated when source changes require it.
- [x] `cargo build -p ironclaw_reborn_cli --features "webui-v2-beta slack-v2-host-beta"` passes.
- [x] `FEATURE_PARITY.md`, subsystem docs, or setup docs are updated if the final behavior differs from the tracked status or documented contract.

## Task 1: Preserve The Audit Frame

**Files:**
- Modify: `docs/superpowers/plans/2026-06-27-slack-pair-readiness-audit.md`

**Interfaces:**
- Consumes: The prior live audit results from this Codex thread.
- Produces: A single durable source of truth for future agents.

- [x] **Step 1: Create this file with the definition of done**

Run:

```bash
sed -n '1,260p' docs/superpowers/plans/2026-06-27-slack-pair-readiness-audit.md
```

Expected: this file exists and contains the Definition Of Done section.

- [x] **Step 2: Update this file after each major triage result**

Run:

```bash
git diff -- docs/superpowers/plans/2026-06-27-slack-pair-readiness-audit.md
```

Expected: the file reflects current facts and does not contain secrets.

- [x] **Step 3: Incorporate both original mission documents**

Evidence:
- Initial Event Subscriptions request requires URL verification plus bot events `message.im` and `app_mention`.
- Later `/pair` mission requires registration, reinstall/scope validation, browser-driven E2E pairing coverage, recovery edge cases, and a final findings table.

## Task 2: Root-Cause The Red Composition Tests

**Files:**
- Inspect: `crates/ironclaw_reborn_composition/src/projection/tests/live_progress_stream.rs`
- Inspect: `crates/ironclaw_reborn_composition/src/runtime.rs`
- Modify only after root cause is proven.

**Interfaces:**
- Consumes: failing tests named in Current State Snapshot.
- Produces: passing full composition lib suite or documented external/unrelated failure proof.

- [x] **Step 1: Reproduce the consistently red projection test**

Run:

```bash
cargo test -q -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --lib projection::tests::live_progress_stream::skill_learned_bubble_delivers_when_sse_resumes_from_advanced_durable_cursor -- --exact
```

Expected before fix: failure at `live_progress_stream.rs` complaining that the learned-skill bubble was not delivered from the advanced live cursor.

- [x] **Step 2: Trace the fixture/event cursor assumptions**

Read:

```bash
sed -n '1,340p' crates/ironclaw_reborn_composition/src/projection/tests/live_progress_stream.rs
rg -n "skill_learned|advanced.*cursor|live cursor|learned-skill|durable cursor" crates/ironclaw_reborn_composition/src/projection crates/ironclaw_reborn_composition/src -g '*.rs'
```

Expected: identify the exact event type or cursor boundary that prevents the learned-skill bubble from appearing.

- [x] **Step 3: Write or adjust the smallest failing regression test**

If the existing failing test already captures the intended behavior, use it as RED. If not, add a narrower test in `crates/ironclaw_reborn_composition/src/projection/tests/live_progress_stream.rs` that fails for the same root cause.

- [x] **Step 4: Implement the minimal projection fix**

Modify only the projection code that owns live cursor replay or learned-skill bubble delivery.

- [x] **Step 5: Verify projection fix**

Run the exact failing test again. Expected: `1 passed; 0 failed`.

- [x] **Step 6: Re-run the two runtime tests in isolation**

Run:

```bash
cargo test -q -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --lib runtime::tests::runtime_nearai_mcp_bootstraps_from_nearai_session_token -- --exact
cargo test -q -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --lib runtime::tests::multi_tool_call_response_survives_surface_change_mid_register -- --exact
```

Expected: both pass in isolation. If either fails, triage separately before touching Slack code.

- [x] **Step 7: Re-run the full composition lib suite**

Run:

```bash
cargo test -q -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --lib
```

Expected: zero failures. If full-suite-only runtime failures recur, inspect shared global state, test ordering, timeouts, and runtime registry cleanup before declaring success.

## Task 3: Settle Blank Unconnected Chat Behavior

**Files:**
- Inspect: `crates/ironclaw_webui_v2_static/static/js/pages/chat/hooks/useChat.js`
- Inspect: `crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChat-send.test.mjs`
- Inspect: extension registry/channel UI files if behavior should be driven by installed Slack state.

**Interfaces:**
- Consumes: prior live observation that blank chat did not show pairing UI.
- Produces: an explicit product contract and automated coverage.

- [x] **Step 1: Locate the source of pairing panel creation**

Run:

```bash
rg -n "pairing_required|pendingOnboarding|Slack.*pair|onboarding" crates/ironclaw_webui_v2_static/static/js/pages/chat
```

Expected: pairing panel creation is tied to activation/tool-result state unless other code already supports installed-unpaired startup.

- [x] **Step 2: Decide and document intended behavior**

If Ben has not provided a separate preference, choose the stricter live-audit interpretation: when Slack is installed but the personal Slack account is unpaired, the chat surface must expose a pairing entry point without requiring a model/tool activation message.

- [x] **Step 3: Add a failing frontend test for the chosen contract**

Run:

```bash
node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChat-send.test.mjs
```

Expected before implementation: the new test fails because no panel appears on blank unpaired chat, or because the documented activation-driven contract is not covered.

- [x] **Step 4: Implement minimal UI/state change**

Prefer existing extension/channel setup APIs and avoid duplicating Slack setup semantics inside chat.

- [x] **Step 5: Verify frontend behavior**

Run:

```bash
node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChat-send.test.mjs
cd crates/ironclaw_webui_v2_static/frontend && npm run build
```

Expected: tests and build pass.

## Task 4: Finish Slack Edge Cases And Evidence

**Files:**
- Inspect: `crates/ironclaw_reborn_composition/src/slack_serve.rs`
- Inspect: `crates/ironclaw_reborn_composition/src/slack_serve/handler_tests.rs`
- Inspect: Slack setup and pairing WebUI code as needed.

**Interfaces:**
- Consumes: local Slack app, local tunnel, local serve, Slack Web session.
- Produces: evidence that all Slack readiness criteria pass.

- [x] **Step 1: Verify server and setup state**

Run:

```bash
lsof -nP -iTCP:8745 -sTCP:LISTEN
set -a; source "$HOME/shared-env/ironclaw/slack-test.env"; set +a; curl -sS -H "Authorization: Bearer $IRONCLAW_REBORN_SLACK_BOT_TOKEN" https://slack.com/api/auth.test | jq '{ok, team_id, user_id, user}'
```

Expected: server listening; Slack auth `ok: true`; expected team and bot user. Do not print token values.

- [x] **Step 2: Capture signed route evidence**

Run signed challenge and signed slash-command requests with helper scripts or one-off shell commands that source the signing secret without printing it.

Expected: challenge echoes body; invalid signatures return `401`; slash command returns ephemeral JSON.

- [x] **Step 3: Test real paired DM with log evidence**

Send a unique real DM in Slack Web. Capture visible response and server logs around that timestamp.

Expected: visible assistant reply plus server log lines showing accepted webhook, turn accepted, provider request, assistant reply, and completed turn. If the current log level does not show those lines, increase local logging and restart before re-running.

- [x] **Step 4: Test repeated pairing-code requests for one Slack user**

Use a real Slack user that is unlinked, or safely unpair/re-pair a test user only after preserving the original paired state. If that is not safe, prove the intended host-state semantics in deterministic tests.

Expected: normal DM pairing-code requests reuse an active unexpired code for the same actor; `/pair` force-mints a recovery code and invalidates the prior `/pair` code for that actor.

- [x] **Step 5: Test TTL deterministically**

Prefer an automated Rust test using the existing code store/clock seam. Do not spend 10 minutes waiting if a deterministic test can cover the expiry contract.

Expected: a code older than the configured TTL is rejected with the user-facing invalid/expired message.

## Task 4A: Harden Chat-First Slack Pairing UX

**Files:**
- Inspect/modify: `crates/ironclaw_webui_v2_static/static/js/pages/chat/hooks/useChat.js`
- Inspect/modify: `crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChat-send.test.mjs`
- Inspect/modify: `crates/ironclaw_webui_v2_static/static/js/pages/chat/components/onboarding-pairing-card.js`
- Inspect/modify: `crates/ironclaw_webui_v2_static/static/js/pages/extensions/components/configure-modal.js`
- Inspect/modify: `crates/ironclaw_reborn_composition/src/slack_connectable_channel.rs`

**Interfaces:**
- Consumes: Product-manager expectation that chat is the primary connection surface.
- Produces: automated coverage for stale codes, continuation, and multi-thread isolation.

- [x] **Step 1: Make the primary Slack pairing copy DM-bot first**

Expected: connectable-channel copy and chat fallback copy say to message the Slack bot for a code, then mention `/pair` only as recovery for expired/stale codes.

- [x] **Step 2: Add stale-code chat regression coverage**

Expected: `submitOnboardingPairing` rejects with the invalid/expired `/pair` copy, keeps `pendingOnboarding`, does not call `sendMessage`, and does not include the stale code in any model-bound body.

- [x] **Step 3: Add multi-thread pending pairing regression coverage**

Expected: thread A and thread B can both have pending Slack pairing state across thread switches; pairing thread B resumes only B and does not dismiss A.

- [x] **Step 4: Add explicit Extensions Configure copy coverage**

Expected: channel Configure modal contains the "message Slack bot / paste code / never sent to the model" semantics.

- [x] **Step 5: Rebuild and rerun frontend tests**

Run:

```bash
node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChat-send.test.mjs
node --test crates/ironclaw_webui_v2_static/static/js/pages/extensions/components/configure-modal.test.mjs
cd crates/ironclaw_webui_v2_static/frontend && npm run build
```

Expected: all pass and `static/dist/app.js` reflects source changes.

- [x] **Step 6: Test locale/i18n state**

If the app has a language switcher for WebUI v2, switch away from English and verify pairing UI does not break layout or show missing keys. If Slack pairing copy is currently intentionally English-only, document the code evidence and product risk.

Expected: no missing translation keys, no layout breakage, or explicitly accepted English-only risk.

## Task 5: Final Readiness Gate

**Files:**
- Modify: this plan file with final evidence.
- Modify: code/docs only as required by prior tasks.

**Interfaces:**
- Consumes: completed Tasks 2-4.
- Produces: a final answer Ben can trust.

- [x] **Step 1: Run formatting, clippy, frontend tests, frontend build, Rust build, and full composition lib tests**

Run:

```bash
cargo fmt --all --check
cargo clippy -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --tests
cargo clippy -p ironclaw_product_workflow --all-features --tests
node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChat-send.test.mjs
(cd crates/ironclaw_webui_v2_static/frontend && npm run build)
cargo build -p ironclaw_reborn_cli --features "webui-v2-beta slack-v2-host-beta"
cargo test -q -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --lib
```

Expected: all pass with zero failures.

- [x] **Step 2: Check status and secret hygiene**

Run:

```bash
git status --short
rg -n "xox[baprs]-|SLACK_(BOT|SIGNING)|IRONCLAW_REBORN_WEBUI_TOKEN|e2e-reborn-v2-bearer-token|ngrok-free.dev" crates docs --glob '!**/target/**'
```

Expected: no committed or newly added raw secrets. Placeholder token text is acceptable only when clearly not a credential and does not match real Slack token prefixes.

- [x] **Step 3: Update final evidence in this file**

Add a dated Final Evidence section containing command results, live Slack test IDs/timestamps safe to share, and any accepted blockers.

- [x] **Step 4: Final response**

Ben should receive a concise answer that states whether readiness is fully satisfied. If not fully satisfied, do not soften it; list the exact remaining blocker and why it cannot be completed in this session.

---

## Final Evidence - 2026-06-27

### Code Changes Audited

- Projection live-progress cursor fix:
  - `RebornProjectionServices` now shares one `Arc<AtomicU64>` live sequence across publishers instead of giving each publisher an independent sequence counter.
  - This fixes the learned-skill bubble replay hole when a new SSE subscriber resumes from an advanced durable cursor.
- Runtime env test isolation fix:
  - NEARAI runtime env tests now use one async/std combined guard for all mutated env keys, avoiding full-suite-only races and nested-lock deadlocks.
- Slack `/pair` recovery:
  - Already-linked actors receive exact ephemeral text `You're already connected.`.
  - `/pair` force-mints a recovery code and invalidates that actor's prior `/pair` code.
  - libSQL coverage now asserts consumed pairing-code records are physically removed, not merely marked consumed.
- Chat-first Slack linking:
  - Blank unpaired chat opens the Slack pairing panel from installed Slack/connectable-channel state.
  - Activation-derived stale panels are cleared when Slack is already connected elsewhere.
  - Stale/expired code submission stays local, keeps the panel open, shows `/pair` recovery guidance, does not continue chat, and does not send the code to the model.
  - Successful pairing resumes the original thread with the continuation message, not whichever chat is currently open.
  - Multiple pending chat threads remain isolated.
  - Explicit Extensions Configure and chat activation panels now say to message the IronClaw Reborn app in Slack, paste the code in the panel, and that the code is never sent to the model.
- Docs/status:
  - `FEATURE_PARITY.md` Slack host-beta note now mentions slash-command pairing recovery and chat-first local code redemption.

### Live Slack Admin Evidence

- Slack app `A0BECPGRKK2`, team `T0AQMKHM7LK`, bot user `U0BDJFDEJRY`.
- Event Subscriptions admin page:
  - Enable Events: on.
  - Request URL: `https://prideful-nuzzle-payroll.ngrok-free.dev/webhooks/slack/events`.
  - Verified badge present.
  - Bot events include `app_mention` and `message.im`.
  - Save Changes remained disabled after reload, proving the settings were persisted.
- Slash Commands admin page:
  - `/pair` exists.
  - Request URL: `https://prideful-nuzzle-payroll.ngrok-free.dev/webhooks/slack/commands`.
  - Description: `Get a fresh Ironclaw pairing code`.
- OAuth scope page includes `commands`, `im:history`, `app_mentions:read`, `chat:write`, and `im:write`.

### Live Route Evidence

- Local server and tunnel:
  - `lsof -nP -iTCP:8745 -sTCP:LISTEN` showed `ironclaw-reborn` listening on `127.0.0.1:8745`.
  - ngrok API showed `https://prideful-nuzzle-payroll.ngrok-free.dev` forwarding to `http://127.0.0.1:8745`.
- Public signed Event Subscriptions challenge:
  - Result: `signed_event=200 body=fresh-public-route-ok`.
- Public method guards:
  - `GET /webhooks/slack/events` returned `405`.
  - `GET /webhooks/slack/commands` returned `405`.
- Public signature guards:
  - Missing event signature: `401`.
  - Invalid event signature: `401`.
  - Missing command signature: `401`.
  - Invalid command signature: `401`.
- Public signed slash command:
  - Minimal synthetic body without `api_app_id` returned `401 {"error":"authentication"}` as expected because installation routing uses the realistic Slack form fields.
  - Full signed Slack-like body returned `200 {"response_type":"ephemeral","text":"You're already connected."}`.

### Live Slack User Evidence

- Real Slack Web actor used for live tests: `U0BDC16TML3`.
- Forced-unpaired live smoke:
  - Backed up the local scratch libSQL DB before mutation to `/tmp/ironclaw-reborn-local-dev-before-unpair-20260627-180604.db`.
  - Deleted only the actor's Slack identity row and personal DM target row from the local scratch runtime, then verified both counts were `0`.
  - Sent real Slack Web DM `live unpaired code request 1782598102002`.
  - Slack bot replied at `1782598104.115899` with `Connect this Slack account to Ironclaw by entering code [REDACTED_CODE] in WebChat.`.
  - Redeemed the code through `POST /api/webchat/v2/extensions/pairing/redeem`; response was `200`, provider `slack`, provider user `ironclaw-testing-pr5362:U0BDC16TML3`.
  - After redeem, the current actor identity row count was `1`; after the repaired paired DM below, the personal DM target row count was also restored to `1`.
  - Verified the current actor's redeemed code had `0` remaining code rows in the local scratch DB.
- Stale-code live route probes:
  - Two pre-existing expired pairing-code rows for older test actors were submitted to the real WebUI redeem route.
  - Both returned `400` with `Invalid or expired Slack pairing code. Run /pair in Slack to get a new one.` and no provider binding.
- Repaired paired-DM live smoke:
  - Sent real Slack Web DM `live repaired paired check 1782598135384`.
  - Slack bot replied at `1782598146.496379` with `Live paired check received: *1782598135384*.`.
  - Signed public `/pair` for the repaired actor returned `200 {"response_type":"ephemeral","text":"You're already connected."}`.
- Forced-unpaired `/pair` recovery live smoke:
  - Backed up the local scratch libSQL DB before mutation to `/tmp/ironclaw-reborn-local-dev-before-unpair-slash-20260627-182116.db`.
  - Signed `/pair` while unpaired returned `200` ephemeral text `Here's your fresh Ironclaw pairing code: [REDACTED_CODE]...`.
  - A second signed `/pair` while still unpaired returned a different code hash and invalidated the first code.
  - Redeeming the first code after the second `/pair` returned `400` with the invalid/expired `/pair` recovery copy.
  - Redeeming the second code returned `200`, provider `slack`, provider user `ironclaw-testing-pr5362:U0BDC16TML3`.
  - After repair, identity rows = `1`, DM target rows = `1`, current actor code row count = `0`, and signed `/pair` again returned `You're already connected.`.
- Forced-unpaired WebUI connectable-channel live smoke:
  - Backed up the local scratch libSQL DB before mutation to `/tmp/ironclaw-reborn-local-dev-before-unpair-matrix-20260627-182213.db`.
  - Deleted the live actor identity + personal DM target and captured immediate counts: identity rows = `0`, DM target rows = `0`.
  - `GET /api/webchat/v2/channels/connectable` returned Slack account connection copy: `Message the IronClaw Reborn app in Slack to get a pairing code...`, placeholder `Enter Slack pairing code...`, and invalid-code copy `Invalid or expired Slack pairing code. Run /pair in Slack to get a new one.`.
  - Repaired via signed unpaired `/pair` + `POST /api/webchat/v2/extensions/pairing/redeem`; after repair, identity rows = `1`, DM target rows = `1`, current actor code row count = `0`, and signed `/pair` returned `You're already connected.`.
- Paired-DM live smoke:
  - Sent two unique Slack DM messages through Slack Web while the public ngrok route was active.
  - Slack bot replied visibly; the concurrent second message produced a busy/rejected state.
  - WebUI timeline for thread `ea1a4b31-9db3-4942-8fdb-d5546ab699c8` showed:
    - First Slack user message accepted and bound to Slack DM `D0BDC6LR6JX`.
    - Second Slack user message recorded as `rejected_busy`.
    - Assistant message finalized for the accepted turn.
- Real `/pair` live smoke:
  - Slack Web `/pair` for the already-linked user showed the ephemeral text `You're already connected.`.
  - The signed public command probe for the same actor returned the same text.

### Automated Verification Evidence

- Formatting/build:
  - `cargo fmt --all` passed.
  - Fresh final `cargo fmt --all --check` passed.
  - Fresh final `cd crates/ironclaw_webui_v2_static/frontend && npm run build` passed.
  - Fresh final `cargo build -p ironclaw_reborn_cli --features "webui-v2-beta slack-v2-host-beta"` passed.
- Rust clippy/tests:
  - Fresh final `cargo clippy -p ironclaw_product_workflow --all-features --tests` passed.
  - Fresh final `cargo clippy -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --tests` passed.
  - Fresh final `cargo test -q -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta slack-v2-host-beta libsql" --lib` passed: `1388 passed; 0 failed; finished in 88.66s`.
- Targeted Rust tests:
  - `slack_inbound_proof_code_connectable_channel_matches_pairing_copy` passed.
  - `filesystem_slack_host_state_rejects_expired_pairing_code` passed.
  - Fresh forced-unpair follow-up `cargo test -q -p ironclaw_reborn_composition --features "libsql slack-v2-host-beta" filesystem_slack_host_state_deletes_libsql_pairing_code_record_after_consumption` passed, proving libSQL consumed-code physical deletion for the current backend.
  - `slack_pair_twice_returns_different_codes_and_invalidates_first` passed.
  - `slack_pair_already_linked_returns_connected_message` passed.
  - `filesystem_slack_host_state_reuses_active_pairing_code_for_actor` passed.
  - `redeem_route_maps_invalid_code_to_bad_request` passed.
- Frontend tests:
  - `node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/useChat-send.test.mjs` passed: 43/43.
  - `node --test crates/ironclaw_webui_v2_static/static/js/pages/chat/lib/extension-onboarding.test.mjs` passed as part of the fresh final combined run.
  - `node --test crates/ironclaw_webui_v2_static/static/js/pages/extensions/components/configure-modal.test.mjs` passed: 4/4.
  - `node --test crates/ironclaw_webui_v2_static/static/js/components/slack-pairing-section.test.mjs` passed: 3/3.
  - Fresh final combined four-file frontend run passed: `55 pass; 0 fail`.

### Unpaired Path Matrix

- Slack DM while unpaired:
  - Live evidence: forced-unpaired Slack Web DM returned the bot pairing-code instruction with code redacted, then WebUI redeem restored the actor.
  - Automated evidence: `filesystem_slack_host_state_reuses_active_pairing_code_for_actor` and pairing notifier coverage.
- `/pair` while unpaired:
  - Live evidence: signed public `/pair` returned an ephemeral fresh code; a second `/pair` returned a different code and invalidated the first; latest code redeemed successfully; post-repair `/pair` returned already-connected.
  - Automated evidence: `slack_pair_twice_returns_different_codes_and_invalidates_first` and `slack_pair_already_linked_returns_connected_message`.
- Explicit Extensions/Configure while unpaired:
  - Live evidence: unpaired `channels/connectable` API exposed Slack account connection copy telling the user to message the bot, paste the code into WebChat, and use `/pair` for stale/expired recovery.
  - Automated evidence: `ConfigureModal renders the pairing panel for a channel extension`, `ConfigureModal pairing redeems then activates, invalidates queries, and closes`, and `SlackPairingSection activates Slack after redeeming a pairing code`.
- Implicit chat activation while unpaired:
  - Automated evidence: `onboardingFromExtensionActivatePreview: opens Slack pairing panel from activation preview`, `onboardingFromToolMessages: opens panel from reloaded timeline tool card`, and `useChat: blank unpaired Slack chat opens pairing panel from extension state`.
- Stale/expired code in chat:
  - Live evidence: expired code rows submitted to the real redeem route returned `400` invalid/expired copy with no provider binding; stale first `/pair` code after second `/pair` also returned `400`.
  - Automated evidence: `useChat.submitOnboardingPairing: stale Slack code stays local and does not resume chat`, `redeem_route_maps_invalid_code_to_bad_request`, and `filesystem_slack_host_state_rejects_expired_pairing_code`.
- Left-over chat continuation after pairing:
  - Automated evidence: `useChat.submitOnboardingPairing: Slack redemption resumes chat without leaking code`, `useChat.submitOnboardingPairing: resumes the pairing panel's thread, not another open chat`, and `onboardingFromToolMessages: suppresses stale Slack panel after continuation`.
- Multiple chats needing Slack connection:
  - Automated evidence: `useChat.submitOnboardingPairing: resumes the pairing panel's thread, not another open chat`, `useChat.send: addresses a second thread in parallel while viewing a running thread`, and the destination-thread busy/not-busy regression tests.

- Hygiene:
  - Fresh final `git diff --check` passed.
  - Fresh final strict changed-file raw token scan for Slack tokens, Slack cookie tokens, and literal secret assignments produced no output.

### Residual Notes

- No remaining blocker is documented for the Slack pairing/linking readiness criteria in this file.
- The forced-unpair live test used a reversible local scratch DB mutation because there is no surfaced product unlink operator flow in the inspected routes. This is acceptable as audit setup evidence only; it is not a product workflow.
- The post-repair local server log stream did not emit the Railway-style accepted/completed trace lines during the final smoke, but Slack history, WebUI route responses, and local DB state proved the real actor moved from unpaired to paired and then produced a visible bot reply. Earlier paired-DM smoke still includes WebUI timeline evidence for accepted input, busy rejection, and finalized assistant output.
