use std::net::IpAddr;
use std::time::Duration;

use crate::security::outbound_trust::{
    OutboundTrustDecision, OutboundTrustRequestContext, OutboundTrustResolver, OutboundTrustSurface,
};
use crate::tools::mcp::config::{McpServerConfig, is_localhost_url};

pub(crate) use crate::security::outbound_trust::is_dangerous_ip;

pub(crate) fn resolve_mcp_outbound_trust_decision(
    resolver: &OutboundTrustResolver,
    server_config: &McpServerConfig,
    url: &str,
) -> OutboundTrustDecision {
    resolver.resolve(&OutboundTrustRequestContext {
        surface: OutboundTrustSurface::McpServer,
        extension_name: &server_config.name,
        url,
        declared_policy_ids: server_config.declared_outbound_trust_policy_ids(),
    })
}

pub(crate) fn build_mcp_http_client(allow_invalid_tls: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none());

    if allow_invalid_tls {
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder
        .build()
        .map_err(|e| format!("Failed to create MCP HTTP client: {e}"))
}

pub(crate) async fn validate_mcp_url(url: &str, allow_private_network: bool) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;

    let scheme = parsed.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(format!("Unsupported scheme: {scheme}"));
    }

    if scheme == "http" {
        if !is_localhost_url(url) {
            let host = parsed.host_str().unwrap_or("");
            return Err(format!(
                "HTTP is only allowed for localhost; use HTTPS for '{host}'"
            ));
        }

        return Ok(());
    }

    if allow_private_network {
        return Ok(());
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    if let Ok(ip) = host.parse::<IpAddr>()
        && is_dangerous_ip(ip)
    {
        return Err(format!("URL points to a restricted IP address: {host}"));
    }

    if host.parse::<IpAddr>().is_err() {
        let addr = format!("{}:{}", host, parsed.port_or_known_default().unwrap_or(443));
        match tokio::net::lookup_host(&addr).await {
            Ok(addrs) => {
                for socket_addr in addrs {
                    if is_dangerous_ip(socket_addr.ip()) {
                        return Err(format!(
                            "URL hostname '{}' resolves to restricted IP address: {}",
                            host,
                            socket_addr.ip()
                        ));
                    }
                }
            }
            Err(e) => {
                return Err(format!("DNS resolution failed for '{host}': {e}"));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::security::outbound_trust::{
        OutboundTrustConfig, OutboundTrustPolicy, OutboundTrustResolver, OutboundTrustRisk,
        OutboundTrustSurface, OutboundTrustTarget,
    };
    use crate::testing::tls::SelfSignedHttpsServer;
    use crate::tools::mcp::config::McpServerConfig;

    #[tokio::test]
    async fn test_outbound_trust_allows_private_self_signed_https_for_mcp_servers() {
        let server = SelfSignedHttpsServer::start(r#"{"ok":true}"#).await;
        let url = server.url("/api/status");

        let config = McpServerConfig::new("test-mcp", &url)
            .with_outbound_trust_policy_ids(vec!["corp-mcp".to_string()]);

        let tls_err = super::build_mcp_http_client(false)
            .expect("client")
            .get(&url)
            .send()
            .await
            .expect_err("self-signed endpoint should fail without invalid-tls trust");
        assert!(
            tls_err.is_request() || tls_err.is_connect() || tls_err.is_body(),
            "expected request failure for untrusted TLS, got: {tls_err}"
        );

        let denied = super::validate_mcp_url(&url, false).await;
        assert!(denied.is_err(), "expected private-network denial");

        let resolver = OutboundTrustResolver::new(OutboundTrustConfig {
            enabled: true,
            policies: vec![OutboundTrustPolicy {
                id: "corp-mcp".to_string(),
                display_name: "corp-mcp".to_string(),
                description: None,
                enabled: true,
                allowed_surfaces: vec![OutboundTrustSurface::McpServer],
                allowed_risks: vec![
                    OutboundTrustRisk::AllowInvalidTls,
                    OutboundTrustRisk::AllowPrivateNetwork,
                ],
                targets: vec![OutboundTrustTarget {
                    host: "127.0.0.1".to_string(),
                    port: Some(server.port()),
                    path_prefix: Some("/api".to_string()),
                }],
            }],
        });

        let decision = super::resolve_mcp_outbound_trust_decision(&resolver, &config, &url);
        assert!(decision.allow_invalid_tls);
        assert!(decision.allow_private_network);

        super::validate_mcp_url(&url, decision.allow_private_network)
            .await
            .expect("private-network trust should allow MCP URL");

        let response = super::build_mcp_http_client(decision.allow_invalid_tls)
            .expect("trusted client")
            .get(&url)
            .send()
            .await
            .expect("trusted request should succeed");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }
}
