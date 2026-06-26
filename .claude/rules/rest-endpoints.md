# REST Endpoint Catalog (WebChat v2 + admin surfaces)

Single index of the host-mounted WebChat v2 HTTP routes, so "what can the
browser / an operator call" is answerable in one place. Keep this current when a
route lands. Auth boundary and the descriptor source (which carries the
per-route body/rate-limit policy) are listed for each.

The canonical machine-readable descriptors live in code:
`ironclaw_webui_v2::webui_v2_routes()` (the core v2 facade) plus the
composition-owned `ProtectedRouteMount` / `PublicRouteMount` descriptor sets
(Slack channel routes, product-auth, SSO login, capability admin). This doc is
the human index over those.

## Core WebChat v2 facade (`ironclaw_webui_v2::webui_v2_routes()`)

Bearer-authenticated unless noted; see
`crates/ironclaw_reborn_composition/CLAUDE.md` → "Entrypoint inventory" for the
full v1→v2 mapping and per-route limits.

| Method | Path | Auth | Notes |
|---|---|---|---|
| POST | `/api/webchat/v2/threads` | Bearer | create thread |
| GET | `/api/webchat/v2/threads` | Bearer | list threads |
| DELETE | `/api/webchat/v2/threads/{thread_id}` | Bearer | delete thread |
| POST | `/api/webchat/v2/threads/{thread_id}/messages` | Bearer | send message |
| GET | `/api/webchat/v2/threads/{thread_id}/timeline` | Bearer | history |
| GET | `/api/webchat/v2/threads/{thread_id}/events` | Bearer or `?token=` | SSE stream (only route honoring `?token=`) |
| GET | `/api/webchat/v2/threads/{tid}/ws` | Bearer | WS stream |
| POST | `/api/webchat/v2/threads/{tid}/runs/{run_id}/cancel` | Bearer | cancel run |
| POST | `/api/webchat/v2/threads/{tid}/runs/{run_id}/gates/{gate_ref}/resolve` | Bearer | resolve gate |
| GET | `/api/webchat/v2/session` | Bearer | session/profile capabilities |
| GET\|POST | `/api/webchat/v2/extensions/*` | Bearer | extension registry/install/activate/remove/setup |
| `/api/webchat/v2/llm/*`, `/api/webchat/v2/operator/*` | Bearer + `operator_webui_config` | operator-only; mounted only when the authenticator opts in |

## Capability admin surface (#5268 — `capability_admin_routes.rs`)

Admin-gated by **`WebUiAuthenticatedCaller::is_admin()`** (the #5266
`UserRole::Admin`), *not* `operator_webui_config`. Mounted via
`WebuiServeConfig::with_protected_route_mount` when the `capability-policy`
feature is built in. Writes the #4544 scoped-lifecycle store the #5267 resolver
reads. Tenant-shared (`AdminShared`) availability only — per-user / config /
identity / approval is the `CapabilityPolicyDelta` surface (#5273, to come).

| Method | Path | Auth | Effect |
|---|---|---|---|
| GET | `/api/webchat/v2/admin/extensions` | Bearer + admin role | list the tenant's installed extensions |
| PUT | `/api/webchat/v2/admin/extensions/{package_id}` | Bearer + admin role | install a tenant-shared extension (optional `{config}` body) |
| DELETE | `/api/webchat/v2/admin/extensions/{package_id}` | Bearer + admin role | uninstall the tenant-shared extension |

Planned (with #5273 enforcement):

| Method | Path | Effect |
|---|---|---|
| PUT | `/api/webchat/v2/admin/users/{user_id}/capabilities/{capability_id}` | set a per-user delta (availability / config / identity-mode / approval) |
| GET | `/api/webchat/v2/admin/users/{user_id}/capabilities` | read a user's effective policy |

## Product-auth + SSO (host-supplied mounts)

| Method | Path | Auth | Source |
|---|---|---|---|
| POST | `/api/reborn/product-auth/oauth/start` | Bearer | product-auth mount |
| GET | `/api/reborn/product-auth/oauth/callback/{flow_id}` | Public (per-IP rate limit) | product-auth mount |
| POST | `/api/reborn/product-auth/manual-token/submit` | Bearer | product-auth mount |
| GET | `/auth/providers`, `/auth/login/{p}`, `/auth/callback/{p}`, `/auth/logout`, `/auth/session/exchange` | Public (login surface) | `ironclaw_reborn_webui_ingress::webui_v2_auth_router` |

## Local-dev auth (no HTTP surface — env config)

- `IRONCLAW_REBORN_WEBUI_TOKEN` + `IRONCLAW_REBORN_WEBUI_USER_ID` — operator
  env-bearer (operator WebUI config + SSO signing key + runtime owner).
- `IRONCLAW_REBORN_USER_TOKENS` — JSON `[{token,user_id,role}]` table layered
  over the operator (#5272), so one process serves several users.
- `IRONCLAW_REBORN_CAPABILITY_POLICY` — activate the per-(tenant,user)
  availability resolver (#5267); default off keeps local-dev `AllowAll`.
