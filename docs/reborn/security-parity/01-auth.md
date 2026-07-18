# WebUI security parity — 01 Authentication

Part of the #3615 audit: inventory the v1 WebUI auth rules and record,
for each, whether WebChat v2 **Keeps**, **Changes**, or **Beta-breaks**
the behavior. This file owns the authentication slice (bearer, DB /
session, OIDC, and the query-token exception). Sibling files
`02-network-limits.md` and `03-headers-errors.md` cover the rest.

- **v1** auth lives in `src/channels/web/platform/auth.rs` (wired by
  `src/channels/web/platform/router.rs`).
- **v2** auth is the three `WebuiAuthenticator` impls in
  `crates/ironclaw_webui/` (`lib.rs`, `session.rs`,
  `oidc.rs`), selected by the host and enforced by the composition
  middleware in `crates/ironclaw_reborn_composition/src/webui/webui_serve.rs`.
  v2 shares **zero** code with v1 by contract (#3886).

Decision legend: **Keep** = behavior preserved (possibly different
implementation); **Change** = intentionally different but not a feature
loss; **Beta-break** = a v1 capability the beta deliberately drops or
relocates, linked to a tracking issue.

## Decision table

| # | Rule | v1 (`auth.rs` unless noted) | v2 | Decision |
|---|------|------------------------------|----|----------|
| 1 | Auth selection | Runtime three-tier fallback env→DB→OIDC→401 (`auth.rs:1058-1134`) | One `WebuiAuthenticator` impl chosen per deployment; no runtime fallback chain (`webui_serve.rs:284`) | **Change** — architecture differs, end state (authenticated `UserId` or 401) equivalent |
| 2 | Constant-time token compare | SHA-256 hash + `subtle::ConstantTimeEq` over digests (`auth.rs:90-94,157-166`) | `EnvBearerAuthenticator` `ct_eq` on raw token (`lib.rs:191`); `InMemorySessionStore::lookup` `ct_eq` per entry (`session.rs:213`) | **Keep** |
| 3 | Bearer prefix parsing | `Bearer ` prefix, case-insensitive (`auth.rs:1025-1045`) | `eq_ignore_ascii_case("Bearer ")` (`webui_serve.rs:805-820`) | **Keep** — locked by `bearer_prefix_is_case_insensitive_parity_with_v1` |
| 4 | DB-backed token store | `DbAuthenticator` LRU cache, 60 s TTL, 1024 entries; invalidate-on-suspend (`auth.rs:202-288`) | Host-supplied `SessionStore`; `InMemorySessionStore` for dev (`session.rs:96-227`). No TTL cache — `revoke` is immediate | **Change** — note revocation latency: v1 up to 60 s stale window vs v2 immediate |
| 5 | OIDC verification | JWKS 1 h cache, 64-key cap, 10 s fetch backoff, `iss`/`aud`/`exp` (`auth.rs:370-632`) | `OidcAuthenticator`: 5-min JWKS TTL, stale-while-revalidate, single-flight, RS/ES only (rejects HS256), `iss`/`aud`/`exp`/`nbf` (`oidc.rs`) | **Keep** — behavior-equivalent; differing cache TTL and an explicit algorithm allowlist. Locked at the route layer by `oidc_signed_token_authenticates_protected_route_and_bad_claims_rejected`, `oidc_hs256_token_rejected_on_route` (alg-confusion: RSA modulus as HMAC secret), and `oidc_not_yet_valid_nbf_token_rejected_on_route`; and at the authenticator layer by `oidc_authenticator_rejects_hs256_tokens` + `oidc_authenticator_rejects_future_nbf` |
| 6 | Email-domain restriction | Server-side `check_email_domain` + `email_verified` requirement (`auth.rs:942-964,1100-1116`) | Delegated to host `UserDirectory` (`auth/user_directory.rs`); dev `EmailUserDirectory` does not restrict. Google `hd` claim still server-checked (`auth/google.rs`) | **Beta-break** → #3580 (host-responsibility seam) |
| 7 | Query-token exception | `?token=` on GET allowlist of three routes: `/api/chat/events`, `/api/logs/events`, `/api/chat/ws` (`auth.rs:979-1007`) | `?token=` honored on exactly one route, `GET /api/webchat/v2/threads/{id}/events`; WS and all mutations are bearer-only (`webui_serve.rs:805-841`) | **Change** — narrowed (tighter); SSE escape hatch kept, `/logs/events` + WS query-token dropped. Locked by `query_token_honored_on_sse_events_route` + `query_token_rejected_on_mutation_route` + `query_token_rejected_on_websocket_route` (the WS-specific drop, locked directly not inferred) |
| 8 | Token transport / precedence | Bearer header > `?token=` > `ironclaw_session` cookie (`auth.rs:1025-1045`); OAuth callback sets `Set-Cookie: HttpOnly` | Bearer header only (+ SSE `?token=`); no cookie. OAuth callback returns one-time `login_ticket` exchanged for a bearer kept in `sessionStorage` | **Beta-break** → #4116 (session-transport divergence; see composition CLAUDE.md "Session transport decision"). Locked by `cookie_session_not_honored_on_protected_route` (cookie ignored) + the `SET_COOKIE`-absent assertion in `callback_success_creates_session_and_redirects_with_login_ticket` |
| 9 | Failure sanitization | 401 `"Invalid or missing auth token"`; 503 `"Database unavailable"`; reasons logged not echoed (`auth.rs:1082-1133`) | All auth failures collapse to `None` → generic 401; reason never leaked; backend faults logged at `warn!` (route-layer 401 in `authenticate_request`/`unauthorized`, `webui_serve.rs:769-803`; session backend fault logging `session.rs:260-272`) | **Keep** — note v1's distinct 503-on-DB-down signal has no v2 analogue (session backend faults also surface as 401, logged for operators). Locked by `unauthorized_body_is_generic_and_leaks_no_reason` (asserts the 401 body is the fixed string and leaks no token / user / cause) |
| 10 | Operator-only config gate | v1 admin role on settings/provider routes | `/api/webchat/v2/llm/*` mounted only when `allows_operator_webui_config()` is true — `EnvBearerAuthenticator` opts in (`lib.rs:198`), the mount decision reads it (`webui_serve.rs:541`); dropped from the route table for multi-user authenticators, which inherit the trait default-false (`webui_serve.rs:116,122-123`) via the `!mount_operator_routes` retain-filter (`webui_serve.rs:556`) | **Change** — explicit capability gate replaces role string. Locked by `operator_config_route_{mounted_for_operator,absent_for_multi_user}_authenticator` (401 vs 404) |

## Intentional beta-breaks (linked)

- **Email-domain restriction → host `UserDirectory`** (#6). v1 enforced
  the allowlist in the gateway; v2 makes account acceptance the host's
  responsibility via `UserDirectory`. The dev `EmailUserDirectory` is
  intentionally permissive. Tracked by the entrypoint/host-responsibility
  inventory in **#3580**.
- **Cookie session → one-time login ticket** (#8). v1 set an `HttpOnly`
  session cookie on the OAuth callback; v2 never sets a cookie — it
  returns a short-lived single-use `login_ticket` in the redirect, which
  the SPA exchanges for a bearer over same-origin JSON. Rationale and
  regression coverage are in `crates/ironclaw_reborn_composition/CLAUDE.md`
  ("Session transport decision"), tracked under **#4116**.

Both breaks inherit the v1-routes hard non-goal (**#3886**): the v2
listener never re-introduces v1 `/auth/*` handlers.

## Test coverage

- **v2 route-layer** (`crates/ironclaw_webui/tests/`):
  - `auth_route_contract.rs` — env-bearer accept/reject, missing /
    empty-token / no-prefix → 401, case-insensitive prefix parity,
    revoke-then-reject, expired-session rejection, `?token=` SSE shim
    accept (incl. resolved-caller identity) + mutation reject + WS
    reject, `ironclaw_session` cookie rejection, sanitized-401-body
    (no reason leak), OIDC route-layer (valid + bad claims, HS256
    alg-confusion, future-`nbf`), operator-config mounting boundary
    (this PR).
  - `session_round_trip.rs` — OAuth callback → session mint → protected
    route → logout revocation. `google_oauth_routes.rs` additionally
    asserts the callback sets no `Set-Cookie` (row 8).
  - `oidc_e2e.rs` — `OidcAuthenticator::authenticate()` in isolation
    (JWKS fetch, kid-miss refresh, single-flight, backoff, HS256
    rejection, future-`nbf` rejection). The route-layer composition is
    covered by the `auth_route_contract.rs` OIDC test above (row 5).
  - `signed_session_multi_user.rs` — per-user session isolation.
- **v1** (`auth.rs:1138-2546`): 60+ unit tests over hashing, query-token
  allowlist, OIDC claims, key cache + backoff, domain restriction.
