use ironclaw::security::outbound_trust::{
    OutboundTrustConfig, OutboundTrustDecision, OutboundTrustPolicy, OutboundTrustRequestContext,
    OutboundTrustResolver, OutboundTrustRisk, OutboundTrustSurface, OutboundTrustTarget,
};

fn decision(
    resolver: &OutboundTrustResolver,
    surface: OutboundTrustSurface,
    extension_name: &str,
    url: &str,
    declared_policy_ids: &[String],
) -> OutboundTrustDecision {
    resolver.resolve(&OutboundTrustRequestContext {
        surface,
        extension_name,
        url,
        declared_policy_ids,
    })
}

fn policy(
    id: &str,
    surfaces: Vec<OutboundTrustSurface>,
    risks: Vec<OutboundTrustRisk>,
    targets: Vec<OutboundTrustTarget>,
) -> OutboundTrustPolicy {
    OutboundTrustPolicy {
        id: id.to_string(),
        display_name: "test".to_string(),
        description: Some("test policy".to_string()),
        enabled: true,
        allowed_surfaces: surfaces,
        allowed_risks: risks,
        targets,
    }
}

fn target(host: &str) -> OutboundTrustTarget {
    OutboundTrustTarget {
        host: host.to_string(),
        port: None,
        path_prefix: None,
    }
}

#[test]
fn matches_exact_host_and_declared_policy() {
    let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
        enabled: true,
        policies: vec![policy(
            "corp-internal-api",
            vec![OutboundTrustSurface::WasmTool],
            vec![OutboundTrustRisk::AllowInvalidTls],
            vec![target("internal-api.example.test")],
        )],
    });

    let result = decision(
        &resolver,
        OutboundTrustSurface::WasmTool,
        "internal_tool",
        "https://internal-api.example.test/api/status",
        &["corp-internal-api".to_string()],
    );

    assert_eq!(
        result.matched_policy_id.as_deref(),
        Some("corp-internal-api")
    );
    assert!(result.allow_invalid_tls);
    assert!(!result.allow_private_network);
}

#[test]
fn requires_declared_policy_id_even_when_target_matches() {
    let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
        enabled: true,
        policies: vec![policy(
            "corp-internal-api",
            vec![OutboundTrustSurface::WasmTool],
            vec![OutboundTrustRisk::AllowInvalidTls],
            vec![target("internal-api.example.test")],
        )],
    });

    let result = decision(
        &resolver,
        OutboundTrustSurface::WasmTool,
        "internal_tool",
        "https://internal-api.example.test/api/status",
        &[],
    );

    assert_eq!(result.matched_policy_id, None);
    assert!(!result.allow_invalid_tls);
    assert!(!result.allow_private_network);
}

#[test]
fn respects_surface_target_port_and_path_prefix() {
    let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
        enabled: true,
        policies: vec![OutboundTrustPolicy {
            id: "corp-mcp".to_string(),
            display_name: "corp-mcp".to_string(),
            description: None,
            enabled: true,
            allowed_surfaces: vec![OutboundTrustSurface::McpServer],
            allowed_risks: vec![OutboundTrustRisk::AllowPrivateNetwork],
            targets: vec![OutboundTrustTarget {
                host: "10.0.0.25".to_string(),
                port: Some(8443),
                path_prefix: Some("/rpc".to_string()),
            }],
        }],
    });

    let allowed = decision(
        &resolver,
        OutboundTrustSurface::McpServer,
        "corp-gateway",
        "https://10.0.0.25:8443/rpc/tools/list",
        &["corp-mcp".to_string()],
    );
    assert_eq!(allowed.matched_policy_id.as_deref(), Some("corp-mcp"));
    assert!(!allowed.allow_invalid_tls);
    assert!(allowed.allow_private_network);

    let wrong_surface = decision(
        &resolver,
        OutboundTrustSurface::WasmChannel,
        "corp-gateway",
        "https://10.0.0.25:8443/rpc/tools/list",
        &["corp-mcp".to_string()],
    );
    assert_eq!(wrong_surface.matched_policy_id, None);

    let wrong_port = decision(
        &resolver,
        OutboundTrustSurface::McpServer,
        "corp-gateway",
        "https://10.0.0.25:443/rpc/tools/list",
        &["corp-mcp".to_string()],
    );
    assert_eq!(wrong_port.matched_policy_id, None);

    let wrong_path = decision(
        &resolver,
        OutboundTrustSurface::McpServer,
        "corp-gateway",
        "https://10.0.0.25:8443/other",
        &["corp-mcp".to_string()],
    );
    assert_eq!(wrong_path.matched_policy_id, None);
}

#[test]
fn disabled_config_or_policy_grants_no_exceptions() {
    let disabled_globally = OutboundTrustResolver::new(OutboundTrustConfig {
        enabled: false,
        policies: vec![policy(
            "corp-internal-api",
            vec![OutboundTrustSurface::WasmTool],
            vec![
                OutboundTrustRisk::AllowInvalidTls,
                OutboundTrustRisk::AllowPrivateNetwork,
            ],
            vec![target("10.42.0.15")],
        )],
    });
    let global_result = decision(
        &disabled_globally,
        OutboundTrustSurface::WasmTool,
        "internal_tool",
        "https://10.42.0.15/api/status",
        &["corp-internal-api".to_string()],
    );
    assert_eq!(global_result.matched_policy_id, None);

    let disabled_policy = OutboundTrustResolver::new(OutboundTrustConfig {
        enabled: true,
        policies: vec![OutboundTrustPolicy {
            enabled: false,
            ..policy(
                "corp-internal-api",
                vec![OutboundTrustSurface::WasmTool],
                vec![OutboundTrustRisk::AllowPrivateNetwork],
                vec![target("10.42.0.15")],
            )
        }],
    });
    let policy_result = decision(
        &disabled_policy,
        OutboundTrustSurface::WasmTool,
        "internal_tool",
        "https://10.42.0.15/api/status",
        &["corp-internal-api".to_string()],
    );
    assert_eq!(policy_result.matched_policy_id, None);
}
