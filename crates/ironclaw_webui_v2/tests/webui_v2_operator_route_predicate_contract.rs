//! Contract tests for operator-wide WebUI route predicates.

use ironclaw_webui_v2::{
    WEBUI_V2_ROUTE_CREATE_THREAD, WEBUI_V2_ROUTE_OPERATOR_GET_CONFIG_KEY,
    WEBUI_V2_ROUTE_OPERATOR_LIST_CONFIG, WEBUI_V2_ROUTE_OPERATOR_SET_CONFIG_KEY,
    WEBUI_V2_ROUTE_OPERATOR_STATUS, is_webui_v2_operator_webui_config_route_id,
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
    assert!(!is_webui_v2_operator_webui_config_route_id(
        WEBUI_V2_ROUTE_CREATE_THREAD
    ));
}
