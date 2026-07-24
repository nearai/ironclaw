//! Convenience constructor for an axum [`Router`] wired to the
//! WebChat v2 handlers.
//!
//! Host composition is free to ignore this and mount each handler directly
//! against its own router; the descriptors in [`crate::webui_v2::descriptors`] are
//! the canonical contract. This module exists so handler-level tests can
//! drive the full route table without re-stating the path/method table.

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post, put};
use ironclaw_host_api::{BoundProductSurface, ProductSurface, ProductSurfaceCaller};
use serde::Serialize;

use crate::webui_v2::descriptors::{
    WEBUI_V2_PATTERN_ADMIN_USER, WEBUI_V2_PATTERN_ADMIN_USER_ROLE,
    WEBUI_V2_PATTERN_ADMIN_USER_SECRET, WEBUI_V2_PATTERN_ADMIN_USER_SECRETS,
    WEBUI_V2_PATTERN_ADMIN_USER_STATUS, WEBUI_V2_PATTERN_ADMIN_USERS,
    WEBUI_V2_PATTERN_AUTOMATION_DETAIL, WEBUI_V2_PATTERN_BROWSE_FS_DIR,
    WEBUI_V2_PATTERN_CANCEL_RUN, WEBUI_V2_PATTERN_COMPLETE_NEARAI_WALLET_LOGIN,
    WEBUI_V2_PATTERN_CREATE_THREAD, WEBUI_V2_PATTERN_DELETE_LLM_PROVIDER,
    WEBUI_V2_PATTERN_DELETE_THREAD, WEBUI_V2_PATTERN_GET_ATTACHMENT,
    WEBUI_V2_PATTERN_GET_LLM_CONFIG, WEBUI_V2_PATTERN_GET_RUN_ARTIFACT,
    WEBUI_V2_PATTERN_GET_SESSION, WEBUI_V2_PATTERN_GET_TIMELINE, WEBUI_V2_PATTERN_IMPORT_EXTENSION,
    WEBUI_V2_PATTERN_INSTALL_EXTENSION, WEBUI_V2_PATTERN_INSTALL_SKILL,
    WEBUI_V2_PATTERN_LIST_AUTOMATIONS, WEBUI_V2_PATTERN_LIST_EXTENSION_REGISTRY,
    WEBUI_V2_PATTERN_LIST_EXTENSIONS, WEBUI_V2_PATTERN_LIST_FS_MOUNTS,
    WEBUI_V2_PATTERN_LIST_LLM_MODELS, WEBUI_V2_PATTERN_LIST_PROJECT_FILES,
    WEBUI_V2_PATTERN_LIST_PROJECTS, WEBUI_V2_PATTERN_LIST_SKILLS, WEBUI_V2_PATTERN_LOGS,
    WEBUI_V2_PATTERN_OPERATOR_CONFIG, WEBUI_V2_PATTERN_OPERATOR_CONFIG_KEY,
    WEBUI_V2_PATTERN_OPERATOR_CONFIG_VALIDATE, WEBUI_V2_PATTERN_OPERATOR_DIAGNOSTICS,
    WEBUI_V2_PATTERN_OPERATOR_EXTENSION_CONFIGURATION,
    WEBUI_V2_PATTERN_OPERATOR_EXTENSION_CONFIGURATION_GROUP, WEBUI_V2_PATTERN_OPERATOR_LOGS,
    WEBUI_V2_PATTERN_OPERATOR_SERVICE_LIFECYCLE, WEBUI_V2_PATTERN_OPERATOR_SETUP,
    WEBUI_V2_PATTERN_OPERATOR_STATUS, WEBUI_V2_PATTERN_OUTBOUND_DELIVERY_TARGETS,
    WEBUI_V2_PATTERN_OUTBOUND_PREFERENCES, WEBUI_V2_PATTERN_PAUSE_AUTOMATION,
    WEBUI_V2_PATTERN_PROJECT_DETAIL, WEBUI_V2_PATTERN_PROJECT_MEMBER_DETAIL,
    WEBUI_V2_PATTERN_PROJECT_MEMBERS, WEBUI_V2_PATTERN_READ_FS_FILE,
    WEBUI_V2_PATTERN_READ_PROJECT_FILE, WEBUI_V2_PATTERN_REMOVE_EXTENSION,
    WEBUI_V2_PATTERN_RESOLVE_GATE, WEBUI_V2_PATTERN_RESUME_AUTOMATION, WEBUI_V2_PATTERN_RETRY_RUN,
    WEBUI_V2_PATTERN_SEARCH_SKILLS, WEBUI_V2_PATTERN_SEND_MESSAGE, WEBUI_V2_PATTERN_SET_ACTIVE_LLM,
    WEBUI_V2_PATTERN_SET_AUTO_ACTIVATE_LEARNED, WEBUI_V2_PATTERN_SET_SKILL_AUTO_ACTIVATE,
    WEBUI_V2_PATTERN_SETTINGS_TOOL_PERMISSION, WEBUI_V2_PATTERN_SETTINGS_TOOLS,
    WEBUI_V2_PATTERN_SETUP_EXTENSION, WEBUI_V2_PATTERN_SKILL_DETAIL,
    WEBUI_V2_PATTERN_START_CODEX_LOGIN, WEBUI_V2_PATTERN_START_NEARAI_LOGIN,
    WEBUI_V2_PATTERN_STAT_FS_PATH, WEBUI_V2_PATTERN_STAT_PROJECT_FILE,
    WEBUI_V2_PATTERN_STREAM_EVENTS, WEBUI_V2_PATTERN_STREAM_EVENTS_WS,
    WEBUI_V2_PATTERN_TEST_LLM_CONNECTION, WEBUI_V2_PATTERN_TRACE_ACCOUNT_LOGIN_LINK,
    WEBUI_V2_PATTERN_TRACE_ACCOUNT_TRACES, WEBUI_V2_PATTERN_TRACE_CREDITS,
    WEBUI_V2_PATTERN_TRACE_HOLD_AUTHORIZE,
};
use crate::webui_v2::handlers;
use crate::webui_v2::sse_capacity::SseCapacity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebUiV2RouteOptions {
    pub mount_llm_config_routes: bool,
    pub mount_operator_routes: bool,
}

impl WebUiV2RouteOptions {
    pub const fn all() -> Self {
        Self {
            mount_llm_config_routes: true,
            mount_operator_routes: true,
        }
    }

    // Also suppresses `operator/*` routes because the legacy LLM config
    // surface and the operator command plane share one trusted-operator gate.
    pub const fn without_llm_config_routes() -> Self {
        Self::without_operator_routes()
    }

    pub const fn without_operator_routes() -> Self {
        Self {
            mount_llm_config_routes: false,
            mount_operator_routes: false,
        }
    }
}

/// Shared state injected into every WebChat v2 handler.
///
/// Handlers receive a single facade so they can never reach into the
/// dispatcher, run-state, or any runtime lane directly. The state also
/// owns the [`SseCapacity`] gate that bounds concurrent SSE streams per
/// `(tenant, user)`; cloning the state shares the same gate so all
/// handler invocations enforce one cap process-wide.
#[derive(Clone)]
pub struct WebUiV2State {
    services: Arc<dyn ProductSurface>,
    sse_capacity: Arc<SseCapacity>,
    reborn_projects_enabled: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct WebUiV2Capabilities {
    pub operator_webui_config: bool,
}

impl WebUiV2State {
    pub fn new(
        services: Arc<dyn ProductSurface>,
        max_concurrent_streams_per_caller: usize,
    ) -> Self {
        Self {
            services,
            sse_capacity: Arc::new(SseCapacity::new(max_concurrent_streams_per_caller)),
            reborn_projects_enabled: false,
        }
    }

    /// Deployment gate for the Reborn Projects WebUI surface (the sidebar
    /// entry and the `/projects` route). Off by default while the surface
    /// is still being finished; host composition reads the
    /// `IRONCLAW_REBORN_PROJECTS` env flag and feeds it here, and the
    /// browser learns it from `GET /session`'s `features.reborn_projects`.
    pub fn with_reborn_projects_enabled(mut self, enabled: bool) -> Self {
        self.reborn_projects_enabled = enabled;
        self
    }

    pub fn reborn_projects_enabled(&self) -> bool {
        self.reborn_projects_enabled
    }

    pub fn services(&self) -> &Arc<dyn ProductSurface> {
        &self.services
    }

    pub fn bind_services(&self, caller: ProductSurfaceCaller) -> BoundProductSurface {
        BoundProductSurface::new(Arc::clone(&self.services), caller)
    }

    pub(crate) fn sse_capacity(&self) -> &Arc<SseCapacity> {
        &self.sse_capacity
    }
}

/// Build a [`Router`] mounting the WebChat v2 routes against the supplied
/// facade. Path patterns match
/// [`crate::webui_v2::descriptors::webui_v2_routes`] exactly; host composition is
/// expected to apply its own auth / CORS / body-limit middleware in front
/// of this router.
pub fn webui_v2_router(state: WebUiV2State) -> Router {
    webui_v2_router_with_options(state, WebUiV2RouteOptions::all())
}

pub fn webui_v2_router_with_options(state: WebUiV2State, options: WebUiV2RouteOptions) -> Router {
    let mut router = Router::new()
        // GET and POST share the `/api/webchat/v2/threads` path
        // (`WEBUI_V2_PATTERN_CREATE_THREAD == WEBUI_V2_PATTERN_LIST_THREADS`);
        // mount both verbs in one `.route()` so axum's matcher
        // dispatches by method.
        .route(
            WEBUI_V2_PATTERN_CREATE_THREAD,
            post(handlers::create_thread).get(handlers::list_threads),
        )
        .route(
            WEBUI_V2_PATTERN_DELETE_THREAD,
            delete(handlers::delete_thread),
        )
        // Admin user-management. Authorization (operator token or admin/owner
        // role) and last-admin protection are enforced in the facade, so these
        // are mounted unconditionally — a non-admin caller gets 403.
        .route(
            WEBUI_V2_PATTERN_ADMIN_USERS,
            get(handlers::admin_list_users).post(handlers::admin_create_user),
        )
        .route(
            WEBUI_V2_PATTERN_ADMIN_USER,
            get(handlers::admin_get_user)
                .patch(handlers::admin_update_user)
                .delete(handlers::admin_delete_user),
        )
        .route(
            WEBUI_V2_PATTERN_ADMIN_USER_STATUS,
            post(handlers::admin_set_user_status),
        )
        .route(
            WEBUI_V2_PATTERN_ADMIN_USER_ROLE,
            post(handlers::admin_set_user_role),
        )
        .route(
            WEBUI_V2_PATTERN_ADMIN_USER_SECRETS,
            get(handlers::admin_list_user_secrets),
        )
        .route(
            WEBUI_V2_PATTERN_ADMIN_USER_SECRET,
            put(handlers::admin_put_user_secret).delete(handlers::admin_delete_user_secret),
        )
        .route(WEBUI_V2_PATTERN_GET_SESSION, get(handlers::get_session))
        .route(WEBUI_V2_PATTERN_SEND_MESSAGE, post(handlers::send_message))
        .route(WEBUI_V2_PATTERN_GET_TIMELINE, get(handlers::get_timeline))
        .route(
            WEBUI_V2_PATTERN_GET_RUN_ARTIFACT,
            get(handlers::get_run_artifact),
        )
        .route(WEBUI_V2_PATTERN_LOGS, get(handlers::query_logs))
        .route(
            WEBUI_V2_PATTERN_LIST_PROJECT_FILES,
            get(handlers::list_project_files),
        )
        .route(
            WEBUI_V2_PATTERN_STAT_PROJECT_FILE,
            get(handlers::stat_project_file),
        )
        .route(
            WEBUI_V2_PATTERN_READ_PROJECT_FILE,
            get(handlers::read_project_file),
        )
        // GET (list) and POST (create) share `/api/webchat/v2/projects`.
        .route(
            WEBUI_V2_PATTERN_LIST_PROJECTS,
            get(handlers::list_projects).post(handlers::create_project),
        )
        // GET (read), POST (update), DELETE share `/projects/{project_id}`.
        .route(
            WEBUI_V2_PATTERN_PROJECT_DETAIL,
            get(handlers::get_project)
                .post(handlers::update_project)
                .delete(handlers::delete_project),
        )
        // GET (list) and POST (add) share `/projects/{project_id}/members`.
        .route(
            WEBUI_V2_PATTERN_PROJECT_MEMBERS,
            get(handlers::list_project_members).post(handlers::add_project_member),
        )
        // POST (update role) and DELETE (revoke) share the member detail path.
        .route(
            WEBUI_V2_PATTERN_PROJECT_MEMBER_DETAIL,
            post(handlers::update_project_member).delete(handlers::remove_project_member),
        )
        .route(
            WEBUI_V2_PATTERN_LIST_FS_MOUNTS,
            get(handlers::list_fs_mounts),
        )
        .route(WEBUI_V2_PATTERN_BROWSE_FS_DIR, get(handlers::browse_fs_dir))
        .route(WEBUI_V2_PATTERN_STAT_FS_PATH, get(handlers::stat_fs_path))
        .route(WEBUI_V2_PATTERN_READ_FS_FILE, get(handlers::read_fs_file))
        .route(
            WEBUI_V2_PATTERN_GET_ATTACHMENT,
            get(handlers::get_attachment),
        )
        .route(WEBUI_V2_PATTERN_STREAM_EVENTS, get(handlers::stream_events))
        .route(
            WEBUI_V2_PATTERN_STREAM_EVENTS_WS,
            get(handlers::stream_events_ws),
        )
        .route(WEBUI_V2_PATTERN_CANCEL_RUN, post(handlers::cancel_run))
        .route(WEBUI_V2_PATTERN_RESOLVE_GATE, post(handlers::resolve_gate))
        .route(WEBUI_V2_PATTERN_RETRY_RUN, post(handlers::retry_run))
        .route(
            WEBUI_V2_PATTERN_LIST_AUTOMATIONS,
            get(handlers::list_automations),
        )
        .route(
            WEBUI_V2_PATTERN_PAUSE_AUTOMATION,
            post(handlers::pause_automation),
        )
        .route(
            WEBUI_V2_PATTERN_RESUME_AUTOMATION,
            post(handlers::resume_automation),
        )
        .route(
            WEBUI_V2_PATTERN_AUTOMATION_DETAIL,
            post(handlers::rename_automation).delete(handlers::delete_automation),
        )
        .route(WEBUI_V2_PATTERN_TRACE_CREDITS, get(handlers::trace_credits))
        .route(
            WEBUI_V2_PATTERN_TRACE_ACCOUNT_TRACES,
            get(handlers::trace_account_traces),
        )
        .route(
            WEBUI_V2_PATTERN_TRACE_HOLD_AUTHORIZE,
            post(handlers::authorize_trace_hold),
        )
        .route(
            WEBUI_V2_PATTERN_TRACE_ACCOUNT_LOGIN_LINK,
            post(handlers::trace_account_login_link),
        )
        .route(
            WEBUI_V2_PATTERN_OUTBOUND_PREFERENCES,
            get(handlers::get_outbound_preferences).post(handlers::set_outbound_preferences),
        )
        .route(
            WEBUI_V2_PATTERN_OUTBOUND_DELIVERY_TARGETS,
            get(handlers::list_outbound_delivery_targets),
        )
        .route(
            WEBUI_V2_PATTERN_LIST_EXTENSIONS,
            get(handlers::list_extensions),
        )
        .route(WEBUI_V2_PATTERN_LIST_SKILLS, get(handlers::list_skills))
        .route(
            WEBUI_V2_PATTERN_SEARCH_SKILLS,
            post(handlers::search_skills),
        )
        .route(
            WEBUI_V2_PATTERN_INSTALL_SKILL,
            post(handlers::install_skill),
        )
        .route(
            WEBUI_V2_PATTERN_SKILL_DETAIL,
            get(handlers::get_skill_content)
                .put(handlers::update_skill)
                .delete(handlers::remove_skill),
        )
        .route(
            WEBUI_V2_PATTERN_SET_SKILL_AUTO_ACTIVATE,
            post(handlers::set_skill_auto_activate),
        )
        .route(
            WEBUI_V2_PATTERN_SET_AUTO_ACTIVATE_LEARNED,
            post(handlers::set_auto_activate_learned),
        )
        .route(
            WEBUI_V2_PATTERN_SETTINGS_TOOLS,
            get(handlers::list_settings_tools).post(handlers::set_settings_tools_auto_approve),
        )
        .route(
            WEBUI_V2_PATTERN_SETTINGS_TOOL_PERMISSION,
            post(handlers::set_settings_tool_permission),
        )
        .route(
            WEBUI_V2_PATTERN_LIST_EXTENSION_REGISTRY,
            get(handlers::list_extension_registry),
        )
        .route(
            WEBUI_V2_PATTERN_INSTALL_EXTENSION,
            post(handlers::install_extension),
        )
        .route(
            WEBUI_V2_PATTERN_REMOVE_EXTENSION,
            post(handlers::remove_extension),
        )
        .route(
            WEBUI_V2_PATTERN_SETUP_EXTENSION,
            get(handlers::get_extension_setup).post(handlers::setup_extension),
        );
    if options.mount_llm_config_routes {
        router = router
            // `WEBUI_V2_PATTERN_GET_LLM_CONFIG == WEBUI_V2_PATTERN_UPSERT_LLM_PROVIDER`
            // (`/llm/providers`); mount GET + POST in one `.route()`.
            .route(
                WEBUI_V2_PATTERN_GET_LLM_CONFIG,
                get(handlers::get_llm_config).post(handlers::upsert_llm_provider),
            )
            .route(
                WEBUI_V2_PATTERN_DELETE_LLM_PROVIDER,
                post(handlers::delete_llm_provider),
            )
            .route(
                WEBUI_V2_PATTERN_SET_ACTIVE_LLM,
                post(handlers::set_active_llm),
            )
            .route(
                WEBUI_V2_PATTERN_TEST_LLM_CONNECTION,
                post(handlers::test_llm_connection),
            )
            .route(
                WEBUI_V2_PATTERN_LIST_LLM_MODELS,
                post(handlers::list_llm_models),
            )
            .route(
                WEBUI_V2_PATTERN_START_NEARAI_LOGIN,
                post(handlers::start_nearai_login),
            )
            .route(
                WEBUI_V2_PATTERN_COMPLETE_NEARAI_WALLET_LOGIN,
                post(handlers::complete_nearai_wallet_login),
            )
            .route(
                WEBUI_V2_PATTERN_START_CODEX_LOGIN,
                post(handlers::start_codex_login),
            );
    }
    if options.mount_operator_routes {
        router = router
            // Admin-only zip upload (#5499): operator-classified, so it is
            // stripped alongside the operator command plane instead of staying
            // mounted (handler-gated only) in deployments without an operator
            // surface.
            .route(
                WEBUI_V2_PATTERN_IMPORT_EXTENSION,
                post(handlers::import_extension),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_SETUP,
                get(handlers::get_operator_setup).post(handlers::run_operator_setup),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_CONFIG,
                get(handlers::list_operator_config),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_CONFIG_VALIDATE,
                get(handlers::reject_reserved_operator_config_key)
                    .post(handlers::validate_operator_config),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_CONFIG_KEY,
                get(handlers::get_operator_config_key).post(handlers::set_operator_config_key),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_DIAGNOSTICS,
                get(handlers::get_operator_diagnostics),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_STATUS,
                get(handlers::get_operator_status),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_LOGS,
                get(handlers::query_operator_logs),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_SERVICE_LIFECYCLE,
                post(handlers::run_operator_service_lifecycle),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_EXTENSION_CONFIGURATION,
                get(handlers::list_extension_admin_configuration),
            )
            .route(
                WEBUI_V2_PATTERN_OPERATOR_EXTENSION_CONFIGURATION_GROUP,
                put(handlers::replace_extension_admin_configuration),
            );
    }
    router.with_state(state)
}
