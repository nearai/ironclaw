//! Axum route composition and server bootstrap.
//!
//! This module owns the wiring between the platform layer and the feature
//! handlers: `start_server` binds the TCP listener, assembles the four
//! routers (`public`, `protected`, `statics`, `projects`), and applies the
//! cross-cutting layers (CORS, body-size limit, panic catch, static
//! security headers, CSP).
//!
//! Per ironclaw#2599: route composition is the single coupling point
//! where platform meets features. Handlers themselves live in either
//! `features/<slice>/` (migrated) or the transitional `handlers/*.rs`
//! flat folder (not yet sliced). No feature handler lives in
//! `server.rs` — that file is a backward-compat re-export shim for
//! external callers and is scheduled for deletion in stage 6. The
//! router module depends on both; handlers must not depend on the
//! router.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::header,
    middleware,
    routing::{get, post, put},
};
use tokio::sync::oneshot;
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::channels::web::auth::{CombinedAuthState, auth_middleware};
use crate::channels::web::features::jobs::{
    job_files_list_handler, job_files_read_handler, jobs_cancel_handler, jobs_detail_handler,
    jobs_events_handler, jobs_list_handler, jobs_prompt_handler, jobs_restart_handler,
    jobs_summary_handler,
};
use crate::channels::web::handlers::engine::{
    engine_mission_detail_handler, engine_mission_fire_handler, engine_mission_pause_handler,
    engine_mission_resume_handler, engine_missions_handler, engine_missions_summary_handler,
    engine_project_detail_handler, engine_projects_handler, engine_projects_overview_handler,
    engine_thread_detail_handler, engine_thread_events_handler, engine_thread_steps_handler,
    engine_threads_handler,
};
use crate::channels::web::handlers::frontend::{
    frontend_layout_handler, frontend_layout_update_handler, frontend_widget_file_handler,
    frontend_widgets_handler,
};
use crate::channels::web::handlers::llm::{
    llm_list_models_handler, llm_providers_handler, llm_test_connection_handler,
};
use crate::channels::web::handlers::memory::{
    memory_list_handler, memory_read_handler, memory_search_handler, memory_tree_handler,
    memory_write_handler,
};
use crate::channels::web::handlers::secrets::{
    user_secrets_delete_handler, user_secrets_import_handler, user_secrets_list_handler,
    user_secrets_put_handler,
};
use crate::channels::web::handlers::skills::{
    skills_install_handler, skills_list_handler, skills_remove_handler, skills_search_handler,
};
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::platform::static_files::{
    BASE_CSP_HEADER, admin_css_handler, admin_html_handler, admin_js_handler, css_handler,
    debug_init_handler, debug_panel_css_handler, debug_panel_js_handler, favicon_handler,
    health_handler, i18n_app_handler, i18n_en_handler, i18n_index_handler, i18n_ko_handler,
    i18n_zh_handler, index_handler, js_handler, project_file_handler, project_index_handler,
    project_redirect_handler, theme_css_handler, theme_init_handler,
};

// Feature slices under `features/<slice>/`. As of ironclaw#2599 stage 4d,
// every route composed below comes from one of these or from a
// transitional `handlers/*.rs` file (auth, engine, frontend, llm,
// memory, secrets, skills, system_prompt, tokens, tool_policy, users,
// webhooks). No feature handler lives in `server.rs` — that file is a
// backward-compat re-export shim waiting on stage 6 deletion.
use crate::channels::web::features::chat::{
    chat_approval_handler, chat_auth_cancel_handler, chat_auth_token_handler, chat_events_handler,
    chat_gate_resolve_handler, chat_history_handler, chat_new_thread_handler, chat_send_handler,
    chat_threads_handler, chat_ws_handler,
};
use crate::channels::web::features::extensions::{
    extensions_activate_handler, extensions_install_handler, extensions_list_handler,
    extensions_readiness_handler, extensions_registry_handler, extensions_remove_handler,
    extensions_setup_handler, extensions_setup_submit_handler, extensions_tools_handler,
};
use crate::channels::web::features::logs::{
    logs_events_handler, logs_level_get_handler, logs_level_set_handler,
};
use crate::channels::web::features::oauth::{
    oauth_callback_handler, relay_events_handler, slack_relay_oauth_callback_handler,
};
use crate::channels::web::features::pairing::{pairing_approve_handler, pairing_list_handler};
use crate::channels::web::features::routines::{
    routines_delete_handler, routines_detail_handler, routines_list_handler, routines_runs_handler,
    routines_summary_handler, routines_toggle_handler, routines_trigger_handler,
};
use crate::channels::web::features::settings::{
    settings_delete_handler, settings_export_handler, settings_get_handler,
    settings_import_handler, settings_list_handler, settings_set_handler,
    settings_tools_list_handler, settings_tools_set_handler,
};
use crate::channels::web::features::status::gateway_status_handler;

/// Start the gateway HTTP server.
///
/// Returns the actual bound `SocketAddr` (useful when binding to port 0).
pub async fn start_server(
    addr: SocketAddr,
    state: Arc<GatewayState>,
    auth: CombinedAuthState,
) -> Result<SocketAddr, crate::error::ChannelError> {
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!("Failed to bind to {}: {}", addr, e),
        }
    })?;
    let bound_addr =
        listener
            .local_addr()
            .map_err(|e| crate::error::ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!("Failed to get local addr: {}", e),
            })?;

    // Public routes (no auth)
    let public = Router::new()
        .route("/api/health", get(health_handler))
        .route("/oauth/callback", get(oauth_callback_handler))
        .route(
            "/oauth/slack/callback",
            get(slack_relay_oauth_callback_handler),
        )
        .route("/relay/events", post(relay_events_handler))
        .route(
            "/api/webhooks/{path}",
            post(crate::channels::web::handlers::webhooks::webhook_trigger_handler),
        )
        // User-scoped webhook endpoint for multi-tenant isolation
        .route(
            "/api/webhooks/u/{user_id}/{path}",
            post(crate::channels::web::handlers::webhooks::webhook_trigger_user_scoped_handler),
        )
        // OAuth social login routes (public, no auth required)
        .route(
            "/auth/providers",
            get(crate::channels::web::handlers::auth::providers_handler),
        )
        .route(
            "/auth/login/{provider}",
            get(crate::channels::web::handlers::auth::login_handler),
        )
        .route(
            "/auth/callback/{provider}",
            get(crate::channels::web::handlers::auth::callback_handler)
                .post(crate::channels::web::handlers::auth::callback_post_handler),
        )
        .route(
            "/auth/logout",
            post(crate::channels::web::handlers::auth::logout_handler),
        )
        // NEAR wallet auth (challenge-response, not OAuth redirect)
        .route(
            "/auth/near/challenge",
            get(crate::channels::web::handlers::auth::near_challenge_handler),
        )
        .route(
            "/auth/near/verify",
            post(crate::channels::web::handlers::auth::near_verify_handler),
        );

    // Protected routes (require auth)
    let auth_state = auth;
    let protected = Router::new()
        // Chat
        .route("/api/chat/send", post(chat_send_handler))
        .route("/api/chat/gate/resolve", post(chat_gate_resolve_handler))
        .route("/api/chat/auth-token", post(chat_auth_token_handler))
        .route("/api/chat/auth-cancel", post(chat_auth_cancel_handler))
        .route("/api/chat/approval", post(chat_approval_handler))
        .route("/api/chat/events", get(chat_events_handler))
        .route("/api/chat/ws", get(chat_ws_handler))
        .route("/api/chat/history", get(chat_history_handler))
        .route("/api/chat/threads", get(chat_threads_handler))
        .route("/api/chat/thread/new", post(chat_new_thread_handler))
        // Memory
        .route("/api/memory/tree", get(memory_tree_handler))
        .route("/api/memory/list", get(memory_list_handler))
        .route("/api/memory/read", get(memory_read_handler))
        .route("/api/memory/write", post(memory_write_handler))
        .route("/api/memory/search", post(memory_search_handler))
        // Jobs
        .route("/api/jobs", get(jobs_list_handler))
        .route("/api/jobs/summary", get(jobs_summary_handler))
        .route("/api/jobs/{id}", get(jobs_detail_handler))
        .route("/api/jobs/{id}/cancel", post(jobs_cancel_handler))
        .route("/api/jobs/{id}/restart", post(jobs_restart_handler))
        .route("/api/jobs/{id}/prompt", post(jobs_prompt_handler))
        .route("/api/jobs/{id}/events", get(jobs_events_handler))
        .route("/api/jobs/{id}/files/list", get(job_files_list_handler))
        .route("/api/jobs/{id}/files/read", get(job_files_read_handler))
        // Logs
        .route("/api/logs/events", get(logs_events_handler))
        .route("/api/logs/level", get(logs_level_get_handler))
        .route(
            "/api/logs/level",
            axum::routing::put(logs_level_set_handler),
        )
        // Extensions
        .route("/api/extensions", get(extensions_list_handler))
        .route(
            "/api/extensions/readiness",
            get(extensions_readiness_handler),
        )
        .route("/api/extensions/tools", get(extensions_tools_handler))
        .route("/api/extensions/registry", get(extensions_registry_handler))
        .route("/api/extensions/install", post(extensions_install_handler))
        .route(
            "/api/extensions/{name}/activate",
            post(extensions_activate_handler),
        )
        .route(
            "/api/extensions/{name}/remove",
            post(extensions_remove_handler),
        )
        .route(
            "/api/extensions/{name}/setup",
            get(extensions_setup_handler).post(extensions_setup_submit_handler),
        )
        // Pairing
        .route("/api/pairing/{channel}", get(pairing_list_handler))
        .route(
            "/api/pairing/{channel}/approve",
            post(pairing_approve_handler),
        )
        // Routines
        .route("/api/routines", get(routines_list_handler))
        .route("/api/routines/summary", get(routines_summary_handler))
        .route("/api/routines/{id}", get(routines_detail_handler))
        .route("/api/routines/{id}/trigger", post(routines_trigger_handler))
        .route("/api/routines/{id}/toggle", post(routines_toggle_handler))
        .route(
            "/api/routines/{id}",
            axum::routing::delete(routines_delete_handler),
        )
        .route("/api/routines/{id}/runs", get(routines_runs_handler))
        // Engine v2
        .route("/api/engine/threads", get(engine_threads_handler))
        .route(
            "/api/engine/threads/{id}",
            get(engine_thread_detail_handler),
        )
        .route(
            "/api/engine/threads/{id}/steps",
            get(engine_thread_steps_handler),
        )
        .route(
            "/api/engine/threads/{id}/events",
            get(engine_thread_events_handler),
        )
        .route("/api/engine/projects", get(engine_projects_handler))
        .route(
            "/api/engine/projects/overview",
            get(engine_projects_overview_handler),
        )
        .route(
            "/api/engine/projects/{id}",
            get(engine_project_detail_handler),
        )
        .route(
            "/api/engine/projects/{id}/widgets",
            get(crate::channels::web::handlers::frontend::project_widgets_handler),
        )
        .route("/api/engine/missions", get(engine_missions_handler))
        .route(
            "/api/engine/missions/summary",
            get(engine_missions_summary_handler),
        )
        .route(
            "/api/engine/missions/{id}",
            get(engine_mission_detail_handler),
        )
        .route(
            "/api/engine/missions/{id}/fire",
            post(engine_mission_fire_handler),
        )
        .route(
            "/api/engine/missions/{id}/pause",
            post(engine_mission_pause_handler),
        )
        .route(
            "/api/engine/missions/{id}/resume",
            post(engine_mission_resume_handler),
        )
        // Skills
        .route("/api/skills", get(skills_list_handler))
        .route("/api/skills/search", post(skills_search_handler))
        .route("/api/skills/install", post(skills_install_handler))
        .route(
            "/api/skills/{name}",
            axum::routing::delete(skills_remove_handler),
        )
        // Settings
        .route("/api/settings", get(settings_list_handler))
        .route("/api/settings/export", get(settings_export_handler))
        .route("/api/settings/import", post(settings_import_handler))
        // User self-service secrets
        .route("/api/secrets", get(user_secrets_list_handler))
        .route("/api/secrets/import", post(user_secrets_import_handler))
        .route(
            "/api/secrets/{name}",
            put(user_secrets_put_handler).delete(user_secrets_delete_handler),
        )
        .route(
            "/api/secrets/{name}/approvals/revoke",
            post(crate::channels::web::handlers::secrets::user_secret_approval_revoke_handler),
        )
        // NOTE: These static routes intentionally shadow `/api/settings/{key}` when
        // key="tools". Axum resolves static routes before parameterized ones, so this
        // works correctly. Avoid adding a setting named literally "tools".
        .route("/api/settings/tools", get(settings_tools_list_handler))
        .route(
            "/api/settings/tools/{name}",
            axum::routing::put(settings_tools_set_handler),
        )
        .route("/api/settings/{key}", get(settings_get_handler))
        .route(
            "/api/settings/{key}",
            axum::routing::put(settings_set_handler),
        )
        .route(
            "/api/settings/{key}",
            axum::routing::delete(settings_delete_handler),
        )
        // LLM utilities
        .route(
            "/api/llm/test_connection",
            post(llm_test_connection_handler),
        )
        .route("/api/llm/list_models", post(llm_list_models_handler))
        .route("/api/llm/providers", get(llm_providers_handler))
        // User management (admin)
        .route(
            "/api/admin/users",
            get(crate::channels::web::handlers::users::users_list_handler)
                .post(crate::channels::web::handlers::users::users_create_handler),
        )
        .route(
            "/api/admin/users/{id}",
            get(crate::channels::web::handlers::users::users_detail_handler)
                .patch(crate::channels::web::handlers::users::users_update_handler)
                .delete(crate::channels::web::handlers::users::users_delete_handler),
        )
        .route(
            "/api/admin/users/{id}/suspend",
            post(crate::channels::web::handlers::users::users_suspend_handler),
        )
        .route(
            "/api/admin/users/{id}/activate",
            post(crate::channels::web::handlers::users::users_activate_handler),
        )
        // Admin secrets provisioning (per-user)
        .route(
            "/api/admin/users/{user_id}/secrets",
            get(crate::channels::web::handlers::secrets::secrets_list_handler),
        )
        .route(
            "/api/admin/users/{user_id}/secrets/{name}",
            put(crate::channels::web::handlers::secrets::secrets_put_handler)
                .delete(crate::channels::web::handlers::secrets::secrets_delete_handler),
        )
        // Admin tool policy
        .route(
            "/api/admin/tool-policy",
            get(crate::channels::web::handlers::tool_policy::tool_policy_get_handler)
                .put(crate::channels::web::handlers::tool_policy::tool_policy_put_handler),
        )
        // Admin system prompt — tighter body cap than the global 10 MB so an
        // oversized payload is rejected before being parsed into memory.
        .route(
            "/api/admin/system-prompt",
            get(crate::channels::web::handlers::system_prompt::get_handler)
                .put(crate::channels::web::handlers::system_prompt::put_handler)
                .layer(DefaultBodyLimit::max(128 * 1024)),
        )
        // Usage reporting (admin)
        .route(
            "/api/admin/usage",
            get(crate::channels::web::handlers::users::usage_stats_handler),
        )
        .route(
            "/api/admin/usage/summary",
            get(crate::channels::web::handlers::users::usage_summary_handler),
        )
        // User self-service profile
        .route(
            "/api/profile",
            get(crate::channels::web::handlers::users::profile_get_handler)
                .patch(crate::channels::web::handlers::users::profile_update_handler),
        )
        // Token management
        .route(
            "/api/tokens",
            get(crate::channels::web::handlers::tokens::tokens_list_handler)
                .post(crate::channels::web::handlers::tokens::tokens_create_handler),
        )
        .route(
            "/api/tokens/{id}",
            axum::routing::delete(crate::channels::web::handlers::tokens::tokens_revoke_handler),
        )
        // Frontend extension API
        .route(
            "/api/frontend/layout",
            get(frontend_layout_handler).put(frontend_layout_update_handler),
        )
        .route("/api/frontend/widgets", get(frontend_widgets_handler))
        .route(
            "/api/frontend/widget/{id}/{*file}",
            get(frontend_widget_file_handler),
        )
        // Gateway control plane
        .route("/api/gateway/status", get(gateway_status_handler))
        // Debug inspection (admin-only — handler enforces via AdminUser extractor)
        .route(
            "/api/debug/prompt",
            get(crate::channels::web::features::debug::debug_prompt_handler),
        )
        // OpenAI-compatible API
        .route(
            "/v1/chat/completions",
            post(crate::channels::web::openai_compat::chat_completions_handler),
        )
        .route(
            "/v1/models",
            get(crate::channels::web::openai_compat::models_handler),
        )
        // OpenAI Responses API (routes through the full agent loop)
        .route(
            "/v1/responses",
            post(crate::channels::web::responses_api::create_response_handler),
        )
        .route(
            "/v1/responses/{id}",
            get(crate::channels::web::responses_api::get_response_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ));

    // Static file routes (no auth, served from embedded strings)
    let statics = Router::new()
        .route("/", get(index_handler))
        .route("/theme.css", get(theme_css_handler))
        .route("/style.css", get(css_handler))
        .route("/app.js", get(js_handler))
        .route("/theme-init.js", get(theme_init_handler))
        .route("/debug-init.js", get(debug_init_handler))
        .route("/debug-panel.js", get(debug_panel_js_handler))
        .route("/debug-panel.css", get(debug_panel_css_handler))
        .route("/favicon.ico", get(favicon_handler))
        .route("/i18n/index.js", get(i18n_index_handler))
        .route("/i18n/en.js", get(i18n_en_handler))
        .route("/i18n/zh-CN.js", get(i18n_zh_handler))
        .route("/i18n/ko.js", get(i18n_ko_handler))
        .route("/i18n-app.js", get(i18n_app_handler))
        // Admin panel SPA (auth handled client-side + API layer)
        .route("/admin", get(admin_html_handler))
        .route("/admin/", get(admin_html_handler))
        .route("/admin/{*path}", get(admin_html_handler))
        .route("/admin.css", get(admin_css_handler))
        .route("/admin.js", get(admin_js_handler));

    // Project file serving (behind auth to prevent unauthorized file access).
    let projects = Router::new()
        .route("/projects/{project_id}", get(project_redirect_handler))
        .route("/projects/{project_id}/", get(project_index_handler))
        .route("/projects/{project_id}/{*path}", get(project_file_handler))
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ));

    // CORS: restrict to same-origin by default. Only localhost/127.0.0.1
    // origins are allowed, since the gateway is a local-first service.
    //
    // `SocketAddr`'s `Display` handles IPv6 bracketing correctly
    // (`[::1]:8080` rather than `::1:8080`), so building the origin off the
    // whole `addr` avoids a broken URL on v6 binds. Parse errors here would
    // mean the `SocketAddr` itself produced an invalid HTTP origin — a
    // startup bug, not a request-time error — so we fail the bootstrap
    // with `ChannelError::StartupFailed` rather than panic.
    let ip_origin = format!("http://{addr}").parse().map_err(|e| {
        crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!("Invalid CORS origin for bound addr {addr}: {e}"),
        }
    })?;
    let localhost_origin = format!("http://localhost:{}", addr.port())
        .parse()
        .map_err(|e| crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!("Invalid CORS origin for localhost:{}: {e}", addr.port()),
        })?;
    let cors = CorsLayer::new()
        .allow_origin([ip_origin, localhost_origin])
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
        ])
        .allow_headers(AllowHeaders::list([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
        ]))
        .allow_credentials(true);

    let app = Router::new()
        .merge(public)
        .merge(statics)
        .merge(projects)
        .merge(protected)
        .layer(DefaultBodyLimit::max(14 * 1024 * 1024)) // 14 MiB request body to cover 10 MiB decoded attachments plus base64/JSON overhead
        .layer(tower_http::catch_panic::CatchPanicLayer::custom(
            |panic_info: Box<dyn std::any::Any + Send + 'static>| {
                let detail = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    (*s).to_string()
                } else {
                    "unknown panic".to_string()
                };
                // Truncate panic payload to avoid leaking sensitive data into logs.
                // Use floor_char_boundary to avoid panicking on multi-byte UTF-8.
                let safe_detail = if detail.len() > 200 {
                    let end = detail.floor_char_boundary(200);
                    format!("{}…", &detail[..end])
                } else {
                    detail
                };
                tracing::error!("Handler panicked: {}", safe_detail);
                axum::http::Response::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .header("content-type", "text/plain")
                    .body(axum::body::Body::from("Internal Server Error"))
                    .unwrap_or_else(|_| {
                        axum::http::Response::new(axum::body::Body::from("Internal Server Error"))
                    })
            },
        ))
        .layer(cors)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            header::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            header::HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("content-security-policy"),
            BASE_CSP_HEADER.clone(),
        ))
        .with_state(state.clone());

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    *state.shutdown_tx.write().await = Some(shutdown_tx);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
                tracing::debug!("Web gateway shutting down");
            })
            .await
        {
            tracing::error!("Web gateway server error: {}", e);
        }
    });

    Ok(bound_addr)
}
