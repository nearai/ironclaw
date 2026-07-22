//! Secret lease protocol + server core for the sandboxed shell profile.
//!
//! Deliberately Docker-agnostic and secrets-store-agnostic: this module
//! defines only a resolver trait ([`SandboxSecretLeaseResolver`]) and a
//! newline-delimited-JSON wire protocol served over a Unix socket. The real
//! `SecretStore`-backed resolver, and the composition wiring that binds this
//! server to a real socket path per sandbox scope, land separately — this
//! module is unit-tested by binding a real Unix socket in a temp dir and
//! driving it with a raw client, mirroring how [`super::egress_proxy`] is
//! tested with a raw TCP client.
//!
//! Redaction here is a TYPE-level property, not a discipline one:
//! [`SandboxSecretLeaseError`] carries no `String`/material/internal-text
//! field, so a resolver's internal error — which might echo raw HTTP body
//! content from an OAuth provider, or otherwise embed secret material — can
//! never reach the wire no matter what the resolver's `Display` impl says.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use ironclaw_host_api::ResourceScope;
use ironclaw_secrets::SecretMaterial;
use secrecy::ExposeSecret;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::watch;

/// Resolver seam: composition plugs the real `SecretStore`-backed
/// implementation in here (a separate task); this module's tests use a
/// `FakeResolver`.
///
/// Callers (composition) are responsible for one-shot semantics at the
/// store layer — each call here must mint and immediately consume a fresh
/// lease, never return material twice for the same underlying secret
/// without a new store-level lease in between.
#[async_trait::async_trait]
pub trait SandboxSecretLeaseResolver: Send + Sync {
    async fn resolve_lease(
        &self,
        scope: &ResourceScope,
        secret_name: &str,
    ) -> Result<SandboxSecretLease, SandboxSecretLeaseError>;
}

/// A short-lived secret lease resolved for a single request.
pub struct SandboxSecretLease {
    pub material: SecretMaterial,
    pub ttl_seconds: Option<u64>,
}

/// Wire-safe failure reason. Deliberately carries no upstream error text or
/// secret material — only a stable token, so a resolver's internal error
/// can never reach the socket. This is what makes redaction a type
/// property: there is no field here to accidentally serialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SandboxSecretLeaseError {
    #[error("unknown secret")]
    UnknownSecret,
    #[error("secret backend unavailable")]
    BackendUnavailable,
}

impl SandboxSecretLeaseError {
    /// Stable wire token for this variant — never the `Display` text (which
    /// is for host-side logs/errors only, and is itself redaction-safe by
    /// construction since the enum carries no material).
    fn wire_reason(self) -> &'static str {
        match self {
            Self::UnknownSecret => "unknown_secret",
            Self::BackendUnavailable => "backend_unavailable",
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct SecretLeaseWireRequest {
    secret_name: String,
}

/// Tagged-enum wire response — illegal states (e.g. `status: "ok"` with no
/// `material`, or `status: "error"` carrying a `material` field) are
/// unrepresentable by construction rather than guarded at read time.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SecretLeaseWireResponse {
    Ok {
        material: String,
        ttl_seconds: Option<u64>,
    },
    Error {
        reason: &'static str,
    },
}

/// The lease server core, not yet bound to a socket.
pub struct SandboxSecretLeaseServer {
    resolver: Arc<dyn SandboxSecretLeaseResolver>,
    scope: ResourceScope,
}

impl SandboxSecretLeaseServer {
    pub fn new(resolver: Arc<dyn SandboxSecretLeaseResolver>, scope: ResourceScope) -> Self {
        Self { resolver, scope }
    }

    /// Binds a Unix listener at `socket_path` (parent directory must
    /// already exist; the socket file itself is created mode 0600) and
    /// serves newline-delimited-JSON lease requests until `shutdown`
    /// fires. One request per connection (client connects, sends one JSON
    /// line, reads one JSON line back, disconnects) — matches the
    /// one-shot-lease shim usage pattern (`ironclaw-lease <name>` per
    /// invocation).
    pub async fn bind_and_serve(
        self,
        socket_path: &Path,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), SandboxSecretLeaseError> {
        let listener = UnixListener::bind(socket_path)
            .map_err(|_| SandboxSecretLeaseError::BackendUnavailable)?;

        // Owner-only socket file permissions — the daemon and the sandbox
        // container's bind-mounted socket share this path, but nothing else
        // on the host should be able to connect.
        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(socket_path, permissions)
            .map_err(|_| SandboxSecretLeaseError::BackendUnavailable)?;

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
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _addr)) => {
                            let resolver = Arc::clone(&self.resolver);
                            let scope = self.scope.clone();
                            tokio::spawn(async move {
                                handle_connection(stream, resolver, scope).await;
                            });
                        }
                        Err(error) => {
                            tracing::debug!(?error, "sandbox secret lease: accept failed");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    resolver: Arc<dyn SandboxSecretLeaseResolver>,
    scope: ResourceScope,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let mut line = String::new();
    let read = reader.read_line(&mut line).await;
    let n = match read {
        Ok(n) => n,
        Err(error) => {
            tracing::debug!(?error, "sandbox secret lease: connection read failed");
            return;
        }
    };
    if n == 0 {
        // Client disconnected without sending a request.
        return;
    }

    let request: Result<SecretLeaseWireRequest, _> = serde_json::from_str(line.trim_end());
    let Ok(request) = request else {
        tracing::debug!("sandbox secret lease: malformed request");
        return;
    };

    let response = match resolver.resolve_lease(&scope, &request.secret_name).await {
        Ok(lease) => {
            tracing::debug!(
                secret_name = %request.secret_name,
                outcome = %"ok",
                "sandbox secret lease"
            );
            SecretLeaseWireResponse::Ok {
                material: lease.material.expose_secret().to_string(),
                ttl_seconds: lease.ttl_seconds,
            }
        }
        Err(error) => {
            tracing::debug!(
                secret_name = %request.secret_name,
                outcome = %"error",
                "sandbox secret lease"
            );
            SecretLeaseWireResponse::Error {
                reason: error.wire_reason(),
            }
        }
    };

    let Ok(mut bytes) = serde_json::to_vec(&response) else {
        return;
    };
    bytes.push(b'\n');
    if let Err(error) = write_half.write_all(&bytes).await {
        tracing::debug!(?error, "sandbox secret lease: connection write failed");
    }
    let _ = write_half.flush().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    /// Fake resolver: returns fixed/incrementing material for known names,
    /// `UnknownSecret` otherwise. `backend_unavailable_secret_names` holds
    /// names that should instead trip `BackendUnavailable` after touching
    /// `unavailable_internal_text` — simulating a resolver whose internal
    /// error text embeds real secret-shaped material that must never reach
    /// the wire.
    struct FakeResolver {
        known: std::collections::HashMap<String, std::sync::Mutex<Vec<String>>>,
        unavailable: std::collections::HashSet<String>,
        unavailable_internal_text: String,
    }

    impl FakeResolver {
        fn with_known(name: &str, materials: Vec<&str>) -> Self {
            let mut known = std::collections::HashMap::new();
            known.insert(
                name.to_string(),
                std::sync::Mutex::new(materials.into_iter().map(String::from).collect()),
            );
            Self {
                known,
                unavailable: std::collections::HashSet::new(),
                unavailable_internal_text: String::new(),
            }
        }

        fn with_unavailable(name: &str, internal_text: &str) -> Self {
            let mut unavailable = std::collections::HashSet::new();
            unavailable.insert(name.to_string());
            Self {
                known: std::collections::HashMap::new(),
                unavailable,
                unavailable_internal_text: internal_text.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl SandboxSecretLeaseResolver for FakeResolver {
        async fn resolve_lease(
            &self,
            _scope: &ResourceScope,
            secret_name: &str,
        ) -> Result<SandboxSecretLease, SandboxSecretLeaseError> {
            if self.unavailable.contains(secret_name) {
                // Internally hold/touch a string containing fake secret
                // material — this is exactly the shape of an upstream OAuth
                // error body a real resolver might carry. It must never
                // escape this function.
                let _ = &self.unavailable_internal_text;
                return Err(SandboxSecretLeaseError::BackendUnavailable);
            }
            match self.known.get(secret_name) {
                Some(materials) => {
                    let mut materials = materials.lock().expect("mutex not poisoned");
                    if materials.is_empty() {
                        return Err(SandboxSecretLeaseError::UnknownSecret);
                    }
                    let material = materials.remove(0);
                    Ok(SandboxSecretLease {
                        material: SecretMaterial::from(material),
                        ttl_seconds: Some(60),
                    })
                }
                None => Err(SandboxSecretLeaseError::UnknownSecret),
            }
        }
    }

    async fn raw_request(socket_path: &Path, body: &str) -> Vec<u8> {
        let mut client = UnixStream::connect(socket_path)
            .await
            .expect("client connects to the lease socket");
        client
            .write_all(format!("{body}\n").as_bytes())
            .await
            .expect("request writes");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("reads the full response then EOF");
        response
    }

    #[tokio::test]
    async fn known_secret_returns_ok_variant_with_material_over_the_socket() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("sock");
        let resolver: Arc<dyn SandboxSecretLeaseResolver> = Arc::new(FakeResolver::with_known(
            "github_token",
            vec!["ghp_fake_material"],
        ));
        let server = SandboxSecretLeaseServer::new(resolver, ResourceScope::system());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let socket_path_for_serve = socket_path.clone();
        let serve_handle = tokio::spawn(async move {
            server
                .bind_and_serve(&socket_path_for_serve, shutdown_rx)
                .await
        });

        // Give the listener a moment to bind before the client connects.
        wait_for_socket(&socket_path).await;

        let raw = raw_request(&socket_path, r#"{"secret_name":"github_token"}"#).await;
        let text = String::from_utf8(raw).expect("valid utf8");
        let value: serde_json::Value = serde_json::from_str(text.trim_end()).expect("valid json");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["material"], "ghp_fake_material");

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    #[tokio::test]
    async fn unknown_secret_returns_error_variant_with_no_material_field() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("sock");
        let resolver: Arc<dyn SandboxSecretLeaseResolver> = Arc::new(FakeResolver::with_known(
            "github_token",
            vec!["ghp_fake_material"],
        ));
        let server = SandboxSecretLeaseServer::new(resolver, ResourceScope::system());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let socket_path_for_serve = socket_path.clone();
        let serve_handle = tokio::spawn(async move {
            server
                .bind_and_serve(&socket_path_for_serve, shutdown_rx)
                .await
        });
        wait_for_socket(&socket_path).await;

        let raw = raw_request(&socket_path, r#"{"secret_name":"nonexistent"}"#).await;
        let text = String::from_utf8(raw).expect("valid utf8");
        assert!(
            !text.contains("material"),
            "error response must not carry a material key: {text}"
        );
        let value: serde_json::Value = serde_json::from_str(text.trim_end()).expect("valid json");
        assert_eq!(value["status"], "error");
        assert_eq!(value["reason"], "unknown_secret");

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    #[tokio::test]
    async fn backend_unavailable_error_never_leaks_resolver_internal_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("sock");
        let resolver: Arc<dyn SandboxSecretLeaseResolver> = Arc::new(
            FakeResolver::with_unavailable("flaky_secret", "upstream said: sk-live-abc123"),
        );
        let server = SandboxSecretLeaseServer::new(resolver, ResourceScope::system());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let socket_path_for_serve = socket_path.clone();
        let serve_handle = tokio::spawn(async move {
            server
                .bind_and_serve(&socket_path_for_serve, shutdown_rx)
                .await
        });
        wait_for_socket(&socket_path).await;

        let raw = raw_request(&socket_path, r#"{"secret_name":"flaky_secret"}"#).await;
        let text = String::from_utf8_lossy(&raw);
        assert!(
            !text.contains("sk-live-abc123"),
            "resolver internal error text leaked onto the wire: {text}"
        );
        assert!(text.contains("backend_unavailable"));

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    #[tokio::test]
    async fn each_request_gets_its_own_connection_and_prior_material_is_not_replayable_from_the_socket()
     {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("sock");
        let resolver: Arc<dyn SandboxSecretLeaseResolver> = Arc::new(FakeResolver::with_known(
            "rotating_secret",
            vec!["material-one", "material-two"],
        ));
        let server = SandboxSecretLeaseServer::new(resolver, ResourceScope::system());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let socket_path_for_serve = socket_path.clone();
        let serve_handle = tokio::spawn(async move {
            server
                .bind_and_serve(&socket_path_for_serve, shutdown_rx)
                .await
        });
        wait_for_socket(&socket_path).await;

        let first = raw_request(&socket_path, r#"{"secret_name":"rotating_secret"}"#).await;
        let second = raw_request(&socket_path, r#"{"secret_name":"rotating_secret"}"#).await;
        assert_ne!(
            first, second,
            "server must call resolve_lease fresh per connection, not cache the first response"
        );
        let first: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&first).trim_end()).expect("valid json");
        let second: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&second).trim_end()).expect("valid json");
        assert_eq!(first["material"], "material-one");
        assert_eq!(second["material"], "material-two");

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    #[tokio::test]
    async fn socket_file_permissions_are_owner_only_after_bind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("sock");
        let resolver: Arc<dyn SandboxSecretLeaseResolver> =
            Arc::new(FakeResolver::with_known("unused", vec!["unused"]));
        let server = SandboxSecretLeaseServer::new(resolver, ResourceScope::system());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let socket_path_for_serve = socket_path.clone();
        let serve_handle = tokio::spawn(async move {
            server
                .bind_and_serve(&socket_path_for_serve, shutdown_rx)
                .await
        });
        wait_for_socket(&socket_path).await;

        let metadata = std::fs::metadata(&socket_path).expect("socket file exists");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected socket file mode 0600, got {mode:o}");

        let _ = shutdown_tx.send(true);
        let _ = serve_handle.await;
    }

    /// Polls for the socket file to appear so tests don't race the spawned
    /// `bind_and_serve` task's initial bind.
    async fn wait_for_socket(socket_path: &Path) {
        for _ in 0..200 {
            if socket_path.exists() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("socket file never appeared at {socket_path:?}");
    }
}
