# LFD Brief: onboarding-channel-first — Onboarding to channel-first approach

**State**: partial — webui_v2 `setup_extension` lifecycle projection exists
(phase + blockers); no unified first-run wizard. **Bar**: 0.90 holdout.
**Profile**: `onboarding`.

## Outcome

A unified channel-first onboarding: a fresh tenant is guided to connect a
channel before anything else; every installable channel/extension routes
through ONE setup path (chat and Settings converge on it); lifecycle
phases and blockers surface truthfully; completion persists; re-entry is
idempotent; invalid credentials fail soft with actionable blockers.

## Spec sources

- `crates/ironclaw_webui_v2/CLAUDE.md` (setup-extension lifecycle projection)
- `crates/ironclaw_extensions/` (install/activate lifecycle)
- Root CLAUDE.md Extension/Auth Invariants (the resolver rules are the
  heart of this spec: never route by credential_name)
- v1 `src/setup/README.md` 7-step wizard (UX reference only)
- `docs/onboard.mdx`

## Stage 0 inner suite

`ironclaw_extensions` + `ironclaw_webui_v2` crate tests, webui-v2 JS tests,
`tests/reborn_group_extensions/`. Green every cycle.

## Eval themes (dev ~35 / holdout ~12)

1. Fresh-tenant guidance (6): first interaction on web with zero channels →
   response surfaces channel-connect guidance (reply + state_pred on
   projected onboarding phase); after a channel connects, guidance stops
   (forbidden: repeat nag).
2. Per-channel setup routing (9): telegram / slack / gmail setup requests
   from chat → routed by `extension_name` with correct setup surface
   payload (state_pred on wire contract fields; forbidden: generic
   auth-card for installable extensions).
3. Blockers (6): missing credential, failed validation, half-installed
   extension → `blockers` array populated with typed entries; resolving the
   blocker advances `phase`.
4. Persistence + idempotent re-entry (7): completed setup survives restart
   (group storage reload); re-running setup on an installed channel is a
   no-op configure path, not a duplicate install (forbidden: second
   installation record).
5. Chat-vs-Settings convergence (4): the same extension set up from chat
   and from Settings produces identical lifecycle records (state_eq on
   normalized record).
6. Fail-soft (3): invalid token submitted → typed error, phase regresses to
   blocked, no crash, no partial credential stored (forbidden state).

## Feature-specific cheats → fences

- **Route by credential_name anyway** (rebuild the old bug) → contracts
  assert the extension_name field on the wire contract; holdout includes a
  credential shared by two extensions (google_oauth_token → gmail vs
  calendar-like) where credential-based routing gives the wrong answer.
- **Hardcode the known channels** → holdout installs a SYNTHETIC registry
  channel (fixture WASM manifest, unseen name) — wizard must route it via
  the generic resolver; dev has no case with that name. Probe renames
  extension display names (resolver keys must not be display strings).
- **Fake phase transitions** (UI state without persistence) → all phase
  contracts are state queries against extension installation records, not
  response echoes.
- **Nag suppression by never guiding** → theme-1 required matchers price
  the missing-guidance direction; forbidden matchers price the nag
  direction. Two-sided by construction.

## caps.json extras

Extension-name literal branching: pattern
`"(telegram|slack|gmail)"` string-literal match-arm additions in
`crates/ironclaw_webui_v2/**` + `crates/ironclaw_extensions/**` diff,
max 4 (some literal wiring is legitimate; a per-channel if-ladder is not —
the synthetic holdout channel is the real fence).

## Live mode

3 live cases: real model handles "help me set up Slack" conversationally →
required: setup routing tool call with extension_name=slack; forbidden:
asking user to paste the token into chat when the setup surface exists.
