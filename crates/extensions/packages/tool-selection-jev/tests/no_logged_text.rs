//! The Jev classifier never logs the conversation, tool descriptions, the
//! service's answers or the API key.
//!
//! Its own test binary: it installs a thread-local tracing subscriber, and
//! tracing caches per-callsite interest process-wide, so tests running beside
//! it on other threads could hide its events.

mod support;

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

use ironclaw_loop_contracts::ToolSelectionClassifier;
use support::{API_KEY, Reply, StubServer, answer_all, candidate, classifier, request};

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

    let candidates = || {
        vec![
            candidate("mail__send", "DESCRIPTION-CANARY send mail.", 50),
            candidate("repo__search", "Search a repository.", 50),
        ]
    };
    let stub = StubServer::start(None, |number, request| {
        if number == 0 {
            let mut reply = match answer_all(request, &BTreeMap::new()) {
                Reply::Status(_, body) => body,
                other => return other,
            };
            reply.insert_str(1, r#""note":"ANSWER-CANARY","#);
            Reply::Status(200, reply)
        } else {
            Reply::Status(401, r#"{"error":"ANSWER-CANARY"}"#.to_string())
        }
    })
    .await;
    let jev = classifier(&stub.url, 2_000);
    jev.classify(&request("QUERY-CANARY please", candidates(), &[], 10))
        .await
        .expect("classified");
    let _ = jev
        .classify(&request("QUERY-CANARY again", candidates(), &[], 10))
        .await;

    let captured = String::from_utf8(logs.0.lock().expect("lock").clone()).expect("utf8");
    assert!(
        captured.contains("Jev scored the candidate tools")
            && captured.contains("Jev tool classification failed"),
        "the capture works: {captured}"
    );
    assert!(
        captured.contains("served_model=jev-1.13.0"),
        "the model the server reported is logged: {captured}"
    );
    for canary in [
        "QUERY-CANARY",
        "DESCRIPTION-CANARY",
        "ANSWER-CANARY",
        API_KEY,
    ] {
        assert!(!captured.contains(canary), "{canary} leaked: {captured}");
    }
}
