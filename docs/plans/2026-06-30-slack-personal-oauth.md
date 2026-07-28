# Slack Personal (user-token) OAuth — Flow A, single shared app

Status: in progress (branch `feat/slack-user-tool-oauth-reborn`)

## Goal

Convert the `slack_user` first-party extension (Slack "personal" tool) from a
**manual `xoxp-` token paste** to a **browser OAuth flow** ("Flow A"), so each
tenant/employee connects their personal Slack with one "Connect" click instead
of creating a Slack app and pasting a token.

- **Single shared Slack app**: the same app that issues the workspace bot token
  (`IRONCLAW_REBORN_SLACK_BOT_TOKEN`) also holds the User Token Scopes and OAuth
  client credentials used here. Number of apps is invisible to the end user.
- **Credential stays separate** in IronClaw: provider id `slack_personal`, its
  own credential — never collides with the bot extension (`slack`).
- **Approach (B) DUPLICATE**: add parallel Slack gate provider / callback state
  next to Google's. Google's code path is left functionally untouched.

## The three OAuth halves in Reborn product-auth (and Slack's divergence)

Extension credentials (`source = product_auth_account`) use the product-auth
OAuth system in `crates/ironclaw_auth` + `crates/ironclaw_reborn_composition`.
Three parts, each currently assuming "standard OAuth" that Slack violates:

| Half | Anchor | Slack divergence |
|---|---|---|
| Authorize/challenge (builds consent URL) | `oauth_gate.rs` `GoogleOAuthGateProvider` + `ironclaw_auth::build_google_authorization_url` | Slack wants `user_scope=` (not `scope=`), authorize `https://slack.com/oauth/v2/authorize` |
| Callback state (encoded in `state`) | `ironclaw_auth/oauth.rs` `GoogleOAuthCallbackState` (prefix `icg1.`) | Provider-agnostic contents; duplicate as `SlackPersonalOAuthCallbackState` (prefix e.g. `ics1.`) with permissive scope validation |
| Token exchange | `oauth_provider_client.rs` `HostOAuthProviderClient` / `parse_token_response` | Flat `access_token` today. Slack user token is nested at `authed_user.access_token`, scopes at `authed_user.scope`; POST `https://slack.com/api/oauth.v2.access` |

## Endpoints / scopes

- Provider id: `slack_personal` (matches current manifest `provider`).
- Authorize: `https://slack.com/oauth/v2/authorize`, user scopes in `user_scope`.
- Token: `https://slack.com/api/oauth.v2.access`.
- User scopes: `search:read`, `channels:history`, `groups:history`, `im:history`,
  `mpim:history`, `channels:read`, `groups:read`, `im:read`, `mpim:read`,
  `users:read`, `chat:write`.
- Token response (user token): `{"ok":true, "authed_user":{"access_token":"xoxp-...","scope":"...","token_type":"user"}, ...}`.

## Implementation steps

### 1. `crates/ironclaw_auth/src/oauth.rs`
- Add consts: `SLACK_PERSONAL_PROVIDER_ID = "slack_personal"`,
  `SLACK_AUTHORIZATION_ENDPOINT`, `SLACK_TOKEN_ENDPOINT`, the user-scope consts,
  `is_allowed_slack_personal_scope`.
- Add `build_slack_personal_authorization_url(...)`: mirrors
  `build_google_authorization_url` but appends `user_scope` (not `scope`) and no
  Google `access_type/prompt/hd` extras. Builds the URL directly (the generic
  `build_authorization_url` hardcodes the `scope` pair).
- Add `SlackPersonalOAuthCallbackState` (dup of `GoogleOAuthCallbackState`,
  prefix `ics1.`, permissive scope validation).
- Export new items from `lib.rs`.

### 2. `crates/ironclaw_reborn_composition/src/oauth_provider_client.rs`
- Add `TokenResponseShape { Standard, SlackAuthedUser }` and a field on
  `HostOAuthProviderSpec` (default `Standard` for Google/Notion → no behavior
  change). `parse_token_response` becomes shape-aware: for `SlackAuthedUser`,
  deserialize `authed_user.{access_token,scope}` and check top-level `ok`.
- Slack has no refresh token by default → `refresh_token = None`, no expiry.

### 3. `crates/ironclaw_reborn_composition/src/slack_personal_oauth.rs` (new)
- `slack_personal_provider_spec() -> HostOAuthProviderSpec` (token endpoint,
  `secret_handle_prefix = "slack_personal"`, `resource: None`,
  `exchange_scope_policy: FallbackToRequested`, `token_response_shape:
  SlackAuthedUser`).
- `SlackPersonalOAuthGateProvider` + registry: DUPLICATE of `oauth_gate.rs`,
  using `build_slack_personal_authorization_url` + `SlackPersonalOAuthCallbackState`.

### 4. Gate + provider wiring (`product_auth_providers.rs`, `auth.rs`, `factory.rs`)
- Register slack provider client in `compose_provider_client_with_runtime`
  (behind `slack-v2-host-beta`).
- Add a parallel `slack_gate_registry` slot on `RebornProductAuthServices` +
  `with_slack_oauth_gate_registry`; chain it after the Google/DCR registries at
  the dispatch site (`auth.rs:1469`) and in `oauth_pkce_verifier_for_flow`.

### 5. Callback route (`product_auth_serve/`)
- Slack callback: add `slack_oauth_callback_handler` (dup of
  `google_oauth_callback_handler`, decodes `SlackPersonalOAuthCallbackState`) +
  a `SLACK_OAUTH_CALLBACK_PATH`, OR route Slack through the existing generic
  `oauth_callback_handler`. Prefer dedicated handler for parity with Google.

### 6. Config (`ironclaw_reborn_cli/src/runtime/mod.rs`)
- `resolve_slack_personal_oauth_config_from_env` reading
  `IRONCLAW_REBORN_SLACK_PERSONAL_CLIENT_ID/SECRET/OAUTH_REDIRECT_URI`; wire via
  a `with_slack_personal_oauth_backend(client)` builder → provider backend config.
- Single-app note: operator points these at the same Slack app as the bot token;
  redirect `http://127.0.0.1:3000/api/reborn/product-auth/oauth/slack/callback`.

### 7. Manifest `crates/ironclaw_first_party_extensions/assets/slack_user/manifest.toml`
- Each capability credential `source`: keep `provider = "slack_personal"`, add
  `setup = { kind = "oauth", scopes = [...] }` + `provider_scopes = [...]`.

### 8. Tests
- authorize URL emits `user_scope` and no `scope`; state round-trips.
- token parse extracts nested `authed_user.access_token` / scope.
- gate challenge routes `slack_personal` requirement to the Slack gate.
- config resolution (reborn-prefixed vars; None when unset).

## Live-test checklist (operator)
1. In the Slack app: add the User Token Scopes above; add redirect
   `.../product-auth/oauth/slack/callback`.
2. Export `IRONCLAW_REBORN_SLACK_PERSONAL_CLIENT_ID/SECRET/OAUTH_REDIRECT_URI`.
3. Rebuild with `--features webui-v2-beta,slack-v2-host-beta`; open Extensions →
   "Slack (personal)" → Connect → authorize → confirm `search_messages` works.

## PKCE risk
The shared exchanger sends `grant_type=authorization_code` + PKCE `code_verifier`.
Slack `oauth.v2.access` supports PKCE and ignores unknown params; verify against
the live app during test. If PKCE proves problematic, add a spec flag to omit it
for Slack.

## Deferred follow-ups (accepted at review, 2026-07-04)

- **F-009 — served Slack OAuth journey test.** No served WebUI/API test yet
  drives Extensions/Chat → OAuth start → callback → activation → source-chat
  resume end-to-end. Accepted as a follow-up risk: the path is triangulated by
  direct-handler tests (scope enforcement, callback claim/PKCE, identity
  binding), the `slack_user` per-user runtime dispatch proof, and the JS
  behavioral suites now running in CI.
- **Generic proof-code redeem route (retired).** The unmounted WebUI scaffolding
  was removed. Shipped channel connection strategies must now be fully
  implemented and manifest-driven; host-generated pairing uses the supported
  code/deep-link/QR flow.
