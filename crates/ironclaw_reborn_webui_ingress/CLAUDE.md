# ironclaw_reborn_webui_ingress guardrails

Host-owned counterpart to `ironclaw_reborn_composition::webui_v2_app`.
Owns the listener binding + serve loop, the bearer authenticators
(`EnvBearerAuthenticator`, `SessionAuthenticator`, `OidcAuthenticator`),
the durable / in-memory `SessionStore` trait + impl, and the WebChat
v2 OAuth login surface that mints sessions.

Path A of `docs/reborn/how-to-port-channel-to-reborn.md` rules apply:
host auth stays host-owned in this crate, no `src/` (v1) imports, no
v1 secrets / settings / DB.

## Surface

| Symbol | Role |
|---|---|
| `serve_webui_v2(opts)` | Bind a `TcpListener` + run `axum::serve` with graceful shutdown |
| `RebornWebuiServeOptions` | Owner-supplied input (addr, router, shutdown receiver) |
| `EnvBearerAuthenticator` | Single-token `WebuiAuthenticator` for the standalone CLI / local dev; accepted tokens map to operator WebUI capabilities |
| `SessionStore` trait | Pluggable session storage; durable impl is host's; `InMemorySessionStore` for local dev / tests |
| `SessionAuthenticator` | `WebuiAuthenticator` that resolves bearer tokens through a `SessionStore`; accepted tokens map to non-operator WebUI capabilities |
| `OidcAuthenticator` | OIDC bearer-token verifier (JWKS + standard claims); accepted tokens map to non-operator WebUI capabilities |
| `webui_v2_auth_router(config) -> PublicRouteMount` | OAuth login router + route descriptors. The descriptors travel with the router so composition can fold them into the descriptor-driven per-route rate-limit / body-limit middleware — same machinery the v2 facade and product-auth callback already use, no side door. |
| `PublicRouteMount` | `{ router, descriptors }` pair handed to `WebuiServeConfig::with_public_route_mount` |
| `OAuthProvider` trait (in `auth/provider.rs`) | Extension point for per-provider URL / code-exchange logic. Deliberately lives in its own module so each provider does not depend on the others. `GoogleProvider` and `GitHubProvider` ship today. |
| `GoogleProvider` (in `auth/google.rs`) | Google OIDC provider (scopes `openid email profile`, PKCE S256, optional `hd` hosted-domain restriction). Built from `GoogleOAuthConfig`. |
| `GitHubProvider` (in `auth/github.rs`) | GitHub OAuth App provider (scopes `read:user user:email`, no PKCE, verified-email preference). Built from `GitHubOAuthConfig`. |
| `OAuthRouterConfig` | Tenant + `SessionStore` + `UserDirectory` + provider list + base URL |
| `UserDirectory` trait | Host-supplied mapping from `(provider, OAuthUserProfile)` to `UserId` |
| `EmailUserDirectory` | Local-dev default impl (verified email → `UserId`); gated on `dev-in-memory-session` |

## Why the OAuth login router lives here

The crate already owns `WebuiAuthenticator` impls, `SessionStore`, and
the session lifecycle types. The OAuth callback's job is exactly that
— turn a provider profile into a `SessionStore::create_session` call
— so the login mint path belongs in the same host-owned crate, not
behind the product/API seam in `ironclaw_reborn_composition`.

SSO sessions are user identity only. They must not inherit operator
WebUI configuration privileges from the deployment. When the CLI
composes SSO plus the env bearer token, the env token remains the
separate operator credential and session/OIDC bearers remain
non-operator.

Composition merges the `PublicRouteMount` supplied by
`webui_v2_auth_router` through
`WebuiServeConfig::with_public_route_mount`. The router merges
outside bearer auth (the user has no session yet); the
descriptors fold into the same per-route policy stack the rest of
the WebChat v2 surface already rides on. That keeps the
product/API boundary intact: composition never sees provider
secrets, never speaks to Google, never reads a `SessionStore` row.

## WebChat v2 OAuth login surface (#4116)

Routes mounted by `webui_v2_auth_router`:

- `GET  /auth/providers` — list configured provider names.
- `GET  /auth/login/{provider}` — mint a pending flow (CSRF state +
  PKCE verifier + sanitized `redirect_after`) and redirect the
  browser to the provider's authorization URL.
- `GET  /auth/callback/{provider}` — single-use state lookup,
  cross-provider replay guard, code exchange via the matching
  `OAuthProvider`, user resolution via `UserDirectory`, session
  mint via `SessionStore`, and redirect to
  `{redirect_after}?login_ticket=<ticket>` (default `/v2`). The
  ticket is short-lived and single-use; the SPA redeems it over
  same-origin JSON so the bearer never appears in a redirect
  `Location` header.
- `POST /auth/session/exchange` — consume the one-time login ticket
  and return `{ token }`.
- `POST /auth/logout` — bearer-protected; calls
  `SessionStore::revoke` and returns `204` on success or when no
  bearer is present, `500` if revocation fails, so the SPA's local
  clear stays unconditional without lying about server-side state.

### Provider trait

`OAuthProvider` is the seam new providers plug into:

```rust
#[async_trait]
pub trait OAuthProvider: Send + Sync + 'static {
    fn name(&self) -> &OAuthProviderName;
    fn authorization_url(&self, callback_url: &str, state: &str, code_challenge: &str) -> String;
    async fn exchange_code(&self, code: &str, callback_url: &str, code_verifier: &str)
        -> Result<OAuthUserProfile, OAuthError>;
}
```

- `GoogleProvider` ships today (OIDC scopes `openid email profile`,
  PKCE S256, optional `hd=` Workspace hint + server-side `hd`
  claim check, audience+issuer validation; signature verification
  is disabled because the `id_token` arrived over TLS directly
  from Google).
- `GitHubProvider` ships today. It uses GitHub's
  OAuth App flow with scopes `read:user user:email`, ignores the
  PKCE challenge the router computes (GitHub does not support PKCE —
  CSRF is the `state` param only), and after the token exchange
  reads `/user` + `/user/emails`, preferring the primary verified
  email, then any verified email, then the unverified profile email
  flagged `email_verified = false` so the `UserDirectory` fails
  closed. Built from `GitHubOAuthConfig` (client id + secret); no
  hosted-domain analogue.
- NEAR wallet login does NOT fit OAuth code flow and will get its
  own pair of endpoints (`/auth/near/challenge` +
  `/auth/near/verify`) plus its own sub-module under `auth/near/`.
  The `SessionStore` + `UserDirectory` + composition seam stay the
  same.

### Security invariants

- **Pending-flow store** is process-local, bounded (1024 entries +
  5-min TTL), and single-use on `take`. A replayed callback cannot
  re-use a state token; cross-provider replay (state minted for
  Google arriving on the GitHub callback) fails closed.
- **Session exchange tickets** are process-local, bounded (1024
  entries + 60-sec TTL), and single-use on `take`. The OAuth
  callback puts only the ticket in the redirect `Location`; the SPA
  redeems it via `POST /auth/session/exchange` to receive the real
  bearer over a same-origin JSON response.
- **CSRF state** is 32 random bytes (hex). **PKCE verifier** is 32
  random bytes (base64url-no-pad → 43 chars). S256 challenge is
  `base64url_no_pad(sha256(verifier))`.
- **Redirect target** (`?redirect_after=`) is sanitized: must start
  with `/`, must not start with `//` or `/\`, must contain only
  RFC-3986 path chars; the percent-decoded form must also pass so
  smuggled sequences like `%2f%2f` (→ `//`) are rejected.
- **Hosted-domain restriction** is enforced server-side from the
  ID token's `hd` claim, not from the `hd=` URL hint.
- **Error mapping**: every failure path redirects to
  `/v2?login_error=<code>` where `<code>` is an opaque enum
  (`invalid_state`, `provider_mismatch`, `denied`,
  `unauthorized`, `exchange_failed`, `server_error`,
  `invalid_request`). Provider error bodies, JWT decode messages,
  and SessionStore errors are logged via `tracing` and never
  echoed back to the client.
- **Session transport** is one-time login ticket in the callback
  redirect (`?login_ticket=<ticket>`) followed by same-origin
  exchange for the bearer — see
  `ironclaw_reborn_composition/CLAUDE.md` → "Session transport
  decision" for the rationale.

### What the SSO router deliberately does NOT do

- No cookie writes (the SPA stores the exchanged bearer in
  `sessionStorage`).
- No DB schema. `UserDirectory` is host-supplied; the crate ships
  only the local-dev `EmailUserDirectory`.
- No retry / refresh-token handling. The callback is one-shot:
  exchange code, mint session, done. Token refresh is the host's
  job if it wants it.
- No v1 `/auth/*` reuse. The crate has zero `src/`-tier dependency
  by contract; that constraint is what lets WebChat v2 declare a
  hard non-goal on v1 routes (issue #3886).

## Test layout

- `src/{auth, oidc, session}/tests` — unit tests per module
  (provider URL building, PKCE math, ID-token decode, pending
  store, redirect sanitization, session lookup).
- `tests/google_oauth_routes.rs` — caller-level tests on
  `webui_v2_auth_router` covering provider discovery, login
  redirect, callback success, state replay, open-redirect bypass,
  provider error, hd denial, ticket exchange, logout revocation.
- `tests/github_oauth_routes.rs` — caller-level tests driving the
  REAL `GitHubProvider` against a local mock GitHub token/user/emails
  server: discovery, login redirect (state + scope, no PKCE),
  callback success minting a session for the primary verified email,
  an all-unverified login minting a provider-sub (`github:<id>`)
  session rather than an email identity, ticket exchange + single-use
  replay, provider-error and exchange-failure redirects, and logout
  revocation.
- `tests/session_round_trip.rs` — end-to-end test composing
  `webui_v2_app` with `SessionAuthenticator` + the OAuth router;
  drives an OAuth callback, exchanges the resulting ticket, uses the bearer on
  `POST /api/webchat/v2/threads`, then revokes and verifies the
  bearer is rejected. This locks the contract called out in
  #4116's acceptance criteria ("session use on a protected
  WebChat v2 route").
- `tests/oidc_e2e.rs` — pre-existing JWKS-signed ID-token e2e
  for the OIDC authenticator path.
- `tests/serve_loop.rs` — listener bind + graceful shutdown.

## Validation

```bash
cargo test -p ironclaw_reborn_webui_ingress --all-features
cargo clippy -p ironclaw_reborn_webui_ingress --all-features --tests -- -D warnings
```
