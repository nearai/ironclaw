//! The Jev classifier against a loopback stub of a decisions endpoint.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use ironclaw_loop_contracts::{
    MAX_CONVERSATION_CONTEXT_BYTES, ToolSelectionClassifier, ToolSelectionError,
};
use serde_json::json;
use support::{
    API_KEY, Reply, STUB_PATH, StubServer, answer_all, candidate, catalog, classifier, request,
};

fn scores(selection: &ironclaw_loop_contracts::ToolSelection) -> Vec<(&str, f32)> {
    selection
        .chosen
        .iter()
        .map(|tool| (tool.name.as_str(), tool.score))
        .collect()
}

#[tokio::test]
async fn one_request_carries_the_model_the_context_every_tool_and_one_noul_each() {
    let stub = StubServer::start(None, |_, request| answer_all(request, &BTreeMap::new())).await;
    let candidates = vec![
        candidate("mail__send", "Send an email message.", 60),
        candidate("repo__search_code", "Search code in a repository.", 60),
    ];
    let long_message = format!("find the flaky test {}", "x".repeat(2 * 1024));
    let selection = classifier(&stub.url, 2_000)
        .classify(&request(&long_message, candidates, &[], 10))
        .await
        .expect("classified");
    assert_eq!(selection.scorer, "jev:jev-latest");

    let requests = stub.requests();
    assert_eq!(requests.len(), 1, "one catalog fits one request");
    let recorded = &requests[0];
    assert_eq!(
        recorded.path, STUB_PATH,
        "posted to the configured URL's own path"
    );
    assert_eq!(
        recorded.header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(recorded.header("content-type"), Some("application/json"));
    let body = &recorded.body;
    assert_eq!(body["model"], "jev-latest");
    let conversation = body["state"]["conversation"]
        .as_array()
        .expect("conversation is a list of user messages");
    assert_eq!(conversation.len(), 1);
    let first = conversation[0].as_str().expect("text");
    assert!(first.starts_with("find the flaky test"));
    assert!(first.len() <= MAX_CONVERSATION_CONTEXT_BYTES);
    assert_eq!(
        body["state"]["tools"],
        json!({
            "mail__send": {
                "description": "Send an email message.",
                "parameters": ["limit", "query"]
            },
            "repo__search_code": {
                "description": "Search code in a repository.",
                "parameters": ["limit", "query"]
            }
        })
    );
    assert_eq!(
        body["questions"],
        json!({
            "mail__send": {
                "type": "noul",
                "instructions": "How likely is it that `tools.mail__send` will be used in the following `conversation`?"
            },
            "repo__search_code": {
                "type": "noul",
                "instructions": "How likely is it that `tools.repo__search_code` will be used in the following `conversation`?"
            }
        })
    );
}

#[tokio::test]
async fn the_top_n_by_probability_is_chosen_with_ties_in_catalog_order() {
    let probabilities: BTreeMap<&str, f32> = [
        ("tool_000__run", 0.2),
        ("tool_001__run", 0.7),
        ("tool_002__run", 0.7),
        ("tool_003__run", 0.95),
        ("tool_004__run", 0.9),
        ("tool_005__run", 0.05),
    ]
    .into_iter()
    .collect();
    let stub = StubServer::start(None, move |_, request| answer_all(request, &probabilities)).await;
    // Pinned tool_003 takes no slot. max 7 minus 3 bridges and 1 pinned
    // leaves room for 3.
    let selection = classifier(&stub.url, 2_000)
        .classify(&request("run things", catalog(6), &["tool_003__run"], 7))
        .await
        .expect("classified");
    assert_eq!(
        scores(&selection),
        vec![
            ("tool_004__run", 0.9),
            ("tool_001__run", 0.7),
            ("tool_002__run", 0.7),
        ]
    );
}

#[tokio::test]
async fn the_token_budget_trims_from_the_lowest_probability_up() {
    let probabilities: BTreeMap<&str, f32> = [("big__a", 0.9), ("big__b", 0.8), ("big__c", 0.7)]
        .into_iter()
        .collect();
    let stub = StubServer::start(None, move |_, request| answer_all(request, &probabilities)).await;
    let mut selection_request = request(
        "anything",
        vec![
            candidate("big__a", "A.", 400),
            candidate("big__b", "B.", 400),
            candidate("big__c", "C.", 400),
        ],
        &[],
        50,
    );
    // Room for two of the three after the 300 reserved tokens.
    selection_request.token_budget = 1_100;
    let selection = classifier(&stub.url, 2_000)
        .classify(&selection_request)
        .await
        .expect("classified");
    assert_eq!(scores(&selection), vec![("big__a", 0.9), ("big__b", 0.8)]);
}

#[tokio::test]
async fn a_large_catalog_is_sliced_into_concurrent_requests_merged_before_the_top_n() {
    let tools = catalog(120);
    // The best tool sits in the last slice, the second best in the first.
    let probabilities: BTreeMap<&str, f32> = [("tool_119__run", 0.99), ("tool_002__run", 0.98)]
        .into_iter()
        .collect();
    let run = || async {
        let probabilities = probabilities.clone();
        // Every slice must arrive before any is answered, so sequential
        // requests would time out.
        let stub = StubServer::start(Some(4), move |_, request| {
            answer_all(request, &probabilities)
        })
        .await;
        let selection = classifier(&stub.url, 3_000)
            .with_max_slice_tokens(2_500)
            .expect("budget")
            .classify(&request("run", tools.clone(), &[], 5))
            .await
            .expect("classified");
        (stub.requests(), selection)
    };

    let (first_requests, selection) = run().await;
    assert!(first_requests.len() >= 4, "{} slices", first_requests.len());
    let mut slices: Vec<Vec<String>> = first_requests
        .iter()
        .map(|recorded| recorded.question_ids())
        .collect();
    slices.sort();
    let asked: Vec<String> = slices.iter().flatten().cloned().collect();
    let expected: Vec<String> = tools.iter().map(|tool| tool.name().to_string()).collect();
    assert_eq!(asked, expected, "every tool exactly once, in catalog order");
    for recorded in &first_requests {
        let tools_in_state: BTreeSet<String> = recorded.body["state"]["tools"]
            .as_object()
            .expect("tools")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            tools_in_state,
            recorded.question_ids().into_iter().collect(),
            "each slice's state holds exactly the tools it asks about"
        );
        assert_eq!(recorded.body["state"]["conversation"], json!(["run"]));
    }
    assert_eq!(
        scores(&selection)[..2],
        [("tool_119__run", 0.99), ("tool_002__run", 0.98)]
    );

    let (second_requests, _) = run().await;
    let mut again: Vec<Vec<String>> = second_requests
        .iter()
        .map(|recorded| recorded.question_ids())
        .collect();
    again.sort();
    assert_eq!(again, slices, "the same slices on every run");
}

async fn failure_for(
    hold_until: Option<usize>,
    slice_tokens: Option<usize>,
    handler: impl Fn(usize, &support::Recorded) -> Reply + Send + Sync + 'static,
) -> ToolSelectionError {
    let stub = StubServer::start(hold_until, handler).await;
    let mut jev = classifier(&stub.url, 400);
    if let Some(tokens) = slice_tokens {
        jev = jev.with_max_slice_tokens(tokens).expect("budget");
    }
    jev.classify(&request("run", catalog(40), &[], 10))
        .await
        .expect_err("classification must fail")
}

#[tokio::test]
async fn every_failure_mode_is_reported_as_a_failure_never_a_partial_answer() {
    let status =
        |code: u16| move |_: usize, _: &support::Recorded| Reply::Status(code, "{}".into());
    assert_eq!(
        failure_for(None, None, status(401)).await,
        ToolSelectionError::Unauthorized
    );
    assert_eq!(
        failure_for(None, None, status(402)).await,
        ToolSelectionError::PaymentRequired
    );
    assert_eq!(
        failure_for(None, None, status(422)).await,
        ToolSelectionError::Rejected { status: 422 }
    );
    // The server names a wait past the 400 ms deadline, so the attempt ends
    // at once. (Without a `retry-after` the backoff keeps retrying, and
    // whether the deadline lands during a wait or a request depends on the
    // machine's load; the retried path is covered below.)
    for code in [429, 503, 529] {
        assert_eq!(
            failure_for(None, None, move |_, _| Reply::RetryAfter(code, 1)).await,
            ToolSelectionError::RateLimited,
            "{code} that never clears"
        );
    }
    assert!(matches!(
        failure_for(None, None, |_, _| Reply::Status(200, "{not json".into())).await,
        ToolSelectionError::InvalidOutput { .. }
    ));
    assert!(matches!(
        failure_for(None, None, |_, _| Reply::Hang).await,
        ToolSelectionError::Timeout { .. }
    ));
    assert!(matches!(
        failure_for(None, None, |_, _| Reply::Close).await,
        ToolSelectionError::Unavailable { .. }
    ));
    // One slice of several fails; the others answer.
    let one_slice_fails = failure_for(Some(2), Some(1_000), |number, request| {
        if number == 0 {
            Reply::Status(500, "{}".into())
        } else {
            answer_all(request, &BTreeMap::new())
        }
    })
    .await;
    assert_eq!(
        one_slice_fails,
        ToolSelectionError::Rejected { status: 500 }
    );
    // A slice that leaves a tool unanswered is invalid output.
    assert!(matches!(
        failure_for(None, None, |_, _| Reply::Status(
            200,
            json!({"answers": {"tool_000__run": {"type": "noul", "noul": 0.5}}}).to_string()
        ))
        .await,
        ToolSelectionError::InvalidOutput { .. }
    ));
}

/// Every overload status is retried until it clears.
#[tokio::test]
async fn an_overload_that_clears_inside_the_timeout_is_retried() {
    for code in [429, 503, 529] {
        let stub = StubServer::start(None, move |number, request| {
            if number == 0 {
                Reply::Status(code, "{}".into())
            } else {
                answer_all(request, &BTreeMap::new())
            }
        })
        .await;
        let selection = classifier(&stub.url, 2_000)
            .classify(&request("run", catalog(3), &[], 10))
            .await
            .unwrap_or_else(|error| panic!("{code}: the retry succeeded, not {error:?}"));
        assert_eq!(selection.chosen.len(), 3);
        assert_eq!(stub.requests().len(), 2, "{code}");
    }
}

/// A `402` (the account cannot pay) is final: one request, no retry.
#[tokio::test]
async fn payment_required_is_not_retried() {
    let stub = StubServer::start(None, |_, _| Reply::Status(402, "{}".into())).await;
    let error = classifier(&stub.url, 2_000)
        .classify(&request("run", catalog(3), &[], 10))
        .await
        .expect_err("402 fails the classification");
    assert_eq!(error, ToolSelectionError::PaymentRequired);
    assert_eq!(error.kind_label(), "payment_required");
    assert_eq!(stub.requests().len(), 1);
}

#[tokio::test]
async fn no_server_at_all_is_unavailable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    drop(listener);
    let error = classifier(&format!("http://127.0.0.1:{port}{STUB_PATH}"), 1_000)
        .classify(&request("run", catalog(3), &[], 10))
        .await
        .expect_err("nothing listens");
    assert!(matches!(error, ToolSelectionError::Unavailable { .. }));
}

/// Jev scores the whole conversation at once, so a re-selection sends it the
/// whole window as the list of user messages, not the local rankers'
/// segments; a slice budget configured far below the default drops the
/// oldest messages first and always keeps the newest.
#[tokio::test]
async fn the_whole_window_is_the_conversation_within_the_token_budget() {
    let stub = StubServer::start(None, |_, request| answer_all(request, &BTreeMap::new())).await;
    let messages: Vec<String> = (0..20)
        .map(|index| format!("message {index:02} {}", "word ".repeat(300)))
        .collect();
    let mut windowed = request("unused", catalog(2), &[], 10);
    windowed.context = ironclaw_loop_contracts::ConversationContext::new(messages);

    classifier(&stub.url, 2_000)
        .classify(&windowed)
        .await
        .expect("classified");
    let sent = stub.requests()[0].body["state"]["conversation"].clone();
    let sent = sent.as_array().expect("a list of user messages");
    assert_eq!(sent.len(), 20, "every message of the window, oldest first");
    assert!(
        sent[0]
            .as_str()
            .is_some_and(|text| text.starts_with("message 00"))
    );
    assert!(
        sent[19]
            .as_str()
            .is_some_and(|text| text.starts_with("message 19"))
    );

    let stub = StubServer::start(None, |_, request| answer_all(request, &BTreeMap::new())).await;
    classifier(&stub.url, 2_000)
        .with_max_slice_tokens(2_000)
        .expect("budget")
        .classify(&windowed)
        .await
        .expect("classified");
    let requests = stub.requests();
    let sent = requests[0].body["state"]["conversation"]
        .as_array()
        .expect("a list of user messages");
    // Half of 2,000 tokens at three bytes a token is 3,000 bytes: room for
    // the newest message of about 1.5 KB, not for two.
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0]
            .as_str()
            .is_some_and(|text| text.starts_with("message 19"))
    );
}
