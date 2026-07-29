//! Production [`AlpacaPort`]: HTTP/1.1 over the sidecar's Unix domain socket
//! (attested-signing §E2).
//!
//! ## Why a hand-rolled client rather than hyper
//!
//! The sidecar deliberately serves with Node's stdlib `http` and no framework;
//! this is the symmetric choice. The peer is a single, local, supervised child
//! of this process — one JSON request and one small JSON response per call,
//! always with `Content-Length` — so the alternative (`hyper` + a UDS connector)
//! would add a dependency family to speak a fraction of a protocol we fully
//! control both ends of. Reviewers who would rather take that dependency should
//! say so; the port boundary makes the swap a single-file change.
//!
//! Everything an attacker could influence is bounded: the response size, the
//! header count, and the total time. The socket lives in a `0700` directory and
//! every request carries the per-boot token, but this code still treats the
//! peer's bytes as untrusted input.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

use crate::alpaca::{AlpacaError, AlpacaPort, BroadcastRequest, CombineRequest, CraftRequest};

/// Wire version this client speaks. Must match the sidecar's `WIRE_VERSION`.
const WIRE_VERSION: u32 = 1;

/// Header carrying the per-boot shared token.
const TOKEN_HEADER: &str = "x-alpaca-token";

/// Hard ceiling on a response body. Sidecar replies are small JSON envelopes;
/// this bounds a compromised or malfunctioning peer's ability to exhaust memory.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Ceiling on the header section, for the same reason.
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Per-call deadlines. Broadcast gets longer: it waits on a chain RPC.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10);
const BROADCAST_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP-over-UDS client for the Alpaca sidecar.
pub struct UdsAlpacaPort {
    socket_path: PathBuf,
    token: String,
}

impl UdsAlpacaPort {
    /// Build a client for a socket path and the per-boot token.
    pub fn new(socket_path: impl AsRef<Path>, token: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_path_buf(),
            token: token.into(),
        }
    }

    async fn call(
        &self,
        method: &str,
        currency_id: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<String, AlpacaError> {
        let envelope = serde_json::json!({
            "version": WIRE_VERSION,
            "currencyId": currency_id,
            "params": params,
        });
        let body = serde_json::to_vec(&envelope).map_err(|error| AlpacaError::BadRequest {
            reason: format!("request could not be encoded: {error}"),
        })?;

        let raw = tokio::time::timeout(timeout, self.round_trip(method, &body))
            .await
            .map_err(|_| AlpacaError::Upstream {
                reason: format!("{method} timed out"),
            })??;

        parse_envelope(&raw)
    }

    /// One request/response over a fresh connection.
    ///
    /// A connection per call: these are infrequent, local, and short, so pooling
    /// would add failure modes (half-open sockets across a sidecar restart) for
    /// no meaningful gain.
    async fn round_trip(&self, method: &str, body: &[u8]) -> Result<Vec<u8>, AlpacaError> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|error| AlpacaError::Unavailable {
                reason: format!("could not connect to the sidecar socket: {error}"),
            })?;

        let request = format!(
            "POST /v1/{method} HTTP/1.1\r\n\
             Host: localhost\r\n\
             {TOKEN_HEADER}: {token}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {len}\r\n\
             Connection: close\r\n\
             \r\n",
            token = self.token,
            len = body.len(),
        );
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(write_failed)?;
        stream.write_all(body).await.map_err(write_failed)?;
        stream.flush().await.map_err(write_failed)?;

        // `Connection: close` means the peer closes when done, so read to EOF
        // under the size ceiling rather than trusting a length it supplied.
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 8 * 1024];
        loop {
            let read = stream
                .read(&mut chunk)
                .await
                .map_err(|error| AlpacaError::Upstream {
                    reason: format!("reading the sidecar response failed: {error}"),
                })?;
            if read == 0 {
                break;
            }
            if buffer.len() + read > MAX_RESPONSE_BYTES {
                return Err(AlpacaError::Upstream {
                    reason: "sidecar response exceeded the maximum size".to_string(),
                });
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        Ok(buffer)
    }
}

fn write_failed(error: std::io::Error) -> AlpacaError {
    AlpacaError::Unavailable {
        reason: format!("writing to the sidecar failed: {error}"),
    }
}

/// Split an HTTP/1.1 response into (status, body).
///
/// Deliberately minimal: the peer is our own sidecar, which always sends a
/// status line, headers, and a body. Anything else is a protocol error rather
/// than something to recover from cleverly.
fn split_response(raw: &[u8]) -> Result<(u16, &[u8]), AlpacaError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| AlpacaError::Upstream {
            reason: "sidecar response had no header terminator".to_string(),
        })?;
    if split > MAX_HEADER_BYTES {
        return Err(AlpacaError::Upstream {
            reason: "sidecar response headers exceeded the maximum size".to_string(),
        });
    }
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| AlpacaError::Upstream {
        reason: "sidecar response headers were not valid utf-8".to_string(),
    })?;
    let status_line = head.lines().next().ok_or_else(|| AlpacaError::Upstream {
        reason: "sidecar response had no status line".to_string(),
    })?;
    // "HTTP/1.1 200 OK"
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| AlpacaError::Upstream {
            reason: "sidecar response had an unparseable status line".to_string(),
        })?;
    Ok((status, &raw[split + 4..]))
}

/// Decode the sidecar's response envelope into a result.
fn parse_envelope(raw: &[u8]) -> Result<String, AlpacaError> {
    let (status, body) = split_response(raw)?;
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| AlpacaError::Upstream {
            reason: format!("sidecar response was not valid JSON: {error}"),
        })?;

    if value.get("version").and_then(serde_json::Value::as_u64) != Some(u64::from(WIRE_VERSION)) {
        // Version skew must fail loudly rather than be best-effort parsed —
        // the same rule the sidecar applies to requests.
        return Err(AlpacaError::BadRequest {
            reason: "sidecar replied with an unsupported wire version".to_string(),
        });
    }

    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        return value
            .get("result")
            .map(|result| match result.as_str() {
                Some(text) => text.to_string(),
                None => result.to_string(),
            })
            .ok_or_else(|| AlpacaError::Upstream {
                reason: "sidecar success envelope carried no result".to_string(),
            });
    }

    // Map the sidecar's category onto ours. An unrecognized code is treated as
    // upstream rather than assumed benign.
    let code = value
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let message = value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("sidecar reported a failure")
        .to_string();
    Err(match code {
        "unauthorized" => AlpacaError::Unavailable {
            reason: "sidecar rejected our token".to_string(),
        },
        "bad_request" => AlpacaError::BadRequest { reason: message },
        "unsupported_chain" => AlpacaError::UnsupportedChain,
        _ => AlpacaError::Upstream {
            reason: format!("{message} (http {status})"),
        },
    })
}

#[async_trait]
impl AlpacaPort for UdsAlpacaPort {
    async fn craft_transaction(&self, request: CraftRequest) -> Result<String, AlpacaError> {
        self.call(
            "craftTransaction",
            request.currency_id.as_str(),
            request.params,
            DEFAULT_CALL_TIMEOUT,
        )
        .await
    }

    async fn combine(&self, request: CombineRequest) -> Result<String, AlpacaError> {
        self.call(
            "combine",
            request.currency_id.as_str(),
            serde_json::json!({
                "unsignedTx": request.unsigned_tx,
                "signature": request.signature,
            }),
            DEFAULT_CALL_TIMEOUT,
        )
        .await
    }

    async fn broadcast(&self, request: BroadcastRequest) -> Result<String, AlpacaError> {
        self.call(
            "broadcast",
            request.currency_id.as_str(),
            serde_json::json!({ "rawTx": request.raw_tx }),
            BROADCAST_TIMEOUT,
        )
        .await
    }

    async fn healthy(&self) -> bool {
        // GET /healthz, unauthenticated by design. Any failure is unhealthy —
        // a supervisor must never read ambiguity as liveness.
        let Ok(Ok(raw)) = tokio::time::timeout(Duration::from_secs(2), async {
            let mut stream = UnixStream::connect(&self.socket_path).await?;
            stream
                .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await?;
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let read = stream.read(&mut chunk).await?;
                if read == 0 || buffer.len() > MAX_HEADER_BYTES {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            Ok::<_, std::io::Error>(buffer)
        })
        .await
        else {
            return false;
        };
        matches!(split_response(&raw), Ok((200, _)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alpaca::CurrencyId;
    use tokio::net::UnixListener;

    /// Serve one canned HTTP response on a socket, capturing the request.
    async fn serve_once(response: &'static str) -> (PathBuf, tokio::task::JoinHandle<String>) {
        // Short path: `sun_path` is capped near 104 bytes.
        let dir = std::env::temp_dir().join(format!("ic-uds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(format!("t{}.sock", unique_suffix()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buffer = vec![0u8; 8192];
            let read = stream.read(&mut buffer).await.expect("read");
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            stream.write_all(response.as_bytes()).await.expect("write");
            stream.shutdown().await.ok();
            request
        });
        (path, handle)
    }

    /// Monotonic per-process counter for socket names.
    ///
    /// A clock-derived suffix collides when tests run in parallel — two sockets
    /// landing on one path made this suite flake before the counter. A test
    /// helper that can produce a false failure is worse than no helper.
    fn unique_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    fn ok_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
    }

    #[tokio::test]
    async fn a_successful_call_round_trips_and_sends_the_token() {
        let (path, server) = serve_once(Box::leak(
            ok_response(r#"{"version":1,"ok":true,"result":"0xcrafted"}"#).into_boxed_str(),
        ))
        .await;
        let port = UdsAlpacaPort::new(&path, "the-boot-token");

        let result = port
            .craft_transaction(CraftRequest {
                currency_id: CurrencyId::new("ethereum_sepolia"),
                params: serde_json::json!({"nonce": 7}),
            })
            .await
            .expect("craft succeeds");
        assert_eq!(result, "0xcrafted");

        let request = server.await.expect("server task");
        assert!(request.starts_with("POST /v1/craftTransaction HTTP/1.1"));
        assert!(
            request.contains("x-alpaca-token: the-boot-token"),
            "every call must carry the per-boot token"
        );
        assert!(
            request.contains(r#""currencyId":"ethereum_sepolia""#),
            "the envelope carries the chain selector"
        );
        assert!(request.contains(r#""version":1"#));
    }

    /// Version skew fails loudly rather than being best-effort parsed — the
    /// same rule the sidecar applies to our requests.
    #[tokio::test]
    async fn an_unsupported_response_version_is_refused() {
        let (path, _server) = serve_once(Box::leak(
            ok_response(r#"{"version":99,"ok":true,"result":"x"}"#).into_boxed_str(),
        ))
        .await;
        let port = UdsAlpacaPort::new(&path, "t");
        assert!(matches!(
            port.broadcast(BroadcastRequest {
                currency_id: CurrencyId::new("c"),
                raw_tx: "0x".to_string(),
            })
            .await,
            Err(AlpacaError::BadRequest { .. })
        ));
    }

    #[tokio::test]
    async fn sidecar_error_categories_map_onto_ours() {
        for (body, expect_unsupported) in [
            (
                r#"{"version":1,"ok":false,"code":"unsupported_chain","message":"no api"}"#,
                true,
            ),
            (
                r#"{"version":1,"ok":false,"code":"upstream","message":"rpc down"}"#,
                false,
            ),
        ] {
            let (path, _server) = serve_once(Box::leak(ok_response(body).into_boxed_str())).await;
            let port = UdsAlpacaPort::new(&path, "t");
            let result = port
                .broadcast(BroadcastRequest {
                    currency_id: CurrencyId::new("c"),
                    raw_tx: "0x".to_string(),
                })
                .await;
            if expect_unsupported {
                assert_eq!(result, Err(AlpacaError::UnsupportedChain));
            } else {
                assert!(matches!(result, Err(AlpacaError::Upstream { .. })));
            }
        }
    }

    /// A rejected token must surface as unavailable, not as a chain failure: it
    /// is a deployment fault, and mapping it to `Upstream` would invite a retry
    /// that can never succeed.
    #[tokio::test]
    async fn a_rejected_token_is_unavailable_not_upstream() {
        let (path, _server) = serve_once(Box::leak(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 62\r\n\r\n{\"version\":1,\"ok\":false,\"code\":\"unauthorized\",\"message\":\"nope\"}"
                .to_string()
                .into_boxed_str(),
        ))
        .await;
        let port = UdsAlpacaPort::new(&path, "wrong");
        assert!(matches!(
            port.combine(CombineRequest {
                currency_id: CurrencyId::new("c"),
                unsigned_tx: "0x".to_string(),
                signature: "0x".to_string(),
            })
            .await,
            Err(AlpacaError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn a_missing_socket_is_unavailable() {
        let port = UdsAlpacaPort::new("/tmp/definitely-not-a-socket-ic", "t");
        assert!(matches!(
            port.craft_transaction(CraftRequest {
                currency_id: CurrencyId::new("c"),
                params: serde_json::Value::Null,
            })
            .await,
            Err(AlpacaError::Unavailable { .. })
        ));
        // And the probe reports unhealthy rather than erroring out.
        assert!(!port.healthy().await);
    }

    #[tokio::test]
    async fn a_malformed_response_is_an_upstream_failure_not_a_panic() {
        for raw in ["not http at all", "HTTP/1.1 200 OK\r\n\r\nnot json"] {
            let (path, _server) = serve_once(Box::leak(raw.to_string().into_boxed_str())).await;
            let port = UdsAlpacaPort::new(&path, "t");
            assert!(matches!(
                port.broadcast(BroadcastRequest {
                    currency_id: CurrencyId::new("c"),
                    raw_tx: "0x".to_string(),
                })
                .await,
                Err(AlpacaError::Upstream { .. })
            ));
        }
    }

    #[tokio::test]
    async fn the_health_probe_reads_a_200_as_healthy() {
        let (path, _server) = serve_once(Box::leak(
            ok_response(r#"{"version":1,"ok":true,"result":"ok"}"#).into_boxed_str(),
        ))
        .await;
        assert!(UdsAlpacaPort::new(&path, "t").healthy().await);
    }

    /// The Rust half of the cross-language contract check.
    ///
    /// These are the SAME files `sidecars/alpaca/test/fixtures.test.ts` reads.
    /// Nothing in either type system connects a TypeScript sidecar to a Rust
    /// caller, so a shared fixture is the only thing that makes a silent
    /// divergence impossible: change the shape on one side and this fails.
    mod fixtures {
        use super::*;

        fn fixture(name: &str) -> Vec<u8> {
            // From this crate up to the workspace root, then into the sidecar.
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../sidecars/alpaca/fixtures")
                .join(format!("{name}.json"));
            std::fs::read(&path)
                .unwrap_or_else(|error| panic!("fixture {} unreadable: {error}", path.display()))
        }

        /// Wrap a fixture body in the HTTP envelope the client actually parses.
        fn as_http(body: &[u8]) -> Vec<u8> {
            let mut raw = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .into_bytes();
            raw.extend_from_slice(body);
            raw
        }

        #[test]
        fn the_success_fixture_decodes_to_its_result() {
            let decoded = parse_envelope(&as_http(&fixture("response-ok")))
                .expect("the golden success envelope must decode");
            assert_eq!(decoded, "0x02f86b0182000782520894");
        }

        #[test]
        fn the_error_fixture_maps_to_the_shared_category() {
            let decoded = parse_envelope(&as_http(&fixture("response-error")));
            assert_eq!(
                decoded,
                Err(AlpacaError::UnsupportedChain),
                "the shared error code must map to the same category on both sides"
            );
        }

        /// The request fixture is what THIS client emits, so its shape is
        /// asserted against a freshly-built envelope rather than eyeballed.
        #[test]
        fn the_request_fixture_matches_what_this_client_sends() {
            let golden: serde_json::Value =
                serde_json::from_slice(&fixture("request-craft")).expect("valid fixture json");
            assert_eq!(
                golden.get("version").and_then(serde_json::Value::as_u64),
                Some(u64::from(WIRE_VERSION)),
                "the fixture must declare the version this client speaks"
            );
            // The envelope keys are the contract; `params` is opaque by design.
            assert!(golden.get("currencyId").is_some());
            assert!(golden.get("params").is_some());
        }
    }
}
