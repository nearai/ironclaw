use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use ironclaw_host_api::*;
use ironclaw_network::{HardenedHttpEgressClient, HttpEgressError, HttpEgressRequest};

#[test]
fn hardened_http_egress_client_fetches_allowed_target_with_dns_pinning() {
    let server = TestHttpServer::spawn(vec![http_response(200, &[], b"{\"ok\":true}")]);
    let client = HardenedHttpEgressClient::new();

    let response = client
        .request(HttpEgressRequest {
            scope: scope(),
            policy: policy_for_host_port("127.0.0.1", server.port(), false, Some(1024)),
            method: NetworkMethod::Get,
            url: server.url("/ok"),
            headers: Vec::new(),
            body: Vec::new(),
            timeout: Some(Duration::from_secs(5)),
            max_response_bytes: Some(1024),
        })
        .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"{\"ok\":true}");
    assert_eq!(server.hits(), 1);
}

#[test]
fn hardened_http_egress_client_denies_private_dns_targets_before_connect() {
    let client = HardenedHttpEgressClient::new();

    let err = client
        .request(HttpEgressRequest {
            scope: scope(),
            policy: policy_for_host_port("localhost", 1, true, Some(1024)),
            method: NetworkMethod::Get,
            url: "http://localhost:1/blocked".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout: Some(Duration::from_secs(1)),
            max_response_bytes: Some(1024),
        })
        .unwrap_err();

    assert!(matches!(err, HttpEgressError::PrivateTargetDenied { .. }));
}

#[test]
fn hardened_http_egress_client_revalidates_redirect_targets_before_following() {
    let server = TestHttpServer::spawn(vec![http_response(
        302,
        &[("Location", "http://127.0.0.1:1/blocked")],
        b"",
    )]);
    let client = HardenedHttpEgressClient::new();

    let err = client
        .request(HttpEgressRequest {
            scope: scope(),
            policy: policy_for_host_port("127.0.0.1", server.port(), false, Some(1024)),
            method: NetworkMethod::Get,
            url: server.url("/redirect"),
            headers: Vec::new(),
            body: Vec::new(),
            timeout: Some(Duration::from_secs(5)),
            max_response_bytes: Some(1024),
        })
        .unwrap_err();

    assert!(matches!(err, HttpEgressError::TargetDenied { .. }));
    assert_eq!(server.hits(), 1);
}

#[test]
fn hardened_http_egress_client_enforces_response_size_cap_while_reading() {
    let server = TestHttpServer::spawn(vec![http_response(200, &[], b"too large")]);
    let client = HardenedHttpEgressClient::new();

    let err = client
        .request(HttpEgressRequest {
            scope: scope(),
            policy: policy_for_host_port("127.0.0.1", server.port(), false, Some(1024)),
            method: NetworkMethod::Get,
            url: server.url("/large"),
            headers: Vec::new(),
            body: Vec::new(),
            timeout: Some(Duration::from_secs(5)),
            max_response_bytes: Some(4),
        })
        .unwrap_err();

    assert!(matches!(err, HttpEgressError::ResponseTooLarge { .. }));
    assert_eq!(server.hits(), 1);
}

fn scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn policy_for_host_port(
    host: &str,
    port: u16,
    deny_private: bool,
    max_egress_bytes: Option<u64>,
) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: host.to_string(),
            port: Some(port),
        }],
        deny_private_ip_ranges: deny_private,
        max_egress_bytes,
    }
}

fn http_response(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        302 => "Found",
        _ => "Status",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .into_bytes();
    for (name, value) in headers {
        response.extend_from_slice(name.as_bytes());
        response.extend_from_slice(b": ");
        response.extend_from_slice(value.as_bytes());
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"\r\n");
    response.extend_from_slice(body);
    response
}

struct TestHttpServer {
    addr: std::net::SocketAddr,
    hits: Arc<AtomicUsize>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestHttpServer {
    fn spawn(responses: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_thread = Arc::clone(&hits);
        let handle = thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                hits_for_thread.fetch_add(1, Ordering::SeqCst);
                drain_request(&mut stream);
                let _ = stream.write_all(&response);
            }
        });
        Self {
            addr,
            hits,
            handle: Some(handle),
        }
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for TestHttpServer {
    fn drop(&mut self) {
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(50));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn drain_request(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0; 1024];
    let _ = stream.read(&mut buf);
}
