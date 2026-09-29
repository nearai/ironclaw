//! The dense ranker never logs the query or schema text.
//!
//! Its own test binary: it installs a thread-local tracing subscriber, and
//! tracing caches per-callsite interest process-wide, so tests running beside
//! it on other threads could hide its events.

mod support;

use std::io::Write;
use std::sync::{Arc, Mutex};

use ironclaw_loop_contracts::ToolRetrievalProvider;
use support::{Failure, StubEmbedder, catalog, provider, tool};

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn neither_the_query_nor_schema_text_is_logged() {
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let embedder = StubEmbedder::new();
    let provider = provider(&embedder);
    let mut definitions = catalog();
    definitions.push(tool("secret.tool", "SCHEMA-CANARY send email"));
    let index = provider.fit(&definitions).await.expect("fit");
    index
        .search("QUERY-CANARY send email", 5)
        .await
        .expect("search");
    embedder.fail_with(Failure::Http);
    let _ = index.search("QUERY-CANARY send email", 5).await;

    let captured = String::from_utf8(logs.0.lock().expect("lock").clone()).expect("utf8");
    assert!(
        captured.contains("dense tool search"),
        "the capture works: {captured}"
    );
    assert!(!captured.contains("QUERY-CANARY"), "{captured}");
    assert!(!captured.contains("SCHEMA-CANARY"), "{captured}");
}
