use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironclaw_host_api::{
    InvocationId, NetworkMethod, NetworkPolicy, NetworkTargetPattern, ResourceScope, TenantId,
    UserId,
};
use ironclaw_network::{
    DEFAULT_RESPONSE_BODY_LIMIT, NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest,
    NetworkHttpResponse, NetworkHttpTransport, NetworkResolver, NetworkTransportRequest,
    NetworkUsage, PolicyNetworkHttpEgress, ReqwestNetworkTransport,
};

#[test]
fn http_egress_authorizes_default_https_port_and_pins_resolved_ip() {
    let resolved_ips = vec![
        IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
        IpAddr::V4(Ipv4Addr::new(93, 184, 216, 35)),
    ];
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        usage: NetworkUsage {
            request_bytes: 0,
            response_bytes: 2,
            resolved_ip: None,
        },
    });
    let requests = transport.requests.clone();
    let egress = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(resolved_ips.clone()),
    );

    let response = egress
        .execute(NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1".to_string(),
            headers: vec![],
            body: vec![],
            policy: policy("api.example.test", Some(443), true, None),
            response_body_limit: Some(1024),
            timeout_ms: None,
        })
        .expect("default HTTPS port should satisfy a 443 policy");

    assert_eq!(response.usage.response_bytes, 2);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].resolved_ips, resolved_ips);
    assert_eq!(requests[0].response_body_limit, Some(1024));
}

#[test]
fn http_egress_forwards_timeout_to_transport() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        usage: NetworkUsage {
            request_bytes: 0,
            response_bytes: 2,
            resolved_ip: None,
        },
    });
    let requests = transport.requests.clone();
    let egress = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );

    egress
        .execute(NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1".to_string(),
            headers: vec![],
            body: vec![],
            policy: policy("api.example.test", Some(443), true, None),
            response_body_limit: Some(1024),
            timeout_ms: Some(250),
        })
        .expect("network response should be returned");

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout_ms, Some(250));
}

#[test]
fn http_egress_denies_private_resolved_host_before_transport() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: vec![],
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let egress = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))]),
    );

    let error = egress
        .execute(NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            policy: policy("api.example.test", Some(443), true, Some(1024)),
            response_body_limit: Some(1024),
            timeout_ms: None,
        })
        .expect_err("private resolved targets should fail closed");

    assert!(error.to_string().contains("private"));
    assert_eq!(error.request_bytes(), 0);
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn http_egress_counts_url_and_headers_in_policy_egress_estimate_before_transport() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: vec![],
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let egress = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );

    let error = egress
        .execute(NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/echo?query=large".to_string(),
            headers: vec![("x-trace".to_string(), "abcdef".to_string())],
            body: vec![],
            policy: policy("api.example.test", Some(443), true, Some(4)),
            response_body_limit: Some(1024),
            timeout_ms: None,
        })
        .expect_err("URL and headers must count toward egress policy estimates");

    assert!(matches!(error, NetworkHttpError::PolicyDenied { .. }));
    assert!(error.to_string().contains("egress estimate"));
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn http_egress_rejects_caller_provided_host_header_before_transport() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: vec![],
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let egress = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );

    let error = egress
        .execute(NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1".to_string(),
            headers: vec![("Host".to_string(), "evil.example.test".to_string())],
            body: vec![],
            policy: policy("api.example.test", Some(443), true, Some(1024)),
            response_body_limit: Some(1024),
            timeout_ms: None,
        })
        .expect_err("caller-provided Host should not be forwarded after URL policy validation");

    assert!(matches!(error, NetworkHttpError::PolicyDenied { .. }));
    assert!(error.to_string().contains("Host header"));
    assert_eq!(error.request_bytes(), 0);
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn http_egress_rejects_userinfo_url_before_transport() {
    let transport = RecordingTransport::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: vec![],
        usage: NetworkUsage::default(),
    });
    let requests = transport.requests.clone();
    let egress = PolicyNetworkHttpEgress::new_with_resolver(
        transport,
        StaticResolver::new(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
    );

    let error = egress
        .execute(NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://user:pass@api.example.test/v1".to_string(),
            headers: vec![],
            body: vec![],
            policy: policy("api.example.test", Some(443), true, Some(1024)),
            response_body_limit: Some(1024),
            timeout_ms: None,
        })
        .expect_err("userinfo credentials in URLs must fail before policy/DNS/transport");

    assert!(matches!(error, NetworkHttpError::InvalidUrl { .. }));
    assert!(error.to_string().contains("userinfo"));
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn reqwest_transport_does_not_follow_redirects() {
    let (url, server) = single_response_server(
        "HTTP/1.1 302 Found\r\nLocation: http://example.invalid/\r\nContent-Length: 0\r\n\r\n",
    );
    let egress = PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::new(Duration::from_secs(2)));

    let response = egress
        .execute(NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url,
            headers: vec![],
            body: vec![],
            policy: policy("127.0.0.1", None, false, None),
            response_body_limit: Some(1024),
            timeout_ms: None,
        })
        .expect("redirect responses should be returned, not followed");
    server.join().unwrap();

    assert_eq!(response.status, 302);
    assert_eq!(response.usage.request_bytes, 0);
    assert_eq!(response.usage.response_bytes, 0);
}

#[test]
fn reqwest_transport_uses_all_resolved_addresses_for_connection_fallback() {
    let (url, server) = single_response_server_for_host(
        "fallback.example.test",
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
    );
    let transport = ReqwestNetworkTransport::new(Duration::from_secs(2));

    let response = transport
        .execute(NetworkTransportRequest {
            method: NetworkMethod::Get,
            url,
            headers: vec![],
            body: vec![],
            resolved_ips: vec![
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            ],
            response_body_limit: Some(1024),
            timeout_ms: None,
        })
        .expect("transport should allow connector fallback across resolved addresses");
    server.join().unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"ok");
}

#[test]
fn reqwest_transport_enforces_streaming_response_limit_separately_from_request_bytes() {
    let (url, server) =
        single_response_server("HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabcdef");
    let egress = PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::new(Duration::from_secs(2)));

    let error = egress
        .execute(NetworkHttpRequest {
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url,
            headers: vec![],
            body: b"hello".to_vec(),
            policy: policy("127.0.0.1", None, false, Some(1024)),
            response_body_limit: Some(5),
            timeout_ms: None,
        })
        .expect_err("response body limit should stop reads after the limit");
    server.join().unwrap();

    assert!(matches!(error, NetworkHttpError::ResponseBodyLimit { .. }));
    assert_eq!(error.request_bytes(), 5);
    assert_eq!(error.response_bytes(), 6);
}

#[test]
fn reqwest_transport_clamps_oversized_explicit_response_limit_to_safe_default() {
    let body_len = DEFAULT_RESPONSE_BODY_LIMIT + 1;
    let (url, server) = sized_response_server(body_len);
    let transport = ReqwestNetworkTransport::new(Duration::from_secs(2));

    let error = transport
        .execute(NetworkTransportRequest {
            method: NetworkMethod::Get,
            url,
            headers: vec![],
            body: vec![],
            resolved_ips: vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
            response_body_limit: Some(DEFAULT_RESPONSE_BODY_LIMIT + 1024),
            timeout_ms: None,
        })
        .expect_err("oversized explicit response limits should still be clamped");
    server.join().unwrap();

    assert!(matches!(
        error,
        NetworkHttpError::ResponseBodyLimit {
            limit: DEFAULT_RESPONSE_BODY_LIMIT,
            ..
        }
    ));
    assert_eq!(error.response_bytes(), DEFAULT_RESPONSE_BODY_LIMIT + 1);
}

#[test]
fn reqwest_transport_clamps_unspecified_response_limit_to_safe_default() {
    let body_len = DEFAULT_RESPONSE_BODY_LIMIT + 1;
    let (url, server) = sized_response_server(body_len);
    let transport = ReqwestNetworkTransport::new(Duration::from_secs(2));

    let error = transport
        .execute(NetworkTransportRequest {
            method: NetworkMethod::Get,
            url,
            headers: vec![],
            body: vec![],
            resolved_ips: vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
            response_body_limit: None,
            timeout_ms: None,
        })
        .expect_err("unspecified response limits should still be bounded");
    server.join().unwrap();

    assert!(matches!(
        error,
        NetworkHttpError::ResponseBodyLimit {
            limit: DEFAULT_RESPONSE_BODY_LIMIT,
            ..
        }
    ));
    assert_eq!(error.response_bytes(), DEFAULT_RESPONSE_BODY_LIMIT + 1);
}

#[derive(Clone)]
struct RecordingTransport {
    response: Result<NetworkHttpResponse, NetworkHttpError>,
    requests: Arc<Mutex<Vec<NetworkTransportRequest>>>,
}

impl RecordingTransport {
    fn ok(response: NetworkHttpResponse) -> Self {
        Self {
            response: Ok(response),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl NetworkHttpTransport for RecordingTransport {
    fn execute(
        &self,
        request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
    }
}

#[derive(Clone)]
struct StaticResolver {
    ips: Vec<IpAddr>,
}

impl StaticResolver {
    fn new(ips: Vec<IpAddr>) -> Self {
        Self { ips }
    }
}

impl NetworkResolver for StaticResolver {
    fn resolve_ips(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, NetworkHttpError> {
        Ok(self.ips.clone())
    }
}

fn single_response_server(response: &'static str) -> (String, std::thread::JoinHandle<()>) {
    single_response_server_for_host("127.0.0.1", response)
}

fn single_response_server_for_host(
    host: &'static str,
    response: &'static str,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://{host}:{port}/test"), handle)
}

fn sized_response_server(body_len: u64) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\n\r\n");
        stream.write_all(header.as_bytes()).unwrap();
        let chunk = vec![b'a'; 8192];
        let mut remaining = body_len;
        while remaining > 0 {
            let write_len = remaining.min(chunk.len() as u64) as usize;
            if stream.write_all(&chunk[..write_len]).is_err() {
                break;
            }
            remaining -= write_len as u64;
        }
    });
    (format!("http://127.0.0.1:{port}/test"), handle)
}

fn policy(
    host_pattern: &str,
    port: Option<u16>,
    deny_private_ip_ranges: bool,
    max_egress_bytes: Option<u64>,
) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: host_pattern.to_string(),
            port,
        }],
        deny_private_ip_ranges,
        max_egress_bytes,
    }
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}
