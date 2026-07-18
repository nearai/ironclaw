//! Contract tests for operator-wide WebUI route predicates.

use ironclaw_webui::webui_v2::{
    WEBUI_V2_ROUTE_CREATE_THREAD, WEBUI_V2_ROUTE_IMPORT_EXTENSION,
    WEBUI_V2_ROUTE_INSTALL_EXTENSION, WEBUI_V2_ROUTE_LIST_SETTINGS_TOOLS,
    WEBUI_V2_ROUTE_OPERATOR_GET_CONFIG_KEY, WEBUI_V2_ROUTE_OPERATOR_LIST_CONFIG,
    WEBUI_V2_ROUTE_OPERATOR_LOGS, WEBUI_V2_ROUTE_OPERATOR_SET_CONFIG_KEY,
    WEBUI_V2_ROUTE_OPERATOR_STATUS, WEBUI_V2_ROUTE_SET_SETTINGS_TOOL_PERMISSION,
    WEBUI_V2_ROUTE_SET_SETTINGS_TOOLS_AUTO_APPROVE, is_webui_v2_operator_webui_config_route_id,
};

#[test]
fn operator_route_predicate_matches_operator_config_routes_only() {
    assert!(is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_OPERATOR_STATUS
    ));
    assert!(is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_OPERATOR_LIST_CONFIG
    ));
    assert!(is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_OPERATOR_GET_CONFIG_KEY
    ));
    assert!(is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_OPERATOR_SET_CONFIG_KEY
    ));
    assert!(is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_OPERATOR_LOGS
    ));
    // #5499 review finding #1: the admin-only zip import route is part of the
    // operator surface — composition strips and pre-gates routes from this
    // predicate, so omitting it exposes the route (and its pre-auth body
    // buffering) in deployments that mount no operator surface.
    assert!(is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_IMPORT_EXTENSION
    ));
    // Install stays a regular authenticated-user route: it references an
    // already-cataloged package by ref and uploads nothing.
    assert!(!is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_INSTALL_EXTENSION
    ));
    assert!(!is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_CREATE_THREAD
    ));
    assert!(!is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_LIST_SETTINGS_TOOLS
    ));
    assert!(!is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_SET_SETTINGS_TOOLS_AUTO_APPROVE
    ));
    assert!(!is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_SET_SETTINGS_TOOL_PERMISSION
    ));
}
