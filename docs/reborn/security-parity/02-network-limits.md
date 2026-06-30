# WebUI security parity — 02 Network controls & limits

Part of the #3615 audit. This file owns the CSRF/origin/CORS and
body/rate/connection-limit slice; see `01-auth.md` for authentication
and `03-headers-errors.md` for headers + error sanitization.

- **v1** lives in `src/channels/web/` (`platform/router.rs`,
  `platform/state.rs`, `features/chat/mod.rs`, `oauth/state_store.rs`).
- **v2** controls are descriptor-driven middleware in
  `crates/ironclaw_reborn_composition/src/` (`webui_serve.rs`,
  `webui_ws_origin.rs`, `webui_rate_limit.rs`, `webui_body_limit.rs`),
  reading per-route policy from `ironclaw_webui_v2::webui_v2_routes()`
  and the host SSO mount descriptors from
  `ironclaw_reborn_webui_ingress::webui_v2_auth_router` (`auth/routes.rs`).

Decision legend as in `01-auth.md`: **Keep** / **Change** / **Beta-break**.

## Decision table

| # | Rule | v1 | v2 | Decision |
|---|------|----|----|----------|
| 1 | CORS allow-list | `CorsLayer` localhost-only origins, credentials (`platform/router.rs:524-558`) | `CorsLayer` from `config.allowed_origins`; **empty list fails closed** (`webui_serve.rs` middleware stack) | **Keep** — host supplies the same-origin allow-list; fail-closed default |
| 2 | WebSocket origin check | `is_local_origin` gate on `/api/chat/ws` (`features/chat/mod.rs:525-527,984-1007`) | `enforce_websocket_origin`; `stream_events_ws` descriptor `SameOriginRequired` (Origin vs Host / canonical_host) → 403 (`webui_ws_origin.rs`) | **Keep** — descriptor-driven, adds canonical-host + allowlist policy shapes |
| 3 | OAuth CSRF state | `state_store.rs`: 32-byte state, 5-min TTL, 1024 cap, single-use, PKCE verifier | Pending-flow store `auth/pending.rs`: 32-byte hex state, 5-min TTL, 1024 cap, single-use `take`, cross-provider replay fails closed, S256 PKCE | **Keep** — same shape; v2 adds explicit cross-provider guard |
| 4 | Redirect sanitization | v1 OAuth redirect handling | `?redirect_after=` must start `/`, reject `//`,`/\`, percent-decoded forms, CRLF, fragment markers (`auth/pending.rs`) | **Keep** (hardened) |
| 5 | Request body limit | axum `DefaultBodyLimit` 14 MiB global; 128 KiB system-prompt route (`platform/router.rs:565,408`) | Outer `RequestBodyLimitLayer` 14 MiB default **plus** descriptor per-route caps: `create_thread` 16 KiB, `send_message` 1 MiB, `cancel_run`/`resolve_gate` 4 KiB, `get_timeline`/`stream_events` `NoBody`; SSO `session_exchange`/`logout` 1 KiB (`webui_body_limit.rs`, `auth/routes.rs:138`) | **Change** — strictly tighter; per-route descriptor caps replace one global limit |
| 6 | Rate limiting | Sliding window: chat 30/60s per-user, webhook 10/60s global (`platform/state.rs:74-140`) | Descriptor-driven sliding window: mutation 60/60s, read 120/60s, stream 30/60s **PerCaller**; public SSO + OAuth callback **PerIp** 60–120/60s; unsupported scope fails closed at composition (`webui_rate_limit.rs`, `auth/routes.rs:140-258`) | **Change** — per-route + dual scope (PerCaller for the API, PerIp for the public surface) |
| 7 | Connection limit | `GATEWAY_MAX_CONNECTIONS` (default 100, SSE+WS combined); `SSE_BROADCAST_BUFFER` (`platform/state.rs`) | `SseCapacity`: 3 concurrent streams per `(tenant,user)`, 5-min max lifetime; WS shares the pool (`ironclaw_webui_v2/src/sse_capacity.rs`) | **Beta-break** → #3615 follow-up — the per-caller cap is *stricter per caller*, but v1's **global** ceiling (total SSE+WS process connections) has no v2 analogue: many authenticated callers / source IPs can exceed v1's total. A global backstop must land in the host `serve` lifecycle; no test locks it yet (none exists to lock) |
| 8 | Peer-IP source | n/a | PerIp limiter keys on host-injected `ConnectInfo<SocketAddr>`, never `X-Forwarded-For`/`X-Real-IP`; missing peer fails closed (`webui_rate_limit.rs`) | **Keep** — trusted transport peer only |

## Test coverage

The bulk of the v2 controls are locked in the composition crate's
caller-level suite and the ingress OAuth-route suite; this PR adds the
gaps on the host-owned public SSO surface plus the CORS fail-closed
default.

**This PR** —
`crates/ironclaw_reborn_webui_ingress/tests/network_limits_contract.rs`:

- `sso_login_enforces_per_ip_rate_limit` — `/auth/login/{provider}` →
  60× 307 then 429 (row 6, PerIp scope).
- `sso_session_exchange_enforces_body_limit` — oversized
  `POST /auth/session/exchange` → 413 (row 5).
- `empty_cors_allowlist_fails_closed` — empty allow-list never echoes
  `Access-Control-Allow-Origin` (row 1).

**Already locked (cross-referenced, not duplicated)** —

- `ironclaw_reborn_composition/tests/webui_v2_serve.rs`: CORS
  allow + reject-disallowed-origin; body-limit 413 on `create_thread`
  and NoBody-on-read; rate-limit 429 after the 60/60s budget +
  per-caller independence; WS same-origin 101/403 (missing, mismatched,
  canonical-host); static security headers (rows 1, 2, 5, 6).
- `ironclaw_reborn_webui_ingress/tests/google_oauth_routes.rs` /
  `github_oauth_routes.rs`: CSRF state single-use replay, cross-provider
  replay (`provider_mismatch`), open-redirect fallback (rows 3, 4).
- `ironclaw_reborn_webui_ingress/src/auth/pending.rs::tests`: state TTL,
  1024-entry eviction, single-use, redirect/CRLF/fragment sanitization
  (rows 3, 4).
- `ironclaw_reborn_composition/src/webui_rate_limit.rs::tests`: PerIp
  uses transport peer not forwarded headers, fail-closed on missing
  peer, unsupported scope rejected at composition (rows 6, 8).
- `ironclaw_webui_v2/src/sse_capacity.rs::tests`: 3-stream per-caller
  cap, independent per caller (row 7).

## Notes

Rows 1–6 and 8 are **Keep** or **Change** — those changes tighten limits
(per-route caps, dual-scope rate limiting) rather than relax them.

### Intentional beta-break (linked)

- **Global connection ceiling → per-caller stream cap** (row 7,
  #3615 follow-up). v1's `GATEWAY_MAX_CONNECTIONS` bounded *total*
  SSE+WS process connections (default 100). v2's `SseCapacity` is a
  stricter cap **per `(tenant,user)`** (3 streams + 5-min lifetime) but
  has **no global backstop**: an attacker controlling many authenticated
  callers — or many source IPs against the public SSO surface — can open
  more total concurrent streams than v1 permitted. This is the one row
  where a v1 capability is genuinely dropped, not merely tightened. A
  global ceiling belongs in the host `serve` listener lifecycle (e.g. a
  `tower` concurrency-limit or a connection semaphore around the bound
  `TcpListener`), not the per-route middleware; it is not yet
  implemented, so no test locks it. Until it lands, treat row 7 as a
  known parity gap on the v2 surface.
