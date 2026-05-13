//! Adapter-aware [`ProtocolHttpEgress`] shim implementations.
//!
//! These shims sit between a [`ProductAdapter`] and the actual outbound HTTP
//! transport. They enforce two contracts:
//!
//! 1. **Declared-host allowlist**: requests to hosts the adapter did not
//!    declare are rejected before any network call. The adapter ships a
//!    static list (via `declared_egress`), and the shim verifies the
//!    request's host is in that list.
//! 2. **Credential materialization**: the adapter never sees the actual
//!    secret value. It carries an `EgressCredentialHandle` (a name), and the
//!    shim resolves that handle into the real credential just before
//!    transport.
//!
//! [`TelegramHttpEgress`] is the only shim in this slice. It POSTs to
//! `api.telegram.org/bot{token}/{path}` — Telegram's URL convention. Other
//! adapters (Slack, Discord) will get their own shim implementations when we
//! port them; do not generalize this one prematurely.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    DeclaredEgressTarget, EgressCredentialHandle, EgressRequest, EgressResponse,
    ProtocolHttpEgress, ProtocolHttpEgressError, RedactedString,
};

/// Resolves a credential handle to its materialized secret value. The
/// composition root wires this against the host secret store.
#[async_trait]
pub trait EgressCredentialResolver: Send + Sync {
    async fn resolve(&self, handle: &EgressCredentialHandle) -> Option<String>;
}

/// HTTPS egress shim for the Telegram Bot API. Constructs
/// `https://api.telegram.org/bot{token}{path}` from the adapter-supplied
/// request, where `{token}` is the resolved credential.
pub struct TelegramHttpEgress {
    http: reqwest::Client,
    declared: Vec<DeclaredEgressTarget>,
    credentials: Arc<dyn EgressCredentialResolver>,
    /// Test-only override: when set, the request URL becomes
    /// `{base_url_for_test}/bot{token}{path}` instead of
    /// `https://{host}/bot{token}{path}`. Production must leave this `None`.
    base_url_for_test: Option<String>,
}

impl TelegramHttpEgress {
    pub fn new(
        declared: Vec<DeclaredEgressTarget>,
        credentials: Arc<dyn EgressCredentialResolver>,
    ) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .user_agent("ironclaw-reborn-telegram-v2/0")
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            declared,
            credentials,
            base_url_for_test: None,
        })
    }

    /// Test-only: redirect the URL prefix to a local mock server. The
    /// declared-host allowlist still gates the request (so tests must still
    /// declare the host they put in the `EgressRequest`).
    #[doc(hidden)]
    pub fn with_base_url_for_test(mut self, base_url: impl Into<String>) -> Self {
        self.base_url_for_test = Some(base_url.into());
        self
    }
}

fn declared_match(declared: &[DeclaredEgressTarget], host: &str) -> Option<DeclaredEgressTarget> {
    declared.iter().find(|d| d.host.as_str() == host).cloned()
}

#[async_trait]
impl ProtocolHttpEgress for TelegramHttpEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        // 1. Allowlist host.
        let host_str = request.host().as_str();
        let Some(matched) = declared_match(&self.declared, host_str) else {
            return Err(ProtocolHttpEgressError::UndeclaredHost {
                host: host_str.to_string(),
            });
        };

        // 2. Resolve credential. The adapter declares a credential handle as
        // part of the declared target (or per-request); for Telegram the bot
        // token is required, so a missing handle is a configuration error.
        let handle = request
            .credential_handle()
            .or(matched.credential_handle.as_ref())
            .ok_or_else(|| ProtocolHttpEgressError::UnauthorizedCredentialHandle {
                handle: "(missing)".to_string(),
            })?;
        let token = self.credentials.resolve(handle).await.ok_or_else(|| {
            ProtocolHttpEgressError::UnknownCredentialHandle {
                handle: handle.as_str().to_string(),
            }
        })?;

        // 3. Build URL: https://api.telegram.org/bot{token}{path}. Tests can
        // redirect the URL prefix to a local mock via `base_url_for_test`.
        let url = if let Some(base) = &self.base_url_for_test {
            format!(
                "{base}/bot{token}{path}",
                base = base.trim_end_matches('/'),
                token = token,
                path = request.path().as_str(),
            )
        } else {
            format!(
                "https://{host}/bot{token}{path}",
                host = host_str,
                token = token,
                path = request.path().as_str(),
            )
        };

        // 4. Build HTTP request.
        let method = match request.method().as_str() {
            "POST" => reqwest::Method::POST,
            "GET" => reqwest::Method::GET,
            other => {
                return Err(ProtocolHttpEgressError::PolicyDenied {
                    reason: RedactedString::new(format!("unsupported method {other}")),
                });
            }
        };
        let mut req = self.http.request(method, &url);
        for header in request.headers() {
            req = req.header(header.name(), header.value());
        }
        if !request.body().is_empty() {
            req = req.body(request.body().to_vec());
        }

        // 5. Send and translate response.
        let response = req.send().await.map_err(|e| {
            if e.is_timeout() {
                ProtocolHttpEgressError::Timeout
            } else {
                ProtocolHttpEgressError::Network(RedactedString::new(e.to_string()))
            }
        })?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|e| ProtocolHttpEgressError::Network(RedactedString::new(e.to_string())))?
            .to_vec();

        Ok(EgressResponse::new(status, body))
    }
}

/// In-memory resolver useful for tests and bootstrap composition where the
/// credential value is known statically.
pub struct StaticCredentialResolver {
    handle: EgressCredentialHandle,
    value: String,
}

impl StaticCredentialResolver {
    pub fn new(handle: EgressCredentialHandle, value: impl Into<String>) -> Self {
        Self {
            handle,
            value: value.into(),
        }
    }
}

#[async_trait]
impl EgressCredentialResolver for StaticCredentialResolver {
    async fn resolve(&self, handle: &EgressCredentialHandle) -> Option<String> {
        if handle.as_str() == self.handle.as_str() {
            Some(self.value.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_adapters::{DeclaredEgressHost, EgressMethod, EgressPath};

    fn telegram_target(handle: &str) -> DeclaredEgressTarget {
        DeclaredEgressTarget::new(
            DeclaredEgressHost::new("api.telegram.org").expect("host"),
            Some(EgressCredentialHandle::new(handle).expect("handle")),
        )
    }

    #[tokio::test]
    async fn undeclared_host_is_rejected() {
        let resolver = Arc::new(StaticCredentialResolver::new(
            EgressCredentialHandle::new("telegram_bot_token").expect("handle"),
            "secret-token",
        ));
        let egress = TelegramHttpEgress::new(vec![telegram_target("telegram_bot_token")], resolver)
            .expect("client");
        let request = EgressRequest::new(
            DeclaredEgressHost::new("evil.example.com").expect("host"),
            EgressMethod::post(),
            EgressPath::new("/sendMessage").expect("path"),
        );
        let result = egress.send(request).await;
        assert!(matches!(
            result,
            Err(ProtocolHttpEgressError::UndeclaredHost { .. })
        ));
    }

    /// Spawn a minimal one-shot HTTP listener that records the incoming
    /// request and returns a canned `200 OK` body. Returns the bound URL +
    /// a oneshot receiver carrying `(request_line, headers, body)`.
    async fn spawn_recording_http_server() -> (
        String,
        tokio::sync::oneshot::Receiver<(String, Vec<(String, String)>, Vec<u8>)>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _peer) = listener.accept().await.expect("accept");
            // Read the request: HTTP/1.1 has Content-Length we honor.
            let mut buf = vec![0u8; 8192];
            let mut total = Vec::new();
            let mut header_end = None;
            while header_end.is_none() {
                let n = socket.read(&mut buf).await.expect("read");
                if n == 0 {
                    break;
                }
                total.extend_from_slice(&buf[..n]);
                if let Some(pos) = total.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = Some(pos);
                }
            }
            let header_end = header_end.expect("headers");
            let header_text = String::from_utf8_lossy(&total[..header_end]).to_string();
            let mut lines = header_text.split("\r\n");
            let request_line = lines.next().unwrap_or("").to_string();
            let mut headers = Vec::new();
            let mut content_length = 0usize;
            for h in lines {
                if let Some((k, v)) = h.split_once(':') {
                    let key = k.trim().to_string();
                    let val = v.trim().to_string();
                    if key.eq_ignore_ascii_case("content-length") {
                        content_length = val.parse().unwrap_or(0);
                    }
                    headers.push((key, val));
                }
            }
            // Continue reading until we have the full body.
            let body_start = header_end + 4;
            let mut body = total[body_start..].to_vec();
            while body.len() < content_length {
                let n = socket.read(&mut buf).await.expect("read body");
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&buf[..n]);
            }
            body.truncate(content_length);
            // Write a minimal 200 OK so reqwest doesn't error.
            let canned = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 27\r\n\r\n{\"ok\":true,\"result\":{\"x\":1}}";
            socket.write_all(canned).await.expect("write");
            socket.flush().await.ok();
            let _ = tx.send((request_line, headers, body));
        });
        (format!("http://{}", addr), rx)
    }

    #[tokio::test]
    async fn real_http_post_includes_bot_token_path_headers_and_body() {
        let (base_url, recv) = spawn_recording_http_server().await;
        let resolver = Arc::new(StaticCredentialResolver::new(
            EgressCredentialHandle::new("telegram_bot_token").expect("handle"),
            "TOKEN-XYZ",
        ));
        let egress = TelegramHttpEgress::new(vec![telegram_target("telegram_bot_token")], resolver)
            .expect("client")
            .with_base_url_for_test(base_url);

        let body = br#"{"chat_id":42,"text":"hi"}"#.to_vec();
        let request = EgressRequest::new(
            DeclaredEgressHost::new("api.telegram.org").expect("host"),
            EgressMethod::post(),
            EgressPath::new("/sendMessage").expect("path"),
        )
        .with_header(
            ironclaw_product_adapters::EgressHeader::new("content-type", "application/json")
                .expect("hdr"),
        )
        .with_body(body.clone())
        .with_credential_handle(Some(
            EgressCredentialHandle::new("telegram_bot_token").expect("handle"),
        ));

        let response = egress.send(request).await.expect("send");
        assert_eq!(response.status(), 200);

        let (request_line, headers, captured_body) = recv.await.expect("captured");
        assert!(
            request_line.starts_with("POST /botTOKEN-XYZ/sendMessage HTTP"),
            "request line was: {request_line}"
        );
        let content_type = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        assert_eq!(content_type, "application/json");
        assert_eq!(captured_body, body);
    }

    #[tokio::test]
    async fn unresolved_credential_is_rejected() {
        let resolver = Arc::new(StaticCredentialResolver::new(
            EgressCredentialHandle::new("different_handle").expect("handle"),
            "irrelevant",
        ));
        let egress = TelegramHttpEgress::new(vec![telegram_target("telegram_bot_token")], resolver)
            .expect("client");
        let request = EgressRequest::new(
            DeclaredEgressHost::new("api.telegram.org").expect("host"),
            EgressMethod::post(),
            EgressPath::new("/sendMessage").expect("path"),
        )
        .with_credential_handle(Some(
            EgressCredentialHandle::new("telegram_bot_token").expect("handle"),
        ));
        let result = egress.send(request).await;
        assert!(matches!(
            result,
            Err(ProtocolHttpEgressError::UnknownCredentialHandle { .. })
        ));
    }
}
