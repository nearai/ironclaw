use std::ffi::OsString;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ironclaw_host_api::action::NetworkMethod;
use ironclaw_network::{NetworkHttpTransport, NetworkTransportRequest, ReqwestNetworkTransport};

const PROXY_ENV_VARS: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

static PROXY_ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
#[allow(
    clippy::await_holding_lock,
    reason = "the process proxy environment must remain serialized until reqwest finishes"
)]
async fn reqwest_transport_ignores_ambient_proxy_and_uses_pinned_address() {
    let _env_lock = PROXY_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let pinned = RecordingServer::start(b"pinned");
    let target_authority = format!("ambient-proxy.example.test:{}", pinned.addr().port());
    let proxy = RecordingServer::start(b"proxy");
    let _proxy_env = ProxyEnvGuard::set(&format!("http://{}", proxy.addr()));

    let transport = ReqwestNetworkTransport::new(Duration::from_secs(2));
    let response = transport
        .execute(NetworkTransportRequest {
            method: NetworkMethod::Get,
            url: format!("http://{target_authority}/test"),
            headers: vec![],
            body: vec![],
            resolved_ips: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
            response_body_limit: Some(1024),
            timeout_ms: None,
        })
        .await
        .expect("the request should reach the approved pinned address");

    let pinned_request = pinned.finish();
    let proxy_request = proxy.finish();
    assert!(
        proxy_request.is_empty(),
        "ambient proxy received the vetted hostname:\n{}",
        String::from_utf8_lossy(&proxy_request)
    );
    assert_eq!(response.body, b"pinned");
    let pinned_request = String::from_utf8_lossy(&pinned_request);
    let mut pinned_request_lines = pinned_request.lines();
    assert_eq!(
        pinned_request_lines.next(),
        Some("GET /test HTTP/1.1"),
        "approved pinned listener received the wrong request target: {pinned_request}"
    );
    let expected_host_header = format!("host: {target_authority}");
    assert!(
        pinned_request_lines.any(|line| line.eq_ignore_ascii_case(&expected_host_header)),
        "approved pinned listener did not receive the original Host header: {pinned_request}"
    );
}

struct ProxyEnvGuard {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl ProxyEnvGuard {
    fn set(proxy_url: &str) -> Self {
        let previous = PROXY_ENV_VARS
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect();
        // SAFETY: the test holds PROXY_ENV_LOCK for this guard's lifetime. This
        // dedicated test binary has no other tests that can read or mutate env.
        unsafe {
            for name in PROXY_ENV_VARS {
                if name.eq_ignore_ascii_case("no_proxy") {
                    std::env::remove_var(name);
                } else {
                    std::env::set_var(name, proxy_url);
                }
            }
        }
        Self { previous }
    }
}

impl Drop for ProxyEnvGuard {
    fn drop(&mut self) {
        // SAFETY: this guard is dropped before the PROXY_ENV_LOCK guard.
        unsafe {
            for (name, value) in self.previous.drain(..).rev() {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

struct RecordingServer {
    addr: std::net::SocketAddr,
    request: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RecordingServer {
    fn start(body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind recording server");
        listener
            .set_nonblocking(true)
            .expect("set recording server nonblocking");
        let addr = listener.local_addr().expect("recording server address");
        let request = Arc::new(Mutex::new(Vec::new()));
        let captured = request.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let should_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("set accepted stream blocking");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .expect("set request read timeout");
                        let mut buf = [0_u8; 2048];
                        let read = stream.read(&mut buf).expect("read request");
                        captured.lock().unwrap().extend_from_slice(&buf[..read]);
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        )
                        .expect("write response headers");
                        stream.write_all(body).expect("write response body");
                        break;
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if should_stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept request: {error}"),
                }
            }
        });
        Self {
            addr,
            request,
            stop,
            thread: Some(thread),
        }
    }

    fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    fn finish(mut self) -> Vec<u8> {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
        Arc::try_unwrap(self.request)
            .expect("server released captured request")
            .into_inner()
            .unwrap()
    }
}
