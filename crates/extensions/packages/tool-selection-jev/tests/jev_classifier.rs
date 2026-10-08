//! The Jev classifier against a loopback stub of a decisions endpoint.

mod support;

use ironclaw_loop_contracts::{ToolSelection, ToolSelectionClassifier};
use serde_json::json;
use support::{
    API_KEY, Recorded, STUB_PATH, StubServer, answer_with, candidate, classifier, classifier_for,
    classify, request,
};

fn scores(selection: &ToolSelection) -> Vec<(&str, f32)> {
    let chosen = selection.chosen.iter();
    chosen
        .map(|tool| (tool.name.as_str(), tool.score))
        .collect()
}

#[tokio::test]
async fn one_request_carries_the_model_the_conversation_every_tool_and_one_noul_each() {
    let stub = StubServer::start(1, |request| answer_with(request, |_| 0.1)).await;
    let candidates = || {
        vec![
            candidate("mail__send", "Send an email message.", 60),
            candidate("repo__search_code", "Search code in a repository.", 60),
        ]
    };
    let asked = request("find the flaky test", candidates(), 10);
    let selection = classify(&stub.url, 2_000, asked).await.expect("classified");
    assert_eq!(selection.scorer, "jev:jev-latest");
    // Equal probabilities: catalog order.
    let chosen = [("mail__send", 0.1), ("repo__search_code", 0.1)];
    assert_eq!(scores(&selection), chosen);

    let requests = stub.requests();
    assert_eq!(requests.len(), 1, "a small catalog fits one request");
    let recorded = &requests[0];
    assert_eq!(recorded.path, STUB_PATH, "posted to the configured URL");
    let header = |name: &str, value: &str| (name.to_string(), value.to_string());
    let bearer = header("authorization", &format!("Bearer {API_KEY}"));
    assert!(recorded.headers.contains(&bearer));
    assert!(
        recorded
            .headers
            .contains(&header("content-type", "application/json"))
    );
    // Each description is in its own question; `state` holds only the
    // conversation, the tool names and their parameter names.
    assert_eq!(
        recorded.body,
        json!({
            "model": "jev-latest",
            "state": {
                "conversation": ["find the flaky test"],
                "tools": {
                    "mail__send": {"parameters": ["limit", "query"]},
                    "repo__search_code": {"parameters": ["limit", "query"]}
                }
            },
            "questions": {
                "mail__send": {
                    "type": "noul",
                    "instructions": "How likely is it that `tools.mail__send` will be used in the following `conversation`, given that it is described as: Send an email message.?"
                },
                "repo__search_code": {
                    "type": "noul",
                    "instructions": "How likely is it that `tools.repo__search_code` will be used in the following `conversation`, given that it is described as: Search code in a repository.?"
                }
            }
        })
    );

    // Nothing is sent when nothing may be chosen, or when the egress policy
    // does not allow the target.
    let none = classify(&stub.url, 2_000, request("go", candidates(), 0)).await;
    assert!(none.expect("empty").chosen.is_empty());
    let elsewhere = classifier_for(&stub.url, 2_000, "jev.example.test");
    let refused = elsewhere.classify(&request("go", candidates(), 10)).await;
    assert_eq!(refused.expect_err("refused").kind_label(), "unavailable");
    assert_eq!(stub.requests().len(), 1);
}

#[tokio::test]
async fn the_top_n_by_probability_is_chosen_within_the_token_budget() {
    let stub = StubServer::start(1, |request| {
        answer_with(request, |name| match name {
            "t__a" => 0.2,
            "t__b" | "t__c" => 0.7,
            "t__d" => 0.95,
            _ => 0.05,
        })
    })
    .await;
    let tools = || {
        let names = ["t__a", "t__b", "t__c", "t__d", "t__e"];
        names.map(|name| candidate(name, "A tool.", 400)).to_vec()
    };
    // The best three; the tie between `b` and `c` keeps catalog order.
    let selection = classify(&stub.url, 2_000, request("go", tools(), 3)).await;
    let top_three = [("t__d", 0.95), ("t__b", 0.7), ("t__c", 0.7)];
    assert_eq!(scores(&selection.expect("classified")), top_three);
    // 1,000 tokens hold two 400-token schemas, not three.
    let mut tight = request("go", tools(), 3);
    tight.token_budget = 1_000;
    let selection = classify(&stub.url, 2_000, tight).await;
    assert_eq!(scores(&selection.expect("classified")), top_three[..2]);
}

/// A thousand tools with full 1 KiB descriptions. With the descriptions in
/// `state` this catalog took 18 slices; in the questions it takes at most 7.
/// The slices go out together, and every answer lands on its own tool.
#[tokio::test]
async fn a_thousand_tools_are_sliced_asked_concurrently_and_merged_before_choosing() {
    let tools: Vec<_> = (0..1_000)
        .map(|index| {
            let description = format!("Run task number {index}. {}", "d".repeat(1_024));
            candidate(&format!("tool_{index:04}__run"), &description, 50)
        })
        .collect();
    // Each tool's probability is its own index, so a misplaced answer shows.
    let probability = |name: &str| name[5..9].parse::<f32>().expect("index") / 1_000.0;
    // No slice is answered until two have arrived: sequential requests
    // would time out.
    let stub = StubServer::start(2, move |request| answer_with(request, probability)).await;
    let jev = classifier(&stub.url, 10_000);
    let asked = request("run", tools.clone(), 1_000);
    let selection = jev.classify(&asked).await.expect("classified");

    let requests = stub.requests();
    let slices = requests.len();
    assert!((2..=7).contains(&slices), "{slices} slices");
    assert_eq!(jev.slice_count(&asked), slices);
    let mut asked: Vec<String> = requests.iter().flat_map(Recorded::question_ids).collect();
    asked.sort();
    let expected: Vec<String> = tools.iter().map(|tool| tool.name().to_string()).collect();
    assert_eq!(asked, expected, "every tool asked exactly once");
    for recorded in &requests {
        let state = &recorded.body["state"];
        assert_eq!(state["conversation"], json!(["run"]));
        let in_state: Vec<_> = state["tools"].as_object().expect("tools").keys().collect();
        assert_eq!(in_state, recorded.question_ids().iter().collect::<Vec<_>>());
        assert!(!state.to_string().contains("Run task number"));
    }
    // Best first is the catalog reversed, each with its own probability.
    assert_eq!(selection.chosen.len(), 1_000);
    for (rank, tool) in selection.chosen.iter().enumerate() {
        assert_eq!(tool.name, format!("tool_{:04}__run", 999 - rank));
        assert_eq!(tool.score, probability(&tool.name));
    }
}

#[tokio::test]
async fn every_failure_is_a_labelled_error_and_one_bad_slice_fails_the_whole_selection() {
    let missing = r#"{"answers": {"t__a": {"type": "noul", "noul": 0.5}}}"#;
    let out_of_range = r#"{"answers": {"t__a": {"type": "noul", "noul": 0.5}, "t__b": {"type": "noul", "noul": 1.5}}}"#;
    for (status, body, label) in [
        (401, "{}", "unauthorized"),
        (402, "{}", "payment_required"),
        (422, "{}", "rejected"),
        (429, "{}", "rate_limited"),
        (500, "{}", "unavailable"),
        (200, "not json", "invalid_output"),
        (200, missing, "invalid_output"),
        (200, out_of_range, "invalid_output"),
    ] {
        let stub = StubServer::start(1, move |_| Some((status, body.to_string()))).await;
        let tools = vec![candidate("t__a", "A.", 50), candidate("t__b", "B.", 50)];
        let outcome = classify(&stub.url, 2_000, request("go", tools, 5)).await;
        assert_eq!(outcome, Err(label), "status {status}, body {body}");
        assert_eq!(stub.requests().len(), 1, "{label}: never retried");
    }

    // Two slices, the second refused: no partial selection.
    let tools: Vec<_> = (0..200)
        .map(|index| candidate(&format!("tool_{index:04}__run"), &"d".repeat(1_024), 50))
        .collect();
    let stub = StubServer::start(2, |request| {
        let first = request.body["questions"].get("tool_0000__run").is_some();
        let overloaded = Some((503, "{}".to_string()));
        if first {
            answer_with(request, |_| 0.5)
        } else {
            overloaded
        }
    })
    .await;
    let outcome = classify(&stub.url, 5_000, request("go", tools, 5)).await;
    assert_eq!(outcome, Err("rate_limited"));
    // The stub answers nothing until both slices have arrived.
    assert_eq!(stub.requests().len(), 2, "never retried");
}

#[tokio::test]
async fn a_silent_service_times_out_and_an_unreachable_one_is_unavailable() {
    let stub = StubServer::start(1, |_| None).await;
    let one_tool = || request("go", vec![candidate("t__a", "A.", 50)], 5);
    let started = std::time::Instant::now();
    assert_eq!(classify(&stub.url, 200, one_tool()).await, Err("timeout"));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));

    // Nothing listens on the port of a listener that was dropped.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    drop(listener);
    let closed = format!("http://127.0.0.1:{port}{STUB_PATH}");
    assert_eq!(
        classify(&closed, 2_000, one_tool()).await,
        Err("unavailable")
    );
}
