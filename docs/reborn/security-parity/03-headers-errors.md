# WebUI security parity — 03 Static headers & sanitized errors

Part of the #3615 audit. This file owns the static-security-header and
sanitized-auth/validation-error slice; see `01-auth.md` and
`02-network-limits.md` for the other two.

- **v1** lives in `src/channels/web/platform/static_files.rs` (CSP) and
  `platform/router.rs` (header layers), with auth-error text in
  `platform/auth.rs`.
- **v2** applies headers via outer `SetResponseHeaderLayer`s in
  `crates/ironclaw_reborn_composition/src/webui_serve.rs`; errors are
  sanitized at the `WebuiAuthenticator` boundary (auth), the
  `WebUiV2HttpError` type (`ironclaw_webui_v2/src/error.rs`), and the
  axum `Json` extractor (validation).

Decision legend as in `01-auth.md`: **Keep** / **Change** / **Beta-break**.

## Decision table

| # | Rule | v1 | v2 | Decision |
|---|------|----|----|----------|
| 1 | `X-Content-Type-Options` | `nosniff` (`platform/router.rs:593-597`) | `nosniff` via outer `SetResponseHeaderLayer` (`webui_serve.rs:708`) | **Keep** |
| 2 | `X-Frame-Options` | `DENY` (`platform/router.rs:598-601`) | `DENY` (`webui_serve.rs:712`) | **Keep** |
| 3 | Content-Security-Policy | `build_csp()` allows CDN/font/img/frame sources for the SPA (`static_files.rs:78-116`) | `default-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'` (`webui_serve.rs:92,716`) | **Change** — v2 default is far stricter (no CDN allowances); the v2 SPA bundles assets, so it does not need v1's CDN/font allowlist |
| 4 | `Referrer-Policy` | (not set by v1 gateway) | `no-referrer` on every response — defense for the SSE `?token=` shim (`webui_serve.rs:728-731`) | **Change** — v2 adds a header v1 lacked |
| 5 | Headers on error responses | layers applied router-wide | `SetResponseHeaderLayer` is outermost, so 401/413/429 carry the same headers | **Keep** — locked by `static_security_headers_present_on_error_response` |
| 6 | Sanitized auth failure | generic `"Invalid or missing auth token"` 401; detail logged not echoed (`auth.rs:1127-1133`) | all auth failures collapse to a generic 401; reason never leaked (`WebuiAuthenticator` contract; `webui_serve.rs:107-125`) | **Keep** |
| 7 | Sanitized validation error | axum extractor rejection → 4xx | `Json<T>` extractor → 400 on malformed body, before the facade; opaque message, no internal detail | **Keep** — locked by `malformed_request_body_returns_sanitized_client_error` |
| 8 | Sanitized OAuth error | v1 OAuth error handling | callback failures redirect to `?login_error=<opaque enum>`; provider/JWT/SessionStore detail logged not echoed (`auth/routes.rs`) | **Keep** — locked by `google_oauth_routes.rs` error-redirect tests |
| 9 | Panic boundary | `CatchPanicLayer` truncates payload (`platform/router.rs:566-592`) | `CatchPanicLayer::custom(panic_handler)` logs truncated detail, returns generic 500 (`webui_serve.rs:706`) | **Keep** |

## Test coverage

**This PR** —
`crates/ironclaw_reborn_webui_ingress/tests/headers_errors_contract.rs`:

- `static_security_headers_present_on_error_response` — an
  unauthenticated 401 still carries `nosniff`, `DENY`, CSP, and
  `Referrer-Policy: no-referrer` (rows 1, 2, 4, 5).
- `csp_directives_are_locked` — the CSP value contains
  `default-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`,
  `base-uri 'self'`, locking the directive content, not just presence
  (row 3).
- `malformed_request_body_returns_sanitized_client_error` — a malformed
  JSON body → 400, never reaches the facade, leaks no path/type/
  traceback (row 7).
- `sse_streams_are_capped_per_caller` — **connection-limit backfill for
  `02-network-limits.md` row 7**: the per-caller SSE concurrency cap
  (default 3) is enforced end-to-end (4th concurrent open → 429, slot
  frees on drop). Landed here because the network-limits PR was already
  open; the `02` catalog row that cross-referenced only the
  `sse_capacity.rs` unit tests is now backed by a route-layer test.

**Already locked (cross-referenced, not duplicated)** —

- `ironclaw_reborn_composition/tests/webui_v2_serve.rs::v2_response_carries_static_security_headers`
  — header presence on a 200 (rows 1, 2, 3).
- Auth-failure 401 sanitization: `webui_v2_serve.rs` (missing/invalid
  bearer) and `auth_route_contract.rs` (issue 1) (row 6).
- OAuth opaque error redirects: `google_oauth_routes.rs` /
  `github_oauth_routes.rs` (row 8).

## Notes / no beta-breaks

All rows are **Keep** or **Change**. The two **Change** rows both make
v2 *stricter* than v1: a tighter default CSP (no CDN/font allowances,
because the v2 SPA bundles its assets) and an added
`Referrer-Policy: no-referrer`. No header or sanitization behavior is
weakened, so nothing here is a beta-break.

This file completes the #3615 authentication / network / headers-error
parity audit: across all three slices, every v1 WebUI security rule is
either preserved (**Keep**) or intentionally tightened (**Change**), and
the only **Beta-breaks** — email-domain restriction moving to the host
`UserDirectory` (#3580) and cookie sessions becoming one-time login
tickets (#4116) — are in `01-auth.md`, both linked. No regression was
found.
