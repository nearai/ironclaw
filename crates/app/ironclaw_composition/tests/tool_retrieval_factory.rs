//! Composition wiring for the `tool_search` ranker: the parsed
//! `REBORN_TOOL_RETRIEVAL` mode selects `native` (bind nothing), `dense` (the
//! embedding ranker over the `[embeddings]` provider) or `hybrid` (BM25F fused
//! with that embedding ranker), and `dense` or `hybrid` without a buildable
//! embeddings provider refuses startup. Parsing the raw setting is
//! `ironclaw_config`'s and is tested there.
//!
//! The dense case runs against a loopback stub of the OpenAI `/v1/embeddings`
//! endpoint, both directly through the port and through `build_reborn_runtime`,
//! so "binds the dense provider" is proven by the runtime actually embedding
//! the authorized tool catalog, not by a constructor returning.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_composition::{
    PollSettings, RebornRuntimeIdentity, RebornRuntimeInput, ToolRetrievalConfigError,
    build_reborn_runtime, resolve_tool_retrieval_provider,
};
use ironclaw_config::{EmbeddingsSection, ToolRetrievalMode};
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_contracts::{ProviderToolDefinition, ToolSearchQueryClass};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse, ToolDisclosureMode,
};
use ironclaw_tool_retrieval::ToolVectorStoreSlot;
use ironclaw_turns::TurnStatus;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| map.get(name).cloned()
}

/// Words the stub endpoint knows; each is one vector dimension.
const VOCABULARY: &[&str] = &["email", "send", "code", "search", "weather", "tool"];

fn bag_of_words(text: &str) -> Vec<f32> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    VOCABULARY
        .iter()
        .map(|term| words.iter().filter(|word| *word == term).count() as f32)
        // Never an all-zero vector: a constant tail dimension.
        .chain(std::iter::once(0.01))
        .collect()
}

/// A loopback `/v1/embeddings` stub that serves any number of requests,
/// answering each input with its bag-of-words vector, and records every input.
async fn embeddings_stub() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let sink = Arc::clone(&sink);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0_u8; 8192];
                let body: serde_json::Value = loop {
                    let n = socket.read(&mut chunk).await.expect("read");
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let head = String::from_utf8_lossy(&buf[..end]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + len {
                        break serde_json::from_slice(&buf[end + 4..end + 4 + len])
                            .expect("json body");
                    }
                };
                let inputs: Vec<String> = body["input"]
                    .as_array()
                    .map(|inputs| {
                        inputs
                            .iter()
                            .filter_map(|input| input.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let data: Vec<_> = inputs
                    .iter()
                    .enumerate()
                    .map(|(i, text)| serde_json::json!({"index": i, "embedding": bag_of_words(text)}))
                    .collect();
                sink.lock().expect("lock").extend(inputs);
                let payload = serde_json::json!({"data": data}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (format!("http://{addr}"), seen)
}

fn embeddings_section(base_url: &str) -> EmbeddingsSection {
    EmbeddingsSection {
        provider: Some("openai_compatible".to_string()),
        base_url: Some(base_url.to_string()),
        model: Some("stub-embed".to_string()),
        ..Default::default()
    }
}

fn tool(capability_id: &str, description: &str) -> ProviderToolDefinition {
    ProviderToolDefinition::from_parts(
        CapabilityId::new(capability_id).expect("capability id"),
        capability_id.replace('.', "__"),
        description,
        serde_json::json!({"type": "object", "properties": {}}),
    )
    .expect("definition")
}

#[tokio::test]
async fn native_binds_nothing_even_with_embeddings_configured() {
    // An unreachable endpoint: native must not touch it.
    let section = embeddings_section("http://127.0.0.1:9");
    let provider = resolve_tool_retrieval_provider(
        ToolRetrievalMode::Native,
        Some(&section),
        &env_of(&[]),
        ToolVectorStoreSlot::new(),
    )
    .await
    .expect("native resolves");
    assert!(provider.is_none());
}

#[tokio::test]
async fn dense_and_hybrid_without_an_embeddings_provider_refuse() {
    // A configured but unbuildable provider (unknown id) refuses too.
    let broken = EmbeddingsSection {
        provider: Some("mystery".to_string()),
        model: Some("m".to_string()),
        ..Default::default()
    };
    for mode in [ToolRetrievalMode::Dense, ToolRetrievalMode::Hybrid] {
        for section in [
            None,
            Some(EmbeddingsSection::default()),
            Some(broken.clone()),
        ] {
            let error = resolve_tool_retrieval_provider(
                mode,
                section.as_ref(),
                &env_of(&[]),
                ToolVectorStoreSlot::new(),
            )
            .await
            .expect_err("dense and hybrid need a buildable embeddings provider");
            assert_eq!(error, ToolRetrievalConfigError::EmbeddingsRequired { mode });
            let message = error.to_string();
            assert!(
                message.contains(&format!("REBORN_TOOL_RETRIEVAL={}", mode.as_str())),
                "{message}"
            );
        }
    }
}

#[tokio::test]
async fn dense_binds_the_embedding_ranker_over_the_configured_endpoint() {
    let (base_url, seen) = embeddings_stub().await;
    // Selected through the EMBEDDING_* overrides alone, no config section.
    let env = env_of(&[
        ("EMBEDDING_PROVIDER", "openai_compatible"),
        ("EMBEDDING_BASE_URL", &base_url),
        ("EMBEDDING_MODEL", "stub-embed"),
    ]);
    let provider = resolve_tool_retrieval_provider(
        ToolRetrievalMode::Dense,
        None,
        &env,
        ToolVectorStoreSlot::new(),
    )
    .await
    .expect("dense resolves")
    .expect("dense binds a provider");
    assert_eq!(provider.ranker_version(), "dense-cosine-v1");

    let index = provider
        .fit(&[
            tool("mail.send", "Send an email"),
            tool("repo.search", "Search code"),
            tool("weather.today", "Weather"),
        ])
        .await
        .expect("fit embeds through the endpoint");
    let outcome = index.search("send email", 5).await.expect("search");
    assert_eq!(outcome.query_class, ToolSearchQueryClass::Lexical);
    assert_eq!(outcome.ranked[0].name, "mail__send");
    assert!(outcome.ranked[0].score > 0.0 && outcome.ranked[0].score <= 1.0);

    let seen = seen.lock().expect("lock").clone();
    assert_eq!(
        seen.len(),
        4,
        "three tool documents and one query: {seen:?}"
    );
    assert!(seen.iter().any(|input| input == "send email"));
}

/// `hybrid` binds BM25F fused with the embedding ranker: a query BM25F
/// cannot answer at all is answered through the dense side, an exact
/// identifier stays an identifier lookup, and a dense endpoint that stops
/// answering leaves plain BM25F ranking rather than a failed search.
#[tokio::test]
async fn hybrid_binds_bm25f_fused_with_the_embedding_ranker() {
    let (base_url, seen) = embeddings_stub().await;
    let section = embeddings_section(&base_url);
    let provider = resolve_tool_retrieval_provider(
        ToolRetrievalMode::Hybrid,
        Some(&section),
        &env_of(&[]),
        ToolVectorStoreSlot::new(),
    )
    .await
    .expect("hybrid resolves")
    .expect("hybrid binds a provider");
    assert_eq!(
        provider.ranker_version(),
        "hybrid-rrf-v1(bounded-bm25f-v1,dense-cosine-v1)"
    );

    let corpus = [
        tool("mail.deliver", "Deliver a message: send an email"),
        tool("repo.search", "Search code"),
        tool("weather.today", "Weather"),
    ];
    let index = provider.fit(&corpus).await.expect("fit");
    assert!(
        seen.lock()
            .expect("lock")
            .iter()
            .any(|input| input.starts_with("tool: ")),
        "the hybrid fit embeds the tool documents"
    );

    let fused = index.search("search tool", 5).await.expect("search");
    assert_eq!(fused.query_class, ToolSearchQueryClass::Lexical);
    assert_eq!(fused.ranked[0].name, "repo__search");
    assert!(
        seen.lock()
            .expect("lock")
            .iter()
            .any(|input| input == "search tool"),
        "a non-identifier query consults the dense side"
    );

    let exact = index.search("weather__today", 5).await.expect("search");
    assert_eq!(exact.query_class, ToolSearchQueryClass::ExactIdentifier);
    assert_eq!(exact.ranked[0].name, "weather__today");

    // Same catalog, dense endpoint unreachable: the fit and every search
    // still succeed, ranked by BM25F alone.
    let unreachable = embeddings_section("http://127.0.0.1:9");
    let degraded = resolve_tool_retrieval_provider(
        ToolRetrievalMode::Hybrid,
        Some(&unreachable),
        &env_of(&[]),
        ToolVectorStoreSlot::new(),
    )
    .await
    .expect("hybrid resolves")
    .expect("hybrid binds a provider");
    let index = degraded
        .fit(&corpus)
        .await
        .expect("fit degrades, never fails");
    let outcome = index.search("search code", 5).await.expect("search");
    assert_eq!(outcome.query_class, ToolSearchQueryClass::Lexical);
    assert_eq!(outcome.ranked[0].name, "repo__search");
}

// ─── through build_reborn_runtime ────────────────────────────────────

#[derive(Debug)]
struct ReplyGateway;

#[async_trait]
impl HostManagedModelGateway for ReplyGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply("ok".to_string()))
    }
}

fn local_host_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalHost,
        resolved_profile: RuntimeProfile::LocalHost,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

/// The resolved dense or hybrid provider, bound through
/// `RebornRuntimeInput::with_tool_retrieval_provider`, is the ranker the real
/// runtime fits over the run's authorized tool surface: the embeddings stub
/// receives the tool documents.
#[tokio::test]
async fn dense_selection_is_fitted_by_the_built_runtime() {
    selection_is_fitted_by_the_built_runtime(ToolRetrievalMode::Dense).await;
}

#[tokio::test]
async fn hybrid_selection_is_fitted_by_the_built_runtime() {
    selection_is_fitted_by_the_built_runtime(ToolRetrievalMode::Hybrid).await;
}

async fn selection_is_fitted_by_the_built_runtime(mode: ToolRetrievalMode) {
    Box::pin(async {
        let (base_url, seen) = embeddings_stub().await;
        let section = embeddings_section(&base_url);
        let root = tempfile::tempdir().expect("tempdir");

        // First process: the runtime fits the ranker over its tool documents.
        run_one_turn(mode, &section, root.path()).await;
        let first = std::mem::take(&mut *seen.lock().expect("lock"));
        assert!(
            first.iter().any(|input| input.starts_with("tool: ")),
            "the runtime must fit the {} ranker over its tool documents, saw {first:?}",
            mode.as_str()
        );

        // Second process over the same storage: the vectors were persisted
        // per user under `/tool-vectors`, so nothing is embedded again.
        run_one_turn(mode, &section, root.path()).await;
        let second = seen.lock().expect("lock").clone();
        assert!(
            !second.iter().any(|input| input.starts_with("tool: ")),
            "after a restart the {} ranker reads its stored vectors instead of \
             re-embedding the catalog, saw {second:?}",
            mode.as_str()
        );
    })
    .await;
}

/// Build a runtime over `root` with a freshly resolved ranker (an empty
/// memory cache, as after a restart), run one turn, and shut it down.
async fn run_one_turn(
    mode: ToolRetrievalMode,
    section: &EmbeddingsSection,
    root: &std::path::Path,
) {
    let input = RebornRuntimeInput::from_build_input(
        ironclaw_composition::local_filesystem_build_input(
            "tool-retrieval-owner",
            root.join("standalone"),
        )
        .with_runtime_policy(local_host_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "tool-retrieval-tenant".to_string(),
        agent_id: "tool-retrieval-agent".to_string(),
        source_binding_id: "tool-retrieval-source".to_string(),
        reply_target_binding_id: "tool-retrieval-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(10),
    })
    .with_model_gateway_override(Arc::new(ReplyGateway))
    .with_tool_disclosure(ToolDisclosureMode::Bridged);
    let provider = resolve_tool_retrieval_provider(
        mode,
        Some(section),
        &env_of(&[]),
        input.tool_vector_store.clone(),
    )
    .await
    .expect("mode resolves")
    .expect("mode binds a provider");
    let input = input.with_tool_retrieval_provider(provider);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        Duration::from_secs(10),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed);
    runtime.shutdown().await.expect("runtime shutdown");
}
