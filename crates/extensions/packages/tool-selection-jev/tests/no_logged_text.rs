//! The Jev classifier never logs the conversation, tool descriptions, the
//! service's answers or the API key.
//!
//! Its own test binary: it installs a thread-local tracing subscriber, and
//! tracing caches per-callsite interest process-wide, so tests running beside
//! it on other threads could hide its events.

mod support;

use std::io::Write;
use std::sync::{Arc, Mutex};

use ironclaw_loop_contracts::ToolSelectionClassifier;
use support::{API_KEY, StubServer, answer_with, candidate, classifier, request};

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
async fn neither_the_query_the_descriptions_the_answers_nor_the_key_is_logged() {
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    // The first request is answered; the second gets an unusable body.
    let stub = StubServer::start(1, |request| {
        if request.body["state"]["conversation"][0] == "QUERY-CANARY please" {
            answer_with(request, |_| 0.1)
        } else {
            Some((200, r#"{"error":"ANSWER-CANARY"}"#.to_string()))
        }
    })
    .await;
    let candidates = || vec![candidate("mail__send", "DESCRIPTION-CANARY send mail.", 50)];
    let jev = classifier(&stub.url, 2_000);
    jev.classify(&request("QUERY-CANARY please", candidates(), 10))
        .await
        .expect("classified");
    jev.classify(&request("QUERY-CANARY again", candidates(), 10))
        .await
        .expect_err("unusable");

    let captured = String::from_utf8(logs.0.lock().expect("lock").clone()).expect("utf8");
    for logged in [
        "ironclaw::reborn::tool_selection",
        "Jev scored the candidate tools",
        "Jev classification response was unusable",
        "Jev tool classification failed",
    ] {
        assert!(captured.contains(logged), "the capture works: {captured}");
    }
    assert!(!captured.contains(" INFO ") && !captured.contains(" WARN "));
    for canary in [
        "QUERY-CANARY",
        "DESCRIPTION-CANARY",
        "ANSWER-CANARY",
        API_KEY,
    ] {
        assert!(!captured.contains(canary), "{canary} leaked: {captured}");
    }
}
