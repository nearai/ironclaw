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

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::NetworkPolicy;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// Resolver seam for the dial-time private-IP guard (E2 hardening 1): the
/// production impl below does real DNS; tests inject a fixed-address
/// resolver so a resolved IP (e.g. a simulated cloud-metadata address) can
/// be asserted against without live DNS or a privileged low-port bind.
#[async_trait]
trait HostResolver: Send + Sync {
    async fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>>;
}

/// Real DNS via `tokio::net::lookup_host` — the production resolver.
struct DnsResolver;

#[async_trait]
impl HostResolver for DnsResolver {
    async fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host((host, port)).await?.collect())
    }
}

/// Returns the matched deny rule if `ip` falls in a private, loopback,
/// link-local, CGNAT, or unique-local range — the exact set enumerated in
/// the E2 amendment. `Ipv4Addr::is_private()` alone does not cover
/// link-local (`169.254.0.0/16`, which includes the `169.254.169.254` cloud
/// metadata address) or CGNAT (`100.64.0.0/10`), so both get an explicit
/// check alongside the std helpers.
fn denied_ip_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => denied_ipv4_reason(v4),
        IpAddr::V6(v6) => {
            // An IPv4-mapped IPv6 address (::ffff:a.b.c.d) carries a v4
            // address end to end; classify it as its mapped v4 form rather
            // than letting it slip through the v6 checks unclassified.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return denied_ipv4_reason(mapped);
            }
            if v6.is_loopback() {
                Some("loopback (::1)")
            } else if v6.segments()[0] >> 8 == 0xfd {
                Some("unique-local (fd00::/8)")
            } else {
                None
            }
        }
    }
}

fn denied_ipv4_reason(v4: Ipv4Addr) -> Option<&'static str> {
    let octets = v4.octets();
    if v4.is_loopback() {
        Some("loopback (127.0.0.0/8)")
    } else if v4.is_private() {
        Some("private (10.0.0.0/8, 172.16.0.0/12, or 192.168.0.0/16)")
    } else if v4.is_link_local() {
        Some("link-local (169.254.0.0/16, incl. cloud metadata 169.254.169.254)")
    } else if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        Some("CGNAT (100.64.0.0/10)")
    } else {
        None
    }
}

/// Resolves `host:port` and applies the dial-time private-IP guard when
/// `deny_private_ips` is set. Returns the first candidate address that
/// passes the guard (or the first candidate at all, when the guard is
/// disabled) as the `Ok` half; a guard rejection is the `Err` half, carrying
/// the matched rule for the audit log. `deny_private_ips` is only ever
/// turned off by test fixtures standing a loopback echo server in for a
/// real origin (see the byte-plumbing test below) — production callers
/// always pass `true`.
async fn resolve_dial_addr(
    resolver: &dyn HostResolver,
    host: &str,
    port: u16,
    deny_private_ips: bool,
) -> std::io::Result<Result<SocketAddr, &'static str>> {
    let addrs = resolver.resolve(host, port).await?;
    if !deny_private_ips {
        return Ok(addrs.into_iter().next().ok_or("no addresses resolved"));
    }
    match addrs
        .iter()
        .find(|addr| denied_ip_reason(addr.ip()).is_none())
    {
        Some(addr) => Ok(Ok(*addr)),
        None => {
            let reason = addrs
                .first()
                .and_then(|addr| denied_ip_reason(addr.ip()))
                .unwrap_or("no addresses resolved");
            Ok(Err(reason))
        }
    }
}

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
    resolver: Arc<dyn HostResolver>,
    /// Always `true` in production (`new`); only a test fixture in this
    /// module's `#[cfg(test)]` submodule constructs this struct directly
    /// with it set `false`, to stand a loopback echo server in for a real
    /// origin without tripping the SSRF guard. See
    /// `connect_to_allowed_host_tunnels_bytes`.
    deny_private_ips: bool,
}

impl EgressAllowlistProxy {
    pub fn new(policy: NetworkPolicy) -> Self {
        Self {
            policy,
            resolver: Arc::new(DnsResolver),
            deny_private_ips: true,
        }
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
            resolver: self.resolver,
            deny_private_ips: self.deny_private_ips,
        })
    }
}

/// A proxy bound to a real local address, ready to `serve`.
pub struct BoundEgressAllowlistProxy {
    listener: TcpListener,
    policy: Arc<NetworkPolicy>,
    resolver: Arc<dyn HostResolver>,
    deny_private_ips: bool,
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
                            let resolver = Arc::clone(&self.resolver);
                            let deny_private_ips = self.deny_private_ips;
                            tokio::spawn(async move {
                                if let Err(error) =
                                    handle_connection(stream, policy, resolver, deny_private_ips)
                                        .await
                                {
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

async fn handle_connection(
    stream: TcpStream,
    policy: Arc<NetworkPolicy>,
    resolver: Arc<dyn HostResolver>,
    deny_private_ips: bool,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let Some(head) = read_request_head(&mut reader).await? else {
        return Ok(());
    };

    if head.method.eq_ignore_ascii_case("CONNECT") {
        handle_connect(
            reader,
            &head.target,
            &policy,
            resolver.as_ref(),
            deny_private_ips,
        )
        .await
    } else {
        handle_plain_http(reader, &head, &policy, resolver.as_ref(), deny_private_ips).await
    }
}

/// `CONNECT host:port HTTP/1.1` — tunnels raw bytes to `host:port` once
/// allowed, replying `200 Connection Established` first; replies `403` and
/// closes on deny. Three checks gate the dial, in order: the hostname
/// allowlist (E2 predates this proxy), the CONNECT port pin to `443` (E2
/// hardening 2 — the proxy only tunnels HTTPS), and the resolved-IP private
/// range guard (E2 hardening 1). Each decision gets one `debug!` audit line
/// naming the host, allow/deny, and the matched rule — never payloads.
async fn handle_connect(
    mut client: BufReader<TcpStream>,
    target: &str,
    policy: &NetworkPolicy,
    resolver: &dyn HostResolver,
    deny_private_ips: bool,
) -> std::io::Result<()> {
    let host = target.rsplit_once(':').map_or(target, |(host, _port)| host);
    let port: Option<u16> = target
        .rsplit_once(':')
        .and_then(|(_host, port)| port.parse().ok());

    if !host_allowed(host, policy) {
        tracing::debug!(
            host,
            action = "deny",
            rule = "not_in_allowlist",
            "egress proxy: CONNECT denied"
        );
        write_denied_response(&mut client, host).await?;
        return Ok(());
    }

    let Some(port) = port else {
        tracing::debug!(
            host,
            action = "deny",
            rule = "malformed_connect_target",
            "egress proxy: CONNECT denied"
        );
        write_denied_response(&mut client, host).await?;
        return Ok(());
    };

    if port != 443 {
        tracing::debug!(
            host,
            port,
            action = "deny",
            rule = "connect_port_not_443",
            "egress proxy: CONNECT denied"
        );
        write_denied_response(&mut client, host).await?;
        return Ok(());
    }

    let dial_addr = match resolve_dial_addr(resolver, host, port, deny_private_ips).await {
        Ok(Ok(addr)) => addr,
        Ok(Err(rule)) => {
            tracing::debug!(host, action = "deny", rule, "egress proxy: CONNECT denied");
            write_denied_response(&mut client, host).await?;
            return Ok(());
        }
        Err(error) => {
            tracing::debug!(host, ?error, "egress proxy: CONNECT DNS resolution failed");
            let body = "egress proxy: origin unreachable";
            let response = format!(
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            client.write_all(response.as_bytes()).await?;
            return client.flush().await;
        }
    };

    let mut origin = match TcpStream::connect(dial_addr).await {
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

    tracing::debug!(
        host,
        action = "allow",
        rule = "allowlist_match",
        "egress proxy: CONNECT allowed"
    );
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
    resolver: &dyn HostResolver,
    deny_private_ips: bool,
) -> std::io::Result<()> {
    let parsed = url::Url::parse(&head.target).ok();
    let host_only = parsed.as_ref().and_then(|url| url.host_str());
    let Some(host_only) = host_only else {
        // Not a well-formed absolute-URI proxy request; nothing to
        // allowlist-check against, so deny rather than forward blind.
        write_denied_response(&mut client, &head.target).await?;
        return Ok(());
    };
    let host_only = host_only.to_string();
    let port = parsed
        .as_ref()
        .and_then(|url| url.port_or_known_default())
        .unwrap_or(80);

    if !host_allowed(&host_only, policy) {
        tracing::debug!(host = %host_only, action = "deny", rule = "not_in_allowlist", "egress proxy: plain HTTP denied");
        write_denied_response(&mut client, &host_only).await?;
        return Ok(());
    }

    let dial_addr = match resolve_dial_addr(resolver, &host_only, port, deny_private_ips).await {
        Ok(Ok(addr)) => addr,
        Ok(Err(rule)) => {
            tracing::debug!(host = %host_only, action = "deny", rule, "egress proxy: plain HTTP denied");
            write_denied_response(&mut client, &host_only).await?;
            return Ok(());
        }
        Err(error) => {
            tracing::debug!(host = %host_only, ?error, "egress proxy: plain HTTP DNS resolution failed");
            let body = "egress proxy: origin unreachable";
            let response = format!(
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            client.write_all(response.as_bytes()).await?;
            return client.flush().await;
        }
    };

    let mut origin = match TcpStream::connect(dial_addr).await {
        Ok(origin) => origin,
        Err(error) => {
            tracing::debug!(
                host = %host_only,
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

    tracing::debug!(host = %host_only, action = "allow", rule = "allowlist_match", "egress proxy: plain HTTP allowed");
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

    /// Test-only resolver that always resolves to a fixed address,
    /// ignoring whatever host/port the request named. Lets a test point
    /// the proxy's dial step at a real local listener (to prove the
    /// tunnel/forward mechanics) or at a synthetic private IP (to prove
    /// the SSRF guard) without live DNS or a privileged low-port bind.
    struct FixedAddrResolver(SocketAddr);

    #[async_trait]
    impl HostResolver for FixedAddrResolver {
        async fn resolve(&self, _host: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
            Ok(vec![self.0])
        }
    }

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
    /// tunnel end to end (not just that the handshake completes). Also
    /// covers the E2 port-pin hardening's allow path (`CONNECT ...:443` on
    /// an allowlisted host proceeds) by naming port 443 in the request line
    /// while a `FixedAddrResolver` transparently redirects the actual dial
    /// to the echo server's real (ephemeral) port — real DNS can't be
    /// pointed at an arbitrary local port, and binding the echo server to
    /// the real port 443 would need root. The echo server is a loopback
    /// stand-in for a real origin, not a policy target, so this test also
    /// disables the private-IP guard (`deny_private_ips: false`) rather
    /// than weakening it in production; the guard itself is proven denying
    /// loopback/private/link-local addresses by
    /// `connect_to_allowlisted_host_resolving_private_ip_is_denied` and the
    /// `denied_ip_reason` unit tests below.
    #[tokio::test]
    async fn connect_to_allowed_host_tunnels_bytes() {
        let echo_listener = TokioTcpListener::bind("127.0.0.1:0")
            .await
            .expect("echo listener binds");
        let echo_addr = echo_listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = echo_listener.accept().await {
                let mut buf = [0u8; 64];
                if let Ok(n) = socket.read(&mut buf).await {
                    let _ = socket.write_all(&buf[..n]).await;
                }
            }
        });

        let proxy = EgressAllowlistProxy {
            policy: policy_allowing(&["127.0.0.1"]),
            resolver: Arc::new(FixedAddrResolver(echo_addr)),
            deny_private_ips: false,
        };
        let bound = proxy.bind("127.0.0.1:0").await.expect("proxy binds");
        let proxy_addr = bound.local_addr();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let serve_handle = tokio::spawn(bound.serve(shutdown_rx));

        let mut client = TcpStream::connect(proxy_addr)
            .await
            .expect("client connects to the proxy");
        client
            .write_all(b"CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n")
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

    /// E2 hardening 1 (SSRF/DNS-rebinding guard) — headline test. A host
    /// passes the hostname allowlist but resolves to the cloud-metadata
    /// link-local address (via an injected resolver, so the assertion does
    /// not depend on live DNS); the dial-time private-IP check must deny it
    /// even though the hostname itself was allowed.
    #[tokio::test]
    async fn connect_to_allowlisted_host_resolving_private_ip_is_denied() {
        let proxy = EgressAllowlistProxy {
            policy: policy_allowing(&["metadata.example"]),
            resolver: Arc::new(FixedAddrResolver(SocketAddr::from((
                [169, 254, 169, 254],
                443,
            )))),
            deny_private_ips: true,
        };
        let bound = proxy.bind("127.0.0.1:0").await.expect("proxy binds");
        let proxy_addr = bound.local_addr();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let serve_handle = tokio::spawn(bound.serve(shutdown_rx));

        let mut client = TcpStream::connect(proxy_addr)
            .await
            .expect("client connects to the proxy");
        client
            .write_all(b"CONNECT metadata.example:443 HTTP/1.1\r\n\r\n")
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
            "expected 403 Forbidden for a private-IP resolution, got: {response}"
        );

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    /// E2 hardening 2 (CONNECT port pin) — an allowlisted host is still
    /// denied when the CONNECT target port isn't 443, closing off pivoting
    /// an allowlisted host to an arbitrary port through the tunnel.
    #[tokio::test]
    async fn connect_to_allowed_host_non_443_port_returns_403() {
        let proxy = EgressAllowlistProxy::new(policy_allowing(&["github.com"]));
        let bound = proxy.bind("127.0.0.1:0").await.expect("proxy binds");
        let proxy_addr = bound.local_addr();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let serve_handle = tokio::spawn(bound.serve(shutdown_rx));

        let mut client = TcpStream::connect(proxy_addr)
            .await
            .expect("client connects to the proxy");
        client
            .write_all(b"CONNECT github.com:22 HTTP/1.1\r\n\r\n")
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
            "expected 403 Forbidden for a non-443 CONNECT port, got: {response}"
        );

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    /// `denied_ip_reason` is the pure classification function both dial
    /// paths gate on; exercise every range named in the E2 amendment plus
    /// one public v4/v6 example each, so the range math is pinned
    /// independent of any live connection.
    #[test]
    fn denied_ip_reason_covers_every_e2_range() {
        let denied = [
            ("10.0.0.5", "private (RFC1918 10/8)"),
            ("172.16.0.5", "private (RFC1918 172.16/12)"),
            ("172.31.255.254", "private (RFC1918 172.16/12 upper bound)"),
            ("192.168.1.1", "private (RFC1918 192.168/16)"),
            ("127.0.0.1", "loopback"),
            ("169.254.169.254", "cloud metadata link-local"),
            ("169.254.1.1", "link-local"),
            ("100.64.0.1", "CGNAT lower bound"),
            ("100.100.100.100", "CGNAT mid-range"),
            ("100.127.255.255", "CGNAT upper bound"),
        ];
        for (ip, label) in denied {
            let ip: IpAddr = ip.parse().expect("valid literal");
            assert!(
                denied_ip_reason(ip).is_some(),
                "expected {ip} ({label}) to be denied"
            );
        }

        let denied_v6 = [
            ("::1", "loopback"),
            ("fd00::1", "unique-local lower bound"),
            (
                "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                "unique-local upper bound",
            ),
            ("::ffff:169.254.169.254", "IPv4-mapped cloud metadata"),
        ];
        for (ip, label) in denied_v6 {
            let ip: IpAddr = ip.parse().expect("valid literal");
            assert!(
                denied_ip_reason(ip).is_some(),
                "expected {ip} ({label}) to be denied"
            );
        }

        let allowed = ["8.8.8.8", "93.184.216.34", "1.1.1.1"];
        for ip in allowed {
            let ip: IpAddr = ip.parse().expect("valid literal");
            assert_eq!(
                denied_ip_reason(ip),
                None,
                "expected public address {ip} to pass the guard"
            );
        }

        let allowed_v6: IpAddr = "2606:4700:4700::1111".parse().expect("valid literal");
        assert_eq!(
            denied_ip_reason(allowed_v6),
            None,
            "expected public v6 address to pass the guard"
        );
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
