//! Route descriptors for admin user lifecycle and managed-user resources.

use super::*;

pub const WEBUI_V2_ROUTE_ADMIN_LIST_USERS: &str = "webui.v2.admin.list_users";
pub const WEBUI_V2_ROUTE_ADMIN_CREATE_USER: &str = "webui.v2.admin.create_user";
pub const WEBUI_V2_ROUTE_ADMIN_CREATE_MANAGED_USER: &str = "webui.v2.admin.create_managed_user";
pub const WEBUI_V2_ROUTE_ADMIN_GET_USER: &str = "webui.v2.admin.get_user";
pub const WEBUI_V2_ROUTE_ADMIN_UPDATE_USER: &str = "webui.v2.admin.update_user";
pub const WEBUI_V2_ROUTE_ADMIN_DELETE_USER: &str = "webui.v2.admin.delete_user";
pub const WEBUI_V2_ROUTE_ADMIN_SET_USER_STATUS: &str = "webui.v2.admin.set_user_status";
pub const WEBUI_V2_ROUTE_ADMIN_SET_USER_ROLE: &str = "webui.v2.admin.set_user_role";
pub const WEBUI_V2_ROUTE_ADMIN_LIST_USER_SECRETS: &str = "webui.v2.admin.list_user_secrets";
pub const WEBUI_V2_ROUTE_ADMIN_PUT_USER_SECRET: &str = "webui.v2.admin.put_user_secret";
pub const WEBUI_V2_ROUTE_ADMIN_DELETE_USER_SECRET: &str = "webui.v2.admin.delete_user_secret";

pub const WEBUI_V2_PATTERN_ADMIN_USERS: &str = "/api/webchat/v2/admin/users";
pub const WEBUI_V2_PATTERN_ADMIN_MANAGED_USERS: &str = "/api/webchat/v2/admin/agents";
pub const WEBUI_V2_PATTERN_ADMIN_USER: &str = "/api/webchat/v2/admin/users/{user_id}";
pub const WEBUI_V2_PATTERN_ADMIN_USER_STATUS: &str = "/api/webchat/v2/admin/users/{user_id}/status";
pub const WEBUI_V2_PATTERN_ADMIN_USER_ROLE: &str = "/api/webchat/v2/admin/users/{user_id}/role";
pub const WEBUI_V2_PATTERN_ADMIN_USER_SECRETS: &str =
    "/api/webchat/v2/admin/users/{user_id}/secrets";
pub const WEBUI_V2_PATTERN_ADMIN_USER_SECRET: &str =
    "/api/webchat/v2/admin/users/{user_id}/secrets/{handle}";

pub(super) fn descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_LIST_USERS,
            NetworkMethod::Get,
            WEBUI_V2_PATTERN_ADMIN_USERS,
            read_policy(
                read_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
                StreamingMode::None,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_CREATE_USER,
            NetworkMethod::Post,
            WEBUI_V2_PATTERN_ADMIN_USERS,
            mutation_policy(
                body_limit_kib(16),
                mutation_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_CREATE_MANAGED_USER,
            NetworkMethod::Post,
            WEBUI_V2_PATTERN_ADMIN_MANAGED_USERS,
            mutation_policy(
                body_limit_kib(16),
                mutation_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_GET_USER,
            NetworkMethod::Get,
            WEBUI_V2_PATTERN_ADMIN_USER,
            read_policy(
                read_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
                StreamingMode::None,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_UPDATE_USER,
            NetworkMethod::Patch,
            WEBUI_V2_PATTERN_ADMIN_USER,
            mutation_policy(
                body_limit_kib(16),
                mutation_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_DELETE_USER,
            NetworkMethod::Delete,
            WEBUI_V2_PATTERN_ADMIN_USER,
            mutation_policy(
                BodyLimitPolicy::NoBody,
                mutation_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_SET_USER_STATUS,
            NetworkMethod::Post,
            WEBUI_V2_PATTERN_ADMIN_USER_STATUS,
            mutation_policy(
                body_limit_kib(4),
                mutation_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_SET_USER_ROLE,
            NetworkMethod::Post,
            WEBUI_V2_PATTERN_ADMIN_USER_ROLE,
            mutation_policy(
                body_limit_kib(4),
                mutation_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_LIST_USER_SECRETS,
            NetworkMethod::Get,
            WEBUI_V2_PATTERN_ADMIN_USER_SECRETS,
            read_policy(
                read_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
                StreamingMode::None,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_PUT_USER_SECRET,
            NetworkMethod::Put,
            WEBUI_V2_PATTERN_ADMIN_USER_SECRET,
            mutation_policy(
                body_limit_kib(16),
                mutation_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
            ),
        ),
        descriptor(
            WEBUI_V2_ROUTE_ADMIN_DELETE_USER_SECRET,
            NetworkMethod::Delete,
            WEBUI_V2_PATTERN_ADMIN_USER_SECRET,
            mutation_policy(
                BodyLimitPolicy::NoBody,
                mutation_rate_limit(),
                AuditTraceClass::UserAction,
                AllowedEffectPath::ProductWorkflow,
            ),
        ),
    ]
}
