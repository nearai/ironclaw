# Slack pairing recovery — `/pair` slash command + discoverability nudge

**Date:** 2026-06-26
**Status:** Approved design, implementation in progress
**Origin:** Slack thread usability bug — a user (Yuting) consumed/expired her one-time
Slack pairing code and had no self-service way to get a new one. Firat: "we need a
recovery command / slash pair … this is a usability bug."

> **Scope reduction (2026-06-26, shipped PR).** The implemented PR is narrowed to
> exactly two behaviors: (1) the `/pair` slash command (Decisions 1 & 4; Components
> A–D) and (2) the web redeem **error** copy instructing the user to run `/pair`
> (part of Decision 5 / Component F). The **discoverability nudge (Decision 2,
> Component E)** and the **standalone Playwright e2e** are **cut** to keep the PR
> tight — the silent unpaired-DM drop is left unchanged, so the
> `SlackUnlinkedNudgeNotification` type, the `send_unlinked_nudge` notifier method,
> and the resolver nudge-cooldown cache are not shipped. The redeem-form
> instructions keep their `/pair` wording, and the invalid/expired-code **error**
> string is set on the **route response body** (`slack_personal_binding_pairing_serve.rs`,
> the `BadRequest` arm), not only in frontend copy: the web pairing card renders
> that JSON `error` verbatim (`slackPairingError` prefers `error.payload.error`),
> so Component F's "pure copy / frontend-only" framing was incomplete — the
> descriptor (`slack_connectable_channel.rs`) and i18n strings are the
> network/non-JSON-error fallback. The nudge remains a documented future
> enhancement.

## Problem

The Reborn v2 Slack personal-binding pairing flow (`crates/ironclaw_reborn_composition/src/slack_*`):

- Mints a pairing code **reactively** (only when an unpaired Slack user messages the
  bot), keyed to `(installation_id, slack_user_id)`, DM'd once.
- The code is **single-use** and **expires in 10 minutes** (`DEFAULT_PAIRING_TTL`,
  `slack_host_state.rs:57`); redeeming consumes + deletes it (`consume_challenge`,
  `slack_host_state.rs:1022`).
- The store's `issue_challenge` **reuses an existing active actor challenge** rather
  than minting fresh (`slack_host_state.rs:919-940`); a 60s resolver dedup
  (`reserve_pairing_challenge`) suppresses re-issue on rapid retries.
- Unpaired inbound messages are **silently dropped** (`resolve_product_actor_user`
  returns `Ok(None)`, `slack_personal_binding_pairing.rs:441`) — "the slack panel ate
  my input message."
- The web "Channel settings" field only **redeems** a code
  (`POST /api/webchat/v2/extensions/pairing/redeem`); it cannot issue one. All redeem
  failures collapse into one string: `"Invalid or expired pairing code."`
  (`slack_personal_binding_pairing_serve.rs:182`).

Net effect: once the single reactively-issued code dies, every recovery surface is a
dead end, and the user gets no feedback. A web-side "request code" button **cannot**
fix this — at first-pair time the backend doesn't yet know the user's Slack identity,
so it has nowhere to deliver a code. Recovery must be **Slack-side**.

## Goals

- Self-service recovery: a user can always get a fresh, usable pairing code.
- Discoverable: a user who DMs the bot cold learns how to recover.
- No change to the redeem path, the binding model, or the single-use/10-min semantics.
- Reuse the existing HMAC verification + installation-resolution substrate.

## Non-goals (YAGNI)

- Web "request code" button (can't deliver — see above).
- DM/channel delivery of the code (ephemeral only).
- Revoke UI / code listing.
- Any change to redeem, binding, TTL, or single-use semantics.
- Legacy v1 Slack channel (`channels-src/slack/`, `src/pairing/`) — untouched.

## Decisions

1. **Surface:** Firat's literal `/pair` Slack slash command (new signed endpoint).
2. **Discoverability nudge:** replace the silent unpaired-DM drop with a rate-limited
   reply pointing to `/pair`.
3. **Delivery:** ephemeral, in-place slash response (private to the invoking user;
   not in channel history; no DM scope needed). Lost it? Run `/pair` again.
4. **Force-fresh semantic:** `/pair` always mints a brand-new code and invalidates any
   prior outstanding one (recovery = always fresh), rather than reuse-if-still-valid.
5. **Bundle a small web copy update** so the redeem form references `/pair` + 10-min
   expiry.

## Components

### A. Slash route + descriptor — `slack_serve.rs` / `serve_slack.rs`
New `SLACK_COMMANDS_PATH = "/webhooks/slack/commands"`, mounted alongside the events
route in the same Slack `PublicRouteMount`. New `IngressRouteDescriptor` mirroring
`slack_events_policy()` (`slack_serve.rs:236`): `ListenerClass::PublicWebhook`,
`IngressAuthScheme::WebhookSignature`, small body limit (~8 KiB), Global pre-auth rate
limit + per-installation bucket.

### B. Command ingress resolution — `slack_serve/installation.rs`
New `resolve_command_ingress(headers, body)`: verify HMAC (reuse `HmacWebhookAuth`,
`slack_host_beta.rs:841` — HMAC is over the raw body, so format-agnostic for the
`application/x-www-form-urlencoded` slash payload), parse the form fields (`command`,
`team_id`, `api_app_id`, `enterprise_id`, `user_id`, `response_url`, `trigger_id`),
resolve the installation via the same selector events use. Reject anything but
`command == "/pair"`.

### C. Force-mint seam — `slack_personal_binding_pairing.rs` (trait) + `slack_host_state.rs` (impl)
Add `reissue_challenge(installation, slack_user)` to
`SlackPersonalBindingPairingChallengeStore`. Under the existing actor lock, expire +
clean any existing actor/code record (reuse `cleanup_actor_pairing_code_record` /
`cleanup_pairing_actor_record`, `slack_host_state.rs:1286-1329`), then mint a brand-new
code via the existing allocation loop. Always returns a fresh, usable code and
invalidates the prior. (The 60s resolver dedup doesn't apply — `/pair` never goes
through `resolve_product_actor_user`.)

### D. Slash handler — `slack_serve.rs`
Orchestrates B → already-paired check (via `RebornUserIdentityLookup`) → C. If already
linked: ephemeral "You're already connected." Otherwise ephemeral
`{response_type:"ephemeral", text:"Your pairing code is \`CODE\` — enter it in IronClaw
→ Settings → Channels → Slack within 10 minutes."}`. Code never enters a non-ephemeral
message and is never logged. Force-mint is two filesystem writes — inside Slack's 3s ack
window (fallback: delayed ephemeral via `response_url`).

### E. Discoverability nudge — `slack_personal_binding_pairing.rs:~427/441`
Replace the silent deduped `Ok(None)` with a rate-limited nudge DM ("You're not linked
yet — run `/pair` in Slack to get a code") via the existing notifier. Still returns
`Ok(None)` (unpaired messages don't become turns). Nudge cooldown reuses/extends the
pending-challenge cache so we nudge at most once per window.

### F. Web copy — `slack-pairing-section.js` + i18n
Redeem-form instructions/error reference `/pair` and the 10-min expiry. Pure copy.

## Data flow (`/pair` happy path)

User types `/pair` (DM or channel) → Slack POSTs signed form to
`/webhooks/slack/commands` → HMAC verified + installation resolved (B) → already-paired?
ephemeral "already connected" : force-mint fresh code (C) → ephemeral reply with code
(D) → user pastes into web redeem form → existing `redeem_challenge` binds. Lost it? Run
`/pair` again — idempotent and cheap.

## Error handling (per `.claude/rules/error-handling.md`)

- Bad/missing signature → 401, no body processing (ingress auth layer).
- Wrong/malformed command → **200 + ephemeral** error text (Slack shows a generic
  failure on non-200 and may retry).
- Installation unresolvable / pairing unavailable → ephemeral "Pairing is temporarily
  unavailable — try again," cause logged server-side (carry the cause; no
  `map_err(|_| …)`; no paths/internals leaked).

## Testing (per `.claude/rules/testing.md` — test through the caller)

- **handler_tests:** bad-sig → 401; valid `/pair` → ephemeral 200 with a code; wrong
  command → ephemeral error; two `/pair`s return different codes and the first is now
  un-redeemable; rate-limit trips.
- **store:** `reissue_challenge` expires old actor+code records and mints new; old code
  rejected by `redeem_challenge`; concurrent reissue is CAS-safe.
- **nudge (caller-level):** one unpaired inbound DM → exactly one nudge within cooldown,
  none after; message does not become a turn.
- **e2e (`tests/e2e`):** cold DM → nudge → `/pair` → code → web redeem → bound. Gated on
  a Slack signing secret in the harness, like existing Slack e2e.
- **web:** source-shape test for the copy (existing pattern).

## Dependencies / ops

- **Slack app manifest:** register the `/pair` slash command with Request URL
  `<public-host>/webhooks/slack/commands` and the `commands` scope — per deployment and
  locally. Document in the `serve_slack` docs.
- Signing secret already wired (`IRONCLAW_REBORN_SLACK_SIGNING_SECRET`); slash reuses it.
