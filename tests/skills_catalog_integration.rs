//! Integration tests for the SkillCatalog + SkillRegistry pipeline.
//!
//! These tests spin up a mock HTTP server (axum) on a random port that mimics
//! the ClawHub registry API, then exercise the full search -> download -> install
//! flow through the real `SkillCatalog` and `SkillRegistry` types.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use ironclaw::skills::catalog::{skill_download_url, SkillCatalog};
use ironclaw::skills::registry::SkillRegistry;
use ironclaw::skills::SkillTrust;

// ---------------------------------------------------------------------------
// Shared test fixtures
// ---------------------------------------------------------------------------

/// Valid SKILL.md content returned by the mock download endpoint.
const VALID_SKILL_MD: &str = "\
---
name: deploy-helper
version: \"1.2.0\"
description: Deployment automation skill
activation:
  keywords: [\"deploy\", \"release\", \"rollout\"]
---

You are a deployment automation assistant.
Help the user deploy services to production safely.
";

/// Invalid SKILL.md content (no YAML frontmatter).
const INVALID_SKILL_MD: &str = "Just some plain text without any frontmatter.";

/// State shared between mock server handlers.
#[derive(Clone)]
struct MockState {
    search_hit_count: Arc<AtomicUsize>,
}

/// Query parameters for the search endpoint.
#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
}

/// Query parameters for the download endpoint.
#[derive(Deserialize)]
struct DownloadQuery {
    #[serde(default)]
    slug: String,
}

/// Build the mock axum router.
fn mock_router(state: MockState) -> Router {
    Router::new()
        .route("/api/v1/search", get(mock_search))
        .route("/api/v1/download", get(mock_download))
        .with_state(state)
}

/// Mock search handler -- returns camelCase JSON matching ClawHub's API format.
async fn mock_search(
    State(state): State<MockState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    state.search_hit_count.fetch_add(1, Ordering::SeqCst);

    let results = if params.q.contains("deploy") {
        serde_json::json!([
            {
                "slug": "acme/deploy-helper",
                "displayName": "Deploy Helper",
                "version": "1.2.0",
                "summary": "Deployment automation skill",
                "score": 0.95
            },
            {
                "slug": "acme/deploy-monitor",
                "displayName": "Deploy Monitor",
                "version": "0.3.1",
                "summary": "Monitor deployments in real-time",
                "score": 0.82
            }
        ])
    } else if params.q.contains("empty") {
        serde_json::json!([])
    } else {
        serde_json::json!([
            {
                "slug": "generic/tool",
                "displayName": "Generic Tool",
                "version": "1.0.0",
                "summary": "A generic tool",
                "score": 0.5
            }
        ])
    };

    axum::Json(results)
}

/// Mock download handler -- returns raw SKILL.md content.
async fn mock_download(Query(params): Query<DownloadQuery>) -> impl IntoResponse {
    if params.slug.contains("bad-skill") {
        (
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            INVALID_SKILL_MD.to_string(),
        )
    } else {
        (
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            VALID_SKILL_MD.to_string(),
        )
    }
}

/// Start the mock server on a random port and return the base URL.
async fn start_mock_server(state: MockState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to random port");
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let router = mock_router(state);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    base_url
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Search the mock catalog and verify results come back with correct fields.
#[tokio::test]
async fn test_catalog_search_returns_results() {
    let state = MockState {
        search_hit_count: Arc::new(AtomicUsize::new(0)),
    };
    let base_url = start_mock_server(state).await;

    let catalog = SkillCatalog::with_url(&base_url);
    let results = catalog.search("deploy").await;

    assert_eq!(results.len(), 2, "Expected 2 search results");

    assert_eq!(results[0].slug, "acme/deploy-helper");
    assert_eq!(results[0].name, "Deploy Helper");
    assert_eq!(results[0].version, "1.2.0");
    assert!((results[0].score - 0.95).abs() < f64::EPSILON);

    assert_eq!(results[1].slug, "acme/deploy-monitor");
    assert_eq!(results[1].name, "Deploy Monitor");
    assert_eq!(results[1].version, "0.3.1");
    assert!((results[1].score - 0.82).abs() < f64::EPSILON);
}

/// Verify that the catalog caches search results (same query does not hit server twice).
#[tokio::test]
async fn test_catalog_search_caches_results() {
    let counter = Arc::new(AtomicUsize::new(0));
    let state = MockState {
        search_hit_count: counter.clone(),
    };
    let base_url = start_mock_server(state).await;

    let catalog = SkillCatalog::with_url(&base_url);

    // First search -- should hit the server.
    let _results = catalog.search("deploy").await;
    assert_eq!(counter.load(Ordering::SeqCst), 1, "First search should hit server");

    // Second search with same query -- should come from cache.
    let _results = catalog.search("deploy").await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "Repeated query should use cache, not hit server again"
    );

    // Third search with a different query -- should hit the server.
    let _results = catalog.search("empty").await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "Different query should hit server"
    );
}

/// Full pipeline: search -> build download URL -> fetch content -> install into registry.
#[tokio::test]
async fn test_install_skill_from_mock_catalog() {
    let state = MockState {
        search_hit_count: Arc::new(AtomicUsize::new(0)),
    };
    let base_url = start_mock_server(state).await;

    // 1. Search
    let catalog = SkillCatalog::with_url(&base_url);
    let results = catalog.search("deploy").await;
    assert!(!results.is_empty());
    let slug = &results[0].slug;

    // 2. Build download URL
    let download_url = skill_download_url(&base_url, slug);

    // 3. Fetch SKILL.md content
    let client = reqwest::Client::new();
    let resp = client
        .get(&download_url)
        .send()
        .await
        .expect("Failed to fetch skill content");
    assert!(resp.status().is_success());
    let content = resp.text().await.expect("Failed to read response body");

    // 4. Install into a fresh registry
    let tmp_dir = tempfile::tempdir().expect("Failed to create tempdir");
    let mut registry = SkillRegistry::new(tmp_dir.path().to_path_buf());
    let name = registry
        .install_skill(&content)
        .await
        .expect("install_skill should succeed");

    // 5. Assertions
    assert_eq!(name, "deploy-helper");
    assert!(registry.has("deploy-helper"));

    let skill = registry
        .find_by_name("deploy-helper")
        .expect("Skill should be findable by name");
    assert_eq!(skill.trust, SkillTrust::Installed);
    assert!(
        skill.prompt_content.contains("deployment automation assistant"),
        "Prompt content should contain expected text"
    );
    assert!(
        skill
            .manifest
            .activation
            .keywords
            .contains(&"deploy".to_string()),
        "Keywords should include 'deploy'"
    );
    assert!(
        skill
            .manifest
            .activation
            .keywords
            .contains(&"release".to_string()),
        "Keywords should include 'release'"
    );
}

/// Installing invalid SKILL.md (no frontmatter) should return an error.
#[tokio::test]
async fn test_install_bad_skill_from_catalog_fails() {
    let state = MockState {
        search_hit_count: Arc::new(AtomicUsize::new(0)),
    };
    let base_url = start_mock_server(state).await;

    // Build download URL for a slug that returns invalid content.
    let download_url = skill_download_url(&base_url, "owner/bad-skill");

    let client = reqwest::Client::new();
    let resp = client
        .get(&download_url)
        .send()
        .await
        .expect("Failed to fetch skill content");
    let content = resp.text().await.expect("Failed to read response body");

    let tmp_dir = tempfile::tempdir().expect("Failed to create tempdir");
    let mut registry = SkillRegistry::new(tmp_dir.path().to_path_buf());
    let result = registry.install_skill(&content).await;

    assert!(
        result.is_err(),
        "Installing invalid SKILL.md should fail, but got: {:?}",
        result
    );
}

/// Verify that `skill_download_url` encodes slashes in slugs and that the
/// encoded URL works against the mock server.
#[tokio::test]
async fn test_download_url_encodes_slug() {
    // Verify URL encoding of the slug.
    let url = skill_download_url("https://clawhub.ai", "owner/my-skill");
    assert!(
        url.contains("owner%2Fmy-skill"),
        "Slug should be URL-encoded: {}",
        url
    );

    // Verify the encoded URL works against the mock server (the download handler
    // receives the decoded slug via axum's Query extractor).
    let state = MockState {
        search_hit_count: Arc::new(AtomicUsize::new(0)),
    };
    let base_url = start_mock_server(state).await;

    let download_url = skill_download_url(&base_url, "owner/my-skill");
    let client = reqwest::Client::new();
    let resp = client
        .get(&download_url)
        .send()
        .await
        .expect("Failed to fetch via encoded URL");
    assert!(
        resp.status().is_success(),
        "Encoded slug URL should work: status {}",
        resp.status()
    );

    let body = resp.text().await.expect("Failed to read body");
    assert!(
        body.contains("deploy-helper"),
        "Should return valid SKILL.md content for non-bad slugs"
    );
}
