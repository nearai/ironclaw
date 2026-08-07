# ironclaw_webui guardrails

The **WebUI host stack** for Reborn WebChat v2 — the single `products`-layer
crate, above `ironclaw_composition`, that turns composition's product/API
surface into a running HTTP server a browser can talk to. It owns four
subsystems that used to live apart (see `README.md` for the fold-in map):

1. **WebChat v2 route surface + SPA** (`src/webui_v2/`, from the former
   `ironclaw_webui_v2` crate) — the axum handlers over `ProductSurface`, the
   `webui_v2_routes()` descriptor table, the `WebUiV2HttpError` redacted wire
   shape, SSE/WebSocket streaming, and the Vite SPA bundle.
2. **Gateway assembly + middleware** (`src/webui_serve.rs`, `src/webui_*.rs`,
   from `ironclaw_composition::webui`) — `webui_v2_app(bundle, config)`
   composes the full `Router` and layers the fixed middleware stack; owns the
   `WebuiAuthenticator` / `WebuiAuthentication` host-auth vocabulary and the
   OpenAI-compat mounts (unconditional — this crate's only feature is `test-support`).
3. **Serve loop + host authentication** (`src/lib.rs`, `src/auth/`,
   `src/session.rs`, `src/oidc.rs`) — `serve_webui_v2` binds the listener and
   runs `axum::serve`; the `Env`/`Session`/`Oidc` authenticators, the
   signed-token session store, and the `/auth/*` OAuth login surface that mints
   sessions.
4. **Product-auth HTTP route serving** (`src/product_auth/`) — host-owned
   routes that parse/bound OAuth/manual-token/account/cleanup HTTP input and
   call `ironclaw_auth::RebornProductAuthServices`. Auth contracts and durable
   services stay in `ironclaw_auth`; composition wires the service bundle into
   this gateway.

Composition deliberately stops at the
`reborn_product_api_crates_do_not_bind_http_ingress` boundary — it returns a
fully composed `Router` but must never bind a socket. This crate is the
host-owned counterpart that binds the `TcpListener` and drives the serve loop.

The "Native host surface" rules of `docs/reborn/how-to-port-channel-to-reborn.md`
apply: host auth stays host-owned in this crate, and behavior is reached through
`ironclaw_product_contracts::surface::ProductSurface`. The crate *does* carry a
direct `ironclaw_assistant` dependency (see `Cargo.toml`), but as of the WS5
transport inversion it is limited to **the frozen operation inventory** — the
`*_VIEW` / `*_COMMAND` / `*_CAPABILITY` descriptor constants a handler names to
call the surface, which PROPOSAL §6.1.3 keeps in product — plus **nine** wire
DTOs whose fields name a crate `ironclaw_product_contracts` may not depend on.
(Corrected 2026-08-02: this read "eleven". The WS5 `attachments widened` slice in
the same PR moved `ProductAttachmentCapabilities`/`product_attachment_capabilities`
into `ironclaw_attachments`, taking the residue baseline **102 → 100**. The
authority is `WEBUI_PRODUCT_SYMBOL_BASELINE` in
`reborn_transport_product_boundary.rs`, not this prose.) Every
other DTO, request body, and descriptor *type* now comes from
`ironclaw_product_contracts`. Never behavior.

That residue is exact, enumerated with per-entry reasons, and shrink-only in
`ironclaw_architecture_tests` (`tests/reborn_transport_product_boundary.rs`, alongside
`tests/reborn_dependency_boundaries.rs`). **Adding an import from
`ironclaw_assistant` will fail that test** — put the type in
`ironclaw_product_contracts` instead. Moving the inventory constants there to
shrink the residue also fails it, deliberately: that is an unresolved §6.1.3 /
§6.9.4 owner decision, not a cleanup.

## Surface

### Route surface + gateway assembly

| Symbol | Role |
|---|---|
| `webui_v2_router(state)` / `webui_v2_router_with_options(state, opts)` | Build the WebChat v2 `axum::Router` from a `WebUiV2State`. |
| `webui_v2_routes() -> Vec<IngressRouteDescriptor>` | The route descriptor table (id, method, pattern, auth, rate/body limit, streaming). Locked by `tests/webui_v2_descriptors_contract.rs`. |
| `WebUiV2State` | Handler state: the `ProductSurface` facade + `SseCapacity` + route options. |
| `WebUiV2HttpError` / `WebUiV2HttpErrorBody` | The only path handlers return HTTP errors through — keeps the redacted-error vocabulary intact. |
| `webui_v2_app(product_surface, config) -> WebuiV2App` | Compose a host-supplied `ProductSurface` + `WebuiServeConfig` into the full middleware-wrapped `Router` (also `webui_v2_app_with_lifecycle`). |
| `WebuiServeConfig` | Host-owned serve config (tenant, authenticator, default agent/project, public/protected mounts, Google OAuth). |
| `WebuiAuthenticator` trait / `WebuiAuthentication` | Host-auth vocabulary the bearer middleware resolves each token through. |

Run and full-thread regression artifact exports are QA-only. Host composition
mounts their routes and exposes their browser affordances only when
`IRONCLAW_REBORN_REGRESSION_ARTIFACT_EXPORT=true`; the default is disabled.

Middleware modules (`src/webui_*.rs`) layer in a fixed order —
**ws-origin → per-route body limit → bearer auth → rate limit → handler** —
turning the `webui_v2_routes()` descriptors into tower layers.

### Serve loop + host authentication

| Symbol | Role |
|---|---|
| `serve_webui_v2(opts)` | Bind a `TcpListener` + run `axum::serve` with graceful shutdown |
| `RebornWebuiServeOptions` | Owner-supplied input (addr, router, shutdown receiver) |
| `EnvBearerAuthenticator` | Single-token `WebuiAuthenticator` for the standalone CLI; accepted tokens map to operator WebUI capabilities |
| `SignedTokenSessionStore` | HMAC-signed bearer mint/lookup with a bounded process-local logout denylist |
| `SessionAuthenticator` | `WebuiAuthenticator` that resolves bearer tokens through `SignedTokenSessionStore` |
| `OidcAuthenticator` | OIDC bearer-token verifier (JWKS + standard claims); accepted tokens map to non-operator WebUI capabilities |
| `webui_v2_auth_router(config) -> PublicRouteMount` | OAuth login router + route descriptors. The descriptors travel with the router so composition can fold them into the descriptor-driven per-route rate-limit / body-limit middleware — same machinery the v2 facade and product-auth callback already use, no side door. |
| `product_auth_route_mount(state) -> ProductAuthRouteMount` | Product-auth route router + descriptors for OAuth start/callback/status, manual-token setup/submit, account selection/recovery/refresh, and lifecycle cleanup. |
| `channel_pairing_route_mount(registry) -> ProtectedRouteMount` | Bearer-authed generic pairing routes for `WebGeneratedCode` channels (`/api/webchat/v2/extensions/{extension_id}/pairing/{mint,status,unpair}`). Moved here from `ironclaw_extension_host` (PROPOSAL §6.8.2 shed list, §6.9.4). **The pairing *service core* stays in `ironclaw_extension_host`** — this module is transport over it and holds no pairing semantics, which is why this crate names `ironclaw_extension_host` at all. Its three patterns are a separate mount and are **not** rows of the frozen `webui_v2/descriptors.rs` table. Composition hands out the *registry*, not the mount (it only dev-depends on this crate); the binary builds the mount. |
| `PublicRouteMount` | `{ router, descriptors }` pair handed to `WebuiServeConfig::with_public_route_mount` |
| `OAuthProvider` trait (in `auth/provider.rs`) | Extension point for per-provider URL / code-exchange logic. Deliberately lives in its own module so each provider does not depend on the others. `GoogleProvider` and `GitHubProvider` ship today. |
| `GoogleProvider` (in `auth/google.rs`) | Google OIDC provider (scopes `openid email profile`, PKCE S256, optional `hd` hosted-domain restriction). Built from `GoogleOAuthConfig`. |
| `GitHubProvider` (in `auth/github.rs`) | GitHub OAuth App provider (scopes `read:user user:email`, no PKCE, verified-email preference). Built from `GitHubOAuthConfig`. |
| `OAuthRouterConfig` | Tenant + `SignedTokenSessionStore` + `UserDirectory` + provider list + base URL |
| `UserDirectory` trait | Host-supplied mapping from `(provider, OAuthUserProfile)` to `UserId` |
| `EmailUserDirectory` | Standalone default impl (verified email → `UserId`); gated on `test-support` |

## `handlers.rs` module-charter map

`src/webui_v2/handlers.rs` is **4,593 lines** and carries a live
`// arch-exempt: large_file` waiver naming plan #5985 (the WebUI route split).
This map is **not** that split and does not discharge that waiver — PROPOSAL
§6.4.15 calls this shape "module-charter work, **not a split**", and §6.9.1
asks for a "module-charter map … the audited ≥11 sub-owners". It says which
concern a change belongs to *while the file is still one file*, so the eventual
split has a decided seam list instead of an argument.

**This table is enforced.** `tests/handlers_module_charter.rs` asserts every
top-level item in `handlers.rs` and its `handlers/` submodules appears in
exactly one row, that every name in a row still exists, and that no name is
claimed twice — so a new handler fails until it is given an owner and a deleted
one fails until its entry goes.

**Owners are conceptual, not positional.** A concern may hold more than one
region of the file (`threads` holds two, split by `admin-users`), because
making the regions contiguous would mean moving code, which is the split this
row explicitly is not. When the split does land, each row below is one
candidate module.

| Sub-owner | Owns | Never contains | Items |
|---|---|---|---|
| `session` | The session-bootstrap response and the feature flags it carries | A durable read — bootstrap must stay cheap and non-blocking | `GLOBAL_AUTO_APPROVE_FEATURE_TIMEOUT`, `WebUiV2SessionResponse`, `WebUiV2Features`, `get_session`, `global_auto_approve_enabled` |
| `threads` | Thread lifecycle, message send, and timeline/thread reads | Run control (that is `runs`) or transport (that is `streaming`) | `create_thread`, `delete_thread`, `send_message`, `get_timeline`, `TimelineQuery`, `list_threads`, `ListThreadsQuery` |
| `admin-users` | Admin user CRUD, role/status, and per-user secrets; parsing `{user_id}`/`{handle}` into domain types at the edge | Authorization logic — the service enforces admin authorization and last-admin protection | `parse_admin_user_id`, `parse_admin_secret_handle`, `read_admin_user_secret`, `admin_list_users`, `admin_create_user`, `admin_get_user`, `admin_update_user`, `admin_delete_user`, `admin_set_user_status`, `admin_set_user_role`, `admin_list_user_secrets`, `admin_put_user_secret`, `admin_delete_user_secret` |
| `workspace-fs` | Project-file and mount-catalog reads, and the workspace path-scoping rules that keep a served path inside its projection | Attachment download (that is `attachments`) | `PROJECT_FS_ROOT`, `ProjectFsQuery`, `list_project_files`, `stat_project_file`, `read_project_file`, `project_fs_download_response`, `FsBrowseQuery`, `list_fs_mounts`, `browse_fs_dir`, `stat_fs_path`, `read_fs_file`, `require_fs_browse_path`, `workspace_scoped_projection_required`, `workspace_projection_for`, `workspace_served_path`, `strip_workspace_prefix`, `project_fs_list_path`, `require_project_fs_path` |
| `projects` | Project CRUD and project membership | Project *files* — those are `workspace-fs` | `ListProjectsQuery`, `list_projects`, `create_project`, `get_project`, `update_project`, `delete_project`, `list_project_members`, `add_project_member`, `update_project_member`, `remove_project_member`, `read_project_member` |
| `attachments` | Attachment download and the filename sanitizing that download depends on | A filesystem path rule — that is `workspace-fs` | `MAX_DOWNLOAD_FILENAME_BYTES`, `sanitized_download_filename`, `get_attachment` |
| `streaming` | Both live transports and everything that shapes a frame: SSE poll/keepalive tuning, capacity and concurrency rejection, cursor tokens, the envelope→event mapping, and the WebSocket drain loop | A product decision — a stream carries what the surface already produced | `SSE_POLL_INTERVAL`, `SSE_IDLE_POLL_MAX_INTERVAL`, `SSE_KEEPALIVE_INTERVAL`, `LAST_EVENT_ID_HEADER`, `sse_poll_interval_for_idle_polls`, `stream_events`, `sse_capacity_rejected`, `sse_concurrency_exhausted`, `StreamEventsQuery`, `stream_connection_id`, `SseErrorPayload`, `webchat_sse_event_from_envelope`, `sse_error_event`, `sse_keep_alive_event`, `build_sse_stream`, `parse_cursor_token`, `cursor_token`, `stream_events_ws`, `ws_drain_loop`, `ws_send_with_timeout` |
| `runs` | Run control: cancel, retry, and gate resolution | Anything that reads a run — that is `threads` or `streaming` | `cancel_run`, `CancelRunPath`, `resolve_gate`, `ResolveGatePath`, `retry_run`, `RetryRunPath` |
| `commands` | The product command surface: listing and executing | A command *constant* — those are `ironclaw_assistant`'s frozen inventory | `list_commands`, `ExecuteCommandBody`, `execute_command` |
| `automations` | Automation listing and lifecycle (pause/resume/rename/delete) | Trigger evaluation — that is the triggers domain | `list_automations`, `pause_automation`, `resume_automation`, `rename_automation`, `delete_automation`, `ListAutomationsQuery` |
| `traces` | Trace credits, account traces, the account login link, and hold authorization | Trace *content* — that is `ironclaw_trace_commons` | `trace_credits`, `trace_account_traces`, `trace_account_login_link`, `authorize_trace_hold` |
| `outbound` | Outbound notification preferences, delivery targets, and the capability-failure→HTTP classification they introduced | Delivery itself — the host owns the coordinator | `get_outbound_preferences`, `set_outbound_preferences`, `CapabilityFailureHttpClass`, `capability_failure_http_class`, `capability_failure_bad_request`, `capability_resolution_succeeded`, `parse_thread_id_for_response`, `outbound_preferences_forbidden`, `outbound_preferences_unavailable`, `list_outbound_delivery_targets`, `outbound_preferences_activity_id` |
| `skills` | Skill discovery, install/update/remove, content reads, and auto-activation | Skill *selection* — that is `ironclaw_skills` | `list_skills`, `search_skills`, `install_skill`, `get_skill_content`, `update_skill`, `remove_skill`, `set_skill_auto_activate`, `set_auto_activate_learned`, `skill_mutation_succeeded`, `skill_mutation_forbidden`, `skill_mutation_unavailable`, `SkillPath`, `SearchSkillsBody`, `InstallSkillBody`, `UpdateSkillBody`, `SetSkillAutoActivateBody` |
| `extensions` | Extension listing, registry browse, install/import/remove, hosted-MCP registration, the setup handshake, and the lifecycle response projections | Admin *configuration* of an installed extension — that is `admin-config` | `list_extensions`, `list_extension_registry`, `install_extension`, `register_hosted_mcp_extension`, `import_extension`, `ironhub_deliver_install`, `remove_extension`, `extension_lifecycle_mutation_succeeded`, `extension_install_succeeded`, `membership_is_visible`, `membership_landed_pending_setup`, `ensure_extension_inventory_readback`, `extension_lifecycle_forbidden`, `extension_lifecycle_unavailable`, `extension_action_completed`, `get_extension_setup`, `setup_extension`, `public_lifecycle_json`, `extension_lifecycle_activity_id`, `ExtensionPackagePath`, `InstallExtensionBody`, `RegisterHostedMcpBody`, `RegisterHostedMcpResponse`, `bounded_hosted_mcp_name`, `RemoveExtensionBody`, `extension_package_ref_for_request` |
| `admin-config` | Per-extension admin configuration: read, replace, idempotency, and its failure projections | Extension lifecycle — that is `extensions` | `ADMIN_CONFIGURATION_IDEMPOTENCY_KEY_MAX_BYTES`, `require_operator_webui_config`, `ExtensionAdminConfigurationPath`, `ExtensionAdminConfigurationValue`, `ReplaceExtensionAdminConfigurationBody`, `ReplaceExtensionAdminConfigurationInput`, `list_extension_admin_configuration`, `replace_extension_admin_configuration`, `query_extension_admin_configuration`, `select_extension_admin_configuration_group`, `admin_configuration_activity_id`, `admin_configuration_conflict`, `admin_configuration_unavailable`, `admin_configuration_forbidden`, `admin_configuration_done_failure`, `admin_configuration_blocked` |
| `dispatch` | The shared `ProductSurface` call shapes every other owner goes through: invoke/query/page helpers, the generic activity-id derivation, and idempotency/client-action-id validation | A route-specific decision — those belong to the owner that made them | `CLIENT_ACTION_ID_MAX_BYTES`, `product_surface_input`, `invoke_product_capability`, `invoke_product_capability_with_activity_id`, `invoke_product_command`, `product_capability_activity_id`, `product_surface_activity_id`, `query_product_view`, `query_product_page`, `decode_product_outbound_events`, `validate_idempotency_key`, `parse_client_action_id` |
| `operator` | The operator console: first-run setup, tool settings, operator config keys, diagnostics, status, logs, and service lifecycle | LLM provider administration — that is `llm-admin` | `SETTINGS_TOOLS_AUTO_APPROVE_KEY`, `SETTINGS_TOOL_CONFIG_PREFIX`, `SETTINGS_TOOL_CAPABILITY_ID_MAX_BYTES`, `get_operator_setup`, `query_operator_setup_response`, `run_operator_setup`, `list_settings_tools`, `SettingsToolsAutoApproveRequest`, `set_settings_tools_auto_approve`, `SettingsToolPermissionPath`, `SettingsToolPermissionRequest`, `set_settings_tool_permission`, `validate_settings_tool_capability_id`, `validate_settings_tool_config_response`, `list_operator_config`, `OperatorConfigKeyPath`, `OPERATOR_CONFIG_KEY_MAX_BYTES`, `OPERATOR_CONFIG_RESERVED_VALIDATE_KEY`, `validate_operator_config_key`, `operator_config_key_error`, `query_operator_config_key_response`, `get_operator_config_key`, `set_operator_config_key`, `reject_reserved_operator_config_key`, `validate_operator_config`, `get_operator_diagnostics`, `get_operator_status`, `query_operator_logs`, `query_logs`, `run_operator_service_lifecycle` |
| `llm-admin` | LLM provider administration and the provider login flows: config snapshot, upsert/delete, active-model selection, connection test, model listing, NEAR AI and Codex login | Anything that *calls* a model | `LlmProviderPath`, `get_llm_config`, `query_llm_config_snapshot`, `upsert_llm_provider`, `delete_llm_provider`, `set_active_llm`, `test_llm_connection`, `list_llm_models`, `start_nearai_login`, `complete_nearai_wallet_login`, `start_codex_login`, `llm_provider_upsert_activity_id` |
| `run-artifact` | Run and thread artifact reads — already its own file, the one seam plan #5985 has taken so far | Anything not artifact-shaped | `handlers/run_artifact.rs::RunArtifactPath`, `handlers/run_artifact.rs::ThreadArtifactPath`, `handlers/run_artifact.rs::query_single`, `handlers/run_artifact.rs::get_run_artifact`, `handlers/run_artifact.rs::get_thread_artifact` |

Three placement calls worth stating, because each is an item whose *name*
suggests one owner and whose *use* is another:

- **`*_activity_id` helpers split three ways.** `product_capability_activity_id`
  and `product_surface_activity_id` are `dispatch` (they are the generic
  derivation every owner reaches). `extension_lifecycle_activity_id`,
  `llm_provider_upsert_activity_id`, `outbound_preferences_activity_id` and
  `admin_configuration_activity_id` are charged to the concern whose request
  shape they read, because each one knows that concern's fields.
- **`capability_failure_http_class` is `outbound`, not `dispatch`**, even though
  the name reads generic: it is the classification the outbound-preferences
  routes introduced and its callers are all in that owner. If a second concern
  starts calling it, it moves to `dispatch` — which is the trigger, stated in
  advance rather than argued later.
- **`get_attachment` is `attachments`, not `workspace-fs`.** Both serve bytes,
  but attachment identity is a thread-scoped ref, not a mount path, and the
  path-scoping rules in `workspace-fs` do not apply to it. Keeping them apart
  is what stops a future path-scoping fix from being assumed to cover
  attachment downloads.

## WebChat v2 route surface (folded from `ironclaw_webui_v2`)

Handlers consume only `ironclaw_product_contracts::surface::ProductSurface`. The bearer
middleware (in this crate's `webui_v2_app`) constructs the
`ProductSurfaceCaller`, carries the matched token's `WebUiV2Capabilities`,
and injects both as axum `Extension`s before the handler runs; handlers fail
closed (`500`) if that layer is missing (locked by
`missing_caller_extension_returns_500`).

| Route ID | Method | Pattern | Streaming | Effect path |
|---|---|---|---|---|
| `webui.v2.get_session` | GET | `/api/webchat/v2/session` | — | `ProjectionOnly` |
| `webui.v2.create_thread` | POST | `/api/webchat/v2/threads` | — | `ProductSurface` |
| `webui.v2.list_threads` | GET | `/api/webchat/v2/threads` (`?limit&cursor`) | — | `ProjectionOnly` |
| `webui.v2.delete_thread` | DELETE | `/api/webchat/v2/threads/{thread_id}` | — | `ProductSurface` |
| `webui.v2.send_message` | POST | `/api/webchat/v2/threads/{thread_id}/messages` | — | `TurnCoordinator` |
| `webui.v2.get_timeline` | GET | `/api/webchat/v2/threads/{thread_id}/timeline` (`?limit&cursor`) | — | `ProjectionOnly` |
| `webui.v2.get_run_artifact` | GET | `/api/webchat/v2/threads/{thread_id}/runs/{run_id}/artifact` | — | `ProjectionOnly` |
| `webui.v2.get_thread_artifact` | GET | `/api/webchat/v2/threads/{thread_id}/artifact` | — | `ProjectionOnly` |
| `webui.v2.logs` | GET | `/api/webchat/v2/logs` | — | `ProjectionOnly` |
| `webui.v2.stream_events` | GET | `/api/webchat/v2/threads/{thread_id}/events` | **SSE** | `ProjectionOnly` |
| `webui.v2.stream_events_ws` | GET | `/api/webchat/v2/threads/{thread_id}/ws` | **WebSocket** | `ProjectionOnly` |
| `webui.v2.cancel_run` / `retry_run` / `resolve_gate` | POST | `…/runs/{run_id}/…` | — | `TurnCoordinator` |
| `webui.v2.list/pause/resume/rename/delete_automation` | GET/POST/DELETE | `/api/webchat/v2/automations…` | — | `ProductSurface` |
| `webui.v2.list/install/import/remove/get_setup/setup_extension/register_hosted_mcp` | GET/POST | `/api/webchat/v2/extensions…` | — | `ProjectionOnly` / `ProductSurface` |
| `webui.v2.ironhub_deliver_install` | POST | `/api/webchat/v2/ironhub/install` | — | `ProductSurface` |
| `webui.v2.*_llm_*` | GET/POST | `/api/webchat/v2/llm/…` | — | `ProjectionOnly` / `ProductSurface` |
| `webui.v2.settings.list_tools` / `set_tools_auto_approve` / `set_tool_permission` | GET/POST | `/api/webchat/v2/settings/tools…` | — | `ProjectionOnly` / `ProductSurface` |
| `webui.v2.operator.*` (setup, config, config/{key}, validate, diagnostics, status, logs, service) | GET/POST | `/api/webchat/v2/operator/…` | — | `ProjectionOnly` / `ProductSurface` |
| `webui.v2.operator.inspector_*` | GET | `/api/webchat/v2/operator/inspector/threads/{thread_id}/runs/{run_id}[/prompt\|/tools/{activity_id}\|/events]` | `events`: SSE; others — | `ProjectionOnly` |
| `webui.v2.admin.*` (users CRUD, status, role, secrets) | GET/POST/PATCH/PUT/DELETE | `/api/webchat/v2/admin/users…` | — | `ProductSurface` |
| `webui.v2.trace_*` (credit, account, account-login-link, holds/authorize) | GET/POST | `/api/webchat/v2/traces/…` | — | `ProductSurface` |

The exact per-route set (methods, query params, auth, rate/body limits) is the
descriptor table in `src/webui_v2/descriptors.rs`; the count/shape is locked by
`tests/webui_v2_descriptors_contract.rs`. Add a route → add a handler **and** a
`webui_v2_routes()` entry, or that test fails.

`webui.v2.get_run_artifact` exports one exact caller-owned run as the versioned
`ironclaw.run_artifact.v1` evidence schema. The facade authorizes the thread
from authenticated tenant/user scope before selecting records by `turn_run_id`,
reconstructs provider tool-call metadata through the model-context read path,
and applies deterministic trace redaction before serialization. Its logs are a
bounded process-local diagnostic sidecar: `logs.complete` is always false and
availability/truncation are explicit. Deployment-wide logs are not exposed
through this caller route.

`webui.v2.get_thread_artifact` applies the same caller ownership and redaction
rules to every replayable message in the thread and queries logs at thread
scope. Its `ironclaw.thread_artifact.v1` messages retain `run_id`, allowing the
fixture importer to reconstruct multiple turns without mixing threads. Export
is all-or-nothing and returns `413` when the thread exceeds 1,000 persisted
messages, 16 MiB of stored message data, or 20 MiB after redaction and log
assembly. The endpoint is limited to six requests per caller per minute.

**Operator-gating.** LLM config, operator setup/config/service-control, and
extension zip-import routes are operator-wide: `webui_v2_app` mounts them only
when the authenticator advertises an operator config surface, and each handler
still rejects with `403` when the injected `WebUiV2Capabilities` lacks
`operator_webui_config`. Multi-user session/OIDC authenticators return
non-operator capabilities. `webui.v2.admin.*` user management is
admin/operator-gated server-side in `ProductSurface` (`AdminUserService`,
last-admin protection); `create_user` returns the one-time API bearer exactly
once in `api_token`. `webui.v2.settings.tools` is a normal authenticated-caller
route (tenant/user-scoped tool-approval settings), not an operator route.

### Streaming model (SSE + WebSocket)

- `stream_events` (SSE) and `stream_events_ws` (WebSocket) render each
  `ProductOutboundEnvelope` into the redacted `WebChatV2EventFrame` schema
  (never raw adapter routing/delivery metadata) with the projection cursor as
  the SSE `id`; the browser resumes via `Last-Event-ID` (preferred over
  `?after_cursor=`).
- Both transports share **one** `SseCapacity` budget keyed by `(tenant, user)`
  (default 3 concurrent; override via `WebUiV2State::with_sse_concurrency_limit`)
  — a caller cannot bypass the cap by mixing SSE and WS. Exhaustion returns
  `429` with `retryable: true`.
- The SPA consumes SSE through `event-source-plus`, which owns event framing,
  `Last-Event-ID`, abort, and retry/backoff over `fetch`/`ReadableStream`. The
  bearer is sent in the `Authorization` header rather than the request URL. A
  bounded, random `connection_id` remains stable for one browser tab across SPA
  mounts and document reloads, while `connection_generation` increments for
  every package-managed request. Fresh top-level navigations use a new identity
  even when a duplicated tab copied `sessionStorage`. A same-caller, same-id
  stream supersedes its prior generation without consuming another slot; a
  delayed older generation receives `204` and cannot cancel the current stream.
  This prevents proxy-reordered closes/opens during thread navigation or reload
  from stranding the replacement stream behind the cap; distinct tabs still
  consume distinct slots.
- A successful facade subscription emits an application-level `keep_alive`
  frame immediately after admission and every 15 seconds while the projection
  is idle. Browser connection state and its activity watchdog use those frames
  as liveness proof; Axum's comment-only transport keep-alive still protects
  proxies, but SSE parser packages do not expose comments to application code.
- Subscription-capable product surfaces keep one projection subscription alive
  for the entire SSE connection. Do not rebuild a one-event subscription after
  each frame: model/tool milestones emitted between teardown and resubscribe
  are not guaranteed to remain in the compacted live state. The product bridge
  revalidates thread visibility on an independent bounded cadence so storage
  I/O never gates individual text frames; drain/poll remains only for
  compatibility surfaces without subscriptions.
- Live assistant text is cumulative within one model call and keyed by both
  turn run and model-call phase. A later model call therefore starts a new
  assistant item instead of replacing an earlier utterance from the same run.
  The SPA marks the prior phase as no longer streaming, retains it as
  intermediate text, and upgrades only the latest phase when the durable final
  reply arrives. These phase items remain live-projection/session state rather
  than durable transcript records.
- Active assistant phases render accumulated Markdown through Streamdown's
  incomplete-Markdown-aware streaming mode. The product projection boundary
  publishes cumulative text at most once per 16 ms browser-paint interval,
  keeping only the latest replaceable snapshot inside that interval. Do not
  raise the projection subscription buffer or restore the old 75 ms window:
  the former only postpones lag under provider microbursts, while the latter
  makes ordinary text visibly chunky. Completed phases continue through the
  existing marked + DOMPurify renderer and code-block enhancement path.
- The packaged SSE client retains `Last-Event-ID` only within one mounted Chat
  route, including retries and visibility recovery. A route/thread remount
  starts at the projection origin so the server returns durable state plus the
  compacted current live state; it does not persist process-local live cursors
  across SPA navigation.
- Every stream is closed after a max lifetime (5 min) and every `socket.send` /
  subscription/drain await is `timeout`-bounded, so a back-pressuring client or
  a stalled facade cannot pin a slot past the budget. Slots are RAII
  (`SseSlot`), released on disconnect / expiry / error. Regressions locked by
  `stream_events_ws_shares_capacity_with_sse_streams` and
  `stream_events_releases_slot_when_facade_drain_stalls_past_max_lifetime`.
- `capability_activity` / `capability_display_preview` frames carry only
  bounded, secret-redacted input/output *summaries* (host paths rejected, URLs
  stripped, byte-bounded) — never raw args/results. Full output stays behind the
  scoped `result_ref` fetch path. See `.claude/rules/gateway-events.md`.

### SPA bundle

The Vite/TypeScript frontend under `frontend/` is compiled by `build.rs` into
Cargo's `OUT_DIR` and served from `src/webui_v2/static_assets/`.
`Dockerfile.reborn` installs `frontend/` deps before the `cargo build` so the
release image bundles compiled assets; `frontend/README.md` covers the JS
toolchain.

## Why the OAuth login router lives here

The crate already owns `WebuiAuthenticator` impls, `SignedTokenSessionStore`,
and the session lifecycle types. The OAuth callback's job is exactly that
— turn a provider profile into a signed session `create_session` call
— so the login mint path belongs in the same host-owned crate, not
behind the product/API seam in `ironclaw_composition`.

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
secrets, never speaks to Google, never parses a signed session token.

## WebChat v2 OAuth login surface (#4116)

Routes mounted by `webui_v2_auth_router`:

- `GET  /auth/providers` — list configured provider names.
- `GET  /auth/login/{provider}` — redirect non-canonical hosts to
  the configured `base_url`, then mint a pending flow (CSRF state +
  PKCE verifier + sanitized `redirect_after`) and redirect the
  browser to the provider's authorization URL.
- `GET  /auth/callback/{provider}` — single-use state lookup,
  cross-provider replay guard, code exchange via the matching
  `OAuthProvider`, user resolution via `UserDirectory`, session
  mint via `SignedTokenSessionStore`, and redirect to
  `{redirect_after}?login_ticket=<ticket>` (default `/`). The
  ticket is short-lived and single-use; the SPA redeems it over
  same-origin JSON so the bearer never appears in a redirect
  `Location` header.
- `POST /auth/session/exchange` — consume the one-time login ticket
  and return `{ token }`.
- `POST /auth/logout` — bearer-protected; calls
  `SignedTokenSessionStore::revoke` and returns `204` with or without
  a bearer, so the SPA's local clear stays unconditional.

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
  The signed session store + `UserDirectory` + composition seam stay the
  same.

### Security invariants

- **Pending-flow store** is process-local, bounded (1024 entries +
  5-min TTL), and single-use on `take`. A replayed callback cannot
  re-use a state token; cross-provider replay (state minted for
  Google arriving on the GitHub callback) fails closed.
- **Canonical login host** is the configured `base_url`. Login
  requests received on any other `Host` redirect to that base URL
  before a pending-flow entry is created, so preview/custom domains
  cannot mint state that the registered provider callback host will
  never see.
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
  `/?login_error=<code>` where `<code>` is an opaque enum
  (`invalid_state`, `provider_mismatch`, `denied`,
  `unauthorized`, `exchange_failed`, `server_error`,
  `invalid_request`). Provider error bodies, JWT decode messages,
  and signed-session errors are logged via `tracing` and never
  echoed back to the client.
- **Session transport** is one-time login ticket in the callback
  redirect (`?login_ticket=<ticket>`) followed by same-origin
  exchange for the bearer — see
  `ironclaw_composition/CONTRACT.md` → "Session transport
  decision" for the rationale.

### What the SSO router deliberately does NOT do

- No cookie writes (the SPA stores the exchanged bearer in
  `sessionStorage`).
- No DB schema. `UserDirectory` is host-supplied; the crate ships
  only the standalone `EmailUserDirectory`.
- No retry / refresh-token handling. The callback is one-shot:
  exchange code, mint session, done. Token refresh is the host's
  job if it wants it.
- No v1 `/auth/*` reuse. The crate has zero `src/`-tier dependency
  by contract; that constraint is what lets WebChat v2 declare a
  hard non-goal on v1 routes (issue #3886).

## Test layout

**Route surface + gateway assembly** (folded from `ironclaw_webui_v2` +
composition):

- `tests/webui_v2_descriptors_contract.rs` — locks the descriptor table
  (count / methods / patterns / auth / rate limits / SSE).
- `tests/webui_v2_handlers_contract.rs` — drives a real axum router from
  `webui_v2_router` against a stub `ProductSurface` (test-through-the-caller).
- `tests/webui_v2_schema_contract.rs`, `tests/webui_v2_operator_config_key_contract.rs`,
  `tests/webui_v2_operator_route_predicate_contract.rs` — wire schema + operator
  gating.
- `tests/headers_errors_contract.rs`, `tests/network_limits_contract.rs`,
  `tests/auth_route_contract.rs` — middleware stack (security headers, body/rate
  limits, bearer auth) over the composed `webui_v2_app`.
- `tests/serve_loop.rs` — listener bind + graceful shutdown.

**Host authentication:**

- `src/auth/` module tests, plus the `mod tests` blocks in `src/oidc.rs` and `src/session.rs` (those two are files, not directories)
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

## Validation

```bash
cargo test -p ironclaw_webui --all-features
cargo clippy -p ironclaw_webui --all-features --all-targets -- -D warnings
# Production shape: default features, no dev-dependencies. `test-support`
# gates public items, so the #7119 unused-import class only shows here
# (mirrors the merge-queue `--lib --bins` lane).
cargo clippy -p ironclaw_webui -- -D warnings
```
