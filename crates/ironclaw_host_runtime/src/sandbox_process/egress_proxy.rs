//! Egress allowlist CONNECT/forward proxy core for the sandboxed shell
//! profile.
//!
//! Enforces [`sandbox_network_policy`](super::network_allowlist::sandbox_network_policy)
//! for real: a sandboxed container's `http_proxy`/`https_proxy` env is
//! pointed at a bound instance of this proxy (see
//! `crates/ironclaw_reborn_composition/src/sandbox_boot.rs`), and every
//! outbound `CONNECT` (HTTPS tunnel) or plain absolute-URI HTTP request the
//! container makes is checked against the policy's `allowed_targets` before
//! any bytes reach the origin.
//!
//! This module is deliberately Docker-agnostic and scheduling-agnostic: it
//! only knows how to bind a `TcpListener` and serve connections against it.
//! Composition (`ironclaw_reborn_composition::sandbox_egress_proxy_task`)
//! connects this core to a real bind address, spawns it, and owns its
//! cancellation — mirroring the `SandboxReaper` core/spawn split.
//!
//! Never logs request/response bodies or full URIs (only the host being
//! allowed/denied, at `debug` level) — secret material in query strings or
//! headers must never reach the logs.

use std::net::SocketAddr;
use std::sync::Arc;

use ironclaw_host_api::NetworkPolicy;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// Errors [`EgressAllowlistProxy::bind`] can return. Deliberately minimal —
/// per-connection failures never propagate up through `serve`, they are
/// logged at `debug` and the connection is dropped.
#[derive(Debug, thiserror::Error)]
pub enum EgressProxyError {
    #[error("failed to bind egress proxy listener: {reason}")]
    BindFailed { reason: String },
}

/// The forward/CONNECT proxy, not yet bound to a socket.
pub struct EgressAllowlistProxy {
    policy: NetworkPolicy,
}

impl EgressAllowlistProxy {
    pub fn new(policy: NetworkPolicy) -> Self {
        Self { policy }
    }

    /// Binds `bind_addr` (e.g. `"127.0.0.1:0"` for tests, `"0.0.0.0:0"` in
    /// production — see composition's spawn task for why) and returns the
    /// bound proxy plus its resolved local address, so the caller can read
    /// back the OS-chosen port before wiring it into
    /// `RebornSandboxConfig::with_network_broker_port`.
    pub async fn bind(
        self,
        bind_addr: &str,
    ) -> Result<BoundEgressAllowlistProxy, EgressProxyError> {
        let listener =
            TcpListener::bind(bind_addr)
                .await
                .map_err(|error| EgressProxyError::BindFailed {
                    reason: format!("{bind_addr}: {error}"),
                })?;
        Ok(BoundEgressAllowlistProxy {
            listener,
            policy: Arc::new(self.policy),
        })
    }
}

/// A proxy bound to a real local address, ready to `serve`.
pub struct BoundEgressAllowlistProxy {
    listener: TcpListener,
    policy: Arc<NetworkPolicy>,
}

impl BoundEgressAllowlistProxy {
    pub fn local_addr(&self) -> SocketAddr {
        // A bound listener always has a resolvable local address; the only
        // failure mode (an already-closed socket) cannot happen for a
        // listener we just created ourselves.
        self.listener
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)))
    }

    /// Accept loop; spawns one task per connection. Returns once `shutdown`
    /// signals `true` — in-flight connections are left to finish on their
    /// own, no new ones are accepted after that point.
    pub async fn serve(self, mut shutdown: watch::Receiver<bool>) {
        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    match changed {
                        Ok(()) if *shutdown.borrow() => break,
                        Ok(()) => continue,
                        Err(_) => break,
                    }
                }
                accepted = self.listener.accept() => {
                    match accepted {
                        Ok((stream, _peer_addr)) => {
                            let policy = Arc::clone(&self.policy);
                            tokio::spawn(async move {
                                if let Err(error) = handle_connection(stream, policy).await {
                                    tracing::debug!(?error, "egress proxy connection ended with an error");
                                }
                            });
                        }
                        Err(error) => {
                            tracing::debug!(?error, "egress proxy accept failed");
                        }
                    }
                }
            }
        }
    }
}

/// Host-only match: exact hostname, or a `*.suffix` glob — the same shape
/// `sandbox_extra_allowed_domains` already accepts. Ports and scheme in
/// [`ironclaw_host_api::NetworkTargetPattern`] are ignored here (the proxy
/// allowlists by host, consistent with `sandbox_network_policy()`'s
/// `port: None` targets).
fn host_allowed(host: &str, policy: &NetworkPolicy) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    policy.allowed_targets.iter().any(|target| {
        let pattern = target.host_pattern.to_ascii_lowercase();
        if pattern == "*" {
            return true;
        }
        match pattern.strip_prefix("*.") {
            Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
            None => host == pattern,
        }
    })
}

/// One HTTP request line plus its headers, as read off the client socket.
struct RequestHead {
    method: String,
    target: String,
    /// Raw header lines exactly as read (each still ending in `\r\n`),
    /// forwarded verbatim on the allow path.
    header_lines: Vec<String>,
}

/// Reads a request line and its headers (up to the blank-line terminator)
/// from `reader`. Returns `Ok(None)` on a clean EOF before any bytes arrive
/// (the client closed without sending a request).
async fn read_request_head<R>(reader: &mut R) -> std::io::Result<Option<RequestHead>>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(None);
    }
    let mut parts = request_line.trim_end().splitn(3, ' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();

    let mut header_lines = Vec::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 || line.trim_end_matches(['\r', '\n']).is_empty() {
            break;
        }
        header_lines.push(line);
    }

    Ok(Some(RequestHead {
        method,
        target,
        header_lines,
    }))
}

/// Writes a `403 Forbidden` response naming `host` as the denied target.
/// The proxy then closes the connection (dropping `stream` after this call
/// sends the TCP FIN) — no tunnel/forward ever opens for a denied host.
async fn write_denied_response<W>(stream: &mut W, host: &str) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let body = format!("egress denied: host not in allowlist: {host}");
    let response = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

async fn handle_connection(stream: TcpStream, policy: Arc<NetworkPolicy>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let Some(head) = read_request_head(&mut reader).await? else {
        return Ok(());
    };

    if head.method.eq_ignore_ascii_case("CONNECT") {
        handle_connect(reader, &head.target, &policy).await
    } else {
        handle_plain_http(reader, &head, &policy).await
    }
}

/// `CONNECT host:port HTTP/1.1` — tunnels raw bytes to `host:port` once
/// allowed, replying `200 Connection Established` first; replies `403` and
/// closes on deny.
async fn handle_connect(
    mut client: BufReader<TcpStream>,
    target: &str,
    policy: &NetworkPolicy,
) -> std::io::Result<()> {
    let host = target.rsplit_once(':').map_or(target, |(host, _port)| host);

    if !host_allowed(host, policy) {
        tracing::debug!(host, "egress proxy: CONNECT denied");
        write_denied_response(&mut client, host).await?;
        return Ok(());
    }

    let mut origin = match TcpStream::connect(target).await {
        Ok(origin) => origin,
        Err(error) => {
            tracing::debug!(host, ?error, "egress proxy: CONNECT origin unreachable");
            let body = "egress proxy: origin unreachable";
            let response = format!(
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            client.write_all(response.as_bytes()).await?;
            return client.flush().await;
        }
    };

    tracing::debug!(host, "egress proxy: CONNECT allowed");
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    client.flush().await?;

    let mut client = client.into_inner();
    copy_bidirectional(&mut client, &mut origin).await?;
    Ok(())
}

/// Plain absolute-URI HTTP (`GET http://host/path HTTP/1.1`, etc.) —
/// forwards the request verbatim to the origin and streams the response
/// back once allowed; replies `403` and closes on deny.
async fn handle_plain_http(
    mut client: BufReader<TcpStream>,
    head: &RequestHead,
    policy: &NetworkPolicy,
) -> std::io::Result<()> {
    let host = match url::Url::parse(&head.target).ok().and_then(|url| {
        url.host_str().map(|host| match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        })
    }) {
        Some(host) => host,
        None => {
            // Not a well-formed absolute-URI proxy request; nothing to
            // allowlist-check against, so deny rather than forward blind.
            write_denied_response(&mut client, &head.target).await?;
            return Ok(());
        }
    };
    let host_only = host.split(':').next().unwrap_or(host.as_str());

    if !host_allowed(host_only, policy) {
        tracing::debug!(host = host_only, "egress proxy: plain HTTP denied");
        write_denied_response(&mut client, host_only).await?;
        return Ok(());
    }

    let mut origin = match TcpStream::connect(&host).await {
        Ok(origin) => origin,
        Err(error) => {
            tracing::debug!(
                host = host_only,
                ?error,
                "egress proxy: plain HTTP origin unreachable"
            );
            let body = "egress proxy: origin unreachable";
            let response = format!(
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            client.write_all(response.as_bytes()).await?;
            return client.flush().await;
        }
    };

    tracing::debug!(host = host_only, "egress proxy: plain HTTP allowed");
    let mut request_head = format!("{} {} HTTP/1.1\r\n", head.method, head.target);
    for header_line in &head.header_lines {
        request_head.push_str(header_line);
    }
    request_head.push_str("\r\n");
    origin.write_all(request_head.as_bytes()).await?;

    let mut client = client.into_inner();
    copy_bidirectional(&mut client, &mut origin).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::NetworkTargetPattern;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener as TokioTcpListener;

    fn policy_allowing(hosts: &[&str]) -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: hosts
                .iter()
                .map(|host| NetworkTargetPattern {
                    scheme: None,
                    host_pattern: (*host).to_string(),
                    port: None,
                })
                .collect(),
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        }
    }

    #[tokio::test]
    async fn bind_returns_a_reachable_local_address() {
        let proxy = EgressAllowlistProxy::new(policy_allowing(&["example.com"]));
        let bound = proxy
            .bind("127.0.0.1:0")
            .await
            .expect("binding an ephemeral port always succeeds");

        assert_ne!(bound.local_addr().port(), 0);
    }

    /// Spins up a local echo server, allowlists its host, drives a raw
    /// `CONNECT` handshake through the proxy, and proves bytes actually
    /// tunnel end to end (not just that the handshake completes).
    #[tokio::test]
    async fn connect_to_allowed_host_tunnels_bytes() {
        let echo_listener = TokioTcpListener::bind("127.0.0.1:0")
            .await
            .expect("echo listener binds");
        let echo_port = echo_listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = echo_listener.accept().await {
                let mut buf = [0u8; 64];
                if let Ok(n) = socket.read(&mut buf).await {
                    let _ = socket.write_all(&buf[..n]).await;
                }
            }
        });

        let proxy = EgressAllowlistProxy::new(policy_allowing(&["127.0.0.1"]));
        let bound = proxy.bind("127.0.0.1:0").await.expect("proxy binds");
        let proxy_addr = bound.local_addr();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let serve_handle = tokio::spawn(bound.serve(shutdown_rx));

        let mut client = TcpStream::connect(proxy_addr)
            .await
            .expect("client connects to the proxy");
        client
            .write_all(format!("CONNECT 127.0.0.1:{echo_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .expect("CONNECT request writes");

        let mut response = [0u8; 128];
        let n = client
            .read(&mut response)
            .await
            .expect("reads the CONNECT response");
        let response = String::from_utf8_lossy(&response[..n]);
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "expected 200 Connection Established, got: {response}"
        );

        client
            .write_all(b"hello through the tunnel")
            .await
            .expect("write tunneled bytes");
        let mut echoed = [0u8; 64];
        let n = client
            .read(&mut echoed)
            .await
            .expect("reads the echoed bytes back through the tunnel");
        assert_eq!(&echoed[..n], b"hello through the tunnel");

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    #[tokio::test]
    async fn connect_to_denied_host_returns_403_and_closes() {
        let echo_listener = TokioTcpListener::bind("127.0.0.1:0")
            .await
            .expect("echo listener binds");
        let echo_port = echo_listener.local_addr().unwrap().port();

        let proxy = EgressAllowlistProxy::new(policy_allowing(&["github.com"]));
        let bound = proxy.bind("127.0.0.1:0").await.expect("proxy binds");
        let proxy_addr = bound.local_addr();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let serve_handle = tokio::spawn(bound.serve(shutdown_rx));

        let mut client = TcpStream::connect(proxy_addr)
            .await
            .expect("client connects to the proxy");
        client
            .write_all(format!("CONNECT 127.0.0.1:{echo_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .expect("CONNECT request writes");

        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("reads the full 403 response then EOF");
        let response = String::from_utf8_lossy(&response);
        assert!(
            response.starts_with("HTTP/1.1 403"),
            "expected 403 Forbidden, got: {response}"
        );

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    #[tokio::test]
    async fn plain_http_to_denied_host_returns_403() {
        let proxy = EgressAllowlistProxy::new(policy_allowing(&["github.com"]));
        let bound = proxy.bind("127.0.0.1:0").await.expect("proxy binds");
        let proxy_addr = bound.local_addr();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let serve_handle = tokio::spawn(bound.serve(shutdown_rx));

        let mut client = TcpStream::connect(proxy_addr)
            .await
            .expect("client connects to the proxy");
        client
            .write_all(b"GET http://example.com/index.html HTTP/1.1\r\nHost: example.com\r\n\r\n")
            .await
            .expect("plain HTTP request writes");

        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("reads the full 403 response then EOF");
        let response = String::from_utf8_lossy(&response);
        assert!(
            response.starts_with("HTTP/1.1 403"),
            "expected 403 Forbidden, got: {response}"
        );

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }
}
