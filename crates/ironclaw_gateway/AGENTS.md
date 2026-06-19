# Agent Map — ironclaw_gateway

## Start Here

- No crate-local `CLAUDE.md` exists yet; use this map plus the web gateway docs below.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these sources of truth before changing behavior:
- `src/channels/web/CLAUDE.md`
- `.claude/rules/gateway-events.md`
- `docs/channels/local.md`

## What This Crate Owns

- Browser frontend asset bundling, layout configuration, branding/tab/widget config types, widget manifests, CSS scoping, and frontend bundle assembly.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- HTTP routes, SSE/WebSocket state, auth, rate limits, CORS/origin checks, channel logic, database access, or runtime workflow behavior.
- Direct `AppEvent` production that bypasses typed source logs and projection rules.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_gateway`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- For user-visible frontend changes, run the narrowest web/gateway test or screenshot flow that covers the changed surface.

## Agent Notes

- Keep this crate as static/frontend composition; server behavior stays in `src/channels/web/`.
- Treat widget IDs, layout JSON, and public config fields as compatibility-sensitive.
- Preserve CSP/nonce-safe asset assembly when adding scripts, styles, or widget injection points.
