//! Deterministic stub embedder and catalog shared by this package's test
//! binaries.
#![allow(dead_code)] // each test binary uses a different subset

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_host_api::capability::CapabilityDescriptionTrust;
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_llm::embeddings::{EmbeddingError, EmbeddingProvider};
use ironclaw_loop_contracts::ProviderToolDefinition;
use ironclaw_tool_retrieval::DenseToolRetrievalProvider;
use serde_json::json;

/// Words the stub embedder knows; each is one vector dimension.
pub const VOCABULARY: &[&str] = &[
    "email",
    "send",
    "message",
    "code",
    "search",
    "repository",
    "calendar",
    "event",
    "create",
    "weather",
    "forecast",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    None,
    Http,
    Timeout,
    /// Vectors of the wrong length (one dimension).
    WrongDimension,
    /// The first vector one dimension short of the rest.
    Ragged,
}

/// Bag-of-words embedder: dimension `i` counts occurrences of `VOCABULARY[i]`.
/// Records every text it is asked to embed and can be told to fail.
pub struct StubEmbedder {
    embedded: Mutex<Vec<String>>,
    calls: Mutex<usize>,
    failure: Mutex<Failure>,
}

impl StubEmbedder {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            embedded: Mutex::new(Vec::new()),
            calls: Mutex::new(0),
            failure: Mutex::new(Failure::None),
        })
    }

    pub fn take_embedded(&self) -> Vec<String> {
        std::mem::take(&mut *self.embedded.lock().expect("lock"))
    }

    pub fn calls(&self) -> usize {
        *self.calls.lock().expect("lock")
    }

    pub fn fail_with(&self, failure: Failure) {
        *self.failure.lock().expect("lock") = failure;
    }
}

#[async_trait]
impl EmbeddingProvider for StubEmbedder {
    fn model_name(&self) -> &str {
        "stub-bag-of-words"
    }

    fn dimension(&self) -> Option<usize> {
        Some(VOCABULARY.len())
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        *self.calls.lock().expect("lock") += 1;
        match *self.failure.lock().expect("lock") {
            Failure::None => {}
            Failure::Http => {
                return Err(EmbeddingError::HttpStatus {
                    status: 503,
                    body: texts.join(" "),
                });
            }
            Failure::Timeout => {
                return Err(EmbeddingError::Timeout {
                    timeout: Duration::from_secs(7),
                });
            }
            Failure::WrongDimension => return Ok(texts.iter().map(|_| vec![1.0]).collect()),
            Failure::Ragged => {
                let mut vectors: Vec<Vec<f32>> =
                    texts.iter().map(|text| bag_of_words(text)).collect();
                if let Some(first) = vectors.first_mut() {
                    first.pop();
                }
                return Ok(vectors);
            }
        }
        self.embedded
            .lock()
            .expect("lock")
            .extend(texts.iter().cloned());
        Ok(texts.iter().map(|text| bag_of_words(text)).collect())
    }
}

pub fn bag_of_words(text: &str) -> Vec<f32> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|character: char| !character.is_ascii_alphanumeric())
        .collect();
    VOCABULARY
        .iter()
        .map(|term| words.iter().filter(|word| *word == term).count() as f32)
        .collect()
}

pub fn tool(capability_id: &str, description: &str) -> ProviderToolDefinition {
    let name = capability_id.replace('.', "__");
    let mut definition = ProviderToolDefinition::from_parts(
        CapabilityId::new(capability_id).expect("capability id"),
        name,
        description,
        json!({"type": "object", "properties": {}}),
    )
    .expect("definition");
    definition.description_trust = CapabilityDescriptionTrust::VerifiedCatalog;
    definition
}

pub fn catalog() -> Vec<ProviderToolDefinition> {
    vec![
        tool("mail.send_message", "Send an email message"),
        tool("repo.search_code", "Search code in a repository"),
        tool("calendar.create_event", "Create a calendar event"),
        tool("weather.forecast", "Weather forecast"),
    ]
}

pub fn provider(embedder: &Arc<StubEmbedder>) -> DenseToolRetrievalProvider {
    DenseToolRetrievalProvider::new(Arc::clone(embedder) as Arc<dyn EmbeddingProvider>)
}
