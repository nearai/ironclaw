//! Composition wiring for the text-embeddings provider: an `[embeddings]`
//! section selects a provider by id, and anything short of a known id yields
//! no provider (fail closed).
//!
//! Drives the public `resolve_embedding_provider` against a loopback stub of
//! the OpenAI `/v1/embeddings` endpoint so the configured case is proven to
//! embed a batch end to end, not merely to construct.

use std::collections::HashMap;

use ironclaw_composition::resolve_embedding_provider;
use ironclaw_config::{EmbeddingsSection, RebornConfigFile};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| map.get(name).cloned()
}

/// Serve one `/v1/embeddings` request: a 2-dimensional vector per input.
/// Returns the base URL and a handle yielding the raw request head + body.
async fn one_shot_stub() -> (String, tokio::task::JoinHandle<(String, serde_json::Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buf = Vec::new();
        let mut chunk = [0_u8; 4096];
        let (head, body) = loop {
            let n = socket.read(&mut chunk).await.expect("read");
            assert!(n > 0, "closed early");
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
                let body: serde_json::Value =
                    serde_json::from_slice(&buf[end + 4..end + 4 + len]).expect("json body");
                break (head, body);
            }
        };
        let count = body["input"].as_array().map_or(0, Vec::len);
        let data: Vec<_> = (0..count)
            .map(|i| serde_json::json!({"index": i, "embedding": [i as f32, 1.0]}))
            .collect();
        let payload = serde_json::json!({"data": data}).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{payload}",
            payload.len()
        );
        socket.write_all(response.as_bytes()).await.expect("write");
        (head, body)
    });
    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn a_configured_section_builds_a_provider_that_embeds_a_batch() {
    let (base_url, served) = one_shot_stub().await;
    let toml = format!(
        "[embeddings]\nprovider = \"openai_compatible\"\nbase_url = \"{base_url}\"\n\
         model = \"nomic-embed-text\"\napi_key_env = \"TEST_EMBED_KEY\"\ndimension = 2\n"
    );
    let config = RebornConfigFile::parse_text(&toml, std::path::Path::new("/test/config.toml"))
        .expect("config parses");

    let env = env_of(&[("TEST_EMBED_KEY", "k-embed")]);
    let provider = resolve_embedding_provider(config.embeddings.as_ref(), &env)
        .await
        .expect("configured provider is built");

    let vectors = provider
        .embed(&["alpha".to_string(), "beta".to_string()])
        .await
        .expect("batch embeds");
    assert_eq!(vectors, vec![vec![0.0, 1.0], vec![1.0, 1.0]]);
    assert_eq!(provider.dimension(), Some(2));

    let (head, body) = served.await.expect("stub finished");
    assert!(head.starts_with("POST /v1/embeddings "), "{head}");
    assert!(
        head.lines()
            .any(|l| l.eq_ignore_ascii_case("authorization: Bearer k-embed")),
        "{head}"
    );
    assert_eq!(
        body,
        serde_json::json!({"model": "nomic-embed-text", "input": ["alpha", "beta"]})
    );
}

#[tokio::test]
async fn an_unset_config_gives_no_provider() {
    // Even with a key lying around in the environment.
    let env = env_of(&[("EMBEDDING_API_KEY", "sk-stray")]);
    assert!(resolve_embedding_provider(None, &env).await.is_none());
    assert!(
        resolve_embedding_provider(Some(&EmbeddingsSection::default()), &env)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn an_unknown_provider_id_gives_no_provider() {
    let env = env_of(&[("EMBEDDING_API_KEY", "sk-test")]);
    let section = EmbeddingsSection {
        provider: Some("not-a-provider".to_string()),
        model: Some("text-embedding-3-small".to_string()),
        ..Default::default()
    };
    assert!(
        resolve_embedding_provider(Some(&section), &env)
            .await
            .is_none()
    );

    // Selecting an unknown id through the env override fails closed too.
    let env = env_of(&[
        ("EMBEDDING_API_KEY", "sk-test"),
        ("EMBEDDING_PROVIDER", "mystery"),
        ("EMBEDDING_MODEL", "m"),
    ]);
    assert!(resolve_embedding_provider(None, &env).await.is_none());
}
