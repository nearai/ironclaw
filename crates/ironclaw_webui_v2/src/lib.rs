//! Reborn WebChat v2 HTTP route surface.
//!
//! This crate ships the minimal native WebUI v2 route set on top of the
//! [`ironclaw_product_workflow::RebornServicesApi`] facade. It is off by
//! default — enable the `webui-v2-beta` Cargo feature to compile it in.
//!
//! ## Boundaries
//!
//! - Handlers consume only [`RebornServicesApi`] for chat, run/gate,
//!   extension, and automation reads. They never reach into the dispatcher,
//!   `HostRuntime`, run-state, DB stores, or any runtime lane.
//! - Auth and CORS are **not** enforced here. Host composition runs the
//!   bearer-token middleware that builds a [`WebUiAuthenticatedCaller`] and
//!   injects it as an `Extension` before traffic reaches these handlers.
//! - The [`IngressRouteDescriptor`] set returned by [`webui_v2_routes`] is
//!   the canonical contract the host composes against: mount path, method,
//!   auth scheme, body / rate limit, streaming mode, audit class, and the
//!   allowed effect path. Adding a new route here requires a matching
//!   descriptor.
//!
//! ## Streaming
//!
//! `stream_events` is exposed as SSE. The current
//! [`RebornServicesApi::stream_events`] is drain-only, so the handler
//! drains once, renders each product envelope into a
//! [`WebChatV2EventFrame`] SSE message with the projection cursor as the
//! SSE id, then polls at a low cadence for newly-arrived events. When the
//! facade gains a real subscription API the handler can migrate without
//! changing the descriptor or browser-visible event schema.
//!
//! Beyond the route descriptor's per-caller request rate limit, the
//! handler caps the number of *concurrent* SSE streams a single
//! `(tenant, user)` may hold and closes any single stream after a fixed
//! maximum lifetime so leaked guards or stuck pollers cannot wedge a
//! caller's slot indefinitely.
//!
//! [`RebornServicesApi`]: ironclaw_product_workflow::RebornServicesApi
//! [`WebChatV2EventFrame`]: crate::WebChatV2EventFrame
//! [`WebUiAuthenticatedCaller`]: ironclaw_product_workflow::WebUiAuthenticatedCaller
//! [`IngressRouteDescriptor`]: ironclaw_host_api::ingress::IngressRouteDescriptor

#![forbid(unsafe_code)]

#[cfg(feature = "webui-v2-beta")]
mod descriptors;
#[cfg(feature = "webui-v2-beta")]
mod error;
#[cfg(feature = "webui-v2-beta")]
mod handlers;
#[cfg(feature = "webui-v2-beta")]
mod router;
#[cfg(feature = "webui-v2-beta")]
mod schema;
#[cfg(feature = "webui-v2-beta")]
mod sse_capacity;
// Browser SPA asset bundle: the JSON route surface and the static bytes it
// drives now ship from one crate behind the single `webui-v2-beta` feature.
#[cfg(feature = "webui-v2-beta")]
pub mod static_assets;

#[allow(deprecated)]
#[cfg(feature = "webui-v2-beta")]
pub use descriptors::is_webui_v2_llm_config_route_id;
#[cfg(feature = "webui-v2-beta")]
pub use descriptors::{
    WEBUI_V2_ROUTE_ACTIVATE_EXTENSION, WEBUI_V2_ROUTE_ADD_PROJECT_MEMBER,
    WEBUI_V2_ROUTE_ADMIN_CREATE_USER, WEBUI_V2_ROUTE_ADMIN_DELETE_USER,
    WEBUI_V2_ROUTE_ADMIN_DELETE_USER_SECRET, WEBUI_V2_ROUTE_ADMIN_GET_USER,
    WEBUI_V2_ROUTE_ADMIN_LIST_USER_SECRETS, WEBUI_V2_ROUTE_ADMIN_LIST_USERS,
    WEBUI_V2_ROUTE_ADMIN_PUT_USER_SECRET, WEBUI_V2_ROUTE_ADMIN_SET_USER_ROLE,
    WEBUI_V2_ROUTE_ADMIN_SET_USER_STATUS, WEBUI_V2_ROUTE_ADMIN_UPDATE_USER,
    WEBUI_V2_ROUTE_BROWSE_FS_DIR, WEBUI_V2_ROUTE_CANCEL_RUN,
    WEBUI_V2_ROUTE_COMPLETE_NEARAI_WALLET_LOGIN, WEBUI_V2_ROUTE_CREATE_PROJECT,
    WEBUI_V2_ROUTE_CREATE_THREAD, WEBUI_V2_ROUTE_DELETE_AUTOMATION,
    WEBUI_V2_ROUTE_DELETE_LLM_PROVIDER, WEBUI_V2_ROUTE_DELETE_PROJECT,
    WEBUI_V2_ROUTE_DELETE_THREAD, WEBUI_V2_ROUTE_GET_ATTACHMENT,
    WEBUI_V2_ROUTE_GET_EXTENSION_SETUP, WEBUI_V2_ROUTE_GET_LLM_CONFIG,
    WEBUI_V2_ROUTE_GET_OUTBOUND_PREFERENCES, WEBUI_V2_ROUTE_GET_PROJECT,
    WEBUI_V2_ROUTE_GET_SESSION, WEBUI_V2_ROUTE_GET_SKILL, WEBUI_V2_ROUTE_GET_TIMELINE,
    WEBUI_V2_ROUTE_INSTALL_EXTENSION, WEBUI_V2_ROUTE_INSTALL_SKILL,
    WEBUI_V2_ROUTE_LIST_AUTOMATIONS, WEBUI_V2_ROUTE_LIST_CONNECTABLE_CHANNELS,
    WEBUI_V2_ROUTE_LIST_EXTENSION_REGISTRY, WEBUI_V2_ROUTE_LIST_EXTENSIONS,
    WEBUI_V2_ROUTE_LIST_FS_MOUNTS, WEBUI_V2_ROUTE_LIST_LLM_MODELS,
    WEBUI_V2_ROUTE_LIST_OUTBOUND_DELIVERY_TARGETS, WEBUI_V2_ROUTE_LIST_PROJECT_FILES,
    WEBUI_V2_ROUTE_LIST_PROJECT_MEMBERS, WEBUI_V2_ROUTE_LIST_PROJECTS,
    WEBUI_V2_ROUTE_LIST_SETTINGS_TOOLS, WEBUI_V2_ROUTE_LIST_SKILLS, WEBUI_V2_ROUTE_LIST_THREADS,
    WEBUI_V2_ROUTE_LOGS, WEBUI_V2_ROUTE_OPERATOR_DIAGNOSTICS,
    WEBUI_V2_ROUTE_OPERATOR_GET_CONFIG_KEY, WEBUI_V2_ROUTE_OPERATOR_GET_SETUP,
    WEBUI_V2_ROUTE_OPERATOR_LIST_CONFIG, WEBUI_V2_ROUTE_OPERATOR_LOGS,
    WEBUI_V2_ROUTE_OPERATOR_RUN_SETUP, WEBUI_V2_ROUTE_OPERATOR_SERVICE_LIFECYCLE,
    WEBUI_V2_ROUTE_OPERATOR_SET_CONFIG_KEY, WEBUI_V2_ROUTE_OPERATOR_STATUS,
    WEBUI_V2_ROUTE_OPERATOR_VALIDATE_CONFIG, WEBUI_V2_ROUTE_PAUSE_AUTOMATION,
    WEBUI_V2_ROUTE_READ_FS_FILE, WEBUI_V2_ROUTE_READ_PROJECT_FILE, WEBUI_V2_ROUTE_REMOVE_EXTENSION,
    WEBUI_V2_ROUTE_REMOVE_PROJECT_MEMBER, WEBUI_V2_ROUTE_REMOVE_SKILL,
    WEBUI_V2_ROUTE_RENAME_AUTOMATION, WEBUI_V2_ROUTE_RESOLVE_GATE,
    WEBUI_V2_ROUTE_RESUME_AUTOMATION, WEBUI_V2_ROUTE_RETRY_RUN, WEBUI_V2_ROUTE_SEARCH_SKILLS,
    WEBUI_V2_ROUTE_SEND_MESSAGE, WEBUI_V2_ROUTE_SET_ACTIVE_LLM,
    WEBUI_V2_ROUTE_SET_AUTO_ACTIVATE_LEARNED, WEBUI_V2_ROUTE_SET_OUTBOUND_PREFERENCES,
    WEBUI_V2_ROUTE_SET_SETTINGS_TOOL_PERMISSION, WEBUI_V2_ROUTE_SET_SETTINGS_TOOLS_AUTO_APPROVE,
    WEBUI_V2_ROUTE_SET_SKILL_AUTO_ACTIVATE, WEBUI_V2_ROUTE_SETUP_EXTENSION,
    WEBUI_V2_ROUTE_START_CODEX_LOGIN, WEBUI_V2_ROUTE_START_NEARAI_LOGIN,
    WEBUI_V2_ROUTE_STAT_FS_PATH, WEBUI_V2_ROUTE_STAT_PROJECT_FILE, WEBUI_V2_ROUTE_STREAM_EVENTS,
    WEBUI_V2_ROUTE_STREAM_EVENTS_WS, WEBUI_V2_ROUTE_TEST_LLM_CONNECTION,
    WEBUI_V2_ROUTE_TRACE_ACCOUNT_LOGIN_LINK, WEBUI_V2_ROUTE_TRACE_ACCOUNT_TRACES,
    WEBUI_V2_ROUTE_TRACE_CREDITS, WEBUI_V2_ROUTE_TRACE_HOLD_AUTHORIZE,
    WEBUI_V2_ROUTE_UPDATE_PROJECT, WEBUI_V2_ROUTE_UPDATE_PROJECT_MEMBER,
    WEBUI_V2_ROUTE_UPDATE_SKILL, WEBUI_V2_ROUTE_UPSERT_LLM_PROVIDER,
    is_webui_v2_operator_webui_config_route_id, webui_v2_routes,
};
#[cfg(feature = "webui-v2-beta")]
pub use error::{WebUiV2HttpError, WebUiV2HttpErrorBody};
#[cfg(feature = "webui-v2-beta")]
pub use handlers::{
    activate_extension, browse_fs_dir, cancel_run, complete_nearai_wallet_login, create_thread,
    delete_automation, delete_llm_provider, delete_thread, get_attachment, get_extension_setup,
    get_llm_config, get_operator_config_key, get_operator_diagnostics, get_operator_setup,
    get_operator_status, get_outbound_preferences, get_session, get_skill_content, get_timeline,
    install_extension, install_skill, list_automations, list_connectable_channels,
    list_extension_registry, list_extensions, list_fs_mounts, list_llm_models,
    list_operator_config, list_outbound_delivery_targets, list_settings_tools, list_skills,
    list_threads, pause_automation, query_logs, query_operator_logs, read_fs_file,
    remove_extension, remove_skill, rename_automation, resolve_gate, resume_automation, retry_run,
    run_operator_service_lifecycle, run_operator_setup, search_skills, send_message,
    set_active_llm, set_auto_activate_learned, set_operator_config_key, set_outbound_preferences,
    set_settings_tool_permission, set_settings_tools_auto_approve, set_skill_auto_activate,
    setup_extension, start_codex_login, start_nearai_login, stat_fs_path, stream_events,
    stream_events_ws, test_llm_connection, trace_account_traces, trace_credits, update_skill,
    upsert_llm_provider,
};
#[cfg(feature = "webui-v2-beta")]
pub use router::{
    WebUiV2Capabilities, WebUiV2RouteOptions, WebUiV2State, webui_v2_router,
    webui_v2_router_with_options,
};
#[cfg(feature = "webui-v2-beta")]
pub use schema::{WebChatV2Event, WebChatV2EventFrame};
// Re-export the static-bundle router factory at the crate root so host
// composition keeps calling `ironclaw_webui_v2::mount_at_prefix(...)`.
#[cfg(feature = "webui-v2-beta")]
pub use sse_capacity::DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER;
#[cfg(feature = "webui-v2-beta")]
pub use static_assets::{mount_at_prefix, serve_root, serve_wildcard, static_router};
