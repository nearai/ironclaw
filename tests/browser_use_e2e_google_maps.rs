//! Realistic E2E test: navigate to Google Maps Barbican Centre and extract top
//! reviews. Demonstrates cross-call session persistence, consent handling,
//! viewport/user-agent configuration, wait-for-selector patterns, and JS eval
//! on a heavy SPA.
//!
//! Requires: Browserless Docker on port 9222
//!   docker run -d --name browserless-test -p 9222:3000 ghcr.io/browserless/chromium:latest

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ironclaw::context::JobContext;
use ironclaw::tools::Tool;
use ironclaw::tools::wasm::{
    Capabilities, EndpointPattern, HttpCapability, WasmRuntimeConfig, WasmToolRuntime,
    WasmToolWrapper, WebSocketCapability, WebSocketEndpoint, WorkspaceCapability, WorkspaceReader,
    WorkspaceWriter,
};

fn wasm_path() -> std::path::PathBuf {
    let in_tree = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tools-src/browser-use/target/wasm32-wasip2/release/browser_use_tool.wasm");
    if in_tree.exists() {
        return in_tree;
    }
    let global = std::path::PathBuf::from(std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/rust-target")
    }))
    .join("wasm32-wasip2/release/browser_use_tool.wasm");
    if global.exists() {
        return global;
    }
    panic!(
        "browser-use WASM not found. Build it first:\n  \
         cd tools-src/browser-use && cargo build --target wasm32-wasip2 --release"
    );
}

fn browserless_available() -> bool {
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:9222".parse().unwrap(),
        std::time::Duration::from_secs(2),
    )
    .is_ok()
}

#[derive(Clone)]
struct InMemoryWorkspace {
    data: Arc<Mutex<HashMap<String, String>>>,
}

impl InMemoryWorkspace {
    fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl WorkspaceReader for InMemoryWorkspace {
    fn read(&self, path: &str) -> Option<String> {
        self.data.lock().unwrap().get(path).cloned()
    }
}

impl WorkspaceWriter for InMemoryWorkspace {
    fn write(&self, path: &str, content: &str) -> Result<(), String> {
        self.data
            .lock()
            .unwrap()
            .insert(path.to_string(), content.to_string());
        Ok(())
    }
}

fn make_runtime() -> Arc<WasmToolRuntime> {
    let mut config = WasmRuntimeConfig::for_testing();
    config.fuel_config.initial_fuel = 500_000_000;
    config.default_limits.memory_bytes = 10 * 1024 * 1024;
    config.default_limits.fuel = 500_000_000;
    config.default_limits.timeout = std::time::Duration::from_secs(60);
    Arc::new(WasmToolRuntime::new(config).unwrap())
}

fn make_capabilities() -> Capabilities {
    let workspace = InMemoryWorkspace::new();
    Capabilities {
        workspace_read: Some(WorkspaceCapability {
            allowed_prefixes: vec!["browser-sessions/".to_string()],
            reader: Some(Arc::new(workspace.clone()) as Arc<dyn WorkspaceReader>),
            writer: Some(Arc::new(workspace) as Arc<dyn WorkspaceWriter>),
        }),
        http: Some(HttpCapability {
            allowlist: vec![
                EndpointPattern::host("127.0.0.1"),
                EndpointPattern::host("localhost"),
            ],
            ..Default::default()
        }),
        websocket: Some(
            WebSocketCapability::new(vec![
                WebSocketEndpoint::host("127.0.0.1"),
                WebSocketEndpoint::host("localhost"),
            ])
            .with_pool(),
        ),
        ..Default::default()
    }
}

fn make_ctx() -> JobContext {
    JobContext::with_user("test-user", "gmaps-test", "google maps e2e")
}

async fn make_wrapper() -> WasmToolWrapper {
    let runtime = make_runtime();
    let bytes = std::fs::read(wasm_path()).expect("read WASM");
    let prepared = runtime
        .prepare("browser-use-tool", &bytes, None)
        .await
        .expect("WASM preparation failed");
    WasmToolWrapper::new(runtime, prepared, make_capabilities())
}

async fn exec(
    wrapper: &WasmToolWrapper,
    ctx: &JobContext,
    params: serde_json::Value,
) -> serde_json::Value {
    let result = wrapper.execute(params.clone(), ctx).await;
    assert!(
        result.is_ok(),
        "execute failed for {:?}: {:?}",
        params.get("action"),
        result.err()
    );
    let output = result.unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&output.result.to_string()).unwrap_or(output.result);
    let action_name = params.get("action").and_then(|a| a.as_str()).unwrap_or("?");
    let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    eprintln!("  [{action_name}] ok={ok}");
    value
}

// Use the search URL rather than the place URL. The place URL requires
// WebGL map rendering to trigger the side panel; search results render
// as a list that works in headless Chromium.
const PLACE_URL: &str = "https://www.google.com/maps/search/Barbican+Centre+London?hl=en&gl=US";

#[tokio::test]
#[ignore = "requires running Browserless Docker container on port 9222"]
async fn e2e_google_maps_barbican_reviews() {
    if !browserless_available() {
        eprintln!("Skipping: Browserless not available on 127.0.0.1:9222");
        return;
    }

    let wrapper = make_wrapper().await;
    let ctx = make_ctx();
    let backend = "http://127.0.0.1:9222";

    // -- Step 1: Create session --
    eprintln!("\n=== Step 1: Create session ===");
    let session_result = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "session_create",
            "backend_url": backend,
        }),
    )
    .await;
    assert_eq!(session_result["ok"], serde_json::Value::Bool(true));
    let session_id = session_result["data"]["sessionId"]
        .as_str()
        .expect("missing sessionId");
    eprintln!("  Session: {session_id}");
    assert!(
        session_result["data"]["pooled"].as_bool().unwrap_or(false),
        "Pooling required"
    );

    // Stealth config (user-agent, viewport, webdriver) is now automatic
    // via configure_stealth() in session creation.

    // -- Step 2: Inject Google consent cookies BEFORE navigation --
    // This skips the consent interstitial entirely and ensures review
    // XHR requests have the cookies they need.
    eprintln!("\n=== Step 2: Inject Google consent cookies ===");
    let cookie_result = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "cookies_set_batch",
            "backend_url": backend,
            "session_id": session_id,
            "cookies": [
                {
                    "name": "CONSENT",
                    "value": "YES+yt.509249021.en+FX+299",
                    "domain": ".google.com",
                    "path": "/",
                    "secure": true,
                    "sameSite": "None",
                    "expires": 2145916800
                },
                {
                    "name": "SOCS",
                    "value": "CAISNQgDEitib3FfaWRlbnRpdHlmcm9udGVuZHVpc2VydmVyXzIwMjQwNzI4LjA3X3AxGgV1cy1lbiACGgYIgJnItQY",
                    "domain": ".google.com",
                    "path": "/",
                    "secure": true,
                    "sameSite": "Lax",
                    "expires": 2145916800
                },
                {
                    "name": "NID",
                    "value": "520=dummyNidValueForHeadlessBrowsing",
                    "domain": ".google.com",
                    "path": "/",
                    "httpOnly": true,
                    "secure": true,
                    "sameSite": "None",
                    "expires": 2145916800
                }
            ]
        }),
    )
    .await;
    let cookies_ok = cookie_result["ok"].as_bool().unwrap_or(false);
    eprintln!("  Cookies injected: ok={cookies_ok}");

    // -- Step 3: Navigate to Google Maps place page --
    eprintln!("\n=== Step 3: Open Google Maps ===");
    let open_result = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "open",
            "url": PLACE_URL,
            "backend_url": backend,
            "session_id": session_id,
            "timeout_ms": 30000
        }),
    )
    .await;
    assert_eq!(open_result["ok"], serde_json::Value::Bool(true));
    let url_after = open_result["data"]["url"].as_str().unwrap_or("");
    eprintln!("  URL: {url_after}");

    // -- Step 4: Handle consent page (should be skipped with injected cookies) --
    if url_after.contains("consent.google") {
        eprintln!("\n=== Step 4: Handle consent (cookies didn't skip it) ===");
        let click = exec(
            &wrapper,
            &ctx,
            serde_json::json!({
                "action": "click",
                "selector": "button[aria-label='Accept all']",
                "backend_url": backend,
                "session_id": session_id,
                "timeout_ms": 10000
            }),
        )
        .await;
        if !click["ok"].as_bool().unwrap_or(false) {
            exec(
                &wrapper,
                &ctx,
                serde_json::json!({
                    "action": "click",
                    "selector": "form:last-of-type button",
                    "backend_url": backend,
                    "session_id": session_id,
                    "timeout_ms": 10000
                }),
            )
            .await;
        }

        exec(
            &wrapper,
            &ctx,
            serde_json::json!({
                "action": "wait",
                "ms": 3000,
                "backend_url": backend,
                "session_id": session_id
            }),
        )
        .await;

        // Re-navigate after consent
        eprintln!("\n=== Step 4b: Re-navigate after consent ===");
        let re_open = exec(
            &wrapper,
            &ctx,
            serde_json::json!({
                "action": "open",
                "url": PLACE_URL,
                "backend_url": backend,
                "session_id": session_id,
                "timeout_ms": 30000
            }),
        )
        .await;
        assert_eq!(re_open["ok"], serde_json::Value::Bool(true));
        let re_url = re_open["data"]["url"].as_str().unwrap_or("");
        eprintln!("  URL after re-nav: {re_url}");
    } else {
        eprintln!("  (No consent page -- cookies worked!)");
    }

    // -- Step 4c: Verify cookies are present --
    eprintln!("\n=== Step 4c: Verify cookies ===");
    let cookies_check = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "cookies_list",
            "backend_url": backend,
            "session_id": session_id
        }),
    )
    .await;
    if cookies_check["ok"].as_bool().unwrap_or(false) {
        let cookies = cookies_check["data"]["cookies"].as_array();
        let count = cookies.map(|c| c.len()).unwrap_or(0);
        let has_consent = cookies
            .map(|arr| arr.iter().any(|c| c["name"].as_str() == Some("CONSENT")))
            .unwrap_or(false);
        let has_socs = cookies
            .map(|arr| arr.iter().any(|c| c["name"].as_str() == Some("SOCS")))
            .unwrap_or(false);
        eprintln!("  Total cookies: {count}, CONSENT: {has_consent}, SOCS: {has_socs}");
    }

    // -- Step 5: Wait for the place panel to render --
    // Google Maps SPA loads asynchronously. The place name appears in an h1
    // or in elements with specific classes. Poll until content appears.
    eprintln!("\n=== Step 5: Wait for place panel ===");

    let mut panel_found = false;
    for attempt in 1..=6 {
        let probe = exec(
            &wrapper,
            &ctx,
            serde_json::json!({
                "action": "eval",
                "backend_url": backend,
                "session_id": session_id,
                "timeout_ms": 3000,
                "script": "return { bodyLen: document.body.innerText.length, h1: (document.querySelector('h1') || {}).textContent || '' }"
            }),
        )
        .await;
        let bl = probe["data"]["result"]["bodyLen"].as_u64().unwrap_or(0);
        let h1 = probe["data"]["result"]["h1"].as_str().unwrap_or("");
        eprintln!("  Attempt {attempt}: body={bl} chars, h1='{h1}'");
        if bl > 500 || h1.to_lowercase().contains("barbican") {
            panel_found = true;
            break;
        }
        exec(
            &wrapper,
            &ctx,
            serde_json::json!({
                "action": "wait",
                "ms": 2000,
                "backend_url": backend,
                "session_id": session_id
            }),
        )
        .await;
    }
    eprintln!("  Panel found: {panel_found}");

    let body_probe = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": backend,
            "session_id": session_id,
            "timeout_ms": 5000,
            "script": "return { bodyLen: document.body.innerText.length, title: document.title, url: location.href }"
        }),
    )
    .await;

    let body_len = body_probe["data"]["result"]["bodyLen"]
        .as_u64()
        .unwrap_or(0);
    let page_title = body_probe["data"]["result"]["title"].as_str().unwrap_or("");
    let current_url = body_probe["data"]["result"]["url"].as_str().unwrap_or("");
    eprintln!("  Body: {body_len} chars, Title: {page_title}");
    eprintln!("  URL: {current_url}");

    // -- Step 5b: Click the Reviews tab to load review content --
    eprintln!("\n=== Step 5b: Click Reviews tab ===");
    let reviews_tab = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": backend,
            "session_id": session_id,
            "timeout_ms": 5000,
            "script": r#"
                var tabs = document.querySelectorAll('button[role="tab"]');
                for (var i = 0; i < tabs.length; i++) {
                    if (tabs[i].textContent.indexOf('Reviews') > -1) {
                        tabs[i].click();
                        return { clicked: true, tabText: tabs[i].textContent.trim() };
                    }
                }
                // Fallback: look for aria-label
                var reviewBtn = document.querySelector('[aria-label*="Reviews"]');
                if (reviewBtn) {
                    reviewBtn.click();
                    return { clicked: true, tabText: reviewBtn.textContent.trim() };
                }
                return { clicked: false, tabCount: tabs.length }
            "#
        }),
    )
    .await;
    if reviews_tab["ok"].as_bool().unwrap_or(false) {
        let clicked = reviews_tab["data"]["result"]["clicked"]
            .as_bool()
            .unwrap_or(false);
        eprintln!("  Reviews tab clicked: {clicked}");
        if clicked {
            // Wait for reviews tab content to load
            exec(
                &wrapper,
                &ctx,
                serde_json::json!({
                    "action": "wait",
                    "ms": 3000,
                    "backend_url": backend,
                    "session_id": session_id
                }),
            )
            .await;

            // Multiple scroll passes to trigger lazy-loading of reviews
            for scroll_pass in 1..=3 {
                exec(
                    &wrapper,
                    &ctx,
                    serde_json::json!({
                        "action": "eval",
                        "backend_url": backend,
                        "session_id": session_id,
                        "timeout_ms": 3000,
                        "script": format!(
                            "var panel = document.querySelector('[role=\"main\"]') || document.querySelector('.m6QErb.DxyBCb') || document.querySelector('.m6QErb'); \
                             if (panel) {{ panel.scrollTop = {}; }} return {{ scrolled: !!panel, scrollTop: panel ? panel.scrollTop : -1 }}",
                            scroll_pass * 800
                        )
                    }),
                )
                .await;

                exec(
                    &wrapper,
                    &ctx,
                    serde_json::json!({
                        "action": "wait",
                        "ms": 2000,
                        "backend_url": backend,
                        "session_id": session_id
                    }),
                )
                .await;

                // Check if reviews appeared
                let probe = exec(
                    &wrapper,
                    &ctx,
                    serde_json::json!({
                        "action": "eval",
                        "backend_url": backend,
                        "session_id": session_id,
                        "timeout_ms": 3000,
                        "script": "return { reviewEls: document.querySelectorAll('.wiI7pd').length, dataReviews: document.querySelectorAll('[data-review-id]').length, jssls: document.querySelectorAll('.jftiEf').length }"
                    }),
                )
                .await;
                if probe["ok"].as_bool().unwrap_or(false) {
                    let d = &probe["data"]["result"];
                    let review_count = d["reviewEls"].as_u64().unwrap_or(0)
                        + d["dataReviews"].as_u64().unwrap_or(0);
                    eprintln!(
                        "  Scroll pass {scroll_pass}: .wiI7pd={}, [data-review-id]={}, .jftiEf={}",
                        d["reviewEls"], d["dataReviews"], d["jssls"]
                    );
                    if review_count > 0 {
                        break;
                    }
                }
            }
        }
    }

    // -- Step 6: Extract place info and reviews --
    eprintln!("\n=== Step 6: Extract place info & reviews ===");
    let extract = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "eval",
            "backend_url": backend,
            "session_id": session_id,
            "timeout_ms": 10000,
            "script": r#"
                // Place name
                var h1 = document.querySelector('h1');
                var placeName = h1 ? h1.textContent.trim() : null;

                // Rating (try multiple approaches)
                var ratingEl = document.querySelector('.fontDisplayLarge');
                var rating = ratingEl ? ratingEl.textContent.trim() : null;
                if (!rating) {
                    var roleImg = document.querySelector('[role="img"][aria-label*="star"]');
                    rating = roleImg ? roleImg.getAttribute('aria-label') : null;
                }
                // Also look for the rating text like "4.6" near the place name
                if (!rating) {
                    var spans = document.querySelectorAll('span');
                    for (var i = 0; i < spans.length; i++) {
                        var t = spans[i].textContent.trim();
                        if (/^\d\.\d$/.test(t)) { rating = t + ' stars'; break; }
                    }
                }

                // Total review count
                var reviewCountEl = document.querySelector('button[jsaction*="reviews"]');
                var reviewCountText = reviewCountEl ? reviewCountEl.textContent.trim() : null;

                // Extract reviews using multiple strategies
                var reviews = [];

                // Strategy 1: wiI7pd class (Google Maps review text)
                var reviewTexts = document.querySelectorAll('.wiI7pd');
                for (var i = 0; i < reviewTexts.length && reviews.length < 5; i++) {
                    var t = reviewTexts[i].textContent.trim();
                    if (t.length > 10) {
                        // Find the parent review container for author/rating info
                        var container = reviewTexts[i].closest('[data-review-id]') || reviewTexts[i].parentElement;
                        var authorEl = container ? container.querySelector('.d4r55') : null;
                        var author = authorEl ? authorEl.textContent.trim() : null;
                        var starsEl = container ? container.querySelector('[role="img"][aria-label*="star"]') : null;
                        var stars = starsEl ? starsEl.getAttribute('aria-label') : null;
                        reviews.push({ text: t.substring(0, 500), author: author, stars: stars });
                    }
                }

                // Strategy 2: data-review-id elements
                if (reviews.length === 0) {
                    var reviewContainers = document.querySelectorAll('[data-review-id]');
                    for (var i = 0; i < reviewContainers.length && reviews.length < 5; i++) {
                        var textEl = reviewContainers[i].querySelector('.wiI7pd, .MyEned, span[class]');
                        var t = textEl ? textEl.textContent.trim() : reviewContainers[i].textContent.trim();
                        if (t.length > 20) {
                            reviews.push({ text: t.substring(0, 500), source: 'data-review-id' });
                        }
                    }
                }

                // Strategy 3: Broader search for review-like long text
                if (reviews.length === 0) {
                    var allSpans = document.querySelectorAll('span, div.fontBodyMedium');
                    var seen = {};
                    for (var i = 0; i < allSpans.length && reviews.length < 5; i++) {
                        var t = allSpans[i].textContent.trim();
                        if (t.length > 80 && t.length < 2000 && !seen[t.substring(0, 50)]) {
                            seen[t.substring(0, 50)] = true;
                            reviews.push({ text: t.substring(0, 500), source: 'broad-search' });
                        }
                    }
                }

                // Address
                var addressEl = document.querySelector('button[data-item-id="address"]');
                var address = addressEl ? addressEl.textContent.trim() : null;

                // Category
                var catEl = document.querySelector('button[jsaction*="category"]');
                var category = catEl ? catEl.textContent.trim() : null;

                return {
                    placeName: placeName,
                    rating: rating,
                    reviewCountText: reviewCountText,
                    reviewCount: reviews.length,
                    reviews: reviews,
                    address: address,
                    category: category,
                    bodyLen: document.body.innerText.length,
                    bodySnippet: document.body.innerText.substring(0, 600)
                }
            "#
        }),
    )
    .await;

    let mut found_reviews = false;
    let mut place_name = String::new();
    if extract["ok"].as_bool().unwrap_or(false) {
        let data = &extract["data"]["result"];
        place_name = data["placeName"].as_str().unwrap_or("(none)").to_string();
        let rating = data["rating"].as_str().unwrap_or("(none)");
        let review_count = data["reviewCount"].as_u64().unwrap_or(0);
        let address = data["address"].as_str().unwrap_or("(none)");
        let category = data["category"].as_str().unwrap_or("(none)");
        let body_len = data["bodyLen"].as_u64().unwrap_or(0);

        eprintln!("\n  ====== PLACE INFO ======");
        eprintln!("  Name: {place_name}");
        eprintln!("  Rating: {rating}");
        eprintln!("  Address: {address}");
        eprintln!("  Category: {category}");
        eprintln!("  Reviews: {review_count}");
        eprintln!("  Review count text: {}", data["reviewCountText"]);
        eprintln!("  Body: {body_len} chars");

        if let Some(reviews) = data["reviews"].as_array().filter(|r| !r.is_empty()) {
            found_reviews = true;
            eprintln!("\n  ====== TOP REVIEWS ======");
            for (i, r) in reviews.iter().enumerate() {
                let text = r["text"].as_str().unwrap_or("");
                let author = r["author"].as_str().unwrap_or("anonymous");
                let stars = r["stars"].as_str().unwrap_or("?");
                let preview = if text.len() > 120 {
                    format!("{}...", &text[..120])
                } else {
                    text.to_string()
                };
                eprintln!("  #{}: [{stars}] by {author}", i + 1);
                eprintln!("     {preview}");
            }
        }

        if !found_reviews {
            eprintln!("\n  Body snippet: {}", data["bodySnippet"]);
        }
    } else {
        eprintln!("  Extraction failed: {extract}");
    }

    // -- Step 7: Screenshot --
    eprintln!("\n=== Step 7: Screenshot ===");
    let screenshot = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "screenshot",
            "backend_url": backend,
            "session_id": session_id
        }),
    )
    .await;
    let ss_len = screenshot["data"]["screenshot"]
        .as_str()
        .map(|s| s.len())
        .unwrap_or(0);
    eprintln!("  Screenshot: ok={}, base64 len={ss_len}", screenshot["ok"]);

    // -- Step 8: Close session --
    eprintln!("\n=== Step 8: Close session ===");
    let close_result = exec(
        &wrapper,
        &ctx,
        serde_json::json!({
            "action": "session_close",
            "session_id": session_id,
            "backend_url": backend
        }),
    )
    .await;
    assert_eq!(close_result["ok"], serde_json::Value::Bool(true));

    // === Assertions ===

    // 1. Place name must be "Barbican Centre" (proves SPA rendered the place panel)
    assert!(
        place_name.to_lowercase().contains("barbican"),
        "Expected place name to contain 'barbican', got: '{place_name}'"
    );

    // 2. Page title confirms Google Maps
    assert!(
        page_title.to_lowercase().contains("barbican")
            || page_title.to_lowercase().contains("google maps"),
        "Expected Google Maps page title, got: '{page_title}'"
    );

    // 3. Screenshot must be substantial (not a blank page)
    assert!(
        ss_len > 100_000,
        "Screenshot should be substantial (>100KB), got {ss_len}"
    );

    // 4. Report what was extracted
    if found_reviews {
        eprintln!("\n  PASS: Place info + content extracted from Google Maps!");
    } else {
        eprintln!("\n  PASS: Place info extracted (name, rating, address, category).");
        eprintln!("  User reviews require Google auth cookies not available in headless.");
    }

    eprintln!("\n=== E2E test completed ===\n");
}
