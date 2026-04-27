use ironclaw_host_api::{
    InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTarget, NetworkTargetPattern,
    ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_network::{NetworkPolicyEnforcer, NetworkRequest, StaticNetworkPolicyEnforcer};

#[tokio::test]
async fn network_policy_allows_exact_scoped_target_without_executing_io() {
    let scope = sample_scope("tenant-a", "user-a");
    let policy = NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: Some(443),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    };
    let enforcer = StaticNetworkPolicyEnforcer::new(policy);

    let permit = enforcer
        .authorize(NetworkRequest {
            scope: scope.clone(),
            target: target(NetworkScheme::Https, "api.example.test", Some(443)),
            method: NetworkMethod::Post,
            estimated_bytes: Some(512),
        })
        .await
        .unwrap();

    assert_eq!(permit.scope, scope);
    assert_eq!(permit.target.host, "api.example.test");
    assert_eq!(permit.method, NetworkMethod::Post);
    assert_eq!(permit.estimated_bytes, Some(512));
}

#[tokio::test]
async fn network_policy_supports_one_label_wildcard_hosts_only() {
    let enforcer = StaticNetworkPolicyEnforcer::new(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "*.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    });
    let scope = sample_scope("tenant-a", "user-a");

    enforcer
        .authorize(NetworkRequest {
            scope: scope.clone(),
            target: target(NetworkScheme::Https, "api.example.test", None),
            method: NetworkMethod::Get,
            estimated_bytes: Some(0),
        })
        .await
        .unwrap();

    let apex = enforcer
        .authorize(NetworkRequest {
            scope,
            target: target(NetworkScheme::Https, "example.test", None),
            method: NetworkMethod::Get,
            estimated_bytes: Some(0),
        })
        .await
        .unwrap_err();
    assert!(apex.is_target_denied());
}

#[tokio::test]
async fn network_policy_denies_scheme_host_port_and_egress_mismatches() {
    let enforcer = StaticNetworkPolicyEnforcer::new(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: Some(443),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10),
    });
    let scope = sample_scope("tenant-a", "user-a");

    let wrong_scheme = enforcer
        .authorize(NetworkRequest {
            scope: scope.clone(),
            target: target(NetworkScheme::Http, "api.example.test", Some(443)),
            method: NetworkMethod::Get,
            estimated_bytes: Some(1),
        })
        .await
        .unwrap_err();
    assert!(wrong_scheme.is_target_denied());

    let wrong_host = enforcer
        .authorize(NetworkRequest {
            scope: scope.clone(),
            target: target(NetworkScheme::Https, "evil.example.test", Some(443)),
            method: NetworkMethod::Get,
            estimated_bytes: Some(1),
        })
        .await
        .unwrap_err();
    assert!(wrong_host.is_target_denied());

    let wrong_port = enforcer
        .authorize(NetworkRequest {
            scope: scope.clone(),
            target: target(NetworkScheme::Https, "api.example.test", Some(8443)),
            method: NetworkMethod::Get,
            estimated_bytes: Some(1),
        })
        .await
        .unwrap_err();
    assert!(wrong_port.is_target_denied());

    let too_large = enforcer
        .authorize(NetworkRequest {
            scope,
            target: target(NetworkScheme::Https, "api.example.test", Some(443)),
            method: NetworkMethod::Post,
            estimated_bytes: Some(11),
        })
        .await
        .unwrap_err();
    assert!(too_large.is_egress_limit_exceeded());
}

#[tokio::test]
async fn network_policy_denies_literal_private_loopback_and_link_local_ips() {
    let enforcer = StaticNetworkPolicyEnforcer::new(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: "127.0.0.1".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    });
    let scope = sample_scope("tenant-a", "user-a");

    let err = enforcer
        .authorize(NetworkRequest {
            scope,
            target: target(NetworkScheme::Http, "127.0.0.1", None),
            method: NetworkMethod::Get,
            estimated_bytes: Some(0),
        })
        .await
        .unwrap_err();

    assert!(err.is_private_target_denied());
    assert!(!format!("{err}").contains("/tmp"));
}

#[tokio::test]
async fn network_policy_is_fail_closed_without_allowed_targets() {
    let enforcer = StaticNetworkPolicyEnforcer::new(NetworkPolicy::default());
    let scope = sample_scope("tenant-a", "user-a");

    let err = enforcer
        .authorize(NetworkRequest {
            scope,
            target: target(NetworkScheme::Https, "api.example.test", None),
            method: NetworkMethod::Get,
            estimated_bytes: None,
        })
        .await
        .unwrap_err();

    assert!(err.is_target_denied());
}

fn target(scheme: NetworkScheme, host: &str, port: Option<u16>) -> NetworkTarget {
    NetworkTarget {
        scheme,
        host: host.to_string(),
        port,
    }
}

fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: None,
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}
