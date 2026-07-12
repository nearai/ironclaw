//! Ignored live canary for Reborn first-party GitHub API contracts.
//!
//! This test intentionally performs read-only GitHub API calls with a real PAT
//! so schema/query changes can be checked against GitHub itself without
//! mutating repositories.
//!
//! Run:
//!   LIVE_CANARY_GITHUB_TOKEN=<pat> \
//!     cargo test -p ironclaw --test reborn_live_github_pat_contract -- --ignored

use reqwest::{Client, StatusCode};
use serde_json::Value;

const API_ROOT: &str = "https://api.github.com";
const OWNER: &str = "nearai";
const REPO: &str = "ironclaw";

#[tokio::test]
#[ignore = "requires LIVE_CANARY_GITHUB_TOKEN or GITHUB_TOKEN with nearai/ironclaw read access"]
async fn reborn_live_github_pat_accepts_read_only_schema_calls() {
    let token = std::env::var("LIVE_CANARY_GITHUB_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .expect("set LIVE_CANARY_GITHUB_TOKEN or GITHUB_TOKEN");
    let client = Client::builder()
        .user_agent("IronClaw-Reborn-GitHub-Live-Contract-Test")
        .build()
        .expect("reqwest client");

    get_json(&client, &token, "/user").await;

    get_json(&client, &token, "/user/repos?per_page=1&type=member&page=1").await;
    get_json(
        &client,
        &token,
        &format!(
            "/repos/{OWNER}/{REPO}/issues?state=all&per_page=1&labels=bug&assignee=none&milestone=*&page=1"
        ),
    )
    .await;
    let pulls = get_json(
        &client,
        &token,
        &format!(
            "/repos/{OWNER}/{REPO}/pulls?state=all&per_page=1&base=main&sort=updated&direction=desc&page=1"
        ),
    )
    .await;
    if let Some(pr_number) = pulls
        .as_array()
        .and_then(|pulls| pulls.first())
        .and_then(|pull| pull.get("number"))
        .and_then(Value::as_u64)
    {
        get_json(
            &client,
            &token,
            &format!("/repos/{OWNER}/{REPO}/pulls/{pr_number}/files?per_page=1&page=1"),
        )
        .await;
    }
    get_json(
        &client,
        &token,
        &format!(
            "/search/issues?q=repo%3A{OWNER}%2F{REPO}%20is%3Apr&per_page=1&sort=reactions-heart&order=desc"
        ),
    )
    .await;
    let content = get_json(
        &client,
        &token,
        &format!("/repos/{OWNER}/{REPO}/contents/Cargo.toml?ref=main"),
    )
    .await;
    assert_eq!(content["encoding"], "base64");
}

async fn get_json(client: &Client, token: &str, path: &str) -> Value {
    let url = format!("{API_ROOT}{path}");
    let response = client
        .get(&url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .unwrap_or_else(|error| panic!("{url} request failed: {error}"));
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| panic!("{url} response body failed: {error}"));
    assert_eq!(status, StatusCode::OK, "{url} returned {status}: {body}");
    serde_json::from_str(&body)
        .unwrap_or_else(|error| panic!("{url} returned invalid JSON: {error}; body={body}"))
}
