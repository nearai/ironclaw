//! Host-owned route descriptors for the Reborn WebChat v2 surface.
//!
//! Host composition consumes [`webui_v2_routes`] and mounts the matching
//! handler from [`crate::handlers`] under each descriptor's pattern. The
//! descriptor is the contract: changing a route's policy here changes what
//! host composition enforces before the handler runs.

use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
use std::num::{NonZeroU32, NonZeroU64};

pub const WEBUI_V2_ROUTE_CREATE_THREAD: &str = "webui.v2.create_thread";
pub const WEBUI_V2_ROUTE_DELETE_THREAD: &str = "webui.v2.delete_thread";
pub const WEBUI_V2_ROUTE_GET_SESSION: &str = "webui.v2.get_session";
pub const WEBUI_V2_ROUTE_SEND_MESSAGE: &str = "webui.v2.send_message";
pub const WEBUI_V2_ROUTE_LIST_THREADS: &str = "webui.v2.list_threads";
pub const WEBUI_V2_ROUTE_GET_TIMELINE: &str = "webui.v2.get_timeline";
pub const WEBUI_V2_ROUTE_GET_ATTACHMENT: &str = "webui.v2.get_attachment";
pub const WEBUI_V2_ROUTE_STREAM_EVENTS: &str = "webui.v2.stream_events";
pub const WEBUI_V2_ROUTE_STREAM_EVENTS_WS: &str = "webui.v2.stream_events_ws";
pub const WEBUI_V2_ROUTE_CANCEL_RUN: &str = "webui.v2.cancel_run";
pub const WEBUI_V2_ROUTE_RESOLVE_GATE: &str = "webui.v2.resolve_gate";
pub const WEBUI_V2_ROUTE_LIST_AUTOMATIONS: &str = "webui.v2.list_automations";
pub const WEBUI_V2_ROUTE_PAUSE_AUTOMATION: &str = "webui.v2.pause_automation";
pub const WEBUI_V2_ROUTE_RESUME_AUTOMATION: &str = "webui.v2.resume_automation";
pub const WEBUI_V2_ROUTE_TRACE_CREDITS: &str = "webui.v2.trace_credits";
pub const WEBUI_V2_ROUTE_TRACE_HOLD_AUTHORIZE: &str = "webui.v2.authorize_trace_hold";
pub const WEBUI_V2_ROUTE_GET_OUTBOUND_PREFERENCES: &str = "webui.v2.get_outbound_preferences";
pub const WEBUI_V2_ROUTE_SET_OUTBOUND_PREFERENCES: &str = "webui.v2.set_outbound_preferences";
pub const WEBUI_V2_ROUTE_LIST_OUTBOUND_DELIVERY_TARGETS: &str =
    "webui.v2.list_outbound_delivery_targets";
pub const WEBUI_V2_ROUTE_LIST_CONNECTABLE_CHANNELS: &str = "webui.v2.list_connectable_channels";
pub const WEBUI_V2_ROUTE_LIST_EXTENSIONS: &str = "webui.v2.list_extensions";
pub const WEBUI_V2_ROUTE_LIST_EXTENSION_REGISTRY: &str = "webui.v2.list_extension_registry";
pub const WEBUI_V2_ROUTE_INSTALL_EXTENSION: &str = "webui.v2.install_extension";
pub const WEBUI_V2_ROUTE_ACTIVATE_EXTENSION: &str = "webui.v2.activate_extension";
pub const WEBUI_V2_ROUTE_REMOVE_EXTENSION: &str = "webui.v2.remove_extension";
pub const WEBUI_V2_ROUTE_GET_EXTENSION_SETUP: &str = "webui.v2.get_extension_setup";
pub const WEBUI_V2_ROUTE_SETUP_EXTENSION: &str = "webui.v2.setup_extension";
pub const WEBUI_V2_ROUTE_LIST_SKILLS: &str = "webui.v2.list_skills";
pub const WEBUI_V2_ROUTE_SEARCH_SKILLS: &str = "webui.v2.search_skills";
pub const WEBUI_V2_ROUTE_INSTALL_SKILL: &str = "webui.v2.install_skill";
pub const WEBUI_V2_ROUTE_GET_SKILL: &str = "webui.v2.get_skill";
pub const WEBUI_V2_ROUTE_UPDATE_SKILL: &str = "webui.v2.update_skill";
pub const WEBUI_V2_ROUTE_REMOVE_SKILL: &str = "webui.v2.remove_skill";
pub const WEBUI_V2_ROUTE_SET_SKILL_AUTO_ACTIVATE: &str = "webui.v2.set_skill_auto_activate";
pub const WEBUI_V2_ROUTE_SET_AUTO_ACTIVATE_LEARNED: &str = "webui.v2.set_auto_activate_learned";
pub const WEBUI_V2_ROUTE_GET_LLM_CONFIG: &str = "webui.v2.get_llm_config";
pub const WEBUI_V2_ROUTE_UPSERT_LLM_PROVIDER: &str = "webui.v2.upsert_llm_provider";
pub const WEBUI_V2_ROUTE_DELETE_LLM_PROVIDER: &str = "webui.v2.delete_llm_provider";
pub const WEBUI_V2_ROUTE_SET_ACTIVE_LLM: &str = "webui.v2.set_active_llm";
pub const WEBUI_V2_ROUTE_TEST_LLM_CONNECTION: &str = "webui.v2.test_llm_connection";
pub const WEBUI_V2_ROUTE_LIST_LLM_MODELS: &str = "webui.v2.list_llm_models";
pub const WEBUI_V2_ROUTE_START_NEARAI_LOGIN: &str = "webui.v2.start_nearai_login";
pub const WEBUI_V2_ROUTE_COMPLETE_NEARAI_WALLET_LOGIN: &str =
    "webui.v2.complete_nearai_wallet_login";
pub const WEBUI_V2_ROUTE_START_CODEX_LOGIN: &str = "webui.v2.start_codex_login";
pub const WEBUI_V2_ROUTE_OPERATOR_GET_SETUP: &str = "webui.v2.operator.get_setup";
pub const WEBUI_V2_ROUTE_OPERATOR_RUN_SETUP: &str = "webui.v2.operator.run_setup";
pub const WEBUI_V2_ROUTE_OPERATOR_LIST_CONFIG: &str = "webui.v2.operator.list_config";
pub const WEBUI_V2_ROUTE_OPERATOR_GET_CONFIG_KEY: &str = "webui.v2.operator.get_config_key";
pub const WEBUI_V2_ROUTE_OPERATOR_SET_CONFIG_KEY: &str = "webui.v2.operator.set_config_key";
pub const WEBUI_V2_ROUTE_OPERATOR_VALIDATE_CONFIG: &str = "webui.v2.operator.validate_config";
pub const WEBUI_V2_ROUTE_OPERATOR_DIAGNOSTICS: &str = "webui.v2.operator.diagnostics";
pub const WEBUI_V2_ROUTE_OPERATOR_STATUS: &str = "webui.v2.operator.status";
pub const WEBUI_V2_ROUTE_OPERATOR_LOGS: &str = "webui.v2.operator.logs";
pub const WEBUI_V2_ROUTE_OPERATOR_SERVICE_LIFECYCLE: &str = "webui.v2.operator.service_lifecycle";
pub const WEBUI_V2_ROUTE_LIST_PROJECT_FILES: &str = "webui.v2.list_project_files";
pub const WEBUI_V2_ROUTE_STAT_PROJECT_FILE: &str = "webui.v2.stat_project_file";
pub const WEBUI_V2_ROUTE_READ_PROJECT_FILE: &str = "webui.v2.read_project_file";
pub const WEBUI_V2_ROUTE_LIST_FS_MOUNTS: &str = "webui.v2.list_fs_mounts";
pub const WEBUI_V2_ROUTE_BROWSE_FS_DIR: &str = "webui.v2.browse_fs_dir";
pub const WEBUI_V2_ROUTE_STAT_FS_PATH: &str = "webui.v2.stat_fs_path";
pub const WEBUI_V2_ROUTE_READ_FS_FILE: &str = "webui.v2.read_fs_file";
pub const WEBUI_V2_ROUTE_LIST_PROJECTS: &str = "webui.v2.list_projects";
pub const WEBUI_V2_ROUTE_CREATE_PROJECT: &str = "webui.v2.create_project";
pub const WEBUI_V2_ROUTE_GET_PROJECT: &str = "webui.v2.get_project";
pub const WEBUI_V2_ROUTE_UPDATE_PROJECT: &str = "webui.v2.update_project";
pub const WEBUI_V2_ROUTE_DELETE_PROJECT: &str = "webui.v2.delete_project";
pub const WEBUI_V2_ROUTE_LIST_PROJECT_MEMBERS: &str = "webui.v2.list_project_members";
pub const WEBUI_V2_ROUTE_ADD_PROJECT_MEMBER: &str = "webui.v2.add_project_member";
pub const WEBUI_V2_ROUTE_UPDATE_PROJECT_MEMBER: &str = "webui.v2.update_project_member";
pub const WEBUI_V2_ROUTE_REMOVE_PROJECT_MEMBER: &str = "webui.v2.remove_project_member";

pub const WEBUI_V2_PATTERN_CREATE_THREAD: &str = "/api/webchat/v2/threads";
pub const WEBUI_V2_PATTERN_LIST_THREADS: &str = "/api/webchat/v2/threads";
pub const WEBUI_V2_PATTERN_DELETE_THREAD: &str = "/api/webchat/v2/threads/{thread_id}";
pub const WEBUI_V2_PATTERN_GET_SESSION: &str = "/api/webchat/v2/session";
pub const WEBUI_V2_PATTERN_SEND_MESSAGE: &str = "/api/webchat/v2/threads/{thread_id}/messages";
pub const WEBUI_V2_PATTERN_GET_TIMELINE: &str = "/api/webchat/v2/threads/{thread_id}/timeline";
pub const WEBUI_V2_PATTERN_GET_ATTACHMENT: &str =
    "/api/webchat/v2/threads/{thread_id}/messages/{message_id}/attachments/{attachment_id}";
pub const WEBUI_V2_PATTERN_STREAM_EVENTS: &str = "/api/webchat/v2/threads/{thread_id}/events";
pub const WEBUI_V2_PATTERN_STREAM_EVENTS_WS: &str = "/api/webchat/v2/threads/{thread_id}/ws";
pub const WEBUI_V2_PATTERN_CANCEL_RUN: &str =
    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel";
pub const WEBUI_V2_PATTERN_RESOLVE_GATE: &str =
    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve";
pub const WEBUI_V2_PATTERN_LIST_AUTOMATIONS: &str = "/api/webchat/v2/automations";
pub const WEBUI_V2_PATTERN_PAUSE_AUTOMATION: &str =
    "/api/webchat/v2/automations/{automation_id}/pause";
pub const WEBUI_V2_PATTERN_RESUME_AUTOMATION: &str =
    "/api/webchat/v2/automations/{automation_id}/resume";
pub const WEBUI_V2_PATTERN_TRACE_CREDITS: &str = "/api/webchat/v2/traces/credit";
pub const WEBUI_V2_PATTERN_TRACE_HOLD_AUTHORIZE: &str =
    "/api/webchat/v2/traces/holds/{submission_id}/authorize";
pub const WEBUI_V2_PATTERN_OUTBOUND_PREFERENCES: &str = "/api/webchat/v2/outbound/preferences";
pub const WEBUI_V2_PATTERN_OUTBOUND_DELIVERY_TARGETS: &str = "/api/webchat/v2/outbound/targets";
pub const WEBUI_V2_PATTERN_LIST_CONNECTABLE_CHANNELS: &str = "/api/webchat/v2/channels/connectable";
pub const WEBUI_V2_PATTERN_LIST_EXTENSIONS: &str = "/api/webchat/v2/extensions";
pub const WEBUI_V2_PATTERN_LIST_EXTENSION_REGISTRY: &str = "/api/webchat/v2/extensions/registry";
pub const WEBUI_V2_PATTERN_INSTALL_EXTENSION: &str = "/api/webchat/v2/extensions/install";
pub const WEBUI_V2_PATTERN_ACTIVATE_EXTENSION: &str =
    "/api/webchat/v2/extensions/{package_id}/activate";
pub const WEBUI_V2_PATTERN_REMOVE_EXTENSION: &str =
    "/api/webchat/v2/extensions/{package_id}/remove";
pub const WEBUI_V2_PATTERN_SETUP_EXTENSION: &str = "/api/webchat/v2/extensions/{package_id}/setup";
pub const WEBUI_V2_PATTERN_LIST_SKILLS: &str = "/api/webchat/v2/skills";
pub const WEBUI_V2_PATTERN_SEARCH_SKILLS: &str = "/api/webchat/v2/skills/search";
pub const WEBUI_V2_PATTERN_INSTALL_SKILL: &str = "/api/webchat/v2/skills/install";
pub const WEBUI_V2_PATTERN_SKILL_DETAIL: &str = "/api/webchat/v2/skills/{name}";
pub const WEBUI_V2_PATTERN_SET_SKILL_AUTO_ACTIVATE: &str =
    "/api/webchat/v2/skills/{name}/auto-activate";
pub const WEBUI_V2_PATTERN_SET_AUTO_ACTIVATE_LEARNED: &str =
    "/api/webchat/v2/skills/auto-activate-learned";
pub const WEBUI_V2_PATTERN_GET_LLM_CONFIG: &str = "/api/webchat/v2/llm/providers";
pub const WEBUI_V2_PATTERN_UPSERT_LLM_PROVIDER: &str = "/api/webchat/v2/llm/providers";
pub const WEBUI_V2_PATTERN_DELETE_LLM_PROVIDER: &str =
    "/api/webchat/v2/llm/providers/{provider_id}/delete";
pub const WEBUI_V2_PATTERN_SET_ACTIVE_LLM: &str = "/api/webchat/v2/llm/active";
pub const WEBUI_V2_PATTERN_TEST_LLM_CONNECTION: &str = "/api/webchat/v2/llm/test-connection";
pub const WEBUI_V2_PATTERN_LIST_LLM_MODELS: &str = "/api/webchat/v2/llm/list-models";
pub const WEBUI_V2_PATTERN_START_NEARAI_LOGIN: &str = "/api/webchat/v2/llm/nearai/login";
pub const WEBUI_V2_PATTERN_COMPLETE_NEARAI_WALLET_LOGIN: &str = "/api/webchat/v2/llm/nearai/wallet";
pub const WEBUI_V2_PATTERN_START_CODEX_LOGIN: &str = "/api/webchat/v2/llm/codex/login";
pub const WEBUI_V2_PATTERN_OPERATOR_SETUP: &str = "/api/webchat/v2/operator/setup";
pub const WEBUI_V2_PATTERN_OPERATOR_CONFIG: &str = "/api/webchat/v2/operator/config";
pub const WEBUI_V2_PATTERN_OPERATOR_CONFIG_KEY: &str = "/api/webchat/v2/operator/config/{key}";
pub const WEBUI_V2_PATTERN_OPERATOR_CONFIG_VALIDATE: &str =
    "/api/webchat/v2/operator/config/validate";
pub const WEBUI_V2_PATTERN_OPERATOR_DIAGNOSTICS: &str = "/api/webchat/v2/operator/diagnostics";
pub const WEBUI_V2_PATTERN_OPERATOR_STATUS: &str = "/api/webchat/v2/operator/status";
pub const WEBUI_V2_PATTERN_OPERATOR_LOGS: &str = "/api/webchat/v2/operator/logs";
pub const WEBUI_V2_PATTERN_OPERATOR_SERVICE_LIFECYCLE: &str = "/api/webchat/v2/operator/service";
pub const WEBUI_V2_PATTERN_LIST_PROJECT_FILES: &str = "/api/webchat/v2/threads/{thread_id}/files";
pub const WEBUI_V2_PATTERN_STAT_PROJECT_FILE: &str =
    "/api/webchat/v2/threads/{thread_id}/files/stat";
pub const WEBUI_V2_PATTERN_READ_PROJECT_FILE: &str =
    "/api/webchat/v2/threads/{thread_id}/files/content";
pub const WEBUI_V2_PATTERN_LIST_FS_MOUNTS: &str = "/api/webchat/v2/fs/mounts";
pub const WEBUI_V2_PATTERN_BROWSE_FS_DIR: &str = "/api/webchat/v2/fs/list";
pub const WEBUI_V2_PATTERN_STAT_FS_PATH: &str = "/api/webchat/v2/fs/stat";
pub const WEBUI_V2_PATTERN_READ_FS_FILE: &str = "/api/webchat/v2/fs/content";
pub const WEBUI_V2_PATTERN_LIST_PROJECTS: &str = "/api/webchat/v2/projects";
pub const WEBUI_V2_PATTERN_CREATE_PROJECT: &str = "/api/webchat/v2/projects";
pub const WEBUI_V2_PATTERN_PROJECT_DETAIL: &str = "/api/webchat/v2/projects/{project_id}";
pub const WEBUI_V2_PATTERN_PROJECT_MEMBERS: &str = "/api/webchat/v2/projects/{project_id}/members";
pub const WEBUI_V2_PATTERN_PROJECT_MEMBER_DETAIL: &str =
    "/api/webchat/v2/projects/{project_id}/members/{user_id}";

/// Return the canonical [`IngressRouteDescriptor`] set for the WebChat v2
/// beta route surface.
///
/// Host composition calls this once at startup, validates the descriptors
/// against its own mount table, and refuses to bind any route whose policy
/// the host cannot enforce.
pub fn webui_v2_routes() -> Vec<IngressRouteDescriptor> {
    vec![
        get_session_descriptor(),
        create_thread_descriptor(),
        delete_thread_descriptor(),
        send_message_descriptor(),
        list_threads_descriptor(),
        get_timeline_descriptor(),
        get_attachment_descriptor(),
        stream_events_descriptor(),
        stream_events_ws_descriptor(),
        cancel_run_descriptor(),
        resolve_gate_descriptor(),
        list_automations_descriptor(),
        pause_automation_descriptor(),
        resume_automation_descriptor(),
        trace_credits_descriptor(),
        authorize_trace_hold_descriptor(),
        get_outbound_preferences_descriptor(),
        set_outbound_preferences_descriptor(),
        list_outbound_delivery_targets_descriptor(),
        list_connectable_channels_descriptor(),
        list_extensions_descriptor(),
        list_extension_registry_descriptor(),
        install_extension_descriptor(),
        activate_extension_descriptor(),
        remove_extension_descriptor(),
        get_extension_setup_descriptor(),
        setup_extension_descriptor(),
        list_skills_descriptor(),
        search_skills_descriptor(),
        install_skill_descriptor(),
        get_skill_descriptor(),
        update_skill_descriptor(),
        remove_skill_descriptor(),
        set_skill_auto_activate_descriptor(),
        set_auto_activate_learned_descriptor(),
        get_llm_config_descriptor(),
        upsert_llm_provider_descriptor(),
        delete_llm_provider_descriptor(),
        set_active_llm_descriptor(),
        test_llm_connection_descriptor(),
        list_llm_models_descriptor(),
        start_nearai_login_descriptor(),
        complete_nearai_wallet_login_descriptor(),
        start_codex_login_descriptor(),
        operator_get_setup_descriptor(),
        operator_run_setup_descriptor(),
        operator_list_config_descriptor(),
        operator_get_config_key_descriptor(),
        operator_set_config_key_descriptor(),
        operator_validate_config_descriptor(),
        operator_diagnostics_descriptor(),
        operator_status_descriptor(),
        operator_logs_descriptor(),
        operator_service_lifecycle_descriptor(),
        list_project_files_descriptor(),
        stat_project_file_descriptor(),
        read_project_file_descriptor(),
        list_fs_mounts_descriptor(),
        browse_fs_dir_descriptor(),
        stat_fs_path_descriptor(),
        read_fs_file_descriptor(),
        list_projects_descriptor(),
        create_project_descriptor(),
        get_project_descriptor(),
        update_project_descriptor(),
        delete_project_descriptor(),
        list_project_members_descriptor(),
        add_project_member_descriptor(),
        update_project_member_descriptor(),
        remove_project_member_descriptor(),
    ]
}

/// Returns whether a route id belongs to the legacy operator-wide LLM config surface.
///
/// Prefer [`is_webui_v2_operator_webui_config_route_id`] for host route gating;
/// this older predicate intentionally excludes newer `operator/*` routes.
#[deprecated(
    note = "Use `is_webui_v2_operator_webui_config_route_id`; this predicate misses the operator/* routes."
)]
pub fn is_webui_v2_llm_config_route_id(route_id: &str) -> bool {
    matches!(
        route_id,
        WEBUI_V2_ROUTE_GET_LLM_CONFIG
            | WEBUI_V2_ROUTE_UPSERT_LLM_PROVIDER
            | WEBUI_V2_ROUTE_DELETE_LLM_PROVIDER
            | WEBUI_V2_ROUTE_SET_ACTIVE_LLM
            | WEBUI_V2_ROUTE_TEST_LLM_CONNECTION
            | WEBUI_V2_ROUTE_LIST_LLM_MODELS
            | WEBUI_V2_ROUTE_START_NEARAI_LOGIN
            | WEBUI_V2_ROUTE_COMPLETE_NEARAI_WALLET_LOGIN
            | WEBUI_V2_ROUTE_START_CODEX_LOGIN
    )
}

/// Returns whether a route id belongs to any operator-wide WebUI config surface.
#[allow(deprecated)]
pub fn is_webui_v2_operator_webui_config_route_id(route_id: &str) -> bool {
    is_webui_v2_llm_config_route_id(route_id)
        || matches!(
            route_id,
            WEBUI_V2_ROUTE_OPERATOR_GET_SETUP
                | WEBUI_V2_ROUTE_OPERATOR_RUN_SETUP
                | WEBUI_V2_ROUTE_OPERATOR_LIST_CONFIG
                | WEBUI_V2_ROUTE_OPERATOR_GET_CONFIG_KEY
                | WEBUI_V2_ROUTE_OPERATOR_SET_CONFIG_KEY
                | WEBUI_V2_ROUTE_OPERATOR_VALIDATE_CONFIG
                | WEBUI_V2_ROUTE_OPERATOR_DIAGNOSTICS
                | WEBUI_V2_ROUTE_OPERATOR_STATUS
                | WEBUI_V2_ROUTE_OPERATOR_LOGS
                | WEBUI_V2_ROUTE_OPERATOR_SERVICE_LIFECYCLE
        )
}

fn get_session_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_SESSION,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_GET_SESSION,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn create_thread_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_CREATE_THREAD,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_CREATE_THREAD,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn send_message_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SEND_MESSAGE,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_SEND_MESSAGE,
        mutation_policy(
            // Message bodies carry user text plus optional base64-encoded inline
            // attachments. 14 MiB matches the gateway-wide body budget and covers
            // base64 of the 10 MiB decoded per-message attachment cap (the facade
            // enforces the 5 MiB-per-file / 10 MiB-total decoded budgets).
            body_limit_kib(14 * 1024),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::TurnCoordinator,
        ),
    )
}

fn delete_thread_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_DELETE_THREAD,
        NetworkMethod::Delete,
        WEBUI_V2_PATTERN_DELETE_THREAD,
        mutation_policy(
            BodyLimitPolicy::NoBody,
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn list_project_files_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_PROJECT_FILES,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_PROJECT_FILES,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn stat_project_file_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_STAT_PROJECT_FILE,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_STAT_PROJECT_FILE,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn read_project_file_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_READ_PROJECT_FILE,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_READ_PROJECT_FILE,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn list_fs_mounts_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_FS_MOUNTS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_FS_MOUNTS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn list_projects_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_PROJECTS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_PROJECTS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn browse_fs_dir_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_BROWSE_FS_DIR,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_BROWSE_FS_DIR,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn create_project_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_CREATE_PROJECT,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_CREATE_PROJECT,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn get_project_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_PROJECT,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_PROJECT_DETAIL,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn stat_fs_path_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_STAT_FS_PATH,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_STAT_FS_PATH,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn update_project_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_UPDATE_PROJECT,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_PROJECT_DETAIL,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn delete_project_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_DELETE_PROJECT,
        NetworkMethod::Delete,
        WEBUI_V2_PATTERN_PROJECT_DETAIL,
        mutation_policy(
            BodyLimitPolicy::NoBody,
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn list_project_members_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_PROJECT_MEMBERS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_PROJECT_MEMBERS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn read_fs_file_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_READ_FS_FILE,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_READ_FS_FILE,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn add_project_member_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_ADD_PROJECT_MEMBER,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_PROJECT_MEMBERS,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn update_project_member_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_UPDATE_PROJECT_MEMBER,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_PROJECT_MEMBER_DETAIL,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn remove_project_member_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_REMOVE_PROJECT_MEMBER,
        NetworkMethod::Delete,
        WEBUI_V2_PATTERN_PROJECT_MEMBER_DETAIL,
        mutation_policy(
            BodyLimitPolicy::NoBody,
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn get_timeline_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_TIMELINE,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_GET_TIMELINE,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn get_attachment_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_ATTACHMENT,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_GET_ATTACHMENT,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            // Reads workspace-backed attachment bytes through the product
            // facade — more than a projection read, so the effect path is
            // ProductWorkflow to keep the fail-closed ingress boundary honest.
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn stream_events_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_STREAM_EVENTS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_STREAM_EVENTS,
        read_policy(
            stream_rate_limit(),
            AuditTraceClass::StreamingSubscription,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::Sse,
        ),
    )
}

fn cancel_run_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_CANCEL_RUN,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_CANCEL_RUN,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::TurnCoordinator,
        ),
    )
}

fn resolve_gate_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_RESOLVE_GATE,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_RESOLVE_GATE,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::TurnCoordinator,
        ),
    )
}

fn list_threads_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_THREADS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_THREADS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn stream_events_ws_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_STREAM_EVENTS_WS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_STREAM_EVENTS_WS,
        ws_read_policy(
            stream_rate_limit(),
            AuditTraceClass::StreamingSubscription,
            AllowedEffectPath::ProjectionOnly,
        ),
    )
}

fn list_automations_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_AUTOMATIONS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_AUTOMATIONS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn pause_automation_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_PAUSE_AUTOMATION,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_PAUSE_AUTOMATION,
        mutation_policy(
            BodyLimitPolicy::NoBody,
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn resume_automation_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_RESUME_AUTOMATION,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_RESUME_AUTOMATION,
        mutation_policy(
            BodyLimitPolicy::NoBody,
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn trace_credits_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_TRACE_CREDITS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_TRACE_CREDITS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn authorize_trace_hold_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_TRACE_HOLD_AUTHORIZE,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_TRACE_HOLD_AUTHORIZE,
        mutation_policy(
            // The submission id is in the path; no request body.
            BodyLimitPolicy::NoBody,
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn get_outbound_preferences_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_OUTBOUND_PREFERENCES,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_OUTBOUND_PREFERENCES,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn set_outbound_preferences_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SET_OUTBOUND_PREFERENCES,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_OUTBOUND_PREFERENCES,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn list_outbound_delivery_targets_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_OUTBOUND_DELIVERY_TARGETS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_OUTBOUND_DELIVERY_TARGETS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn list_connectable_channels_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_CONNECTABLE_CHANNELS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_CONNECTABLE_CHANNELS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn list_extensions_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_EXTENSIONS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_EXTENSIONS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn list_extension_registry_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_EXTENSION_REGISTRY,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_EXTENSION_REGISTRY,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn install_extension_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_INSTALL_EXTENSION,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_INSTALL_EXTENSION,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn activate_extension_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_ACTIVATE_EXTENSION,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_ACTIVATE_EXTENSION,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn remove_extension_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_REMOVE_EXTENSION,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_REMOVE_EXTENSION,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn get_extension_setup_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_EXTENSION_SETUP,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_SETUP_EXTENSION,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn setup_extension_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SETUP_EXTENSION,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_SETUP_EXTENSION,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn list_skills_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_SKILLS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_SKILLS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn search_skills_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SEARCH_SKILLS,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_SEARCH_SKILLS,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn install_skill_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_INSTALL_SKILL,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_INSTALL_SKILL,
        mutation_policy(
            body_limit_kib(128),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn get_skill_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_SKILL,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_SKILL_DETAIL,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
            StreamingMode::None,
        ),
    )
}

fn update_skill_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_UPDATE_SKILL,
        NetworkMethod::Put,
        WEBUI_V2_PATTERN_SKILL_DETAIL,
        mutation_policy(
            body_limit_kib(128),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn remove_skill_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_REMOVE_SKILL,
        NetworkMethod::Delete,
        WEBUI_V2_PATTERN_SKILL_DETAIL,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn set_skill_auto_activate_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SET_SKILL_AUTO_ACTIVATE,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_SET_SKILL_AUTO_ACTIVATE,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn set_auto_activate_learned_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SET_AUTO_ACTIVATE_LEARNED,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_SET_AUTO_ACTIVATE_LEARNED,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn get_llm_config_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_LLM_CONFIG,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_GET_LLM_CONFIG,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn upsert_llm_provider_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_UPSERT_LLM_PROVIDER,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_UPSERT_LLM_PROVIDER,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn delete_llm_provider_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_DELETE_LLM_PROVIDER,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_DELETE_LLM_PROVIDER,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn set_active_llm_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SET_ACTIVE_LLM,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_SET_ACTIVE_LLM,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn test_llm_connection_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_TEST_LLM_CONNECTION,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_TEST_LLM_CONNECTION,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn list_llm_models_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_LLM_MODELS,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_LIST_LLM_MODELS,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn start_nearai_login_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_START_NEARAI_LOGIN,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_START_NEARAI_LOGIN,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn complete_nearai_wallet_login_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_COMPLETE_NEARAI_WALLET_LOGIN,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_COMPLETE_NEARAI_WALLET_LOGIN,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn start_codex_login_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_START_CODEX_LOGIN,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_START_CODEX_LOGIN,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn operator_get_setup_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_GET_SETUP,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_OPERATOR_SETUP,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn operator_run_setup_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_RUN_SETUP,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_OPERATOR_SETUP,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn operator_list_config_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_LIST_CONFIG,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_OPERATOR_CONFIG,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn operator_get_config_key_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_GET_CONFIG_KEY,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_OPERATOR_CONFIG_KEY,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn operator_set_config_key_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_SET_CONFIG_KEY,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_OPERATOR_CONFIG_KEY,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn operator_validate_config_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_VALIDATE_CONFIG,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_OPERATOR_CONFIG_VALIDATE,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn operator_diagnostics_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_DIAGNOSTICS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_OPERATOR_DIAGNOSTICS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn operator_status_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_STATUS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_OPERATOR_STATUS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn operator_logs_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_LOGS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_OPERATOR_LOGS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn operator_service_lifecycle_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_OPERATOR_SERVICE_LIFECYCLE,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_OPERATOR_SERVICE_LIFECYCLE,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn ws_read_policy(
    rate_limit: RateLimitPolicy,
    audit: AuditTraceClass,
    effect_path: AllowedEffectPath,
) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit,
        cors: CorsPolicy::SameOriginOnly,
        // WS upgrade is gated by host composition's same-origin
        // check; declared here so the descriptor is the contract a
        // future allowlist-based deployment overrides.
        websocket_origin: WebSocketOriginPolicy::SameOriginRequired,
        streaming: StreamingMode::WebSocket,
        audit,
        effect_path,
    })
    .expect("webui v2 WS read policy must validate") // safety: combination LocalGateway + bearer + AuthenticatedCaller + WebSocket + SameOriginRequired is a permitted shape; other parts are crate-local constants
}

fn descriptor(
    route_id: &str,
    method: NetworkMethod,
    pattern: &str,
    policy: IngressPolicy,
) -> IngressRouteDescriptor {
    IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
        .expect("webui v2 route descriptor must validate at startup") // safety: route_id/pattern are crate-local literals known to satisfy IngressRouteId / IngressRoutePattern; policy is constructed by sibling helpers that validate their own inputs
}

fn mutation_policy(
    body_limit: BodyLimitPolicy,
    rate_limit: RateLimitPolicy,
    audit: AuditTraceClass,
    effect_path: AllowedEffectPath,
) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit,
        rate_limit,
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit,
        effect_path,
    })
    .expect("webui v2 mutation policy must validate") // safety: all parts are crate-local constants; the combination (LocalGateway + bearer required + AuthenticatedCaller + None streaming) is a permitted shape, locked in by the descriptor contract test
}

fn read_policy(
    rate_limit: RateLimitPolicy,
    audit: AuditTraceClass,
    effect_path: AllowedEffectPath,
    streaming: StreamingMode,
) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit,
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming,
        audit,
        effect_path,
    })
    .expect("webui v2 read policy must validate") // safety: streaming is either None or Sse (both permitted with bearer + AuthenticatedCaller); other parts are crate-local constants
}

fn bearer_required() -> IngressAuthPolicy {
    IngressAuthPolicy::Required {
        schemes: vec![IngressAuthScheme::BearerToken],
    }
}

fn body_limit_kib(kib: u64) -> BodyLimitPolicy {
    let bytes = kib
        .checked_mul(1024)
        .and_then(NonZeroU64::new)
        .expect("body limit must be non-zero"); // safety: all call sites pass crate-local positive constants (4, 16, 1024); overflow at u64 * 1024 is impossible for these
    BodyLimitPolicy::Limited { max_bytes: bytes }
}

fn mutation_rate_limit() -> RateLimitPolicy {
    rate_limit_per_caller(60, 60)
}

fn read_rate_limit() -> RateLimitPolicy {
    rate_limit_per_caller(120, 60)
}

fn stream_rate_limit() -> RateLimitPolicy {
    // Shared budget for the SSE (`stream_events`) and WebSocket
    // (`stream_events_ws`) routes. SSE sessions are long-lived; the
    // per-tenant/user concurrency cap (3 streams, enforced in
    // `WebUiV2State::SseCapacity`) does the real bounding. The
    // request-rate window here is just for burst protection against
    // reconnect storms.
    //
    // Set to 30/60s — the SSE route additionally accepts `?token=…`
    // because `EventSource` can't set headers, which leaks the
    // bearer into browser history, server access logs, and proxy
    // logs. Keeping the request rate higher than necessary widens
    // the replay surface for a logged token, so the budget is capped
    // at 2x a worst-case exponential-backoff reconnect cycle (≈ 1,
    // 2, 4, 8, 16, 32s per minute = 6 opens) rather than parity with
    // the mutation budget. The WS route doesn't carry the same
    // URL-token risk (headers + `WebSocketOriginPolicy::SameOriginRequired`),
    // but the lower limit costs it nothing — the same reconnect-storm
    // math applies, the same concurrency cap is the real load gate,
    // and using one helper for both keeps the descriptors aligned.
    rate_limit_per_caller(30, 60)
}

fn rate_limit_per_caller(max: u32, window_secs: u32) -> RateLimitPolicy {
    RateLimitPolicy::Limited {
        scope: RateLimitScope::PerCaller,
        max_requests: NonZeroU32::new(max).expect("max_requests must be non-zero"), // safety: all call sites pass crate-local positive constants (12, 60, 120)
        window_seconds: NonZeroU32::new(window_secs).expect("window_seconds must be non-zero"), // safety: all call sites pass crate-local positive constants (60)
    }
}
