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
| `EnvBearerAuthenticator` | Single-token `WebuiAuthenticator` for the standalone CLI / local dev |
| `SessionStore` trait | Pluggable session storage; durable impl is host's; `InMemorySessionStore` for local dev / tests |
| `SessionAuthenticator` | `WebuiAuthenticator` that resolves bearer tokens through a `SessionStore` |
| `OidcAuthenticator` | OIDC bearer-token verifier (JWKS + standard claims) |
| `webui_v2_auth_router(config) -> PublicRouteMount` | OAuth login router + route descriptors. The descriptors travel with the router so composition can fold them into the descriptor-driven per-route rate-limit / body-limit middleware — same machinery the v2 facade and product-auth callback already use, no side door. |
| `PublicRouteMount` | `{ router, descriptors }` pair handed to `WebuiServeConfig::with_public_route_mount` |
| `OAuthProvider` trait (in `auth/provider.rs`) | Extension point for per-provider URL / code-exchange logic. Deliberately lives in its own module so each provider does not depend on the others. `GoogleProvider` and `GitHubProvider` ship today. |
| `GoogleProvider` (in `auth/google.rs`) | Google OIDC provider (scopes `openid email profile`, PKCE S256, optional `hd` hosted-domain restriction). Built from `GoogleOAuthConfig`. |
| `GitHubProvider` (in `auth/github.rs`) | GitHub OAuth App provider (scopes `read:user user:email`, no PKCE, verified-email preference). Built from `GitHubOAuthConfig`. |
| `NearLoginProvider` (in `auth/near/`) | NEAR wallet login — NEP-413 challenge/verify, not OAuth code flow, so it does NOT implement `OAuthProvider`. Owns a bounded single-use nonce store + the `view_access_key` RPC client. Built from `NearAuthConfig` (`NearNetwork` + optional RPC override); wired via `OAuthRouterConfig::with_near_provider`. |
| `OAuthRouterConfig` | Tenant + `SessionStore` + `UserDirectory` + provider list + base URL |
| `UserDirectory` trait | Host-supplied mapping from `(provider, OAuthUserProfile)` to `UserId` |
| `EmailUserDirectory` | Local-dev default impl (verified email → `UserId`); gated on `dev-in-memory-session` |

## Why the OAuth login router lives here

The crate already owns `WebuiAuthenticator` impls, `SessionStore`, and
the session lifecycle types. The OAuth callback's job is exactly that
— turn a provider profile into a `SessionStore::create_session` call
— so the login mint path belongs in the same host-owned crate, not
behind the product/API seam in `ironclaw_reborn_composition`.

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

- `GET  /auth/providers` — list configured provider names (includes
  `near` when a `NearLoginProvider` is wired).
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
- `GET  /auth/near/challenge` — mint a single-use NEP-413 nonce +
  the message the wallet signs (`{ nonce, message, recipient,
  network }`). `404` when NEAR is not configured.
- `POST /auth/near/verify` — consume the nonce, verify the Ed25519
  signature is bound to it, confirm the public key is an active
  access key on the account via `view_access_key` RPC, resolve the
  user through `UserDirectory`, mint a session, and return the
  bearer over same-origin JSON (`{ token }`). No `login_ticket`
  round-trip: this flow has no provider redirect, so there is no
  `Location` header for the bearer to leak through.

  The two NEAR routes are always mounted (so the descriptor list the
  per-route policy middleware folds in stays static); the handlers
  `404` when no `NearLoginProvider` is wired, mirroring the v1
  gateway's `/auth/near/*` behavior when NEAR auth is disabled.

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
- NEAR wallet login does NOT fit OAuth code flow, so it is NOT an
  `OAuthProvider`. It ships as `NearLoginProvider` under `auth/near/`
  with its own `/auth/near/challenge` + `/auth/near/verify` pair,
  wired through `OAuthRouterConfig::with_near_provider`. The
  `SessionStore` + `UserDirectory` + composition seam are unchanged:
  `verify` projects a normalized `OAuthUserProfile` (provider-user-id
  = NEAR account id, no email, `email_verified = false`) and the same
  `UserDirectory::resolve` mints the session. NEP-413 verification is
  re-implemented in `auth/near/verify.rs` rather than shared with v1's
  `src/channels/web/oauth/near.rs` because this crate carries no
  `src/`-tier dependency by contract; both copies pin the same tag /
  field orderings and have their own tests.

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
  decision" for the rationale. NEAR verify is the exception: it has
  no redirect, so it returns the bearer directly over its
  same-origin JSON response (no ticket needed).
- **NEAR nonce store** is process-local, bounded (1024 entries +
  5-min TTL), and single-use on `consume` — a replayed verify with an
  already-spent nonce fails closed before any crypto runs. **NEP-413
  binding** rejects raw-message signatures: only payloads that frame
  the nonce verify, so a captured signature cannot be replayed.
  **Wrong-network / unknown access keys** fail at the
  `view_access_key` RPC (a key valid on another chain is absent on the
  configured network's RPC → `401`). **Suspended / unrecognized
  accounts** fail when `UserDirectory::resolve` returns `Unknown` →
  `403`. RPC backend faults map to `503`, distinct from the `401`
  auth-miss path, so operators can tell an infra outage from a bad
  login.

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
  ticket exchange + single-use replay, provider-error and
  exchange-failure redirects, and logout revocation.
- `tests/near_auth_routes.rs` — caller-level tests driving the REAL
  `NearLoginProvider` against a local mock NEAR RPC server: provider
  discovery (advertises `near` only when configured), `404` when not
  configured, challenge shape, full challenge→sign→verify minting a
  session bound to `near:<account_id>`, replayed-nonce and
  unknown-nonce rejection (`400`), invalid-signature (`401`),
  wrong-network access key (`401`), RPC backend fault (`503`), and
  malformed public key (`400`). Locks #4181's acceptance criteria.
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
