# ironclaw_webui

The host-owned WebChat v2 HTTP gateway — the single crate that turns a
host-supplied `ProductSurface` into a running web server a browser can talk
to. It owns the route surface and its embedded single-page application,
gateway assembly and the fixed middleware order, the listener/serve loop, all
host-side authentication, and the product-auth HTTP route serving. It is the
sole listener in the product family: every external request that is not a
public webhook authenticates here before touching product, and this crate
alone constructs authenticated-caller evidence.

- **Family / layer:** `product` / `products` · **Package:** `ironclaw_webui` · **Manifest:** `crates/product/ironclaw_webui/Cargo.toml`
- **Use this when:** changing the browser-facing surface — a route, the SPA,
  middleware, an authenticator, an OAuth login provider, or streaming.
- **Don't use this when:** changing what a command or view *does* →
  `ironclaw_assistant` behind `ironclaw_product_contracts`; an OpenAI-shaped
  wire change → `ironclaw_openai_compat`; wiring the surface and stores →
  `ironclaw_composition` (this crate is handed the finished surface by the
  `ironclaw` binary and deliberately stores no threads, transcripts, or
  projections).

## Public surface

- `serve_webui_v2(RebornWebuiServeOptions)` — listener bind + `axum::serve` +
  graceful shutdown (the one sanctioned socket bind in the product/API tier).
- `webui_v2_app(product_surface, config)` — the composed `axum::Router` with
  the fixed middleware order: ws-origin → body limit → bearer/session/OIDC
  auth → rate limit → handler.
- `webui_v2_router(state)` / `webui_v2_routes()` — the route builder and the
  frozen descriptor table (97 routes, contract-locked; re-derive:
  `rg -c 'pub const WEBUI_V2_ROUTE_' src/webui_v2/descriptors.rs`).
- Handlers dispatch to `ironclaw_product_contracts::surface::ProductSurface`
  and render redacted responses through `WebUiV2HttpError`.
- `WebuiAuthenticator` + implementations (`EnvBearerAuthenticator`,
  `SessionAuthenticator`, `OidcAuthenticator`), `SignedTokenSessionStore`,
  `webui_v2_auth_router` (`/auth/*` OAuth login via the `OAuthProvider`
  trait — Google/GitHub), `product_auth_route_mount`,
  `channel_pairing_route_mount`.
- The Vite SPA under `frontend/`, compiled by `build.rs` and embedded.

## Depends on / consumed by

- **Normal workspace deps (10):** `ironclaw_product_contracts` (the surface
  it speaks), `ironclaw_host_api` (caller/ingress vocabulary + the sealed
  evidence home), `ironclaw_host_ingress` (route-mount carriers),
  `ironclaw_openai_compat` (caller-scope stamping on shared protected
  mounts), `ironclaw_auth` (durable product-auth services),
  `ironclaw_extension_contracts`, `ironclaw_attachments` (the advertised
  attachment ceilings — one home for size limits, PROPOSAL §6.4.9),
  `ironclaw_common`, `ironclaw_extension_host` (**bounded**: the pairing
  service core only, never lifecycle authority or installation stores —
  §6.9.4's pairing amendment), and `ironclaw_assistant` (**frozen**: 100
  symbols — 91 command/view/capability constants + 9 wire DTOs — exact-match
  and shrink-only; §12.11 D-B rules this edge charter-sanctioned and
  permanent).
- **Consumed by (1):** the `ironclaw` binary (`crates/app/ironclaw_cli`),
  whose `serve` drives `webui_v2_app` + `serve_webui_v2`.
  (`ironclaw_composition` uses this crate only as a dev-dependency, for the
  every-descriptor-is-mounted regression.)

## Invariants

- **The route table is a contract** — add a route only as handler +
  `webui_v2_routes()` entry (`tests/webui_v2_descriptors_contract.rs`).
- **The product residue is pinned** at 100 symbols, exact-match/shrink-only:
  `reborn_transport_product_boundary.rs` (`WEBUI_PRODUCT_SYMBOL_BASELINE`).
- **`CONTRACT.md` here is the module spec and is gate-pinned** — the 19-owner
  `handlers.rs` charter map is enforced by
  `tests/handlers_module_charter.rs`; the root `AGENTS.md` Module Specs table
  names it. Do not reflow or renumber; edit only with
  `cargo test -p ironclaw_webui` green.
- **Only this crate binds a listener** in the product family
  (`reborn_dependency_boundaries.rs::reborn_product_api_crates_do_not_bind_http_ingress`
  — webui is the stated exemption), and its `BoundaryRule` pins the dependency
  set (`reborn_crate_dependency_boundaries_hold`).
- One cargo feature only: `test-support` (compiles `EmailUserDirectory` for
  standalone deployments and tests); the OpenAI-compat mounts and extension
  administration surface are unconditional.

## Tests

```bash
cargo test  -p ironclaw_webui --all-features
cargo clippy -p ironclaw_webui --all-features --all-targets -- -D warnings
cargo test  -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold
```

The SPA builds through `build.rs` (Vite) — any build needs Node.js plus
corepack/pnpm; `frontend/README.md` covers the JS/TS toolchain.

## See also

Module spec (route table, streaming model, SSE caps, OAuth login security
contract, charter map): `CONTRACT.md` — the spec is the tiebreaker · working
rules: `AGENTS.md` · family rules: `crates/product/AGENTS.md` · design record:
`docs/reborn/target-architecture/families/product.md` (§6.9.4).
