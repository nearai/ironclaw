//! Behavior of the dense tool ranker through the public retrieval port, with a
//! deterministic stub embedder in place of an embedding endpoint.

mod support;

use std::time::Duration;

use ironclaw_loop_contracts::{ToolRetrievalError, ToolRetrievalProvider, ToolSearchQueryClass};
use ironclaw_tool_retrieval::{DENSE_RANKER_VERSION, MAX_CORPUS_DEFINITIONS};
use support::{Failure, StubEmbedder, catalog, provider, tool};

#[tokio::test]
async fn ranks_the_catalog_by_cosine_similarity_with_scores() {
    let embedder = StubEmbedder::new();
    let provider = provider(&embedder);
    assert_eq!(provider.ranker_version(), DENSE_RANKER_VERSION);
    let index = provider.fit(&catalog()).await.expect("fit");

    let outcome = index.search("send an email", 10).await.expect("search");

    assert_eq!(outcome.query_class, ToolSearchQueryClass::Lexical);
    assert_eq!(outcome.names(), vec!["mail__send_message"]);
    let top = &outcome.ranked[0];
    assert!(top.score > 0.0 && top.score <= 1.0, "score {}", top.score);

    let outcome = index
        .search("search code or create an event", 10)
        .await
        .expect("search");
    assert_eq!(
        outcome.names(),
        vec!["repo__search_code", "calendar__create_event"]
    );
    assert!(outcome.ranked[0].score > outcome.ranked[1].score);
    assert!(outcome.ranked.iter().all(|tool| tool.score > 0.0));

    let limited = index
        .search("search code or create an event", 1)
        .await
        .expect("search");
    assert_eq!(limited.names(), vec!["repo__search_code"]);
}

#[tokio::test]
async fn equal_scores_break_on_capability_id() {
    let embedder = StubEmbedder::new();
    // Names outside the stub vocabulary, identical descriptions: equal vectors.
    let twins = vec![
        tool("zulu.notify", "Send an email"),
        tool("alpha.notify", "Send an email"),
    ];
    let index = provider(&embedder).fit(&twins).await.expect("fit");
    let outcome = index.search("send email", 10).await.expect("search");
    assert_eq!(outcome.names(), vec!["alpha__notify", "zulu__notify"]);
    assert_eq!(outcome.ranked[0].score, outcome.ranked[1].score);
}

#[tokio::test]
async fn unrelated_query_is_no_match() {
    let embedder = StubEmbedder::new();
    let index = provider(&embedder).fit(&catalog()).await.expect("fit");
    let outcome = index.search("zebra", 10).await.expect("search");
    assert_eq!(outcome.query_class, ToolSearchQueryClass::NoMatch);
    assert!(outcome.ranked.is_empty());
}

#[tokio::test]
async fn exact_identifier_ranks_first_at_the_top_of_the_scale() {
    let embedder = StubEmbedder::new();
    let index = provider(&embedder).fit(&catalog()).await.expect("fit");
    for query in ["weather.forecast", "WEATHER__FORECAST"] {
        let outcome = index.search(query, 10).await.expect("search");
        assert_eq!(outcome.query_class, ToolSearchQueryClass::ExactIdentifier);
        assert_eq!(outcome.ranked[0].name, "weather__forecast");
        assert_eq!(outcome.ranked[0].score, 1.0);
    }
}

#[tokio::test]
async fn only_new_or_changed_tools_are_re_embedded() {
    let embedder = StubEmbedder::new();
    let provider = provider(&embedder);

    provider.fit(&catalog()).await.expect("first fit");
    assert_eq!(embedder.take_embedded().len(), 4, "cold cache embeds all");

    let calls_before = embedder.calls();
    provider.fit(&catalog()).await.expect("refit");
    assert_eq!(
        embedder.calls(),
        calls_before,
        "an unchanged catalog makes no embedding call"
    );

    let mut changed = catalog();
    changed[3] = tool("weather.forecast", "Weather forecast and email alerts");
    changed.push(tool("repo.search_issues", "Search issues in a repository"));
    provider.fit(&changed).await.expect("fit after change");
    let embedded = embedder.take_embedded();
    assert_eq!(embedded.len(), 2, "one changed tool plus one new tool");
    assert!(embedded.iter().any(|text| text.contains("email alerts")));
    assert!(embedded.iter().any(|text| text.contains("search issues")));
}

#[tokio::test]
async fn a_catalog_change_changes_the_ranking() {
    let embedder = StubEmbedder::new();
    let provider = provider(&embedder);
    let before = provider.fit(&catalog()).await.expect("fit");
    assert_eq!(
        before
            .search("weather forecast", 10)
            .await
            .expect("search")
            .names(),
        vec!["weather__forecast"]
    );

    // The weather tool is gone and the calendar tool now covers forecasts.
    let changed = vec![
        tool("mail.send_message", "Send an email message"),
        tool(
            "calendar.create_event",
            "Create a calendar event from a forecast",
        ),
    ];
    let after = provider.fit(&changed).await.expect("refit");
    assert_eq!(
        after
            .search("weather forecast", 10)
            .await
            .expect("search")
            .names(),
        vec!["calendar__create_event"],
        "the new index ranks the changed tool and never names the removed one"
    );
    // The earlier index is a snapshot and is unaffected.
    assert_eq!(
        before
            .search("weather forecast", 10)
            .await
            .expect("search")
            .names(),
        vec!["weather__forecast"]
    );
}

#[tokio::test]
async fn ranking_is_deterministic_across_fits_and_input_order() {
    let embedder = StubEmbedder::new();
    let first = provider(&embedder).fit(&catalog()).await.expect("fit");
    let mut reversed = catalog();
    reversed.reverse();
    let second = provider(&embedder).fit(&reversed).await.expect("fit");

    for query in ["search code or create an event", "send email", "repository"] {
        let expected = first.search(query, 10).await.expect("search");
        assert_eq!(first.search(query, 10).await.expect("search"), expected);
        assert_eq!(second.search(query, 10).await.expect("search"), expected);
    }
}

#[tokio::test]
async fn embedder_failures_surface_as_port_errors() {
    let embedder = StubEmbedder::new();
    let provider = provider(&embedder);

    embedder.fail_with(Failure::Http);
    let error = provider.fit(&catalog()).await.expect_err("fit fails");
    assert_eq!(error.kind_label(), "unavailable");
    assert!(
        !error.to_string().contains("email"),
        "no document text in the error: {error}"
    );

    embedder.fail_with(Failure::Ragged);
    let error = provider.fit(&catalog()).await.expect_err("fit fails");
    assert_eq!(error.kind_label(), "invalid_output");

    // Nothing from the failed fits was cached: a healthy fit embeds all.
    embedder.fail_with(Failure::None);
    embedder.take_embedded();
    let index = provider.fit(&catalog()).await.expect("fit");
    assert_eq!(embedder.take_embedded().len(), 4);
    embedder.fail_with(Failure::Timeout);
    let error = index
        .search("send an email", 5)
        .await
        .expect_err("search fails");
    assert_eq!(
        error,
        ToolRetrievalError::Timeout {
            elapsed: Duration::from_secs(7)
        }
    );

    embedder.fail_with(Failure::WrongDimension);
    let error = index
        .search("send an email", 5)
        .await
        .expect_err("search fails");
    assert_eq!(error.kind_label(), "invalid_output");
}

#[tokio::test]
async fn empty_query_zero_limit_and_empty_corpus_skip_the_embedder() {
    let embedder = StubEmbedder::new();
    let provider = provider(&embedder);
    let index = provider.fit(&catalog()).await.expect("fit");
    let calls = embedder.calls();

    for (query, limit) in [("   ", 5), ("send email", 0)] {
        let outcome = index.search(query, limit).await.expect("search");
        assert_eq!(outcome.query_class, ToolSearchQueryClass::NoMatch);
    }
    let empty = provider.fit(&[]).await.expect("empty fit");
    let outcome = empty.search("send email", 5).await.expect("search");
    assert!(outcome.ranked.is_empty());
    assert_eq!(embedder.calls(), calls);
}

#[tokio::test]
async fn oversized_corpus_is_refused_before_embedding() {
    let embedder = StubEmbedder::new();
    let corpus: Vec<_> = (0..=MAX_CORPUS_DEFINITIONS)
        .map(|index| tool(&format!("bulk.tool_{index}"), "Bulk tool"))
        .collect();
    let error = provider(&embedder)
        .fit(&corpus)
        .await
        .expect_err("corpus too large");
    assert_eq!(
        error,
        ToolRetrievalError::CorpusTooLarge {
            definitions: MAX_CORPUS_DEFINITIONS + 1,
            limit: MAX_CORPUS_DEFINITIONS,
        }
    );
    assert_eq!(embedder.calls(), 0);
}

#[tokio::test]
async fn several_queries_are_embedded_in_one_request_and_ranked_separately() {
    let embedder = StubEmbedder::new();
    let index = provider(&embedder).fit(&catalog()).await.expect("fit");
    embedder.take_embedded();
    let calls = embedder.calls();

    let outcomes = index
        .search_many(&["send an email", "   ", "search code", "zebra"], 10)
        .await
        .expect("searched");

    assert_eq!(embedder.calls(), calls + 1, "one embedding request");
    assert_eq!(
        embedder.take_embedded(),
        ["send an email", "search code", "zebra"],
        "blank queries are not embedded"
    );
    // Each query ranks exactly as it would alone.
    for (outcome, query) in outcomes
        .iter()
        .zip(["send an email", "   ", "search code", "zebra"])
    {
        assert_eq!(outcome, &index.search(query, 10).await.expect("search"));
    }
    assert_eq!(outcomes[0].names(), vec!["mail__send_message"]);
    assert_eq!(outcomes[1].query_class, ToolSearchQueryClass::NoMatch);
    assert_eq!(outcomes[2].names(), vec!["repo__search_code"]);
    assert!(outcomes[3].ranked.is_empty());

    let calls = embedder.calls();
    assert!(
        index
            .search_many(&["  ", ""], 10)
            .await
            .expect("searched")
            .iter()
            .all(|outcome| outcome.ranked.is_empty())
    );
    assert_eq!(embedder.calls(), calls, "nothing to embed, no request");
}
