# WebUI security parity — 03 Static headers & sanitized errors

Part of the #3615 audit. This file owns the static-security-header and
sanitized-auth/validation-error slice; see `01-auth.md` and
`02-network-limits.md` for the other two.

- **v1** lives in `src/channels/web/platform/static_files.rs` (CSP) and
  `platform/router.rs` (header layers), with auth-error text in
  `platform/auth.rs`.
- **v2** applies headers via outer `SetResponseHeaderLayer`s in
  `crates/ironclaw_reborn_composition/src/webui/webui_serve.rs`; errors are
  sanitized at the `WebuiAuthenticator` boundary (auth), the
  `WebUiV2HttpError` type (`ironclaw_webui/src/error.rs`), and the
  axum `Json` extractor (validation).

Decision legend as in `01-auth.md`: **Keep** / **Change** / **Beta-break**.

## Decision table

| # | Rule | v1 | v2 | Decision |
|---|------|----|----|----------|
| 1 | `X-Content-Type-Options` | `nosniff` (`platform/router.rs:593-597`) | `nosniff` via outer `SetResponseHeaderLayer` (`webui_serve.rs:708`) | **Keep** |
| 2 | `X-Frame-Options` | `DENY` (`platform/router.rs:598-601`) | `DENY` (`webui_serve.rs:712`) | **Keep** |
| 3a | CSP — API/JSON routes | `build_csp()` allows CDN/font/img/frame sources for the SPA (`static_files.rs:78-116`) | `default-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'self'`, applied by the composition layer with `SetResponseHeaderLayer::if_not_present` (`webui_serve.rs:92,716`) | **Change** — strict default for every route that does not set its own CSP (all `/api/webchat/v2/*` JSON routes). Does **not** apply to the HTML document, which sets its own CSP first (3b) |
| 3b | CSP — SPA document (`/` index) | `build_csp()` (CDN/font allowances) | `render_index_with_nonce` sets a CSP for same-origin scripts, styles, fonts, assets, and connections; inline scripts require a per-request nonce while same-origin external bundles are allowed by `'self'`, and inline styles remain allowed (`ironclaw_webui/src/static_assets/router.rs`). Because the composition CSP is `if_not_present`, this document CSP wins on the shell | **Keep** — hardened over v1 with same-origin resources, a per-request inline-script nonce, `object-src 'none'`, and no `unsafe-eval` or `'unsafe-inline'` scripts |
| 3c | CSP — wallet-connect popup | (n/a) | `script-src 'self' 'unsafe-inline' https:; style-src 'self' 'unsafe-inline'; connect-src 'self' https:; frame-src 'self' https: data:` (`ironclaw_webui/src/static_assets/router.rs`) | **Change (looser, scoped)** — the isolated wallet popup deliberately relaxes `script-src` to `'unsafe-inline' https:` so the wallet connector can load remote executors and reach relays; scoped to the single `/wallet/connect` route, not the app shell |
| 4 | `Referrer-Policy` | (not set by v1 gateway) | `no-referrer` on every response — defense for the SSE `?token=` shim (`webui_serve.rs:728-731`) | **Change** — v2 adds a header v1 lacked |
| 5 | Headers on error responses | layers applied router-wide | `SetResponseHeaderLayer` is outermost, so 401/413/429 carry the same headers | **Keep** — locked by `static_security_headers_present_on_error_response` |
| 6 | Sanitized auth failure | generic `"Invalid or missing auth token"` 401; detail logged not echoed (`auth.rs:1127-1133`) | all auth failures collapse to a generic 401; reason never leaked (`WebuiAuthenticator` contract; `webui_serve.rs:107-125`) | **Keep** |
| 7 | Sanitized validation error | axum extractor rejection → 4xx | `Json<T>` extractor → 400 on malformed body, before the facade. The body is axum's **standard `JsonRejection`** text (e.g. `Failed to parse the request body as JSON: …line N column M`) — it carries no filesystem paths, Rust type names, tracebacks, or secrets, but it is **not** a fully opaque string (it includes serde's structural parse position, which is not sensitive) | **Keep** — locked by `malformed_request_body_returns_sanitized_client_error` (asserts no path / type-name / traceback / token leak) |
| 8 | Sanitized OAuth error | v1 OAuth error handling | callback failures redirect to `?login_error=<opaque enum>`; provider/JWT/SessionStore detail logged not echoed (`auth/routes.rs`) | **Keep** — locked by `google_oauth_routes.rs` error-redirect tests |
| 9 | Panic boundary | `CatchPanicLayer` truncates payload (`platform/router.rs:566-592`) | `CatchPanicLayer::custom(panic_handler)` logs truncated detail (`tracing::error!`, not echoed), returns a generic `500 Internal Server Error`; sits inside the header layer so the 500 still carries the static security headers (`webui_serve.rs:706,926-953`) | **Keep** — locked by `panic_boundary_returns_sanitized_500` (a panicking handler with a sensitive message → 500 whose body is exactly `Internal Server Error`, leaking no path / SQL / token / `::`) |

## Test coverage

**This PR** —
`crates/ironclaw_webui/tests/headers_errors_contract.rs`:

- `static_security_headers_present_on_error_response` — an
  unauthenticated 401 still carries `nosniff`, `DENY`, CSP, and
  `Referrer-Policy: no-referrer` (rows 1, 2, 4, 5).
- `csp_directives_are_locked` — the API-route CSP value contains
  `default-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`,
  `base-uri 'self'`, locking the directive content, not just presence
  (row 3a).
- `panic_boundary_returns_sanitized_500` — a handler that panics with a
  sensitive-looking message → `500` whose body is exactly
  `Internal Server Error` (no path / SQL / token / `::` leak), with the
  static security headers still present (row 9).
- `malformed_request_body_returns_sanitized_client_error` — a malformed
  JSON body → 400, never reaches the facade, leaks no path / type-name /
  traceback / token (row 7).
- `sse_streams_are_capped_per_caller` — **connection-limit backfill for
  `02-network-limits.md` row 7**: the per-caller SSE concurrency cap
  (default 3) is enforced end-to-end (4th concurrent open → 429, slot
  frees on drop). Landed here because the network-limits PR was already
  open; the `02` catalog row that cross-referenced only the
  `sse_capacity.rs` unit tests is now backed by a route-layer test.

**Already locked (cross-referenced, not duplicated)** —

- `ironclaw_reborn_composition/tests/webui_v2_serve.rs::v2_response_carries_static_security_headers`
  — header presence on a 200 (rows 1, 2, 3a).
- `ironclaw_webui/src/static_assets/router.rs::tests`:
  `standalone_spa_shell_carries_matching_csp_nonce` (the document nonce
  matches the CSP) and `spa_document_csp_allowlist_is_locked` (the
  document CSP pins resources to the same origin, keeps the inline-script
  nonce, and carries no `unsafe-eval` / no `'unsafe-inline'` in
  `script-src`), plus
  `wallet_connect_popup_gets_relaxed_csp_and_spa_shell_stays_strict`
  (rows 3b, 3c).
- Auth-failure 401 sanitization: `webui_v2_serve.rs` (missing/invalid
  bearer) and `auth_route_contract.rs` (issue 1) (row 6).
- OAuth opaque error redirects: `google_oauth_routes.rs` /
  `github_oauth_routes.rs` (row 8).

## Notes

All rows are **Keep** or **Change**; no beta-break in this slice.

CSP needs care because there are **two distinct policies**, not one:

- The composition layer sets the strict `default-src 'self'; object-src
  'none'; frame-ancestors 'none'; base-uri 'self'` with
  `if_not_present` (row 3a). It governs the JSON API surface — anything
  that does not set its own CSP.
- The HTML shell sets its **own** CSP first (row 3b), so the strict
  default never reaches it. The document is CDN-free: Vite emits the app
  bundle under `/assets/`, and fonts are vendored under `/vendor/`. Its
  external scripts are allowed by `'self'`; only inline scripts need the
  per-request nonce. The wallet-connect popup (row 3c) is intentionally
  looser (`script-src 'self' 'unsafe-inline' https:`), scoped to one route.

The `Referrer-Policy: no-referrer` header (row 4) is a genuine v2
addition. No header or sanitization behavior is weakened, so nothing
here is a beta-break.

This file completes the #3615 authentication / network / headers-error
parity audit: across all three slices, every v1 WebUI security rule is
either preserved (**Keep**) or intentionally tightened (**Change**), and
the only **Beta-breaks** — email-domain restriction moving to the host
`UserDirectory` (#3580) and cookie sessions becoming one-time login
tickets (#4116) — are in `01-auth.md`, both linked. No regression was
found.
