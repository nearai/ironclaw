//! Caller-level tests for [`RebornProviderAdmin::probe_candidate`] against a
//! live loopback HTTP stub.
//!
//! Lives outside `src/` on purpose: the architecture boundary test
//! `reborn_product_api_crates_do_not_bind_http_ingress`
//! (`crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`)
//! greps every `.rs` file under this crate's `src/` for
//! `TcpListener::bind` with no `#[cfg(test)]` awareness — a text scan, not
//! a compile-aware check, by design. A stub server that never runs in
//! production still trips it if it lives in-module. `tests/` sits outside
//! the scanned roots; `webui_v2_serve.rs` in this same crate already binds
//! a loopback listener here for the same reason.

#![cfg(feature = "root-llm-provider")]

use ironclaw_llm::ProviderProtocol;
use ironclaw_reborn_composition::{ProviderRepo, RebornProviderAdmin};
use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};

/// One captured request: method, path, and the raw `Authorization` header
/// value (if any).
struct CapturedProbeRequest {
    method_and_path: String,
    authorization: Option<String>,
}

/// Serve one canned model-listing response on a loopback port, capturing
/// the request that reached it. Mirrors `rig_adapter`'s
/// `endpoint_against_canned_response` test helper, plus request capture so
/// this test can assert `probe_candidate` actually reached the CONFIGURED
/// base URL with the ENTERED key — the seam that regressed once already
/// (probe used an empty base URL and always reported "could not reach").
async fn spawn_models_stub(
    status_line: &'static str,
    body: &'static str,
) -> (String, tokio::sync::oneshot::Receiver<CapturedProbeRequest>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let base_url = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((sock, _)) = listener.accept().await else {
            return;
        };
        let mut reader = BufReader::new(sock);
        let mut request_line = String::new();
        let _ = reader.read_line(&mut request_line).await;
        let mut authorization = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':')
                && name.trim().eq_ignore_ascii_case("authorization")
            {
                authorization = Some(value.trim().to_string());
            }
        }
        let _ = tx.send(CapturedProbeRequest {
            method_and_path: request_line.trim_end_matches(['\r', '\n']).to_string(),
            authorization,
        });
        let response = format!(
            "{status_line}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}",
            body.len()
        );
        let mut sock = reader.into_inner();
        let _ = sock.write_all(response.as_bytes()).await;
        let _ = sock.flush().await;
    });
    (base_url, rx)
}

/// Write a `providers.json` overlay entry pointed at `base_url`, mirroring
/// how a real onboard candidate is built from the registry: a fresh id
/// (never a builtin), OpenAI-compatible protocol, and no `api_key_env`
/// (the "entered" key is the inline candidate `probe_candidate` takes,
/// never persisted to env or overlay).
fn write_stub_provider(home: &RebornHome, base_url: &str) {
    std::fs::create_dir_all(home.path()).expect("create reborn home dir");
    ProviderRepo::new(home.providers_file_path())
        .upsert(ironclaw_llm::registry::ProviderDefinition {
            id: "stub-probe-provider".to_string(),
            aliases: Vec::new(),
            protocol: ProviderProtocol::OpenAiCompletions,
            default_base_url: Some(base_url.to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: None,
            api_key_required: true,
            model_env: "STUB_PROBE_MODEL".to_string(),
            default_model: "stub-model".to_string(),
            description: "stub probe provider".to_string(),
            extra_headers_env: None,
            setup: None,
            unsupported_params: Vec::new(),
        })
        .expect("write stub provider overlay entry");
}

/// Pins the bug this test was written to catch: `probe_candidate` must
/// build its request against the provider's CONFIGURED base URL (from the
/// registry/overlay) carrying the ENTERED key (the inline candidate
/// argument), not a blank/default URL. A regression back to an empty base
/// URL would make the stub never see a connection and this test would time
/// out / fail on `ok`.
#[tokio::test]
async fn probe_candidate_hits_the_configured_base_url_with_the_entered_key() {
    let (base_url, request_rx) =
        spawn_models_stub("HTTP/1.1 200 OK", r#"{"data":[{"id":"stub-model-1"}]}"#).await;

    let temp = tempfile::tempdir().expect("tempdir");
    let home = RebornHome::resolve_from_env_parts(
        Some(temp.path().join("reborn-home").as_os_str().to_os_string()),
        None,
        None,
    )
    .expect("valid reborn home");
    write_stub_provider(&home, &base_url);

    let admin = RebornProviderAdmin::new(RebornBootConfig::new(home, RebornProfile::LocalDev));

    let outcome = admin
        .probe_candidate(
            "stub-probe-provider",
            Some(secrecy::SecretString::from("sk-entered-key")),
            None,
        )
        .await
        .expect("stub-probe-provider is registered");

    assert!(
        outcome.ok,
        "probe against the live stub must succeed: {outcome:?}"
    );
    assert_eq!(outcome.models, vec!["stub-model-1".to_string()]);

    let captured = request_rx
        .await
        .expect("stub must have received exactly one request");
    assert_eq!(captured.method_and_path, "GET /v1/models HTTP/1.1");
    assert_eq!(
        captured.authorization.as_deref(),
        Some("Bearer sk-entered-key"),
        "probe must carry the entered key as a Bearer token"
    );
}

/// A 401 from the configured endpoint must surface as `ok: false` (never a
/// bare `Err`) — matching `probe_candidate_provider`'s no-separate-error-
/// channel contract that `probe_candidate` forwards. Awaits the captured
/// request (not just `outcome.ok`) so a transport failure (wrong URL,
/// connection refused) can't pass identically to a real 401.
#[tokio::test]
async fn probe_candidate_reports_401_as_not_ok() {
    let (base_url, request_rx) = spawn_models_stub("HTTP/1.1 401 Unauthorized", "").await;

    let temp = tempfile::tempdir().expect("tempdir");
    let home = RebornHome::resolve_from_env_parts(
        Some(temp.path().join("reborn-home").as_os_str().to_os_string()),
        None,
        None,
    )
    .expect("valid reborn home");
    write_stub_provider(&home, &base_url);

    let admin = RebornProviderAdmin::new(RebornBootConfig::new(home, RebornProfile::LocalDev));

    let outcome = admin
        .probe_candidate(
            "stub-probe-provider",
            Some(secrecy::SecretString::from("sk-wrong-key")),
            None,
        )
        .await
        .expect("stub-probe-provider is registered");

    assert!(!outcome.ok, "a 401 must report ok: false, got {outcome:?}");

    let captured = request_rx
        .await
        .expect("stub must have received exactly one request");
    assert_eq!(captured.method_and_path, "GET /v1/models HTTP/1.1");
    assert_eq!(
        captured.authorization.as_deref(),
        Some("Bearer sk-wrong-key"),
        "probe must carry the entered key as a Bearer token"
    );
}
